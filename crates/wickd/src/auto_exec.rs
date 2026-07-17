//! Autonomous practice execution for `wickd watch --auto` — AGT-627,
//! trust-ladder **Stage 2**.
//!
//! `wickd watch --semi-auto` (AGT-599) stops at surfacing a *pending proposal*
//! that a human/agent later approves with `wickd approve`. This module is the
//! next rung: with `--auto`, a tradeable strategy signal is turned into a
//! practice-account order **without a human keystroke** — but only ever on the
//! practice environment (trust-ladder Stage 2 is practice-only by construction).
//!
//! ## Every guardrail is inherited, never re-implemented
//!
//! This module contains **no order-submission code**. An entry signal is routed
//! through [`trade::execute_place_auto`] and a close signal through
//! [`trade::execute_close_auto`] — the exact same AGT-626 guarded auto entry
//! points `wickd approve --auto` uses. So autonomous execution inherits, for
//! free and identically:
//!
//! * the **practice-only arming gate** (`arm_auto_practice`): a `--live`-env
//!   submit fails closed before any order — autonomy can never fire a live order;
//! * the **position-risk caps + daily-loss kill-switch** (AGT-595,
//!   `risk::enforce_live_place`/`enforce_live_close`);
//! * the **append-only audit ledger** (AGT-596/610/612): every attempt and its
//!   terminal outcome (placed / rejected / close) lands a row;
//! * **strategy attribution** (AGT-630): the signal's strategy rides to OANDA as
//!   clientExtensions and into the audit `strategy` column.
//!
//! Belt-and-braces, `--auto` against the live env is *also* rejected at argument
//! time ([`reject_auto_live`]) before the daemon starts — so the fail-closed
//! arming gate is a second line of defense, never the first.
//!
//! ## What this module DOES own
//!
//! Two things the guarded path can't do for us, because they're specific to a
//! long-running watch loop that sees a signal *stream*:
//!
//! 1. **Per-instrument position state** (AC4). The core watcher deliberately
//!    emits an `Entry` signal on *every* candle the entry condition holds ("user
//!    can decide whether to scale in" — `multi_watcher::create_signal`). Left
//!    unguarded that would place a fresh order every candle. This module tracks
//!    which instruments it has an open position on and **suppresses a duplicate
//!    entry** while one is open; a close clears the state so the next entry fires.
//! 2. **Sizing from the operator, not the script** (AC3, AGT-599). The order's
//!    `|units|` comes from the `--units` flag (default
//!    [`crate::pending::DEFAULT_PROPOSED_UNITS`]); only the *direction* comes
//!    from the signal. A script's own `suggested_units`-style sizing stays
//!    advisory and never sizes an autonomous order.
//!
//! ## Threading
//!
//! [`EventSink`] methods are synchronous and are called from inside the watcher's
//! async loop, so we can't `.await` an order submission there. [`AutoExecSink`]
//! therefore only *classifies* a signal into an [`AutoIntent`] and hands it to a
//! dedicated executor task over an unbounded channel; [`run_executor`] owns the
//! position state and performs the (serialized) async submissions. Serializing
//! through one task also keeps position-state updates race-free and avoids
//! hammering the OS keychain from multiple tasks at once.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rust_decimal::prelude::ToPrimitive;
use serde::Serialize;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use anyhow::{bail, Result};
use wickd_core::config::OandaEnvironment;
use wickd_core::event_sink::EventSink;
use wickd_core::models::Position;
use wickd_core::oanda::streaming::{PriceUpdate, StreamError, StreamHealthStatus};
use wickd_core::shared::PositionDirection;
use wickd_core::strategy::{
    MatchStatusUpdateEvent, MatchType, PatternMatchEvent, StrategyErrorEvent, StrategyStatusEvent,
    WatcherTickEvent,
};

use crate::audit::{self, AuditEntry};
use crate::commands::trade::{self, EntryPlan};
use crate::sink::SignalSink;
use crate::vault_store::env_str;

/// AC2 (belt-and-braces): reject `--auto` against the live environment before
/// the daemon starts. Autonomous execution is permitted for the practice
/// environment ONLY (trust-ladder Stage 2 is practice-only). This runs at
/// argument time — long before any credential unlock, network call, or audit
/// row — so an `--auto --env live` invocation never even opens a watch loop. The
/// guarded auto path's own `arm_auto_practice` gate would also fail closed on a
/// live submit, so this is a redundant early stop, not the sole safeguard. Pure
/// and directly unit-testable.
pub fn reject_auto_live(auto: bool, env: OandaEnvironment) -> Result<()> {
    if auto && env == OandaEnvironment::Live {
        bail!(
            "refusing `--auto` against the live environment: autonomous execution is \
             permitted for the practice environment only (trust-ladder Stage 2 is \
             practice-only) — a live order still requires an interactive TTY \
             confirmation via `wickd approve --live`"
        );
    }
    Ok(())
}

/// The actionable intent a strategy signal maps to under `--auto`. Pure data
/// handed from the (sync) sink to the (async) executor. `Enter` carries the
/// script-supplied SL/TP (AC1) and the direction (the ONLY thing the signal
/// contributes to sizing — magnitude comes from `--units`, AC3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoIntent {
    Enter {
        instrument: String,
        direction: PositionDirection,
        sl: Option<String>,
        tp: Option<String>,
        strategy: String,
    },
    Close {
        instrument: String,
    },
}

