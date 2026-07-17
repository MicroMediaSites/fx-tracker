//! Durable alert queue — the agent-pollable delivery store (AGT-620).
//!
//! `wickd` fires two kinds of alert on two separate long-running commands:
//! price-level crossings (`wickd alert run`, via `wickd alert run`'s sink)
//! and strategy-signal Buy/Sell alerts (`wickd watch`, via
//! `wickd watch`'s signal-alert sink). Neither of those is durable on its own:
//! a fire is an NDJSON line on stdout that scrolls away. An agent that wants to
//! *react* to alerts needs a store it can poll/tail across invocations.
//!
//! This module is that store: an **append-only NDJSON log** at
//! `~/.wickd/alert-queue.ndjson`. Append-only is the natural shape for a
//! poll/tail feed — new events land at the end, and `wickd queue list
//! [--follow]` reads them back (AC2). Each line is one [`QueuedAlert`].
//!
//! ## D3 — alerts and execution-proposals never share a store (AC1)
//!
//! This file is deliberately **separate** from `~/.wickd/pending.json` (the
//! execution-proposal store owned by [`crate::pending`]). An alert landing in
//! this queue is *not* an execution proposal and never auto-becomes one. The
//! only bridge from here to `pending.json` is the explicit `wickd queue
//! promote <id>` action (AC3, see `wickd queue`) — and only for
//! strategy-signal (Buy/Sell) alerts, which carry an order intent. Price-level
//! alerts are not promotable: a "EUR_USD crossed 1.0900" event says nothing
//! about a side or size, so it has no proposal to promote into.
//!
//! ## Schema (`~/.wickd/alert-queue.ndjson`)
//!
//! One JSON object per line, e.g.:
//!
//! ```jsonc
//! {"id":"<queue-uuid>","ts":"2026-06-30T00:00:00+00:00",
//!  "payload":{"kind":"strategy-signal","instrument":"EUR_USD","signal":"buy",
//!             "proposal":{ /* a full pending::PendingSignal */ }}}
//! {"id":"<queue-uuid>","ts":"2026-06-30T00:00:05Z",
//!  "payload":{"kind":"price-level","instrument":"EUR_USD","level":"1.0900",
//!             "direction":"cross-up","price":"1.0905"}}
//! ```

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use std::str::FromStr;

use anyhow::anyhow;

use crate::pending::PendingSignal;
use crate::shared::PositionDirection;

/// Which way a price-level cross must go to fire an alert. Lives here (the
/// queue wire format) since AGT-652 moved the daemon's client-visible contract
/// into wickd-core; the CLI's `alert` module re-exports it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    /// Fires when price crosses from below the level to at/above it.
    CrossUp,
    /// Fires when price crosses from above the level to at/below it.
    CrossDown,
    /// Fires on a cross in either direction.
    Either,
}

impl FromStr for Direction {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "cross-up" | "crossup" | "up" => Ok(Direction::CrossUp),
            "cross-down" | "crossdown" | "down" => Ok(Direction::CrossDown),
            "either" | "both" => Ok(Direction::Either),
            other => Err(anyhow!(
                "unknown direction '{other}' (expected cross-up, cross-down, or either)"
            )),
        }
    }
}

/// The actionable half of a strategy's per-candle evaluation (Buy/Sell).
/// Serializes `buy`/`sell` on the queue wire; the CLI's `signal_alert` module
/// re-exports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertSignal {
    Buy,
    Sell,
}

impl AlertSignal {
    /// Classify a position direction as its alert signal.
    pub fn from_direction(direction: PositionDirection) -> Self {
        match direction {
            PositionDirection::Long => AlertSignal::Buy,
            PositionDirection::Short => AlertSignal::Sell,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            AlertSignal::Buy => "buy",
            AlertSignal::Sell => "sell",
        }
    }
}

/// Queue file name under `~/.wickd/`.
pub const QUEUE_FILE: &str = "alert-queue.ndjson";

