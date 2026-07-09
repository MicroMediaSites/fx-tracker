//! Persisted pending-signal store (AGT-599 — semi-auto execution, Stage 1).
//!
//! `wickd` is a headless, one-shot CLI: `wickd watch` is one process, and the
//! approve/execute of a surfaced signal happens in a *separate* invocation. An
//! in-memory store (core's `strategy::pending_store` is a global `Mutex<Vec<_>>`)
//! cannot bridge two processes, so the trust-ladder needs a durable file.
//!
//! This module persists *pending signal proposals* to a JSON file at
//! `~/.wickd/pending.json`, mirroring the home-dir + JSON-store pattern of
//! [`crate::vault_store`] and [`crate::audit`]. Each record is a signal that
//! `wickd watch --semi-auto` surfaced and that is **awaiting explicit approval**
//! before any order is built or submitted.
//!
//! ## The invariant (AC1/AC4)
//!
//! Writing a pending record is the *only* thing the watch/signal path does with
//! a tradeable signal — it never builds or submits an order. A record sitting in
//! this store **never auto-fires**: the only way it becomes an order is a
//! separate, explicit `wickd approve <id>` invocation (paper by default; `--live`
//! to arm). There is deliberately no execution code anywhere in this module.
//!
//! ## Schema (`~/.wickd/pending.json`)
//!
//! ```json
//! { "version": 1, "signals": [ { "id": "...", "ts": "...", "instrument": "EUR_USD",
//!   "side": "long", "units": 1000, "strategy": "ma-crossover",
//!   "reason": "fast SMA crossed above slow", "sl": "1.0800", "tp": "1.0950",
//!   "entry_price": "1.0850", "status": "pending" } ] }
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::strategy::{MatchType, PatternMatchEvent};

/// Conservative default order size (units) proposed for a surfaced signal. The
/// watcher's risk settings can size hypothetical signals arbitrarily; Stage 1
/// deliberately proposes a small, predictable size so an approval never arms a
/// surprise position. The risk caps (`risk::enforce_live_place`) still apply at
/// execution time regardless of this value.
pub const DEFAULT_PROPOSED_UNITS: i64 = 1000;

/// Status of a stored pending signal.
pub const STATUS_PENDING: &str = "pending";
/// Status once an approval has consumed the signal (kept for history).
pub const STATUS_CONSUMED: &str = "consumed";

/// One signal awaiting approval. Plain data: the watch/semi-auto path appends
/// these, `wickd pending` lists them, and `wickd approve <id>` reads one, builds
/// an order from it via the guarded trade path, then marks it consumed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingSignal {
    /// Stable id (the triggering pattern-match UUID).
    pub id: String,
    /// RFC3339 timestamp the signal was detected.
    pub ts: String,
    /// Instrument, e.g. EUR_USD.
    pub instrument: String,
    /// Proposed side: "long" | "short".
    pub side: String,
    /// Proposed signed units (negative = short). Deliberately the
    /// conservative [`DEFAULT_PROPOSED_UNITS`] — an approval never arms a
    /// surprise position (AC1/AC4).
    pub units: i64,
    /// The strategy's OWN calculated size (from its risk settings), signed,
    /// when the signal carried one. Advisory: surfaces in the ticket view as
    /// "strategy sized N"; never the default an approval executes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_units: Option<i64>,
    /// Strategy that produced the signal.
    pub strategy: String,
    /// Human-readable reason the conditions matched.
    pub reason: String,
    /// Proposed stop-loss (OANDA precision), if the signal carried one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sl: Option<String>,
    /// Proposed take-profit, if the signal carried one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tp: Option<String>,
    /// Price at match time, for reference (not used to build the market order).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_price: Option<String>,
    /// Lifecycle: "pending" until an approval consumes it.
    pub status: String,
}

/// On-disk store. `version` allows future format migrations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PendingStore {
    pub version: u32,
    #[serde(default)]
    pub signals: Vec<PendingSignal>,
}

/// Build a [`PendingSignal`] from a surfaced pattern-match event. **Pure**: no
/// I/O, no order construction — it only translates a detected *entry* signal
/// into a proposal record. Returns `None` for non-tradeable matches (exits,
/// partial exits, or an entry with no direction) so the watch path records a
/// pending order only for actionable buy/sell signals.
///
/// This is the AC1 seam: capturing a signal produces *data*, never an order.
pub fn pending_from_match(ev: &PatternMatchEvent) -> Option<PendingSignal> {
    let pm = &ev.pattern_match;
    if pm.match_type != MatchType::Entry {
        return None;
    }
    let direction = pm.direction?;
    let sign = match direction {
        crate::shared::PositionDirection::Long => 1,
        crate::shared::PositionDirection::Short => -1,
    };
    let units = sign * DEFAULT_PROPOSED_UNITS;
    // Carry the strategy's own risk-based sizing through as advisory data.
    // `units` stays at the conservative default (the AC1/AC4 invariant); the
    // suggestion is for the human reviewing the proposal.
    let suggested_units = pm
        .position_size
        .and_then(|d| {
            use rust_decimal::prelude::ToPrimitive;
            d.round().to_i64()
        })
        .filter(|n| *n > 0)
        .map(|n| sign * n);
    Some(PendingSignal {
        id: pm.id.clone(),
        ts: pm.created_at.to_rfc3339(),
        instrument: pm.instrument.clone(),
        side: direction.as_str().to_string(),
        units,
        suggested_units,
        strategy: ev.strategy_name.clone(),
        reason: pm.reason.clone(),
        sl: pm.stop_loss.map(|d| d.to_string()),
        tp: pm.take_profit.map(|d| d.to_string()),
        entry_price: pm.entry_price.map(|d| d.to_string()),
        status: STATUS_PENDING.to_string(),
    })
}

