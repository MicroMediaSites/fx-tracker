//! `wickd approve` — approve a pending signal and execute it (AGT-599, Stage 1).
//!
//!   wickd approve <signal-id>                 # paper (dry-run), never submits
//!   wickd approve <signal-id> --live          # arm a real order (TTY keystroke)
//!
//! This is the trust-ladder's **explicit approval step**: a signal surfaced by
//! `wickd watch --semi-auto` (recorded in `~/.wickd/pending.json`) becomes an
//! order ONLY here, and ONLY for the signal id named on the command line. There
//! is no other path from a stored pending signal to an OANDA order — a record
//! sitting in the store never auto-fires (AC1/AC2/AC4).
//!
//! ## How it inherits the guard rails (AC3)
//!
//! `approve` does NOT re-implement order submission. It loads the pending
//! signal, builds the market-order params from it ([`pending::order_from_pending`]),
//! and hands them to the **same** guarded sequence `trade place` uses
//! ([`trade::execute_place`]): paper by default; `--live` (with confirm) →
//! `audit::record_required` (fatal pre-submit) → `risk::enforce_live_place`
//! (caps) → OANDA submit → audit outcome. So an approved order inherits arming,
//! caps, and the audit ledger for free.
//!
//! On a successful (non-paper-and-not-rejected) run the pending signal is marked
//! consumed so it cannot be approved twice.

use anyhow::{anyhow, bail, Result};
use clap::Args;

use wickd_core::config::OandaEnvironment;

use crate::commands::trade;
use crate::output::{exit, Out};
use crate::pending;

#[derive(Args, Debug)]
pub struct ApproveArgs {
    /// Id of the pending signal to approve (see `wickd pending`).
    pub signal_id: String,
    /// OANDA account/endpoint a *live* order targets (practice|live). Does NOT
    /// arm submission — pass --live for that. Default: practice.
    #[arg(long, default_value = "practice")]
    pub env: String,
    /// Named account within --env whose credentials are used (AGT-625), e.g.
    /// h004. Default: the single/default account.
    #[arg(long, default_value = crate::vault_store::DEFAULT_ACCOUNT)]
    pub account: String,
    /// Arm REAL order submission. Without it, the approval is paper (dry-run):
    /// it emits the would-be order and never contacts OANDA.
    #[arg(long)]
    pub live: bool,
    /// Retained for compatibility; does NOT arm a live submit. A live approval
    /// requires an interactive TTY keystroke (AGT-613) — --yes cannot supply it.
    #[arg(long)]
    pub yes: bool,
    /// Arm a NON-INTERACTIVE live submit for autonomous PRACTICE trading
    /// (AGT-626). Only meaningful with --live; practice env only — a --auto
    /// approval against the LIVE env FAILS CLOSED. This is how the Stage 2
    /// autonomous loop approves a pending signal without a human keystroke.
    #[arg(long)]
    pub auto: bool,
}

pub async fn run(args: ApproveArgs, out: Out) -> ! {
    match approve(args).await {
        Ok(v) => {
            out.ok(&v);
            std::process::exit(exit::OK);
        }
        Err(e) => {
            let msg = format!("{e:#}");
            // Reuse the trade error classifier so an approval's risk-cap /
            // auth / validation errors map to the same exit codes (AC3).
            out.fail(trade::execution_exit_code(&msg), "approve_failed", msg);
        }
    }
}

/// AGT-610 (AC3): decide whether an `execute_place` result is a TRUE
/// rejection (the order was cancelled at OANDA) as opposed to anything else
/// that should consume the pending signal (filled, partial, accepted-but-
/// resting, or paper). Pure — directly testable without touching OANDA or the
/// pending store.
///
/// Before this fix the check here was `ok:false && submitted:true`, which a
/// resting (accepted but not yet filled) limit/stop order ALSO satisfies —
/// conflating "rejected" with "accepted, still working" and leaving a resting
/// order's signal un-consumed (re-approvable, i.e. a double-submit risk).
/// `execute_place` now stamps an explicit `outcome` field, so only
/// `outcome == "rejected"` counts as a rejection.
pub(crate) fn is_rejected(result: &serde_json::Value) -> bool {
    result.get("outcome").and_then(|v| v.as_str()) == Some("rejected")
}