/// The kind-tagged payload of a queued alert.
///
/// Internally tagged on `kind` so a reader can branch on the alert type without
/// positional guessing, and so [`QueuedAlert::promotable_proposal`] can hand
/// back the embedded proposal for exactly the one promotable variant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum QueuedPayload {
    /// A price-level alert fired (`wickd alert run`). **Not promotable** — a
    /// bare level crossing carries no side/size, so there is no order intent to
    /// bridge into a pending proposal.
    PriceLevel {
        instrument: String,
        /// The level that was crossed (OANDA precision, as a string).
        level: String,
        /// The typed cross direction (serializes `cross-up`/`cross-down`/`either`).
        direction: Direction,
        /// The price that triggered the fire.
        price: String,
    },
    /// A strategy-signal Buy/Sell alert (`wickd watch`). **Promotable**: it
    /// carries the fully-formed [`PendingSignal`] the fire maps to, so `wickd
    /// queue promote <id>` can append that proposal into `pending.json`.
    StrategySignal {
        instrument: String,
        /// The typed Buy/Sell signal (serializes `buy`/`sell`).
        signal: AlertSignal,
        /// The execution proposal this alert promotes into. Built at fire time
        /// via [`crate::pending::pending_from_match`] so a promotion is a pure
        /// move of an already-well-formed record, never a re-derivation.
        /// Boxed: the proposal dwarfs the price-level variant
        /// (clippy::large_enum_variant); serde is transparent to the Box.
        proposal: Box<PendingSignal>,
        /// The watcher's `--account` (issue #8). Distinguishes otherwise
        /// identical rows when several watchers run the same strategy/pair on
        /// different accounts. `Option` + `default` so rows queued before this
        /// field existed still parse; `skip_serializing_if` keeps new rows
        /// readable by pre-field consumers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account: Option<String>,
        /// The watcher's candle granularity, e.g. `M5` (issue #8). Same
        /// backward-compatibility contract as `account`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        granularity: Option<String>,
    },
}

/// One entry in the alert queue: a stable id for reference/promotion, the fire
/// timestamp, and the kind-tagged payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueuedAlert {
    /// Stable queue-entry id (`wickd queue promote <id>` names this). A fresh
    /// uuid per entry, distinct from any id inside the payload — a re-fire is a
    /// genuinely new queue event even when it repeats a level or a signal.
    pub id: String,
    /// RFC3339 timestamp of the fire (the source event's own time).
    pub ts: String,
    pub payload: QueuedPayload,
}

impl QueuedAlert {
    fn new_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Build a queued price-level alert (not promotable).
    pub fn price_level(
        ts: String,
        instrument: String,
        level: String,
        direction: Direction,
        price: String,
    ) -> Self {
        Self {
            id: Self::new_id(),
            ts,
            payload: QueuedPayload::PriceLevel { instrument, level, direction, price },
        }
    }

    /// Build a queued strategy-signal alert from the fire's [`PendingSignal`]
    /// proposal (promotable). `signal` is the typed Buy/Sell classification.
    /// `account` / `granularity` identify WHICH watcher fired (issue #8) —
    /// pass them when known; `None` only for legacy/unknown-origin fires.
    pub fn strategy_signal(
        ts: String,
        signal: AlertSignal,
        proposal: PendingSignal,
        account: Option<String>,
        granularity: Option<String>,
    ) -> Self {
        Self {
            id: Self::new_id(),
            ts,
            payload: QueuedPayload::StrategySignal {
                instrument: proposal.instrument.clone(),
                signal,
                proposal: Box::new(proposal),
                account,
                granularity,
            },
        }
    }

    /// The execution proposal this alert promotes into, or `None` if it is not
    /// a promotable (strategy-signal) alert. This is the AC3 gate: only a
    /// strategy-signal alert yields a proposal; a price-level alert never does.
    pub fn promotable_proposal(&self) -> Option<&PendingSignal> {
        match &self.payload {
            QueuedPayload::StrategySignal { proposal, .. } => Some(proposal),
            QueuedPayload::PriceLevel { .. } => None,
        }
    }
}

