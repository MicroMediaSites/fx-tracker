//! CLI [`EventSink`] implementations.
//!
//! - [`NdjsonSink`] backs `stream`: it serializes the price events the streamer
//!   produces and no-ops the strategy-watcher methods (no embedded watcher).
//! - [`SignalSink`] backs the `watch` daemon: it serializes the *strategy
//!   signal* events the embedded [`StrategyWatcher`] produces — pattern matches,
//!   ticks, status, errors, and match-status updates — plus any price/health
//!   events, each as one NDJSON line with an `"event"` discriminator.
//!
//! [`StrategyWatcher`]: wickd_core::strategy::StrategyWatcher
//!
//! Both share the same line shape: `serde_json` value of the payload with an
//! `"event"` key inserted, then `println!`'d (line-atomic via the stdout lock).

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;

use wickd_core::event_sink::EventSink;
use wickd_core::oanda::streaming::{PriceUpdate, StreamError, StreamHealthStatus};
use wickd_core::strategy::{
    MatchStatusUpdateEvent, PatternMatchEvent, StrategyErrorEvent, StrategyStatusEvent,
    WatcherTickEvent,
};

use crate::feed::{self, Format};

/// An [`EventSink`] that drops every event. Used by `wickd watch --format
/// human` as the base sink under the strategy-signal alert layer, so the raw
/// NDJSON signal firehose stays silent and only the human fire lines print
/// (AGT-619).
pub struct NoopSink;

impl EventSink for NoopSink {
    fn price_update(&self, _event: &PriceUpdate) {}
    fn stream_error(&self, _event: &StreamError) {}
    fn stream_health(&self, _event: &StreamHealthStatus) {}
    fn pattern_matched(&self, _event: &PatternMatchEvent) {}
    fn strategy_status(&self, _event: &StrategyStatusEvent) {}
    fn strategy_error(&self, _event: &StrategyErrorEvent) {}
    fn match_status_update(&self, _event: &MatchStatusUpdateEvent) {}
    fn watcher_tick(&self, _event: &WatcherTickEvent) {}
}

/// [`EventSink`] for `wickd stream` (AGT-614).
///
/// Unlike the other sinks, writes go through a locked [`std::io::Write`] call
/// whose `Result` is inspected: when the consumer of our stdout goes away
/// (e.g. `wickd stream | head` closes its read end once it has the lines it
/// wants), the next write fails with [`std::io::ErrorKind::BrokenPipe`] rather
/// than panicking `println!`'s internal `.unwrap()`. On that error we flip
/// `closed` (so further emits are silently skipped instead of failing again)
/// and wake the paired [`tokio::sync::Notify`] handed back by [`NdjsonSink::new`],
/// which `stream`'s run loop races against `ctrl_c()` so the process exits 0
/// instead of hanging forever waiting on a Ctrl-C that will never come.
///
/// When constructed with [`NdjsonSink::with_hub`] the sink also publishes each
/// finished line into a [`tokio::sync::broadcast`] channel (AGT-615), which the
/// socket hub (`crate::stream_hub`) fans out to every connected client. stdout
/// still receives the identical line, so the single-consumer path is unchanged.
pub struct NdjsonSink {
    closed: Arc<AtomicBool>,
    consumer_gone: Arc<tokio::sync::Notify>,
    /// Present only for `wickd stream`'s socket hub: every emitted line is also
    /// broadcast here for fan-out. `None` for the other sinks (e.g. `AlertSink`).
    hub: Option<tokio::sync::broadcast::Sender<String>>,
}

impl NdjsonSink {
    /// Build a sink plus the [`tokio::sync::Notify`] handle the caller should
    /// race against `ctrl_c()` to detect the consumer's pipe closing. No socket
    /// fan-out (stdout only).
    pub fn new() -> (Self, Arc<tokio::sync::Notify>) {
        let consumer_gone = Arc::new(tokio::sync::Notify::new());
        (
            Self {
                closed: Arc::new(AtomicBool::new(false)),
                consumer_gone: consumer_gone.clone(),
                hub: None,
            },
            consumer_gone,
        )
    }