async fn approve(args: ApproveArgs) -> Result<serde_json::Value> {
    let env = OandaEnvironment::from_str(&args.env).map_err(|e| anyhow!(e.to_string()))?;

    // Load the named pending signal. An unknown id (or one already consumed) is
    // a validation error — the "pending signal" token routes it accordingly.
    let signal = pending::get(&args.signal_id)?
        .ok_or_else(|| anyhow!("no pending signal '{}'", args.signal_id))?;
    if signal.status != pending::STATUS_PENDING {
        bail!(
            "pending signal '{}' is already {} — refusing to re-execute",
            args.signal_id,
            signal.status
        );
    }

    // Build the order from the signal and run it through the SAME guarded path
    // `trade place` uses. `approve` never submits directly — this is the only
    // line that can turn a signal into a live order, and only with --live.
    let (instrument, units, sl, tp) = pending::order_from_pending(&signal);
    // Stage 1 approvals are market entries; the plan carries only the signal's
    // SL/TP. The limit/stop kinds (AGT-612) are reachable via `trade place`.
    // AGT-630: the signal's strategy rides along so the order reaches OANDA
    // with clientExtensions attribution and the audit row names the strategy —
    // the approve path ALWAYS knows the strategy, so this is unconditional.
    // AGT-625: --account selects which named account's credentials place it.
    let plan = trade::EntryPlan::market(sl, tp).with_strategy(Some(signal.strategy.clone()));
    // AGT-626: `--auto` takes the non-interactive practice arming (Stage 2
    // autonomy); otherwise the default interactive (TTY-keystroke) arming. Both
    // run the identical guarded place sequence — only the arming gate differs.
    let result = if args.auto {
        trade::execute_place_auto(env, &args.account, &instrument, units, plan, args.live).await?
    } else {
        trade::execute_place(env, &args.account, &instrument, units, plan, args.live, args.yes)
            .await?
    };

    // Consume the signal once it has been acted on (paper, filled, partial, or
    // accepted-but-resting). Only a TRUE rejection leaves it pending so it can
    // be retried (see [`is_rejected`] for AGT-610/AC3 rationale).
    let consumed = if is_rejected(&result) {
        false
    } else {
        pending::consume(&signal.id)?
    };

    let mut out = result;
    if let Some(obj) = out.as_object_mut() {
        obj.insert("signal_id".to_string(), serde_json::Value::String(signal.id.clone()));
        obj.insert("consumed".to_string(), serde_json::Value::Bool(consumed));
        obj.insert("strategy".to_string(), serde_json::Value::String(signal.strategy.clone()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending::PendingSignal;

    fn temp_store() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static C: AtomicU64 = AtomicU64::new(0);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "wickd-approve-test-{}-{}.json",
            std::process::id(),
            C.fetch_add(1, Ordering::Relaxed)
        ));
        p
    }

    fn sample(id: &str) -> PendingSignal {
        PendingSignal {
            id: id.to_string(),
            ts: "2026-06-30T00:00:00+00:00".to_string(),
            instrument: "EUR_USD".to_string(),
            side: "long".to_string(),
            units: 1000,
            suggested_units: None,
            strategy: "ma-crossover".to_string(),
            reason: "fast SMA crossed above slow".to_string(),
            sl: Some("1.0800".to_string()),
            tp: Some("1.0950".to_string()),
            entry_price: Some("1.0850".to_string()),
            status: pending::STATUS_PENDING.to_string(),
        }
    }

    // AC4: approving WITHOUT --live is paper — the guarded path returns a
    // would-be order that was never submitted, and there is no branch in this
    // module that submits a live order absent the explicit `--live` flag. We
    // exercise the pure decision via trade::execution_mode (shared arming).
    #[test]
    fn approve_without_live_is_paper() {
        assert_eq!(trade::execution_mode(false), trade::Mode::Paper);
        assert_eq!(trade::execution_mode(true), trade::Mode::Live);
    }

    // The order built from a pending signal matches the signal's params — the
    // only thing that flows from signal → order is this pure mapping.
    #[test]
    fn order_built_from_signal_matches_params() {
        let sig = sample("abc");
        let (instrument, units, sl, tp) = pending::order_from_pending(&sig);
        assert_eq!(instrument, "EUR_USD");
        assert_eq!(units, 1000);
        assert_eq!(sl.as_deref(), Some("1.0800"));
        assert_eq!(tp.as_deref(), Some("1.0950"));
    }

    // AGT-630 (AC1/AC2): the plan an approval hands to the guarded place path
    // carries the pending signal's strategy — the same construction `approve()`
    // performs — so the order reaches OANDA with clientExtensions attribution
    // and the audit row's strategy column is populated on this path.
    #[test]
    fn approve_plan_carries_the_signals_strategy() {
        let sig = sample("abc");
        let (_, _, sl, tp) = pending::order_from_pending(&sig);
        let plan = trade::EntryPlan::market(sl, tp).with_strategy(Some(sig.strategy.clone()));
        assert_eq!(plan.strategy.as_deref(), Some("ma-crossover"));
    }

    // An already-consumed signal cannot be re-approved: the lookup + status
    // guard rejects it before any order is built (no double execution).
    #[test]
    fn consumed_signal_is_rejected_by_store_guard() {
        let path = temp_store();
        let mut sig = sample("xyz");
        sig.status = pending::STATUS_CONSUMED.to_string();
        pending::append_at(&path, &sig).unwrap();

        // consume_at on an already-consumed id is a no-op (returns false).
        assert!(!pending::consume_at(&path, "xyz").unwrap());
        // It is not in the pending list either.
        assert!(pending::list_at(&path).unwrap().is_empty());

        let _ = std::fs::remove_file(&path);
    }

    // AGT-610 (AC3): a true rejection (order cancelled at OANDA) — and ONLY a
    // true rejection — is what `is_rejected` flags.
    #[test]
    fn is_rejected_true_for_cancelled_order() {
        let cancelled = serde_json::json!({
            "ok": false,
            "submitted": true,
            "filled": false,
            "outcome": "rejected",
            "reason": "MARKET_HALTED",
        });
        assert!(is_rejected(&cancelled));
    }

    // AGT-610 (AC3) regression: a resting (accepted-but-not-yet-filled)
    // limit/stop order must NOT be treated as a rejection, even though it
    // shares the same `ok:false, submitted:true` shape a true rejection used
    // to be judged by pre-fix. Filled/partial/paper outcomes must not be
    // flagged either.
    #[test]
    fn is_rejected_false_for_resting_filled_partial_and_paper_outcomes() {
        let resting = serde_json::json!({
            "ok": true,
            "submitted": true,
            "filled": false,
            "outcome": "resting",
        });
        assert!(!is_rejected(&resting));

        let filled = serde_json::json!({"ok": true, "submitted": true, "outcome": "filled"});
        assert!(!is_rejected(&filled));

        let partial = serde_json::json!({"ok": true, "submitted": true, "outcome": "partial"});
        assert!(!is_rejected(&partial));

        // Paper responses carry no "outcome" field at all.
        let paper = serde_json::json!({"ok": true, "mode": "paper", "submitted": false});
        assert!(!is_rejected(&paper));

        // AGT-610 regression: the OLD ambiguous shape (ok:false && submitted:
        // true, no outcome) must no longer be treated as rejected on its own —
        // that shape is exactly what a resting order used to be
        // indistinguishable from.
        let old_ambiguous_shape = serde_json::json!({"ok": false, "submitted": true});
        assert!(!is_rejected(&old_ambiguous_shape));
    }

    // AGT-610 (AC3) end-to-end-of-the-store regression: a resting order's
    // signal gets consumed (not left retryable), so a SECOND approve attempt
    // on the same signal id is rejected by the pending-store status guard —
    // proving the double-submit risk described in the ticket is closed.
    #[test]
    fn resting_order_consumes_signal_blocking_a_second_approve_attempt() {
        let path = temp_store();
        let sig = sample("resting-1");
        pending::append_at(&path, &sig).unwrap();

        // The JSON shape `execute_place` now returns for an OANDA-accepted
        // but not-yet-filled order.
        let resting_result = serde_json::json!({
            "ok": true,
            "mode": "live",
            "submitted": true,
            "filled": false,
            "outcome": "resting",
            "instrument": "EUR_USD",
        });
        assert!(!is_rejected(&resting_result));

        // `approve()`'s consume step: not rejected -> consume the signal, the
        // same call it makes on the real (default-store) path.
        assert!(pending::consume_at(&path, &sig.id).unwrap());

        // A second `approve` on the same id must now be refused: the status
        // guard at the top of `approve()` bails on anything other than
        // STATUS_PENDING, and the signal is no longer pending.
        let reloaded = pending::get_at(&path, &sig.id).unwrap().unwrap();
        assert_eq!(reloaded.status, pending::STATUS_CONSUMED);
        assert_ne!(reloaded.status, pending::STATUS_PENDING);
        // It has also dropped out of the pending listing an agent would see.
        assert!(pending::list_at(&path).unwrap().is_empty());

        let _ = std::fs::remove_file(&path);
    }
}