/// Path to the alert queue (`<data home>/alert-queue.ndjson`;
/// `~/.wickd/alert-queue.ndjson` unless `WICKD_HOME` overrides the data home —
/// tests/smokes only, never live).
pub fn queue_path() -> Result<PathBuf> {
    let home = crate::paths::wickd_data_home().map_err(anyhow::Error::msg)?;
    Ok(home.join(QUEUE_FILE))
}

/// Append one alert to the append-only log at `path` (creating the parent dir),
/// as a single NDJSON line. Tests pass a temp path so they never touch the real
/// `~/.wickd/alert-queue.ndjson`.
pub fn append_at(path: impl AsRef<Path>, entry: &QueuedAlert) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create alert-queue dir {}", parent.display()))?;
    }
    let line = serde_json::to_string(entry).context("could not serialize queued alert")?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening alert queue at {}", path.display()))?;
    writeln!(file, "{line}")
        .with_context(|| format!("appending to alert queue at {}", path.display()))?;
    Ok(())
}

/// Read every queued alert from `path`, oldest first (file/append order — the
/// natural order for a tail). Returns an empty vec if the queue does not exist.
pub fn list_at(path: impl AsRef<Path>) -> Result<Vec<QueuedAlert>> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(vec![]);
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading alert queue at {}", path.display()))?;
    let mut out = Vec::new();
    for (i, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let entry: QueuedAlert = serde_json::from_str(line)
            .with_context(|| format!("alert-queue line {} is not valid JSON", i + 1))?;
        out.push(entry);
    }
    Ok(out)
}