    /// Like [`NdjsonSink::new`] but also fans every emitted line out to `hub`
    /// (the socket-hub broadcast, AGT-615) in addition to stdout.
    pub fn with_hub(
        hub: tokio::sync::broadcast::Sender<String>,
    ) -> (Self, Arc<tokio::sync::Notify>) {
        let consumer_gone = Arc::new(tokio::sync::Notify::new());
        (
            Self {
                closed: Arc::new(AtomicBool::new(false)),
                consumer_gone: consumer_gone.clone(),
                hub: Some(hub),
            },
            consumer_gone,
        )
    }

    fn emit<T: Serialize>(&self, event: &str, payload: &T) {
        if self.closed.load(Ordering::Relaxed) {
            return; // Consumer already gone; don't keep trying to write.
        }

        let Some(line) = wickd_core::ndjson::event_line(event, payload) else { return };

        // Fan the identical line out to any socket-hub clients (AGT-615). Send
        // never blocks — a lagging client is dropped on its own receiver — so
        // this can't stall the OANDA read loop. No subscribers is not an error.
        if let Some(hub) = &self.hub {
            let _ = hub.send(line.clone());
        }

        // Lock + write directly (rather than `println!`) so a broken pipe
        // surfaces as an `Err` we can act on instead of a panic.
        let mut stdout = std::io::stdout().lock();
        if let Err(e) = writeln!(stdout, "{line}") {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                self.closed.store(true, Ordering::Relaxed);
                self.consumer_gone.notify_one();
            }
            // Any other write error: nothing more we can do from a sink method
            // (EventSink is sync/non-fallible) — just drop the line.
        }
    }
}

impl EventSink for NdjsonSink {
    fn price_update(&self, event: &PriceUpdate) {
        self.emit("price-update", event);
    }
    fn stream_error(&self, event: &StreamError) {
        self.emit("stream-error", event);
    }
    fn stream_health(&self, event: &StreamHealthStatus) {
        self.emit("stream-health", event);
    }

    // Not produced by `stream` (no embedded watcher) — intentionally ignored.
    fn pattern_matched(&self, _event: &PatternMatchEvent) {}
    fn strategy_status(&self, _event: &StrategyStatusEvent) {}
    fn strategy_error(&self, _event: &StrategyErrorEvent) {}
    fn match_status_update(&self, _event: &MatchStatusUpdateEvent) {}
    fn watcher_tick(&self, _event: &WatcherTickEvent) {}
}

/// [`EventSink`] for the `watch` daemon.
///
/// The inverse emphasis of [`NdjsonSink`]: the strategy-watcher events are the
/// point (`pattern-matched`, `watcher-tick`, `strategy-status`, `strategy-error`,
/// `match-status-update`), and the price/health events from any attached stream
/// are passed through too. Every event becomes one NDJSON line on stdout with an
/// `"event"` discriminator. Monitoring only — the sink never places an order.
pub struct SignalSink;

impl SignalSink {
    /// Render `payload` to a single NDJSON line: the payload object with an
    /// `"event"` discriminator inserted. Kept separate from [`Self::emit`] so it
    /// is testable without capturing stdout.
    fn render<T: Serialize>(event: &str, payload: &T) -> Option<String> {
        wickd_core::ndjson::event_line(event, payload)
    }

    pub(crate) fn emit<T: Serialize>(&self, event: &str, payload: &T) {
        // `println!` takes the stdout lock per call, so concurrent emits from
        // the watcher's tasks stay line-atomic.
        if let Some(line) = Self::render(event, payload) {
            println!("{line}");
        }
    }
}

impl EventSink for SignalSink {
    // Strategy signals — the reason this sink exists.
    fn pattern_matched(&self, event: &PatternMatchEvent) {
        self.emit("pattern-matched", event);
    }
    fn strategy_status(&self, event: &StrategyStatusEvent) {
        self.emit("strategy-status", event);
    }
    fn strategy_error(&self, event: &StrategyErrorEvent) {
        self.emit("strategy-error", event);
    }
    fn match_status_update(&self, event: &MatchStatusUpdateEvent) {
        self.emit("match-status-update", event);
    }
    fn watcher_tick(&self, event: &WatcherTickEvent) {
        self.emit("watcher-tick", event);
    }