/// Classify a surfaced pattern-match event into an [`AutoIntent`], or `None` for
/// a non-actionable signal. **Pure**: no I/O, no order construction — this is the
/// AC1 seam that decides entry-vs-close routing. An `Entry` with a direction is
/// an order to place (carrying its script SL/TP); an `Exit`/`PartialExit` is a
/// close of whatever we hold on that instrument; an `Entry` with no direction is
/// ignored (nothing to act on).
pub fn intent_from_match(ev: &PatternMatchEvent) -> Option<AutoIntent> {
    let pm = &ev.pattern_match;
    if pm.match_type == MatchType::Entry {
        let direction = pm.direction?;
        Some(AutoIntent::Enter {
            instrument: pm.instrument.clone(),
            direction,
            sl: pm.stop_loss.map(|d| d.to_string()),
            tp: pm.take_profit.map(|d| d.to_string()),
            strategy: ev.strategy_name.clone(),
        })
    } else {
        // Exit / PartialExit: close the position we hold on this instrument.
        Some(AutoIntent::Close {
            instrument: pm.instrument.clone(),
        })
    }
}

/// The side string the guarded close path expects, derived from signed units.
fn side_of(units: i64) -> &'static str {
    if units < 0 {
        "short"
    } else {
        "long"
    }
}

/// The side string the guarded close path expects, from a [`PositionDirection`].
/// The `OpenPosition`/`execute_close_auto` boundary is stringly-typed ("long" |
/// "short"); this is the single conversion point from the enum into it.
fn side_str(direction: PositionDirection) -> &'static str {
    match direction {
        PositionDirection::Long => "long",
        PositionDirection::Short => "short",
    }
}

/// One open position this watch loop is tracking (AC4). Minimal: a close targets
/// an instrument + side, so that's all the executor needs to remember.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenPosition {
    /// "long" | "short" — the side to hand the guarded close path.
    side: &'static str,
}

/// What the executor should do with an intent given the current position state.
/// Split out from the async task so the AC4 dedup / close-routing decision is
/// unit-testable without OANDA. Pure over `(&intent, &positions, units_flag)`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ExecDecision {
    /// Place a market entry (units already resolved from `--units` + direction).
    Place {
        instrument: String,
        units: i64,
        sl: Option<String>,
        tp: Option<String>,
        strategy: String,
    },
    /// Close the open position on `instrument` (side tracked locally).
    Close { instrument: String, side: &'static str },
    /// An entry arrived while a position is already open on the instrument — do
    /// NOT place a second order (AC4).
    SkipDuplicateEntry { instrument: String },
    /// A close arrived while we hold nothing on the instrument — nothing to do.
    SkipCloseWhenFlat { instrument: String },
}

/// Owns the per-instrument position state and the resolved order size for one
/// `wickd watch --auto` run. All execution goes through the AGT-626 guarded auto
/// entry points; this struct only decides *whether* and *with what size*.
pub struct AutoExecutor {
    /// Environment for the guarded submit. Guaranteed practice by
    /// [`reject_auto_live`] at startup; the guarded path's `arm_auto_practice`
    /// gate is the ultimate authority regardless.
    env: OandaEnvironment,
    /// Named account whose credentials place the order (AGT-625).
    account: String,
    /// Order size magnitude from `--units` (AC3). The signal supplies only the
    /// direction; this supplies the size. Always positive.
    units: i64,
    /// Instruments we currently believe we hold a position on.
    positions: HashMap<String, OpenPosition>,
}

impl AutoExecutor {
    pub fn new(env: OandaEnvironment, account: String, units: i64) -> Self {
        Self {
            env,
            account,
            units,
            positions: HashMap::new(),
        }
    }

    /// Signed units for a direction at the configured `--units` magnitude (AC3).
    fn signed_units(&self, direction: PositionDirection) -> i64 {
        match direction {
            PositionDirection::Long => self.units,
            PositionDirection::Short => -self.units,
        }
    }

    /// Decide what to do with an intent given the current position state.
    /// **Pure** (no I/O, no state mutation) — the AC4 duplicate-entry and
    /// close-when-flat guards live here so they're directly testable.
    fn decide(&self, intent: &AutoIntent) -> ExecDecision {
        match intent {
            AutoIntent::Enter {
                instrument,
                direction,
                sl,
                tp,
                strategy,
            } => {
                if self.positions.contains_key(instrument) {
                    // AC4: a position is already open on this instrument — an
                    // entry signal must NOT place a second order.
                    ExecDecision::SkipDuplicateEntry {
                        instrument: instrument.clone(),
                    }
                } else {
                    ExecDecision::Place {
                        instrument: instrument.clone(),
                        units: self.signed_units(*direction),
                        sl: sl.clone(),
                        tp: tp.clone(),
                        strategy: strategy.clone(),
                    }
                }
            }
            AutoIntent::Close { instrument } => match self.positions.get(instrument) {
                Some(pos) => ExecDecision::Close {
                    instrument: instrument.clone(),
                    side: pos.side,
                },
                None => ExecDecision::SkipCloseWhenFlat {
                    instrument: instrument.clone(),
                },
            },
        }
    }

    /// Record that a placement actually opened a position (AC4 state update).
    fn record_open(&mut self, instrument: &str, units: i64) {
        self.positions.insert(
            instrument.to_string(),
            OpenPosition {
                side: side_of(units),
            },
        );
    }

    /// Record that a close removed our position (AC4 state update).
    fn record_close(&mut self, instrument: &str) {
        self.positions.remove(instrument);
    }

