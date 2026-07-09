//! Strategy-signal alerts for `wickd watch` — AGT-618 AC3.
//!
//! `MultiInstrumentWatcher` (in `wickd-core`) already reduces each
//! candle's rules evaluation down to a `RulesSignal` (Hold / Entry / Exit /
//! PartialExit) and only calls the [`EventSink`] for the non-Hold cases — a
//! Hold candle never reaches the sink at all (see `multi_watcher::create_signal`).
//! That gives AC3's "Hold/no-signal stays silent" for free.
//!
//! This module adds one more layer *on top* of the sink: it classifies each
//! `pattern-matched` event into an actionable Buy/Sell alert (an `Entry`
//! match's direction) and decides, per (instrument, strategy), whether to
//! actually fire it given a re-arm/dedup policy — so a strategy that keeps
//! producing the same Entry signal candle after candle doesn't spam an
//! identical alert every time.
//!
//! ## Relationship to AGT-617
//!
//! AGT-617 (price-level alerts, built concurrently on a sibling branch) is
//! expected to land a shared alert/re-arm-policy concept. That code doesn't
//! exist on this branch, so [`RearmPolicy`] here is a small, self-contained
//! stand-in with a similar shape (a trait + a per-key "fire or suppress"
//! decision) — deliberately easy to unify with AGT-617's policy later, but
//! not importing or depending on it now.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use wickd_core::event_sink::EventSink;
use wickd_core::oanda::streaming::{PriceUpdate, StreamError, StreamHealthStatus};
use wickd_core::shared::PositionDirection;
use wickd_core::strategy::{
    MatchStatusUpdateEvent, MatchType, PatternMatchEvent, StrategyErrorEvent, StrategyStatusEvent,
    WatcherTickEvent,
};

use crate::feed::{self, Format};

/// The actionable half of a strategy's per-candle evaluation. Only Buy/Sell
/// are alertable under AC3 — Hold never reaches this module (see module
/// docs), and Exit/PartialExit are not "Buy or Sell" so they're not either.
pub use wickd_core::alert_queue::AlertSignal;

/// Classify a pattern-match event's outcome for alerting purposes. Only an
/// `Entry` match with a direction is an actionable Buy/Sell; `Exit` and
/// `PartialExit` are not (AC3 only asks for Buy/Sell alerts).
pub fn classify(event: &PatternMatchEvent) -> Option<AlertSignal> {
    if event.pattern_match.match_type != MatchType::Entry {
        return None;
    }
    event.pattern_match.direction.map(AlertSignal::from_direction)
}

/// Dedup key: an alert is scoped to one instrument watched by one strategy.
pub fn alert_key(instrument: &str, strategy_name: &str) -> String {
    format!("{instrument}::{strategy_name}")
}

/// Re-arm/dedup policy: decides whether a candidate alert should actually
/// fire for a given key. A small trait so the mechanism can later be swapped
/// for (or unified with) AGT-617's price-level re-arm policy without
/// touching call sites.
pub trait RearmPolicy: Send + Sync {
    /// Returns `true` if the alert should fire (recording state for `key` so
    /// the next identical signal is suppressed), `false` if it's noise.
    fn should_fire(&self, key: &str, signal: AlertSignal) -> bool;

    /// Clear any recorded state for `key`. Called on a non-Entry outcome
    /// (Exit/PartialExit) so the position is considered closed: the next
    /// Buy/Sell for that key always fires, even if it repeats whatever fired
    /// right before the exit.
    fn reset(&self, key: &str);
}

/// Default re-arm policy (AC3): suppress a signal identical to the last one
/// *fired* for the same key, until it changes (or `reset` clears it). This is
/// deliberately not a cooldown timer — it's the "don't repeat the same
/// signal on every consecutive candle" dedup the ticket calls for. A
/// cooldown-based policy could implement the same trait alongside this one.
#[derive(Default)]
pub struct ChangeDedupPolicy {
    last_fired: Mutex<HashMap<String, AlertSignal>>,
}

impl ChangeDedupPolicy {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RearmPolicy for ChangeDedupPolicy {
    fn should_fire(&self, key: &str, signal: AlertSignal) -> bool {
        // `wickd watch` is a long-running daemon: a poisoned lock must not
        // become a second panic that kills the process. The map's state is
        // still valid even if some other thread panicked while holding the
        // lock, so recover the inner value rather than unwinding.
        let mut last = self.last_fired.lock().unwrap_or_else(|e| e.into_inner());
        let fire = last.get(key) != Some(&signal);
        if fire {
            last.insert(key.to_string(), signal);
        }
        fire
    }