/// Fetch a single queued alert by its queue-entry id from `path`.
pub fn get_at(path: impl AsRef<Path>, id: &str) -> Result<Option<QueuedAlert>> {
    Ok(list_at(path)?.into_iter().find(|e| e.id == id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending::STATUS_PENDING;

    fn temp_queue() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let mut p = std::env::temp_dir();
        p.push(format!("wickd-queue-test-{pid}-{nanos}-{n}.ndjson"));
        p
    }

    fn sample_proposal(id: &str, side: &str, units: i64) -> PendingSignal {
        PendingSignal {
            id: id.to_string(),
            ts: "2026-06-30T00:00:00+00:00".to_string(),
            instrument: "EUR_USD".to_string(),
            side: side.to_string(),
            units,
            suggested_units: None,
            strategy: "ma-crossover".to_string(),
            reason: "fast SMA crossed above slow".to_string(),
            sl: Some("1.0800".to_string()),
            tp: Some("1.0950".to_string()),
            entry_price: Some("1.0850".to_string()),
            status: STATUS_PENDING.to_string(),
        }
    }

    // AC1/AC2: append → list round-trip on a temp path. Order is preserved
    // (oldest first, as appended — tail order), and get_at finds by id.
    #[test]
    fn append_list_round_trip_preserves_order() {
        let path = temp_queue();

        let a = QueuedAlert::strategy_signal(
            "2026-06-30T00:00:00Z".to_string(),
            AlertSignal::Buy,
            sample_proposal("match-1", "long", 1000),
            Some("h004".to_string()),
            Some("M5".to_string()),
        );
        let b = QueuedAlert::price_level(
            "2026-06-30T00:00:05Z".to_string(),
            "EUR_USD".to_string(),
            "1.0900".to_string(),
            Direction::CrossUp,
            "1.0905".to_string(),
        );

        append_at(&path, &a).unwrap();
        append_at(&path, &b).unwrap();

        let listed = list_at(&path).unwrap();
        assert_eq!(listed.len(), 2);
        // Oldest first: a was appended first.
        assert_eq!(listed[0].id, a.id);
        assert_eq!(listed[1].id, b.id);

        // get_at resolves each by its queue-entry id.
        assert_eq!(get_at(&path, &a.id).unwrap().unwrap(), a);
        assert_eq!(get_at(&path, &b.id).unwrap().unwrap(), b);
        assert!(get_at(&path, "nope").unwrap().is_none());

        let _ = std::fs::remove_file(&path);
    }

    // AC3 gate: only strategy-signal alerts expose a promotable proposal.
    #[test]
    fn only_strategy_signal_is_promotable() {
        let strat = QueuedAlert::strategy_signal(
            "2026-06-30T00:00:00Z".to_string(),
            AlertSignal::Sell,
            sample_proposal("match-2", "short", -1000),
            None,
            None,
        );
        let proposal = strat.promotable_proposal().expect("strategy-signal is promotable");
        assert_eq!(proposal.id, "match-2");
        assert_eq!(proposal.side, "short");
        // The queue mirrors the proposal's instrument at the payload top level.
        assert_eq!(proposal.instrument, "EUR_USD");

        let price = QueuedAlert::price_level(
            "2026-06-30T00:00:05Z".to_string(),
            "EUR_USD".to_string(),
            "1.0900".to_string(),
            Direction::CrossUp,
            "1.0905".to_string(),
        );
        assert!(
            price.promotable_proposal().is_none(),
            "a price-level alert carries no order intent — not promotable"
        );
    }

    #[test]
    fn missing_queue_lists_empty() {
        let path = temp_queue();
        assert!(list_at(&path).unwrap().is_empty());
        assert!(get_at(&path, "anything").unwrap().is_none());
        let _ = std::fs::remove_file(&path);
    }

    // Issue #8 backward compatibility: rows queued BEFORE the account /
    // granularity fields existed must still parse (as None), and a full
    // round-trip preserves the new fields.
    #[test]
    fn strategy_signal_fields_are_backward_compatible() {
        // A verbatim pre-#8 line: no account, no granularity.
        let legacy = r#"{"id":"q-1","ts":"2026-06-30T00:00:00Z","payload":{"kind":"strategy-signal","instrument":"EUR_USD","signal":"buy","proposal":{"id":"match-1","ts":"2026-06-30T00:00:00+00:00","instrument":"EUR_USD","side":"long","units":1000,"strategy":"ma-crossover","reason":"fast SMA crossed above slow","sl":"1.0800","tp":"1.0950","entry_price":"1.0850","status":"pending"}}}"#;
        let entry: QueuedAlert = serde_json::from_str(legacy).expect("legacy row parses");
        match &entry.payload {
            QueuedPayload::StrategySignal { account, granularity, .. } => {
                assert_eq!(account, &None);
                assert_eq!(granularity, &None);
            }
            other => panic!("expected strategy-signal, got {other:?}"),
        }

        // None fields are omitted on the wire (old readers see the old shape).
        let json = serde_json::to_string(&entry).unwrap();
        assert!(!json.contains("\"account\""));
        assert!(!json.contains("\"granularity\""));

        // Round-trip with the fields set preserves them.
        let path = temp_queue();
        let tagged = QueuedAlert::strategy_signal(
            "2026-06-30T00:00:00Z".to_string(),
            AlertSignal::Buy,
            sample_proposal("match-3", "long", 1000),
            Some("tf-m5".to_string()),
            Some("M5".to_string()),
        );
        append_at(&path, &tagged).unwrap();
        let listed = list_at(&path).unwrap();
        assert_eq!(listed.len(), 1);
        match &listed[0].payload {
            QueuedPayload::StrategySignal { account, granularity, .. } => {
                assert_eq!(account.as_deref(), Some("tf-m5"));
                assert_eq!(granularity.as_deref(), Some("M5"));
            }
            other => panic!("expected strategy-signal, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    // Each entry gets its own id even when the underlying event repeats, so a
    // re-fire is addressable as a distinct queue event.
    #[test]
    fn entries_get_distinct_ids() {
        let a = QueuedAlert::price_level(
            "t".to_string(),
            "EUR_USD".to_string(),
            "1.0900".to_string(),
            Direction::CrossUp,
            "1.0905".to_string(),
        );
        let b = QueuedAlert::price_level(
            "t".to_string(),
            "EUR_USD".to_string(),
            "1.0900".to_string(),
            Direction::CrossUp,
            "1.0905".to_string(),
        );
        assert_ne!(a.id, b.id);
    }
}