    /// Seed a pre-existing position into the state at startup (AGT-628, AC1).
    /// Same effect on [`Self::decide`] as a filled [`Self::record_open`], but
    /// keyed on the [`PositionDirection`] OANDA already reports — so after warmup
    /// a duplicate entry on this instrument is suppressed and a close signal
    /// routes to the guarded close path against the side we actually hold.
    /// Idempotent per instrument (a re-adopt just overwrites the side).
    fn adopt(&mut self, instrument: &str, direction: PositionDirection) {
        self.positions.insert(
            instrument.to_string(),
            OpenPosition {
                side: side_str(direction),
            },
        );
    }

    /// Test-only view of whether an instrument is tracked as open. Used to prove
    /// startup reconciliation seeded (or deliberately did NOT seed) state.
    #[cfg(test)]
    fn is_open(&self, instrument: &str) -> bool {
        self.positions.contains_key(instrument)
    }
}

/// A pre-existing OANDA position adopted at startup (AGT-628, AC1). The watch
/// loop seeds its per-instrument state from these so, after indicator warmup, a
/// duplicate entry stays suppressed and the strategy's close logic resumes
/// against the side actually held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptedPosition {
    pub instrument: String,
    /// The side the guarded close path will target. Serializes to lowercase
    /// "long"/"short" on the emitted signal-stream event.
    pub side: PositionDirection,
    /// Signed units actually held on the account, as OANDA reports them
    /// (informational for the emitted event + audit row — sizing of *new*
    /// autonomous orders still comes from `--units`, never from here).
    pub units: i64,
}

/// An open OANDA position on an instrument NOT on the watchlist (AGT-628, AC2).
/// Reported at startup for observability, then left completely untouched — the
/// autonomous loop never places or closes anything it isn't watching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnwatchedPosition {
    pub instrument: String,
    pub side: PositionDirection,
    pub units: i64,
}

/// The result of classifying the account's open positions against the watched
/// instruments (AGT-628).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reconciliation {
    pub adopted: Vec<AdoptedPosition>,
    pub unwatched: Vec<UnwatchedPosition>,
}

/// Classify the account's open positions against the watched instruments
/// (AGT-628, AC1/AC2). **Pure**: no I/O, no state mutation — the adopt-vs-report
/// decision is directly unit-testable without OANDA. A flat position is ignored;
/// a non-flat position on a watched instrument is adopted; a non-flat position on
/// any other instrument is reported as unwatched (and never touched). Output is
/// sorted by instrument so the startup emission + audit rows are deterministic.
pub fn reconcile_positions(positions: &[Position], watchlist: &[String]) -> Reconciliation {
    let watched: HashSet<&str> = watchlist.iter().map(String::as_str).collect();
    let mut adopted = Vec::new();
    let mut unwatched = Vec::new();
    for p in positions {
        if p.is_flat() {
            continue;
        }
        let side = if p.is_short() {
            PositionDirection::Short
        } else {
            PositionDirection::Long
        };
        let units = p.units.to_i64().unwrap_or(0);
        if watched.contains(p.instrument.as_str()) {
            adopted.push(AdoptedPosition {
                instrument: p.instrument.clone(),
                side,
                units,
            });
        } else {
            unwatched.push(UnwatchedPosition {
                instrument: p.instrument.clone(),
                side,
                units,
            });
        }
    }
    adopted.sort_by(|a, b| a.instrument.cmp(&b.instrument));
    unwatched.sort_by(|a, b| a.instrument.cmp(&b.instrument));
    Reconciliation { adopted, unwatched }
}

/// Apply a startup reconciliation (AGT-628): seed each adopted position into the
/// executor state, emit every adoption/report on the signal stream, and record
/// each adoption in the audit log — the default-path wrapper `watch --auto` uses.
/// See [`apply_reconciliation_at`].
pub fn apply_reconciliation(exec: &mut AutoExecutor, recon: &Reconciliation, env: OandaEnvironment) {
    // Resolve the default audit store up front so the adoption rows land there.
    // If path resolution fails, still seed + emit (audit is fire-and-forget and
    // must never abort an autonomous watch) and let the per-row default writer
    // surface the warning.
    match audit::audit_path() {
        Ok(path) => apply_reconciliation_at(exec, recon, env, &path),
        Err(_) => apply_reconciliation_inner(exec, recon, env, None),
    }
}

/// As [`apply_reconciliation`], but writing audit rows to the store at `audit_path`
/// — split out (mirroring the `_at` convention in [`crate::audit`]/[`crate::pending`])
/// so tests exercise the exact seed + emit + audit path against a throwaway store
/// instead of `~/.wickd/audit.db`.
pub fn apply_reconciliation_at(
    exec: &mut AutoExecutor,
    recon: &Reconciliation,
    env: OandaEnvironment,
    audit_path: &Path,
) {
    apply_reconciliation_inner(exec, recon, env, Some(audit_path));
}