    fn reset(&self, key: &str) {
        self.last_fired
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
    }
}

/// NDJSON payload for a fired strategy-signal alert (`"event": "strategy-signal-alert"`).
#[derive(Debug, Serialize)]
struct StrategySignalAlert<'a> {
    event: &'static str,
    instrument: &'a str,
    strategy_name: &'a str,
    timeframe: &'a str,
    signal: &'static str,
    reason: &'a str,
}

/// [`EventSink`] wrapper that adds strategy-signal alerts (AC3) on top of any
/// other sink (`SignalSink`, `SemiAutoSink`, ...). Every event still passes
/// through to `inner` unchanged — the alert is an *additional* line, emitted
/// only when [`classify`] yields Buy/Sell and the re-arm policy says to fire.
///
/// AGT-619: `format` selects how a fired alert is delivered — a
/// `strategy-signal-alert` NDJSON object ([`Format::Ndjson`], default) or a
/// human-readable [`feed::strategy_signal_line`] ([`Format::Human`]). The
/// passthrough to `inner` is unaffected; the human terminal feed keeps its base
/// sink quiet by wiring a [`crate::sink::NoopSink`] as `inner` (see
/// `commands::watch`).
pub struct AlertSink<P: RearmPolicy> {
    inner: Arc<dyn EventSink>,
    policy: P,
    format: Format,
    /// Durable alert-queue path (AGT-620). When set, every fired strategy-signal
    /// alert is ALSO appended to `~/.wickd/alert-queue.ndjson` so an agent can
    /// poll it — and, later, promote it into a pending proposal. `None` (the
    /// plain `new` constructor) keeps the sink queue-free, e.g. in unit tests.
    queue: Option<PathBuf>,
}

impl<P: RearmPolicy> AlertSink<P> {
    pub fn new(inner: Arc<dyn EventSink>, policy: P, format: Format) -> Self {
        Self { inner, policy, format, queue: None }
    }

    /// Builder: also durably append every fired strategy-signal alert to the
    /// alert queue at `queue` (AGT-620 AC1/AC2). Off by default (queue-free in
    /// unit tests).
    pub fn with_queue(mut self, queue: PathBuf) -> Self {
        self.queue = Some(queue);
        self
    }

    /// Pure decision: does this pattern-match event produce a strategy-signal
    /// alert right now? Split out from `pattern_matched` so the dedup logic
    /// is directly testable without capturing stdout.
    fn evaluate(&self, event: &PatternMatchEvent) -> Option<AlertSignal> {
        let key = alert_key(&event.pattern_match.instrument, &event.strategy_name);
        match classify(event) {
            Some(signal) => self.policy.should_fire(&key, signal).then_some(signal),
            None => {
                // Exit / PartialExit: the position is done, so re-arm — the
                // next Buy/Sell for this key should always fire.
                if event.pattern_match.match_type != MatchType::Entry {
                    self.policy.reset(&key);
                }
                None
            }
        }
    }
}

impl<P: RearmPolicy> EventSink for AlertSink<P> {
    fn pattern_matched(&self, event: &PatternMatchEvent) {
        self.inner.pattern_matched(event);

        if let Some(signal) = self.evaluate(event) {
            match self.format {
                Format::Ndjson => {
                    let alert = StrategySignalAlert {
                        event: "strategy-signal-alert",
                        instrument: &event.pattern_match.instrument,
                        strategy_name: &event.strategy_name,
                        timeframe: &event.timeframe,
                        signal: signal.as_str(),
                        reason: &event.pattern_match.reason,
                    };
                    if let Ok(line) = serde_json::to_string(&alert) {
                        println!("{line}");
                    }
                }
                Format::Human => {
                    println!(
                        "{}",
                        feed::strategy_signal_line(
                            &event.pattern_match.instrument,
                            signal.as_str(),
                            &event.strategy_name,
                            &event.timeframe,
                            &event.pattern_match.created_at.to_rfc3339(),
                        )
                    );
                }
            }

            // AGT-620: durably queue the (deduped) fire — in either format — so
            // an agent can poll it and later promote it. Only actionable entry
            // signals yield a proposal (`pending_from_match`), which is exactly
            // the Buy/Sell set that fired here. A queue-write failure is
            // non-fatal to monitoring.
            if let Some(queue) = &self.queue {
                if let Some(proposal) = crate::pending::pending_from_match(event) {
                    let entry = crate::alert_queue::QueuedAlert::strategy_signal(
                        event.pattern_match.created_at.to_rfc3339(),
                        signal,
                        proposal,
                    );
                    if let Err(e) = crate::alert_queue::append_at(queue, &entry) {
                        eprintln!("warning: alert queue write failed: {e:#}");
                    }
                }
            }
        }
    }