    // Price/health pass-through (only produced if a price stream is attached).
    fn price_update(&self, event: &PriceUpdate) {
        self.emit("price-update", event);
    }
    fn stream_error(&self, event: &StreamError) {
        self.emit("stream-error", event);
    }
    fn stream_health(&self, event: &StreamHealthStatus) {
        self.emit("stream-health", event);
    }
}

/// [`EventSink`] for `wickd watch --semi-auto` (AGT-599, trust-ladder Stage 1).
///
/// It behaves exactly like [`SignalSink`] — every event is still emitted as one
/// NDJSON line, so the daemon stays a monitoring stream — but it adds **one**
/// side effect on a tradeable entry signal: it appends a *pending proposal* to
/// the durable store (`~/.wickd/pending.json`, or `store_path` for tests).
///
/// Crucially this sink **never executes an order**. Recording a pending record
/// is the only thing it does with a signal; nothing here imports or calls an
/// order-placement path. A surfaced signal becomes an order only via a separate,
/// explicit `wickd approve <id>` invocation (AC1/AC4).
pub struct SemiAutoSink {
    inner: SignalSink,
    store_path: std::path::PathBuf,
}

impl SemiAutoSink {
    /// Build a semi-auto sink that appends pending proposals to `store_path`.
    pub fn new(store_path: std::path::PathBuf) -> Self {
        Self { inner: SignalSink, store_path }
    }
}