/// Shared body: for each adopted position emit the adoption line FIRST (the
/// "emit first, side-effect second" sink convention), then seed the executor
/// state (AC1) and record the adoption in the audit log (AC2). Unwatched
/// positions are reported but never seeded and never touched (AC2). An `audit`
/// path routes writes to a test store; `None` uses the default `~/.wickd/audit.db`.
fn apply_reconciliation_inner(
    exec: &mut AutoExecutor,
    recon: &Reconciliation,
    env: OandaEnvironment,
    audit_path: Option<&Path>,
) {
    for a in &recon.adopted {
        emit_auto_line(&serde_json::json!({
            "event": "auto-position-adopted",
            "instrument": a.instrument,
            "side": a.side,
            "units": a.units,
            "reason": "existing OANDA position on a watched instrument — seeded so \
                       duplicate entries stay suppressed and the strategy's close logic \
                       resumes after warmup",
        }));
        exec.adopt(&a.instrument, a.side);
        // mode="live" here means "a real broker position" (not a paper/dry-run
        // `not_submitted` decision) — the paper-vs-real distinction the `mode`
        // column carries. Which OANDA endpoint it lives on is the separate
        // `environment` column (practice, for `--auto`). See audit.rs schema.
        let entry = AuditEntry::now("adopt", "live", "adopted")
            .env(env_str(env))
            .instrument(&a.instrument)
            .units(a.units)
            .detail(Some(
                "startup reconciliation: adopted existing open position on a watched instrument"
                    .to_string(),
            ));
        match audit_path {
            Some(path) => audit::record_decision_at(path, entry),
            None => audit::record_decision(entry),
        }
    }
    for u in &recon.unwatched {
        // AC2: reported for observability, then left completely untouched — no
        // state seeded, no order ever placed or closed on an unwatched instrument.
        emit_auto_line(&serde_json::json!({
            "event": "auto-position-unwatched",
            "instrument": u.instrument,
            "side": u.side,
            "units": u.units,
            "reason": "open OANDA position on an instrument not in the watchlist — \
                       reported at startup, never touched",
        }));
    }
}

/// Did a guarded-place result actually open a position? Only a filled/partial
/// market entry does; a rejection (or the resting/paper shapes, which `--auto`
/// market orders never produce) does not — so the position state is only set
/// when there's really a position. Pure; mirrors `approve::is_rejected`'s reliance
/// on the explicit `outcome` discriminator (AGT-610/612).
fn opened_position(result: &serde_json::Value) -> bool {
    matches!(
        result.get("outcome").and_then(|v| v.as_str()),
        Some("filled") | Some("partial")
    )
}

/// A close that fails because the position DOESN'T EXIST is definitive
/// broker truth: we are flat (the stop/TP filled, or someone closed it in
/// the OANDA UI). Clearing tracked state on this error is what stops a
/// corpse from vetoing future entries (2026-07-17 poisoning). Any other
/// close error keeps state — a transient failure may leave a real position
/// open, and a phantom "open" is recoverable (the skip path revalidates)
/// while a phantom "flat" could double-enter.
fn close_error_means_flat(error_msg: &str) -> bool {
    // Deliberately narrow: OANDA's closeout refusal phrasing, not a generic
    // "does not exist" (which a DNS/proxy error could also contain — and a
    // spurious match here clears state for a position that is still open).
    error_msg.contains("requested to be closed out does not exist")
}

/// NDJSON line for an autonomous execution event, emitted so the `--auto` daemon
/// stays observable on stdout alongside the raw signal stream. Line-atomic via
/// the stdout lock (`println!`), matching the sinks' convention.
fn emit_auto_line<T: Serialize>(payload: &T) {
    if let Ok(line) = serde_json::to_string(payload) {
        println!("{line}");
    }
}

/// Merge an `"event"` discriminator into a guarded-path result object and print
/// it as one NDJSON line.
fn emit_auto_result(event: &str, mut result: serde_json::Value) {
    if let Some(obj) = result.as_object_mut() {
        obj.insert(
            "event".to_string(),
            serde_json::Value::String(event.to_string()),
        );
        emit_auto_line(&result);
    } else {
        emit_auto_line(&serde_json::json!({ "event": event, "result": result }));
    }
}