/// Build the market-order parameters from a pending signal. **Pure**: returns
/// `(instrument, units, sl, tp)` for the guarded place path — no network, no
/// decision about paper/live (that is the approver's `--live` flag).
pub fn order_from_pending(
    p: &PendingSignal,
) -> (String, i64, Option<String>, Option<String>) {
    (p.instrument.clone(), p.units, p.sl.clone(), p.tp.clone())
}

/// Path to the pending store (`<data home>/pending.json`; `~/.wickd/pending.json`
/// unless `WICKD_HOME` overrides the data home — tests/smokes only, never live).
pub fn pending_path() -> Result<PathBuf> {
    let home = crate::paths::wickd_data_home().map_err(anyhow::Error::msg)?;
    Ok(home.join("pending.json"))
}

/// Load the store at `path`, or an empty one if it does not exist yet. Tests
/// pass a temp path so they never touch the real `~/.wickd/pending.json`.
pub fn load_at(path: impl AsRef<Path>) -> Result<PendingStore> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(PendingStore { version: 1, signals: vec![] });
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading pending store at {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| "pending store is corrupt or not valid JSON")
}

/// Write the store to `path` (creating the parent dir), pretty-printed.
pub fn save_at(path: impl AsRef<Path>, store: &PendingStore) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create pending dir {}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(store)
        .context("could not serialize pending store")?;
    std::fs::write(path, body)
        .with_context(|| format!("writing pending store at {}", path.display()))?;
    Ok(())
}

/// Append one pending signal to the store at `path` (load → push → save).
pub fn append_at(path: impl AsRef<Path>, signal: &PendingSignal) -> Result<()> {
    let path = path.as_ref();
    let mut store = load_at(path)?;
    store.version = 1;
    store.signals.push(signal.clone());
    save_at(path, &store)
}

/// List pending (un-consumed) signals from `path`, newest first.
pub fn list_at(path: impl AsRef<Path>) -> Result<Vec<PendingSignal>> {
    let store = load_at(path)?;
    let mut pending: Vec<PendingSignal> = store
        .signals
        .into_iter()
        .filter(|s| s.status == STATUS_PENDING)
        .collect();
    pending.reverse();
    Ok(pending)
}

/// Fetch a single signal by id from `path` (regardless of status).
pub fn get_at(path: impl AsRef<Path>, id: &str) -> Result<Option<PendingSignal>> {
    let store = load_at(path)?;
    Ok(store.signals.into_iter().find(|s| s.id == id))
}

/// Mark a pending signal consumed at `path`. Returns true if a *pending* signal
/// with that id was found and flipped. Idempotent: a re-approval of an
/// already-consumed id returns false (no second order should be built from it).
pub fn consume_at(path: impl AsRef<Path>, id: &str) -> Result<bool> {
    let path = path.as_ref();
    let mut store = load_at(path)?;
    let mut changed = false;
    for s in store.signals.iter_mut() {
        if s.id == id && s.status == STATUS_PENDING {
            s.status = STATUS_CONSUMED.to_string();
            changed = true;
        }
    }
    if changed {
        save_at(path, &store)?;
    }
    Ok(changed)
}

// --- Default-path convenience wrappers (production callers) ---
//
// The watch/semi-auto sink appends via `append_at` with an explicit path
// (resolved once at startup); these wrappers serve the one-shot verbs.

/// List pending signals from the default store, newest first.
pub fn list() -> Result<Vec<PendingSignal>> {
    list_at(pending_path()?)
}

/// Fetch one signal by id from the default store.
pub fn get(id: &str) -> Result<Option<PendingSignal>> {
    get_at(pending_path()?, id)
}

