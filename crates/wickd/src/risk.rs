//! Position-risk guardrails (AGT-595).
//!
//! Hard caps enforced on the **live** execution path *before* an order is
//! submitted to OANDA. Three independent guards:
//!
//!   1. **max position size** — reject orders whose `|units|` exceed a cap.
//!   2. **max open positions** — reject a NEW position once the open-position
//!      count is at/over a cap.
//!   3. **daily-loss kill-switch** — once today's realized P&L breaches a loss
//!      threshold, halt all live execution (both opens and closes).
//!
//! Caps load from a local config file `~/.wickd/risk.json`. A **missing config
//! means no caps are set**, i.e. no enforcement — a safe, explicit default so
//! upgrading doesn't silently start rejecting orders.
//!
//! The daily-loss kill-switch is self-contained: today's realized P&L is
//! tracked in a small local state file `~/.wickd/daily_pl.json`
//! (`{date, realized_pl}`). After each live fill we add the fill's realized P&L;
//! on a new UTC day the running total resets to zero. This module owns all of
//! that state I/O and never reaches a database — that is a different ticket.
//!
//! Enforcement is split so the decision logic is offline-testable:
//!   * [`enforce_pre_trade`] — a **pure** function over already-gathered inputs
//!     (caps, units, current open count, today's realized P&L). No network, no
//!     filesystem. This is what the unit tests exercise (one per cap).
//!   * [`enforce_live_place`] / [`enforce_live_close`] — thin async wrappers
//!     that gather the live inputs (open positions, persisted daily P&L) and
//!     delegate to the pure logic, mapping a rejection to a structured error.

use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use wickd_core::oanda::endpoints;
use wickd_core::oanda::OandaClient;

/// Configured risk caps. Every field is optional; `None` means "no cap on this
/// dimension" so an empty/missing config enforces nothing.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RiskCaps {
    /// Reject any order whose absolute units exceed this.
    pub max_position_size: Option<i64>,
    /// Reject a new position once this many positions are already open.
    pub max_open_positions: Option<usize>,
    /// Loss threshold (a positive magnitude). Once today's realized P&L is at or
    /// below `-daily_loss_limit`, the kill-switch trips and halts execution.
    pub daily_loss_limit: Option<Decimal>,
}

impl RiskCaps {
    /// True when no cap is configured — lets callers skip all I/O entirely.
    fn is_unset(&self) -> bool {
        self.max_position_size.is_none()
            && self.max_open_positions.is_none()
            && self.daily_loss_limit.is_none()
    }
}

/// A structured pre-trade rejection. Each variant carries the offending value
/// and the cap it breached so the error message is actionable. Every rendered
/// message contains the literal token `risk cap` so the `trade.rs` error
/// classifier routes it to `exit::VALIDATION`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskRejection {
    /// `|units|` exceeded `max_position_size`.
    PositionSize { units: i64, cap: i64 },
    /// Open-position count was at/over `max_open_positions`.
    MaxOpenPositions { current: usize, cap: usize },
    /// Today's realized P&L breached the daily-loss limit (kill-switch).
    DailyLossKillSwitch { realized: Decimal, limit: Decimal },
}

impl RiskRejection {
    /// Human/agent-readable message. Contains `risk cap` for error routing.
    pub fn message(&self) -> String {
        match self {
            RiskRejection::PositionSize { units, cap } => format!(
                "risk cap: order of {} units exceeds the max position size of {cap} \
                 — lower --units or raise `max_position_size` in ~/.wickd/risk.json",
                units.abs()
            ),
            RiskRejection::MaxOpenPositions { current, cap } => format!(
                "risk cap: {current} open position(s) at/over the max-open-positions limit of {cap} \
                 — close a position or raise `max_open_positions` in ~/.wickd/risk.json"
            ),
            RiskRejection::DailyLossKillSwitch { realized, limit } => format!(
                "risk cap: daily-loss kill-switch tripped — today's realized P&L {realized} breached \
                 the loss limit of -{limit}; live execution (opens AND closes) is halted \
                 — to resume, raise or remove `daily_loss_limit` in ~/.wickd/risk.json"
            ),
        }
    }
}

impl std::fmt::Display for RiskRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for RiskRejection {}

/// Pure pre-trade enforcement: given the caps and the gathered live inputs,
/// decide whether to allow the order. No I/O — directly unit-testable.
///
/// * `units` — the order's signed units (sign ignored; magnitude is capped).
/// * `current_open` — number of positions currently open (for a NEW position).
/// * `day_realized_pl` — today's running realized P&L (negative = a loss).
pub fn enforce_pre_trade(
    caps: &RiskCaps,
    units: i64,
    current_open: usize,
    day_realized_pl: Decimal,
) -> Result<(), RiskRejection> {
    if let Some(cap) = caps.max_position_size {
        if units.abs() > cap {
            return Err(RiskRejection::PositionSize { units, cap });
        }
    }
    if let Some(cap) = caps.max_open_positions {
        if current_open >= cap {
            return Err(RiskRejection::MaxOpenPositions { current: current_open, cap });
        }
    }
    check_kill_switch(caps, day_realized_pl)?;
    Ok(())
}