/// The executor task: drain intents, decide, and route each through the AGT-626
/// guarded auto path, updating position state on real fills/closes. Runs until
/// the sender is dropped (the sink is dropped on watch shutdown), then returns.
///
/// A submit that errors (risk-cap rejection, kill-switch trip, auth/network
/// failure) is logged as an NDJSON `auto-*-error` line and the loop continues —
/// a single failed order must never kill a long-running autonomous watch, and a
/// tripped kill-switch simply means nothing places until it's cleared. Because
/// state is only advanced on a real fill/close, a failed placement leaves the
/// instrument flat so a later signal can retry.
pub async fn run_executor(mut exec: AutoExecutor, mut rx: UnboundedReceiver<AutoIntent>) {
    while let Some(intent) = rx.recv().await {
        // At most one re-decide per intent: a SkipDuplicateEntry whose tracked
        // position turns out to be a broker-side corpse (stop/TP filled while
        // we weren't looking) clears the state and decides again — which is
        // guaranteed to Place, so the loop runs at most twice.
        let mut revalidated = false;
        loop {
        match exec.decide(&intent) {
            ExecDecision::Place {
                instrument,
                units,
                sl,
                tp,
                strategy,
            } => {
                // The plan carries the script-supplied SL/TP (AC1) and the
                // strategy attribution (AGT-630). Market entry only.
                let plan = EntryPlan::market(sl, tp).with_strategy(Some(strategy));
                // live=true: submit to the (practice) account for real — that IS
                // the paper-trading target. `env` is practice (AC2) so
                // `arm_auto_practice` permits it; a live env would fail closed.
                match trade::execute_place_auto(exec.env, &exec.account, &instrument, units, plan, true)
                    .await
                {
                    Ok(result) => {
                        if opened_position(&result) {
                            exec.record_open(&instrument, units);
                        }
                        emit_auto_result("auto-order-placed", result);
                    }
                    Err(e) => emit_auto_line(&serde_json::json!({
                        "event": "auto-order-error",
                        "action": "place",
                        "instrument": instrument,
                        "units": units,
                        "error": format!("{e:#}"),
                    })),
                }
            }
            ExecDecision::Close { instrument, side } => {
                match trade::execute_close_auto(exec.env, &exec.account, &instrument, side, true).await
                {
                    Ok(result) => {
                        // A close reduces exposure; clear the state whether or
                        // not OANDA reported a fill (an empty position is still
                        // "flat" going forward).
                        exec.record_close(&instrument);
                        emit_auto_result("auto-position-closed", result);
                    }
                    Err(e) => {
                        let msg = format!("{e:#}");
                        let definitively_flat = close_error_means_flat(&msg);
                        if definitively_flat {
                            exec.record_close(&instrument);
                        }
                        emit_auto_line(&serde_json::json!({
                            "event": "auto-close-error",
                            "action": "close",
                            "instrument": instrument,
                            "side": side,
                            "error": msg,
                            "state_cleared": definitively_flat,
                        }));
                    }
                }
            }
            ExecDecision::SkipDuplicateEntry { instrument } => {
                // Trust-but-verify (2026-07-17): the tracked position may be a
                // corpse — the broker filled its stop/TP and our map never
                // learned. Verify against the broker ONLY on this conflict
                // path; if actually flat, clear the corpse and re-decide (the
                // entry then places). Verification failure keeps the skip —
                // fail closed, never double-enter on uncertainty.
                if !revalidated {
                    revalidated = true;
                    match trade::position_open_at_broker(exec.env, &exec.account, &instrument).await {
                        Ok(false) => {
                            exec.record_close(&instrument);
                            emit_auto_line(&serde_json::json!({
                                "event": "auto-skip-revalidated",
                                "reason": "tracked position no longer exists at the broker — state cleared, re-deciding",
                                "instrument": instrument,
                            }));
                            continue;
                        }
                        Ok(true) => {}
                        Err(e) => emit_auto_line(&serde_json::json!({
                            "event": "auto-skip-verify-error",
                            "instrument": instrument,
                            "error": format!("{e:#}"),
                        })),
                    }
                }
                emit_auto_line(&serde_json::json!({
                    "event": "auto-skip",
                    "reason": "position already open on instrument",
                    "instrument": instrument,
                }));
            }
            ExecDecision::SkipCloseWhenFlat { instrument } => {
                emit_auto_line(&serde_json::json!({
                    "event": "auto-skip",
                    "reason": "no open position to close",
                    "instrument": instrument,
                }));
            }
        }
        break;
        }
    }
}

/// [`EventSink`] for `wickd watch --auto` (AGT-627, trust-ladder Stage 2).
///
/// Behaves like [`SignalSink`] — every event is still emitted as one NDJSON line,
/// so the signal stream downstream consumers rely on is byte-for-byte unchanged
/// (AC5) — and additionally, on a tradeable signal, hands an [`AutoIntent`] to
/// the executor task for autonomous submission. Recording/emitting is the only
/// synchronous work here; the actual (async) order goes out on the executor task.
///
/// Like [`crate::sink::SemiAutoSink`], this keeps its NDJSON output regardless of
/// `--format` (autonomous execution is a machine workflow); the human-feed alert
/// line, if any, is still added by the outer `signal_alert::AlertSink`.
pub struct AutoExecSink {
    inner: SignalSink,
    tx: UnboundedSender<AutoIntent>,
}

impl AutoExecSink {
    pub fn new(tx: UnboundedSender<AutoIntent>) -> Self {
        Self {
            inner: SignalSink,
            tx,
        }
    }
}