/// Mark a signal consumed in the default store.
pub fn consume(id: &str) -> Result<bool> {
    consume_at(pending_path()?, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::PositionDirection;
    use crate::strategy::PatternMatch;
    use rust_decimal_macros::dec;

    fn temp_store() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let mut p = std::env::temp_dir();
        p.push(format!("wickd-pending-test-{pid}-{nanos}-{n}.json"));
        p
    }

    fn entry_event(direction: PositionDirection) -> PatternMatchEvent {
        let pm = PatternMatch::entry(
            "wickd-watch".to_string(),
            "cfg-1".to_string(),
            "EUR_USD".to_string(),
            direction,
            dec!(1.0850),
            Some(dec!(1.0800)),
            Some(dec!(1.0950)),
            Some(dec!(5000)),
            "fast SMA crossed above slow".to_string(),
            None,
            false,
        );
        PatternMatchEvent {
            pattern_match: pm,
            strategy_name: "ma-crossover".to_string(),
            timeframe: "H1".to_string(),
        }
    }

    // AC1: capturing a signal only constructs a pending record — there is no
    // order-submission anywhere in this conversion. A long entry → a long
    // proposal at the conservative default size; sl/tp carried through.
    #[test]
    fn pending_from_long_entry_is_a_proposal_not_an_order() {
        let ev = entry_event(PositionDirection::Long);
        let sig = pending_from_match(&ev).expect("entry produces a pending proposal");
        assert_eq!(sig.instrument, "EUR_USD");
        assert_eq!(sig.side, "long");
        assert_eq!(sig.units, DEFAULT_PROPOSED_UNITS);
        assert_eq!(sig.strategy, "ma-crossover");
        assert_eq!(sig.status, STATUS_PENDING);
        assert_eq!(sig.sl.as_deref(), Some("1.0800"));
        assert_eq!(sig.tp.as_deref(), Some("1.0950"));
        // Conservative default ignores the strategy's larger sizing (5000)…
        assert_eq!(sig.units.abs(), DEFAULT_PROPOSED_UNITS);
        // …but the strategy's own risk-based size rides along as advisory.
        assert_eq!(sig.suggested_units, Some(5000));
    }

    #[test]
    fn pending_from_short_entry_is_negative_units() {
        let ev = entry_event(PositionDirection::Short);
        let sig = pending_from_match(&ev).unwrap();
        assert_eq!(sig.side, "short");
        assert_eq!(sig.units, -DEFAULT_PROPOSED_UNITS);
        // The advisory size is signed like `units`.
        assert_eq!(sig.suggested_units, Some(-5000));
    }

    // Old stores (records written before suggested_units existed) still parse.
    #[test]
    fn pre_suggested_units_records_deserialize() {
        let old = r#"{"id":"x","ts":"t","instrument":"EUR_USD","side":"long",
            "units":1000,"strategy":"ma-crossover","reason":"r","status":"pending"}"#;
        let sig: PendingSignal = serde_json::from_str(old).unwrap();
        assert_eq!(sig.suggested_units, None);
        assert_eq!(sig.units, 1000);
    }

    // Exit signals are not tradeable proposals — they never become pending.
    #[test]
    fn exit_match_produces_no_pending() {
        let pm = PatternMatch::exit(
            "u".to_string(),
            "cfg".to_string(),
            "EUR_USD".to_string(),
            PositionDirection::Long,
            "rr exit".to_string(),
            None,
        );
        let ev = PatternMatchEvent {
            pattern_match: pm,
            strategy_name: "rsi".to_string(),
            timeframe: "H1".to_string(),
        };
        assert!(pending_from_match(&ev).is_none());
    }

    // Round-trip: append → list (newest first) → consume → gone from the list.
    #[test]
    fn store_round_trip_append_list_consume() {
        let path = temp_store();

        let a = pending_from_match(&entry_event(PositionDirection::Long)).unwrap();
        let mut b = pending_from_match(&entry_event(PositionDirection::Short)).unwrap();
        b.id = "second-id".to_string();

        append_at(&path, &a).unwrap();
        append_at(&path, &b).unwrap();

        let listed = list_at(&path).unwrap();
        assert_eq!(listed.len(), 2);
        // Newest first: b was appended last.
        assert_eq!(listed[0].id, "second-id");
        assert_eq!(listed[1].id, a.id);

        // get returns the right record.
        assert_eq!(get_at(&path, "second-id").unwrap().unwrap().side, "short");

        // Consume the second; it disappears from the pending list.
        assert!(consume_at(&path, "second-id").unwrap());
        let listed = list_at(&path).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, a.id);

        // Re-consuming is a no-op (no second order can be built from it).
        assert!(!consume_at(&path, "second-id").unwrap());
        // The consumed record still exists in the file, just not "pending".
        assert_eq!(
            get_at(&path, "second-id").unwrap().unwrap().status,
            STATUS_CONSUMED
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn order_from_pending_extracts_params() {
        let sig = pending_from_match(&entry_event(PositionDirection::Short)).unwrap();
        let (instrument, units, sl, tp) = order_from_pending(&sig);
        assert_eq!(instrument, "EUR_USD");
        assert_eq!(units, -DEFAULT_PROPOSED_UNITS);
        assert_eq!(sl.as_deref(), Some("1.0800"));
        assert_eq!(tp.as_deref(), Some("1.0950"));
    }

    #[test]
    fn missing_store_lists_empty() {
        let path = temp_store();
        assert!(list_at(&path).unwrap().is_empty());
        assert!(get_at(&path, "nope").unwrap().is_none());
        let _ = std::fs::remove_file(&path);
    }
}