/// The kill-switch portion in isolation — used both by [`enforce_pre_trade`]
/// (opens) and the live-close path (closes are execution too, so a tripped
/// kill-switch halts them as well per AC3's "halt all live execution").
fn check_kill_switch(caps: &RiskCaps, day_realized_pl: Decimal) -> Result<(), RiskRejection> {
    if let Some(limit) = caps.daily_loss_limit {
        if day_realized_pl <= -limit {
            return Err(RiskRejection::DailyLossKillSwitch { realized: day_realized_pl, limit });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Config + daily-P&L state file I/O (self-contained, no DB).
// ---------------------------------------------------------------------------

/// Path to the risk-caps config (`~/.wickd/risk.json`).
pub fn risk_config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not resolve home directory")?;
    Ok(home.join(".wickd").join("risk.json"))
}

/// Path to the daily-P&L state file (`~/.wickd/daily_pl.json`).
fn daily_pl_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not resolve home directory")?;
    Ok(home.join(".wickd").join("daily_pl.json"))
}

/// Load configured caps, or the empty (no-enforcement) default if the file is
/// absent. A present-but-corrupt file is a hard error: failing closed on a
/// guardrail config is safer than silently disarming it.
pub fn load_caps() -> Result<RiskCaps> {
    let path = risk_config_path()?;
    if !path.exists() {
        return Ok(RiskCaps::default());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("reading risk config at {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("risk config at {} is not valid JSON", path.display()))
}

/// Persisted daily realized-P&L state.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DailyPl {
    /// UTC calendar day this total applies to (`YYYY-MM-DD`).
    date: String,
    /// Running realized P&L for `date`.
    realized_pl: Decimal,
}

/// Today's UTC date as `YYYY-MM-DD`.
fn today_utc() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

/// Today's running realized P&L. Returns zero if no state file exists or the
/// stored state is from a previous day (a new day resets the total).
pub fn load_today_realized_pl() -> Result<Decimal> {
    let path = daily_pl_path()?;
    if !path.exists() {
        return Ok(Decimal::ZERO);
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("reading daily P&L state at {}", path.display()))?;
    let state: DailyPl = serde_json::from_str(&raw)
        .with_context(|| format!("daily P&L state at {} is corrupt", path.display()))?;
    if state.date != today_utc() {
        Ok(Decimal::ZERO)
    } else {
        Ok(state.realized_pl)
    }
}

/// Add a realized-P&L delta (from a fill) to today's running total and persist
/// it, resetting first if the stored state is stale (a new day).
fn record_fill_pl(delta: Decimal) -> Result<()> {
    let path = daily_pl_path()?;
    let current = load_today_realized_pl()?; // already resets across day boundaries
    let updated = DailyPl { date: today_utc(), realized_pl: current + delta };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
    }
    let body = serde_json::to_string_pretty(&updated)?;
    std::fs::write(&path, body)
        .with_context(|| format!("writing daily P&L state at {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Best-effort: parse an OANDA realized-P&L string (e.g. a fill's `pl`) and fold
/// it into today's running total for the kill-switch. A parse/write failure is
/// logged-and-ignored — the order has already filled, so we must not fail the
/// command after the fact (worst case the kill-switch undercounts this fill).
pub fn record_fill(realized_pl_str: &str) {
    let delta = match Decimal::from_str(realized_pl_str.trim()) {
        Ok(d) => d,
        // Unexpected OANDA P&L format: warn (the kill-switch will undercount
        // this fill) rather than silently dropping it.
        Err(_) => {
            eprintln!(
                "warning: could not parse realized P&L '{realized_pl_str}' for kill-switch \
                 — this fill is not counted toward the daily-loss total"
            );
            return;
        }
    };
    if let Err(e) = record_fill_pl(delta) {
        eprintln!("warning: could not record realized P&L for kill-switch: {e:#}");
    }
}

// ---------------------------------------------------------------------------
// Live-path enforcement wrappers (gather inputs, delegate to pure logic).
// ---------------------------------------------------------------------------

/// Enforce all caps before a live **place** (a new/added position). Gathers the
/// open-position count and today's realized P&L, then delegates to the pure
/// [`enforce_pre_trade`]. A breached cap becomes a structured error whose
/// message routes to `exit::VALIDATION`. Skips all network I/O when no caps are
/// configured.
pub async fn enforce_live_place(oanda: &OandaClient, units: i64) -> Result<()> {
    let caps = load_caps()?;
    if caps.is_unset() {
        return Ok(());
    }
    // Only fetch positions if the open-count cap is actually set.
    let current_open = if caps.max_open_positions.is_some() {
        endpoints::get_positions(oanda)
            .await
            .context("OANDA positions fetch failed (risk cap pre-check)")?
            .len()
    } else {
        0
    };
    let day_pl = load_today_realized_pl()?;
    enforce_pre_trade(&caps, units, current_open, day_pl).map_err(|r| anyhow!(r.message()))
}

/// Enforce the kill-switch before a live **close**. Closes reduce exposure, so
/// the size and max-open caps do not apply — but a close is still live
/// execution, so a tripped daily-loss kill-switch halts it too (AC3). Note the
/// deliberate trade-off: once tripped you cannot use `wickd` to close either;
/// flip/raise the limit in `~/.wickd/risk.json` to resume.
pub async fn enforce_live_close(_oanda: &OandaClient) -> Result<()> {
    let caps = load_caps()?;
    if caps.daily_loss_limit.is_none() {
        return Ok(());
    }
    let day_pl = load_today_realized_pl()?;
    check_kill_switch(&caps, day_pl).map_err(|r| anyhow!(r.message()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn caps(size: Option<i64>, open: Option<usize>, loss: Option<Decimal>) -> RiskCaps {
        RiskCaps { max_position_size: size, max_open_positions: open, daily_loss_limit: loss }
    }

    #[test]
    fn ac1_rejects_oversize_units() {
        let c = caps(Some(500), None, None);
        // 1000 > 500 → rejected (long).
        let err = enforce_pre_trade(&c, 1000, 0, Decimal::ZERO).unwrap_err();
        assert_eq!(err, RiskRejection::PositionSize { units: 1000, cap: 500 });
        // Magnitude is what matters: a -1000 short is rejected too.
        let err = enforce_pre_trade(&c, -1000, 0, Decimal::ZERO).unwrap_err();
        assert!(matches!(err, RiskRejection::PositionSize { .. }));
        assert!(err.message().contains("risk cap"));
        // Exactly at the cap is allowed (strict >).
        assert!(enforce_pre_trade(&c, 500, 0, Decimal::ZERO).is_ok());
        assert!(enforce_pre_trade(&c, -500, 0, Decimal::ZERO).is_ok());
    }

    #[test]
    fn ac2_rejects_beyond_max_open_positions() {
        let c = caps(None, Some(3), None);
        // 3 already open, cap 3 → a NEW position is rejected (>=).
        let err = enforce_pre_trade(&c, 100, 3, Decimal::ZERO).unwrap_err();
        assert_eq!(err, RiskRejection::MaxOpenPositions { current: 3, cap: 3 });
        assert!(err.message().contains("risk cap"));
        // 4 open is also rejected.
        assert!(enforce_pre_trade(&c, 100, 4, Decimal::ZERO).is_err());
        // Under the cap is allowed.
        assert!(enforce_pre_trade(&c, 100, 2, Decimal::ZERO).is_ok());
    }

    #[test]
    fn ac3_kill_switch_halts_on_daily_loss_breach() {
        let c = caps(None, None, Some(dec!(100)));
        // Down 150 (<= -100) → kill-switch trips.
        let err = enforce_pre_trade(&c, 100, 0, dec!(-150)).unwrap_err();
        assert_eq!(
            err,
            RiskRejection::DailyLossKillSwitch { realized: dec!(-150), limit: dec!(100) }
        );
        assert!(err.message().contains("kill-switch"));
        assert!(err.message().contains("risk cap"));
        // Exactly at the limit also trips (<=).
        assert!(enforce_pre_trade(&c, 100, 0, dec!(-100)).is_err());
        // A smaller loss, or a profit, does not.
        assert!(enforce_pre_trade(&c, 100, 0, dec!(-99.99)).is_ok());
        assert!(enforce_pre_trade(&c, 100, 0, dec!(250)).is_ok());
    }

    #[test]
    fn allows_within_all_limits() {
        let c = caps(Some(1000), Some(5), Some(dec!(500)));
        assert!(enforce_pre_trade(&c, 800, 2, dec!(-100)).is_ok());
    }

    #[test]
    fn unset_caps_enforce_nothing() {
        let c = RiskCaps::default();
        // Wildly oversize, many open, deep loss — all allowed when unset.
        assert!(enforce_pre_trade(&c, 10_000_000, 999, dec!(-1_000_000)).is_ok());
        assert!(c.is_unset());
    }

    #[test]
    fn kill_switch_only_check_ignores_size_and_open() {
        let c = caps(Some(10), Some(1), Some(dec!(100)));
        // check_kill_switch ignores size/open caps — only the loss matters.
        assert!(check_kill_switch(&c, dec!(-50)).is_ok());
        assert!(check_kill_switch(&c, dec!(-100)).is_err());
    }
}