impl EventSink for AutoExecSink {
    fn pattern_matched(&self, event: &PatternMatchEvent) {
        // AC5: emit the raw signal line unchanged first — monitoring is intact.
        self.inner.pattern_matched(event);
        // Then dispatch the intent to the executor task. A dropped receiver
        // (executor gone) is logged, never fatal to the monitoring stream.
        if let Some(intent) = intent_from_match(event) {
            if let Err(e) = self.tx.send(intent) {
                eprintln!("warning: auto-exec dispatch failed (executor stopped): {e}");
            }
        }
    }
    fn strategy_status(&self, event: &StrategyStatusEvent) {
        self.inner.strategy_status(event);
    }
    fn strategy_error(&self, event: &StrategyErrorEvent) {
        self.inner.strategy_error(event);
    }
    fn match_status_update(&self, event: &MatchStatusUpdateEvent) {
        self.inner.match_status_update(event);
    }
    fn watcher_tick(&self, event: &WatcherTickEvent) {
        self.inner.watcher_tick(event);
    }
    fn price_update(&self, event: &PriceUpdate) {
        self.inner.price_update(event);
    }
    fn stream_error(&self, event: &StreamError) {
        self.inner.stream_error(event);
    }
    fn stream_health(&self, event: &StreamHealthStatus) {
        self.inner.stream_health(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wickd_core::strategy::PatternMatch;
    use rust_decimal_macros::dec;

    const UNITS: i64 = crate::pending::DEFAULT_PROPOSED_UNITS;

    fn entry_event(instrument: &str, direction: PositionDirection) -> PatternMatchEvent {
        let pm = PatternMatch::entry(
            "wickd-watch".to_string(),
            "cfg-1".to_string(),
            instrument.to_string(),
            direction,
            dec!(1.0850),
            Some(dec!(1.0800)),
            Some(dec!(1.0950)),
            // Strategy's own (larger) sizing — must stay advisory (AC3).
            Some(dec!(5000)),
            "synthetic entry".to_string(),
            None,
            false,
        );
        PatternMatchEvent {
            pattern_match: pm,
            strategy_name: "revert_adx".to_string(),
            timeframe: "H1".to_string(),
        }
    }

    fn exit_event(instrument: &str, direction: PositionDirection) -> PatternMatchEvent {
        let pm = PatternMatch::exit(
            "wickd-watch".to_string(),
            "cfg-1".to_string(),
            instrument.to_string(),
            direction,
            "synthetic exit".to_string(),
            None,
        );
        PatternMatchEvent {
            pattern_match: pm,
            strategy_name: "revert_adx".to_string(),
            timeframe: "H1".to_string(),
        }
    }

    fn exec() -> AutoExecutor {
        AutoExecutor::new(OandaEnvironment::Practice, "default".to_string(), UNITS)
    }

    // --- reject_auto_live (AC2) ---

    #[test]
    fn auto_against_live_is_rejected() {
        let err = reject_auto_live(true, OandaEnvironment::Live).unwrap_err();
        assert!(err.to_string().contains("practice environment only"));
    }

    #[test]
    fn auto_against_practice_is_allowed() {
        assert!(reject_auto_live(true, OandaEnvironment::Practice).is_ok());
    }

    #[test]
    fn non_auto_never_rejected_even_on_live() {
        // Without --auto, live is a normal (TTY-gated) path — not our concern.
        assert!(reject_auto_live(false, OandaEnvironment::Live).is_ok());
        assert!(reject_auto_live(false, OandaEnvironment::Practice).is_ok());
    }

    // --- intent_from_match (AC1 routing) ---

    #[test]
    fn long_entry_maps_to_enter_with_script_sl_tp() {
        let intent = intent_from_match(&entry_event("EUR_USD", PositionDirection::Long)).unwrap();
        assert_eq!(
            intent,
            AutoIntent::Enter {
                instrument: "EUR_USD".to_string(),
                direction: PositionDirection::Long,
                // AC1: the script-supplied SL/TP ride along.
                sl: Some("1.0800".to_string()),
                tp: Some("1.0950".to_string()),
                strategy: "revert_adx".to_string(),
            }
        );
    }

    #[test]
    fn exit_maps_to_close() {
        let intent = intent_from_match(&exit_event("EUR_USD", PositionDirection::Long)).unwrap();
        assert_eq!(
            intent,
            AutoIntent::Close {
                instrument: "EUR_USD".to_string()
            }
        );
    }

    #[test]
    fn entry_without_direction_is_ignored() {
        // An entry match with no direction carries nothing to act on.
        let mut ev = entry_event("EUR_USD", PositionDirection::Long);
        ev.pattern_match.direction = None;
        assert!(intent_from_match(&ev).is_none());
    }

    // --- AC3: sizing comes from --units, never the script ---

    #[test]
    fn size_comes_from_units_flag_not_the_script_suggestion() {
        let e = exec();
        let intent = intent_from_match(&entry_event("EUR_USD", PositionDirection::Long)).unwrap();
        match e.decide(&intent) {
            ExecDecision::Place { units, .. } => {
                // --units (1000 default), NOT the script's suggested 5000.
                assert_eq!(units, UNITS);
                assert_ne!(units.unsigned_abs() as i64, 5000);
            }
            other => panic!("expected Place, got {other:?}"),
        }
    }

    #[test]
    fn short_entry_produces_negative_units() {
        let e = AutoExecutor::new(OandaEnvironment::Practice, "default".to_string(), 2000);
        let intent = intent_from_match(&entry_event("GBP_USD", PositionDirection::Short)).unwrap();
        match e.decide(&intent) {
            ExecDecision::Place { units, .. } => assert_eq!(units, -2000),
            other => panic!("expected Place, got {other:?}"),
        }
    }

    // --- AC4: per-instrument position state prevents duplicate entries ---

    #[test]
    fn duplicate_entry_while_open_is_suppressed() {
        let mut e = exec();
        let entry = intent_from_match(&entry_event("EUR_USD", PositionDirection::Long)).unwrap();

        // First entry → Place.
        assert!(matches!(e.decide(&entry), ExecDecision::Place { .. }));
        // Simulate the placement filling and opening a position.
        e.record_open("EUR_USD", e.signed_units(PositionDirection::Long));

        // A second entry on the SAME instrument while open → suppressed, no order.
        assert_eq!(
            e.decide(&entry),
            ExecDecision::SkipDuplicateEntry {
                instrument: "EUR_USD".to_string()
            }
        );
    }

    #[test]
    fn entry_on_a_different_instrument_is_not_suppressed() {
        let mut e = exec();
        e.record_open("EUR_USD", UNITS);
        // GBP_USD is still flat — its entry places normally.
        let intent = intent_from_match(&entry_event("GBP_USD", PositionDirection::Long)).unwrap();
        assert!(matches!(e.decide(&intent), ExecDecision::Place { .. }));
    }

    #[test]
    fn close_then_reentry_fires_again() {
        let mut e = exec();
        let entry = intent_from_match(&entry_event("EUR_USD", PositionDirection::Long)).unwrap();
        let close = intent_from_match(&exit_event("EUR_USD", PositionDirection::Long)).unwrap();

        e.record_open("EUR_USD", UNITS);
        // Close targets the tracked side.
        assert_eq!(
            e.decide(&close),
            ExecDecision::Close {
                instrument: "EUR_USD".to_string(),
                side: "long"
            }
        );
        e.record_close("EUR_USD");
        // Now flat again → the next entry is NOT suppressed.
        assert!(matches!(e.decide(&entry), ExecDecision::Place { .. }));
    }

    #[test]
    fn close_while_flat_is_a_noop() {
        let e = exec();
        let close = intent_from_match(&exit_event("EUR_USD", PositionDirection::Long)).unwrap();
        assert_eq!(
            e.decide(&close),
            ExecDecision::SkipCloseWhenFlat {
                instrument: "EUR_USD".to_string()
            }
        );
    }

    #[test]
    fn close_targets_the_side_we_actually_hold() {
        let mut e = exec();
        // Opened SHORT.
        e.record_open("USD_JPY", e.signed_units(PositionDirection::Short));
        let close = intent_from_match(&exit_event("USD_JPY", PositionDirection::Short)).unwrap();
        assert_eq!(
            e.decide(&close),
            ExecDecision::Close {
                instrument: "USD_JPY".to_string(),
                side: "short"
            }
        );
    }

    // --- opened_position: state only advances on a real fill ---

    #[test]
    fn only_filled_or_partial_marks_a_position_open() {
        assert!(opened_position(&serde_json::json!({"outcome": "filled"})));
        assert!(opened_position(&serde_json::json!({"outcome": "partial"})));
        // A rejection must NOT mark a position open — the instrument stays flat
        // so a later signal can retry.
        assert!(!opened_position(&serde_json::json!({"outcome": "rejected"})));
        // Resting/paper shapes (which --auto market orders never produce) also
        // don't count.
        assert!(!opened_position(&serde_json::json!({"outcome": "resting"})));
        assert!(!opened_position(&serde_json::json!({"mode": "paper"})));
    }

    #[test]
    fn side_of_signed_units() {
        assert_eq!(side_of(1000), "long");
        assert_eq!(side_of(-1000), "short");
    }

    // --- AGT-628: startup open-position reconciliation ---

    fn position(instrument: &str, units: rust_decimal::Decimal) -> Position {
        Position {
            instrument: instrument.to_string(),
            units,
            average_price: dec!(1.0),
            unrealized_pl: dec!(0),
            realized_pl: dec!(0),
        }
    }

    fn watchlist(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// A throwaway audit db path, same pattern as `audit`'s own tests — so the
    /// reconciliation audit writes never touch the real `~/.wickd/audit.db`.
    fn temp_audit_db() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let mut p = std::env::temp_dir();
        p.push(format!("wickd-reconcile-test-{pid}-{nanos}-{n}.db"));
        p
    }

    // AC1/AC2: a non-flat position on a watched instrument is adopted (with its
    // real side + held units); one on an unwatched instrument is reported; a
    // flat position is ignored entirely.
    #[test]
    fn reconcile_classifies_watched_unwatched_and_flat() {
        let positions = vec![
            position("EUR_USD", dec!(1000)),   // watched, long
            position("GBP_USD", dec!(-2500)),  // watched, short
            position("USD_JPY", dec!(5000)),   // NOT watched
            position("AUD_USD", dec!(0)),      // flat → ignored even though watched
        ];
        let recon = reconcile_positions(&positions, &watchlist(&["EUR_USD", "GBP_USD", "AUD_USD"]));

        assert_eq!(
            recon.adopted,
            vec![
                AdoptedPosition {
                    instrument: "EUR_USD".into(),
                    side: PositionDirection::Long,
                    units: 1000
                },
                AdoptedPosition {
                    instrument: "GBP_USD".into(),
                    side: PositionDirection::Short,
                    units: -2500
                },
            ]
        );
        assert_eq!(
            recon.unwatched,
            vec![UnwatchedPosition {
                instrument: "USD_JPY".into(),
                side: PositionDirection::Long,
                units: 5000
            }]
        );
    }

    // Output is deterministic (sorted by instrument) regardless of OANDA order.
    #[test]
    fn reconcile_output_is_sorted_by_instrument() {
        let positions = vec![
            position("USD_JPY", dec!(100)),
            position("EUR_USD", dec!(100)),
            position("GBP_USD", dec!(100)),
        ];
        let recon = reconcile_positions(&positions, &watchlist(&["EUR_USD", "GBP_USD", "USD_JPY"]));
        let order: Vec<&str> = recon.adopted.iter().map(|a| a.instrument.as_str()).collect();
        assert_eq!(order, vec!["EUR_USD", "GBP_USD", "USD_JPY"]);
    }

    // The adoption/report event carries the side as lowercase "long"/"short"
    // (the wire contract the signal stream + downstream agents rely on).
    #[test]
    fn adopted_side_serializes_lowercase_for_the_event() {
        // Mirrors what apply_reconciliation_inner emits: `"side": a.side`.
        let v = serde_json::json!({ "side": PositionDirection::Short });
        assert_eq!(v["side"], "short");
        let v = serde_json::json!({ "side": PositionDirection::Long });
        assert_eq!(v["side"], "long");
    }

    // No open positions → an empty, side-effect-free reconciliation.
    #[test]
    fn reconcile_with_no_positions_is_empty() {
        let recon = reconcile_positions(&[], &watchlist(&["EUR_USD"]));
        assert_eq!(recon, Reconciliation::default());
    }

    // AC1: applying the reconciliation SEEDS the executor so a later entry on the
    // adopted instrument is suppressed (no double-entry) and a close routes to the
    // side actually held — while an unwatched position is NOT seeded (AC2).
    #[test]
    fn apply_reconciliation_seeds_adopted_and_never_seeds_unwatched() {
        let db = temp_audit_db();
        let mut e = exec();

        let recon = reconcile_positions(
            &[
                position("EUR_USD", dec!(1000)),  // watched, long → adopt
                position("USD_JPY", dec!(-3000)), // unwatched → report only
            ],
            &watchlist(&["EUR_USD", "GBP_USD"]),
        );
        apply_reconciliation_at(&mut e, &recon, OandaEnvironment::Practice, &db);

        // Adopted: tracked as open, so a fresh entry is a suppressed duplicate…
        assert!(e.is_open("EUR_USD"));
        let entry = intent_from_match(&entry_event("EUR_USD", PositionDirection::Long)).unwrap();
        assert_eq!(
            e.decide(&entry),
            ExecDecision::SkipDuplicateEntry { instrument: "EUR_USD".into() }
        );
        // …and a close resumes against the long we adopted.
        let close = intent_from_match(&exit_event("EUR_USD", PositionDirection::Long)).unwrap();
        assert_eq!(
            e.decide(&close),
            ExecDecision::Close { instrument: "EUR_USD".into(), side: "long" }
        );

        // Unwatched: never seeded — a close on it is a flat no-op, and it was
        // never turned into a tracked position we might close.
        assert!(!e.is_open("USD_JPY"));

        let _ = std::fs::remove_file(&db);
    }

    // AC1: an adopted SHORT resumes its close against the short side.
    #[test]
    fn adopted_short_closes_short() {
        let db = temp_audit_db();
        let mut e = exec();
        let recon = reconcile_positions(&[position("USD_JPY", dec!(-4000))], &watchlist(&["USD_JPY"]));
        apply_reconciliation_at(&mut e, &recon, OandaEnvironment::Practice, &db);

        let close = intent_from_match(&exit_event("USD_JPY", PositionDirection::Short)).unwrap();
        assert_eq!(
            e.decide(&close),
            ExecDecision::Close { instrument: "USD_JPY".into(), side: "short" }
        );
        let _ = std::fs::remove_file(&db);
    }

    // AC2: every adoption lands exactly one audit row (action=adopt,
    // outcome=adopted, with the held side/units + env); unwatched positions
    // write NO audit row (they're merely reported on the stream).
    #[test]
    fn adoption_is_recorded_in_the_audit_log_and_unwatched_is_not() {
        let db = temp_audit_db();
        let mut e = exec();
        let recon = reconcile_positions(
            &[
                position("EUR_USD", dec!(1000)),  // watched → audited
                position("GBP_USD", dec!(-2000)), // watched → audited
                position("USD_JPY", dec!(5000)),  // unwatched → NOT audited
            ],
            &watchlist(&["EUR_USD", "GBP_USD"]),
        );
        apply_reconciliation_at(&mut e, &recon, OandaEnvironment::Practice, &db);

        let conn = audit::open_at(&db).unwrap();
        let rows = audit::query(&conn, 10).unwrap();
        // Exactly the two adoptions — the unwatched position produced no row.
        assert_eq!(rows.len(), 2, "one audit row per adopted position only");
        for r in &rows {
            assert_eq!(r["action"], "adopt");
            assert_eq!(r["outcome"], "adopted");
            assert_eq!(r["environment"], "practice");
            assert_eq!(r["mode"], "live");
        }
        let instruments: Vec<&str> = rows.iter().map(|r| r["instrument"].as_str().unwrap()).collect();
        assert!(instruments.contains(&"EUR_USD"));
        assert!(instruments.contains(&"GBP_USD"));
        assert!(!instruments.contains(&"USD_JPY"));

        let _ = std::fs::remove_file(&db);
    }

    // An empty reconciliation applies cleanly: no state seeded, no audit rows.
    #[test]
    fn apply_empty_reconciliation_is_a_noop() {
        let db = temp_audit_db();
        let mut e = exec();
        apply_reconciliation_at(&mut e, &Reconciliation::default(), OandaEnvironment::Practice, &db);
        assert!(!e.is_open("EUR_USD"));
        // No audit db was even created (record_decision_at only opens on write).
        assert!(!db.exists());
    }

    // ---- broker-corpse recovery (2026-07-17 entry poisoning) ----

    #[test]
    fn close_error_means_flat_matches_the_oanda_message() {
        assert!(close_error_means_flat(
            "OANDA position close failed: OANDA API error: The Position requested to be closed out does not exist"
        ));
        assert!(!close_error_means_flat("connection timed out"));
        assert!(!close_error_means_flat("rate limited"));
        // The wide match this replaced would have cleared state on these:
        assert!(!close_error_means_flat("proxy host does not exist"));
        assert!(!close_error_means_flat("DNS: name does not exist"));
    }

    #[test]
    fn clearing_a_corpse_lets_the_next_entry_place() {
        let mut e = AutoExecutor::new(OandaEnvironment::Practice, "tf-m1".into(), UNITS);
        e.record_open("USD_JPY", e.signed_units(PositionDirection::Long));
        let intent = intent_from_match(&entry_event("USD_JPY", PositionDirection::Long)).unwrap();
        // Broker stop-fills; the map still holds the corpse and vetoes entries.
        assert!(matches!(e.decide(&intent), ExecDecision::SkipDuplicateEntry { .. }));
        // The revalidation/definitive-close paths clear it via record_close…
        e.record_close("USD_JPY");
        // …after which the same intent places: the retry loop terminates.
        assert!(matches!(e.decide(&intent), ExecDecision::Place { .. }));
    }
}