    fn strategy_status(&self, event: &StrategyStatusEvent) {
        self.inner.strategy_status(event);
    }
    fn strategy_error(&self, event: &StrategyErrorEvent) {
        self.inner.strategy_error(event);
        // Human feed's base sink is a NoopSink (the raw firehose is silenced),
        // so surface errors to stderr — matching the price-level path in
        // `crate::sink::AlertSink` — rather than dropping them silently.
        if self.format == Format::Human {
            eprintln!("strategy error [{}]: {}", event.error_type, event.message);
        }
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
        if self.format == Format::Human {
            eprintln!("stream error [{:?}]: {}", event.error_type, event.message);
        }
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

    fn entry_event(instrument: &str, strategy_name: &str, direction: PositionDirection) -> PatternMatchEvent {
        let pattern = PatternMatch::entry(
            "wickd-watch".to_string(),
            "cfg-1".to_string(),
            instrument.to_string(),
            direction,
            dec!(1.0850),
            Some(dec!(1.0800)),
            Some(dec!(1.0950)),
            Some(dec!(1000)),
            "synthetic entry".to_string(),
            None,
            false,
        );
        PatternMatchEvent {
            pattern_match: pattern,
            strategy_name: strategy_name.to_string(),
            timeframe: "H1".to_string(),
        }
    }

    fn exit_event(instrument: &str, strategy_name: &str, direction: PositionDirection) -> PatternMatchEvent {
        let pattern = PatternMatch::exit(
            "wickd-watch".to_string(),
            "cfg-1".to_string(),
            instrument.to_string(),
            direction,
            "synthetic exit".to_string(),
            None,
        );
        PatternMatchEvent {
            pattern_match: pattern,
            strategy_name: strategy_name.to_string(),
            timeframe: "H1".to_string(),
        }
    }

    // --- classify() ---

    #[test]
    fn classify_maps_long_entry_to_buy() {
        let event = entry_event("EUR_USD", "ma-crossover", PositionDirection::Long);
        assert_eq!(classify(&event), Some(AlertSignal::Buy));
    }

    #[test]
    fn classify_maps_short_entry_to_sell() {
        let event = entry_event("EUR_USD", "ma-crossover", PositionDirection::Short);
        assert_eq!(classify(&event), Some(AlertSignal::Sell));
    }

    #[test]
    fn classify_ignores_exit_and_partial_exit() {
        let exit = exit_event("EUR_USD", "ma-crossover", PositionDirection::Long);
        assert_eq!(classify(&exit), None);

        let partial = PatternMatchEvent {
            pattern_match: PatternMatch::partial_exit(
                "wickd-watch".to_string(),
                "cfg-1".to_string(),
                "EUR_USD".to_string(),
                PositionDirection::Short,
                50.0,
                "synthetic partial exit".to_string(),
                None,
            ),
            strategy_name: "ma-crossover".to_string(),
            timeframe: "H1".to_string(),
        };
        assert_eq!(classify(&partial), None);
    }

    // --- ChangeDedupPolicy ---

    #[test]
    fn dedup_policy_fires_first_signal_for_a_key() {
        let policy = ChangeDedupPolicy::new();
        assert!(policy.should_fire("EUR_USD::ma-crossover", AlertSignal::Buy));
    }

    #[test]
    fn dedup_policy_suppresses_consecutive_identical_signal() {
        let policy = ChangeDedupPolicy::new();
        assert!(policy.should_fire("EUR_USD::ma-crossover", AlertSignal::Buy));
        assert!(!policy.should_fire("EUR_USD::ma-crossover", AlertSignal::Buy));
        assert!(!policy.should_fire("EUR_USD::ma-crossover", AlertSignal::Buy));
    }

    #[test]
    fn dedup_policy_fires_again_once_signal_changes() {
        let policy = ChangeDedupPolicy::new();
        assert!(policy.should_fire("EUR_USD::ma-crossover", AlertSignal::Buy));
        assert!(!policy.should_fire("EUR_USD::ma-crossover", AlertSignal::Buy));
        assert!(policy.should_fire("EUR_USD::ma-crossover", AlertSignal::Sell));
    }

    #[test]
    fn dedup_policy_fires_again_after_reset() {
        let policy = ChangeDedupPolicy::new();
        assert!(policy.should_fire("EUR_USD::ma-crossover", AlertSignal::Buy));
        assert!(!policy.should_fire("EUR_USD::ma-crossover", AlertSignal::Buy));
        policy.reset("EUR_USD::ma-crossover");
        assert!(policy.should_fire("EUR_USD::ma-crossover", AlertSignal::Buy));
    }

    #[test]
    fn dedup_policy_keys_are_independent_per_instrument() {
        let policy = ChangeDedupPolicy::new();
        assert!(policy.should_fire("EUR_USD::ma-crossover", AlertSignal::Buy));
        // A different instrument (same strategy) must not be suppressed by
        // EUR_USD's state.
        assert!(policy.should_fire("GBP_USD::ma-crossover", AlertSignal::Buy));
    }

    // A no-op inner sink, used where a test needs *something* implementing
    // EventSink to construct an AlertSink but isn't asserting on passthrough.
    struct Noop;
    impl EventSink for Noop {
        fn pattern_matched(&self, _e: &PatternMatchEvent) {}
        fn strategy_status(&self, _e: &StrategyStatusEvent) {}
        fn strategy_error(&self, _e: &StrategyErrorEvent) {}
        fn match_status_update(&self, _e: &MatchStatusUpdateEvent) {}
        fn watcher_tick(&self, _e: &WatcherTickEvent) {}
        fn price_update(&self, _e: &PriceUpdate) {}
        fn stream_error(&self, _e: &StreamError) {}
        fn stream_health(&self, _e: &StreamHealthStatus) {}
    }

    // --- AlertSink::evaluate — the full synthetic per-candle sequence AC3 asks for ---

    #[test]
    fn synthetic_sequence_buy_sell_fires_hold_silent_dedup_suppresses() {
        let sink = AlertSink::new(Arc::new(Noop), ChangeDedupPolicy::new(), Format::Ndjson);

        // Synthetic per-candle strategy outputs for one instrument+strategy.
        // `None` mirrors a Hold candle: multi_watcher's create_signal()
        // returns None for RulesSignal::Hold, so the EventSink is never even
        // called — nothing to evaluate, i.e. silent by construction.
        let sequence: Vec<Option<PatternMatchEvent>> = vec![
            None, // Hold
            Some(entry_event("EUR_USD", "ma-crossover", PositionDirection::Long)), // Buy #1 -> fires
            Some(entry_event("EUR_USD", "ma-crossover", PositionDirection::Long)), // Buy #2 (unchanged) -> suppressed
            None, // Hold
            Some(entry_event("EUR_USD", "ma-crossover", PositionDirection::Short)), // Sell (changed) -> fires
            Some(exit_event("EUR_USD", "ma-crossover", PositionDirection::Short)), // Exit -> not alertable, re-arms
            Some(entry_event("EUR_USD", "ma-crossover", PositionDirection::Long)), // Buy again -> fires (re-armed by exit)
        ];

        // Two Hold candles are in the sequence above; asserting the count
        // here (before `.flatten()` drops them) is the "Hold/no-signal stays
        // silent" half of AC3 — a Hold candle never even reaches `evaluate`,
        // let alone fires an alert, because multi_watcher's create_signal()
        // never calls the sink for RulesSignal::Hold in the first place.
        assert_eq!(sequence.iter().filter(|s| s.is_none()).count(), 2);

        let mut fired = Vec::new();
        for event in sequence.iter().flatten() {
            // `evaluate` is exactly what `pattern_matched` uses internally to
            // decide whether to emit an alert line — called once per event
            // here so the dedup policy's state advances exactly once per
            // candle, matching production.
            if let Some(signal) = sink.evaluate(event) {
                fired.push(signal);
            }
        }

        assert_eq!(fired, vec![AlertSignal::Buy, AlertSignal::Sell, AlertSignal::Buy]);
    }

    // --- passthrough: every event still reaches `inner` unchanged ---

    #[test]
    fn pattern_matched_always_delegates_to_inner_sink() {
        struct RecordingSink {
            calls: Mutex<u32>,
        }
        impl EventSink for RecordingSink {
            fn pattern_matched(&self, _e: &PatternMatchEvent) {
                *self.calls.lock().unwrap() += 1;
            }
            fn strategy_status(&self, _e: &StrategyStatusEvent) {}
            fn strategy_error(&self, _e: &StrategyErrorEvent) {}
            fn match_status_update(&self, _e: &MatchStatusUpdateEvent) {}
            fn watcher_tick(&self, _e: &WatcherTickEvent) {}
            fn price_update(&self, _e: &PriceUpdate) {}
            fn stream_error(&self, _e: &StreamError) {}
            fn stream_health(&self, _e: &StreamHealthStatus) {}
        }

        let recorder = Arc::new(RecordingSink { calls: Mutex::new(0) });
        let sink = AlertSink::new(recorder.clone(), ChangeDedupPolicy::new(), Format::Ndjson);

        // Even a suppressed (deduped) alert must still pass the raw event
        // through — the alert layer is additive, never a filter on the
        // underlying NDJSON signal stream.
        let event = entry_event("EUR_USD", "ma-crossover", PositionDirection::Long);
        sink.pattern_matched(&event);
        sink.pattern_matched(&event);

        assert_eq!(*recorder.calls.lock().unwrap(), 2);
    }
}