impl EventSink for SemiAutoSink {
    fn pattern_matched(&self, event: &PatternMatchEvent) {
        // Always emit the signal line — monitoring is unchanged.
        self.inner.pattern_matched(event);
        // Record a pending proposal for actionable entry signals. This is a
        // FILE write, never an order. Exit/partial-exit signals and entries
        // without a direction return None and are not recorded.
        if let Some(sig) = crate::pending::pending_from_match(event) {
            match crate::pending::append_at(&self.store_path, &sig) {
                Ok(()) => self.inner.emit("pending-recorded", &sig),
                Err(e) => eprintln!("warning: pending store write failed: {e:#}"),
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

/// [`EventSink`] for `wickd alert run` (AGT-617).
///
/// The inverse emphasis of [`SemiAutoSink`]: instead of a strategy watcher's
/// signals with a price pass-through, this sink's *entire* reason for being
/// is the price stream — `pattern_matched`/`watcher_tick`/etc. are no-ops
/// (there is no embedded strategy watcher here), and every `price_update`
/// drives the alert evaluator. Like [`NdjsonSink`] it still emits the raw
/// `price-update`/`stream-error`/`stream-health` lines unchanged; on top of
/// that, for each tick it re-loads the alert store, evaluates every alert for
/// the ticked instrument via [`crate::alert::evaluate`], persists any
/// status change (fire or re-arm) back to disk, and delivers one line per fire.
/// Monitoring only — this sink never places an order.
///
/// AGT-619: `format` selects how fires (and the passthrough stream) are
/// delivered. In [`Format::Ndjson`] (default) it behaves exactly as before —
/// raw `price-update`/`stream-*` NDJSON plus an `alert-fired` line per fire. In
/// [`Format::Human`] it suppresses the per-tick firehose and prints one
/// human-readable [`feed::price_level_line`] per fire instead (stream errors
/// still surface, to stderr).
pub struct AlertSink {
    inner: NdjsonSink,
    store_path: std::path::PathBuf,
    format: Format,
    /// Durable alert-queue path (AGT-620). When set, every fired price-level
    /// alert is ALSO appended to `~/.wickd/alert-queue.ndjson` for an agent to
    /// poll. Price-level fires are NOT promotable (no order intent). `None` (the
    /// plain `new` constructor) keeps the sink queue-free, e.g. in unit tests.
    queue: Option<std::path::PathBuf>,
}

impl AlertSink {
    /// Build an alert sink that evaluates against, and persists to, `store_path`,
    /// delivering fires in `format`.
    ///
    /// `alert run` doesn't need the broken-pipe/consumer-gone notification
    /// `NdjsonSink::new` also hands back (that's `stream`'s clean-exit
    /// mechanism, AGT-614 AC4) — it discards the `Notify` handle and keeps
    /// running until `ctrl_c()` regardless of whether anything reads stdout.
    pub fn new(store_path: std::path::PathBuf, format: Format) -> Self {
        let (inner, _consumer_gone) = NdjsonSink::new();
        Self { inner, store_path, format, queue: None }
    }

    /// Builder: also durably append every fired alert to the alert queue at
    /// `queue` (AGT-620 AC1/AC2). Off by default (unit tests stay queue-free).
    pub fn with_queue(mut self, queue: std::path::PathBuf) -> Self {
        self.queue = Some(queue);
        self
    }

    /// Re-load the store, evaluate every alert for `update.instrument` against
    /// this tick, persist any change, and emit `alert-fired` lines for fires.
    /// Isolated from `price_update` so a store I/O error can be reported
    /// (`Result`) without the `EventSink` trait's synchronous, infallible
    /// methods leaking that plumbing.
    fn evaluate_tick(&self, update: &PriceUpdate) -> anyhow::Result<()> {
        let tick = crate::alert::PriceTick::try_from(update)?;
        let mut store = crate::alert::load_at(&self.store_path)?;
        // `evaluate` unconditionally records the tick's price on the alert
        // (`last_price`) even when it neither fires nor re-arms — that
        // baseline has to survive the reload on the *next* tick or crossing
        // detection breaks (every tick would look like a fresh first
        // observation). So any alert this tick touched is "changed", not
        // just ones that fired or flipped status.
        let mut changed = false;

        for alert in store.alerts.iter_mut().filter(|a| a.instrument == update.instrument) {
            changed = true;
            if let Some(fired) = crate::alert::evaluate(alert, tick) {
                match self.format {
                    Format::Ndjson => self.inner.emit(
                        "alert-fired",
                        &serde_json::json!({
                            "alert_id": fired.alert_id,
                            "instrument": update.instrument,
                            "level": fired.level,
                            "direction": fired.direction,
                            "price": fired.price,
                            "time": update.time,
                        }),
                    ),
                    Format::Human => {
                        println!("{}", feed::price_level_line(&update.instrument, &fired, &update.time));
                    }
                }

                // AGT-620: durably queue the fire (in either format) so an agent
                // can poll it. A price-level fire is deliberately NOT promotable
                // — a bare level crossing carries no side/size to build an order.
                if let Some(queue) = &self.queue {
                    let entry = crate::alert_queue::QueuedAlert::price_level(
                        update.time.clone(),
                        update.instrument.clone(),
                        fired.level.to_string(),
                        fired.direction,
                        fired.price.to_string(),
                    );
                    if let Err(e) = crate::alert_queue::append_at(queue, &entry) {
                        eprintln!("warning: alert queue write failed: {e:#}");
                    }
                }
            }
        }

        if changed {
            crate::alert::save_at(&self.store_path, &store)?;
        }
        Ok(())
    }
}

impl EventSink for AlertSink {
    fn price_update(&self, event: &PriceUpdate) {
        // NDJSON mode echoes every raw tick; the human feed shows only fires,
        // so the per-tick firehose is suppressed there.
        if self.format == Format::Ndjson {
            self.inner.price_update(event);
        }
        if let Err(e) = self.evaluate_tick(event) {
            eprintln!("warning: alert evaluation failed: {e:#}");
        }
    }
    fn stream_error(&self, event: &StreamError) {
        match self.format {
            Format::Ndjson => self.inner.stream_error(event),
            // Keep the human feed to fires, but never swallow a stream failure.
            Format::Human => eprintln!("stream error [{:?}]: {}", event.error_type, event.message),
        }
    }
    fn stream_health(&self, event: &StreamHealthStatus) {
        // Health heartbeats are pure telemetry — NDJSON only; not feed-worthy.
        if self.format == Format::Ndjson {
            self.inner.stream_health(event);
        }
    }

    // Not produced by `alert run` (no embedded strategy watcher) — intentionally ignored.
    fn pattern_matched(&self, _event: &PatternMatchEvent) {}
    fn strategy_status(&self, _event: &StrategyStatusEvent) {}
    fn strategy_error(&self, _event: &StrategyErrorEvent) {}
    fn match_status_update(&self, _event: &MatchStatusUpdateEvent) {}
    fn watcher_tick(&self, _event: &WatcherTickEvent) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use wickd_core::shared::PositionDirection;
    use wickd_core::strategy::{PatternMatch, WatcherStatus};
    use rust_decimal_macros::dec;
    use serde_json::Value;

    // AC4: the loop -> signal path. The watcher calls `pattern_matched` with a
    // `PatternMatchEvent`; assert the daemon sink serializes it to one JSON line
    // carrying the `pattern-matched` discriminator and the match payload.
    #[test]
    fn signal_sink_renders_pattern_match_line() {
        let pattern = PatternMatch::entry(
            "user1".to_string(),
            "cfg-1".to_string(),
            "EUR_USD".to_string(),
            PositionDirection::Long,
            dec!(1.0850),
            Some(dec!(1.0800)),
            Some(dec!(1.0950)),
            Some(dec!(1000)),
            "fast SMA crossed above slow".to_string(),
            None,
            false,
        );
        let event = PatternMatchEvent {
            pattern_match: pattern,
            strategy_name: "ma-crossover".to_string(),
            timeframe: "H1".to_string(),
        };

        let line = SignalSink::render("pattern-matched", &event).expect("line");
        // One line, no embedded newline (NDJSON invariant).
        assert!(!line.contains('\n'));

        let v: Value = serde_json::from_str(&line).expect("valid json");
        assert_eq!(v["event"], "pattern-matched");
        assert_eq!(v["strategy_name"], "ma-crossover");
        assert_eq!(v["timeframe"], "H1");
        assert_eq!(v["pattern_match"]["instrument"], "EUR_USD");
        assert_eq!(v["pattern_match"]["match_type"], "entry");
        assert_eq!(v["pattern_match"]["direction"], "long");
    }

    #[test]
    fn signal_sink_renders_watcher_tick_line() {
        let event = WatcherTickEvent {
            config_id: "cfg-1".to_string(),
            instrument: "EUR_USD".to_string(),
            timeframe: "H1".to_string(),
            candle_time: "2024-01-01T00:00:00+00:00".to_string(),
            close_price: "1.0850".to_string(),
            signal_result: "Hold".to_string(),
        };
        let line = SignalSink::render("watcher-tick", &event).expect("line");
        let v: Value = serde_json::from_str(&line).expect("valid json");
        assert_eq!(v["event"], "watcher-tick");
        assert_eq!(v["instrument"], "EUR_USD");
        assert_eq!(v["signal_result"], "Hold");
    }

    #[test]
    fn signal_sink_renders_status_line() {
        let event = StrategyStatusEvent {
            config_id: "cfg-1".to_string(),
            status: WatcherStatus::Running,
            message: None,
        };
        let line = SignalSink::render("strategy-status", &event).expect("line");
        let v: Value = serde_json::from_str(&line).expect("valid json");
        assert_eq!(v["event"], "strategy-status");
        assert_eq!(v["status"], "running");
    }

    // AC1: a signal arriving at the semi-auto sink writes a PENDING record and
    // nothing else — there is no order-submission path reachable from here (the
    // sink holds no OANDA client). Feed an entry match; assert the pending store
    // gained exactly one pending proposal and no live order could have fired.
    #[test]
    fn semi_auto_sink_records_pending_does_not_execute() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static C: AtomicU64 = AtomicU64::new(0);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "wickd-sink-test-{}-{}.json",
            std::process::id(),
            C.fetch_add(1, Ordering::Relaxed)
        ));

        let sink = SemiAutoSink::new(path.clone());
        let pattern = PatternMatch::entry(
            "wickd-watch".to_string(),
            "cfg-1".to_string(),
            "EUR_USD".to_string(),
            PositionDirection::Long,
            dec!(1.0850),
            Some(dec!(1.0800)),
            Some(dec!(1.0950)),
            Some(dec!(1000)),
            "fast SMA crossed above slow".to_string(),
            None,
            false,
        );
        let event = PatternMatchEvent {
            pattern_match: pattern,
            strategy_name: "ma-crossover".to_string(),
            timeframe: "H1".to_string(),
        };

        // The signal path's only effect: a pending record on disk.
        sink.pattern_matched(&event);

        let pending = crate::pending::list_at(&path).unwrap();
        assert_eq!(pending.len(), 1, "exactly one pending proposal recorded");
        assert_eq!(pending[0].instrument, "EUR_USD");
        assert_eq!(pending[0].side, "long");
        assert_eq!(pending[0].status, crate::pending::STATUS_PENDING);

        let _ = std::fs::remove_file(&path);
    }

    fn temp_alert_path() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static C: AtomicU64 = AtomicU64::new(0);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "wickd-alertsink-test-{}-{}.json",
            std::process::id(),
            C.fetch_add(1, Ordering::Relaxed)
        ));
        p
    }

    fn price(instrument: &str, bid: &str, ask: &str) -> PriceUpdate {
        PriceUpdate {
            instrument: instrument.to_string(),
            bid: bid.to_string(),
            ask: ask.to_string(),
            spread: "0".to_string(),
            time: "2026-06-30T00:00:00Z".to_string(),
            tradeable: true,
        }
    }

    // AGT-617 AC2/AC4: the sink drives the alert evaluator on every tick and
    // persists the fire back to the store — status flips armed -> fired on
    // disk, observable via `wickd alert list`, without any network call.
    #[test]
    fn alert_sink_fires_and_persists_status() {
        let path = temp_alert_path();
        let alert = crate::alert::Alert::new(
            "EUR_USD".to_string(),
            dec!(1.0900),
            crate::alert::Direction::CrossUp,
            crate::alert::Source::Mid,
            dec!(5),
        );
        let id = alert.id.clone();
        crate::alert::add_at(&path, &alert).unwrap();

        let sink = AlertSink::new(path.clone(), Format::Ndjson);
        // Seed below the level, then cross above it.
        sink.price_update(&price("EUR_USD", "1.0850", "1.0850"));
        sink.price_update(&price("EUR_USD", "1.0905", "1.0905"));

        let stored = crate::alert::list_at(&path)
            .unwrap()
            .into_iter()
            .find(|a| a.id == id)
            .unwrap();
        assert_eq!(stored.status, crate::alert::Status::Fired);

        let _ = std::fs::remove_file(&path);
    }

    // AGT-619: the human-feed variant still drives the evaluator and persists
    // the fire to disk exactly like NDJSON mode — only the delivery of the fire
    // line differs, not the store side effect.
    #[test]
    fn alert_sink_human_format_still_fires_and_persists() {
        let path = temp_alert_path();
        let alert = crate::alert::Alert::new(
            "EUR_USD".to_string(),
            dec!(1.0900),
            crate::alert::Direction::CrossUp,
            crate::alert::Source::Mid,
            dec!(5),
        );
        let id = alert.id.clone();
        crate::alert::add_at(&path, &alert).unwrap();

        let sink = AlertSink::new(path.clone(), Format::Human);
        sink.price_update(&price("EUR_USD", "1.0850", "1.0850"));
        sink.price_update(&price("EUR_USD", "1.0905", "1.0905"));

        let stored = crate::alert::list_at(&path)
            .unwrap()
            .into_iter()
            .find(|a| a.id == id)
            .unwrap();
        assert_eq!(stored.status, crate::alert::Status::Fired);

        let _ = std::fs::remove_file(&path);
    }

    // A tick for a different instrument than any stored alert is a no-op —
    // the store is left untouched (no spurious write/status change).
    #[test]
    fn alert_sink_ignores_unrelated_instrument() {
        let path = temp_alert_path();
        let alert = crate::alert::Alert::new(
            "EUR_USD".to_string(),
            dec!(1.0900),
            crate::alert::Direction::CrossUp,
            crate::alert::Source::Mid,
            dec!(5),
        );
        let id = alert.id.clone();
        crate::alert::add_at(&path, &alert).unwrap();

        let sink = AlertSink::new(path.clone(), Format::Ndjson);
        sink.price_update(&price("GBP_USD", "1.2500", "1.2500"));
        sink.price_update(&price("GBP_USD", "1.3000", "1.3000"));

        let stored = crate::alert::list_at(&path)
            .unwrap()
            .into_iter()
            .find(|a| a.id == id)
            .unwrap();
        assert_eq!(stored.status, crate::alert::Status::Armed);
        assert!(stored.last_price.is_none(), "EUR_USD alert never saw a matching tick");

        let _ = std::fs::remove_file(&path);
    }
}
