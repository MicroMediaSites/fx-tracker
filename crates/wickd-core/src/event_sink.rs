//! [`EventSink`] — the abstraction that replaces direct Tauri event emission
//! in the strategy watchers and the OANDA price streamer.
//!
//! The watchers/streamer hold a `&dyn EventSink` (or `Arc<dyn EventSink>`) and
//! call the typed methods below instead of `app_handle.emit("name", &payload)`.
//! This keeps the trading core free of any UI/transport dependency.
//!
//! Implementations:
//! - `TauriEventSink` (in `src-tauri`) re-emits each event as the identically
//!   named Tauri event and fires OS notifications on pattern matches, so the
//!   desktop UI behaves exactly as before.
//! - `NdjsonSink` (in the `wickd` CLI) serializes each event as one JSON line
//!   to stdout for agents to consume.
//!
//! The trait is intentionally **synchronous and object-safe**: emitting must
//! never block the watcher loop, so implementations push to a channel / re-emit
//! / write a line rather than awaiting.

use crate::oanda::streaming::{PriceUpdate, StreamError, StreamHealthStatus};
use crate::strategy::{
    MatchStatusUpdateEvent, PatternMatchEvent, StrategyErrorEvent, StrategyStatusEvent,
    WatcherTickEvent,
};

/// Receives every real-time event produced by the strategy watchers and the
/// price streamer. See module docs. Event names in the doc comments match the
/// Tauri event names the desktop app has always used.
pub trait EventSink: Send + Sync {
    /// A strategy entry/exit pattern matched (`"pattern-matched"`).
    fn pattern_matched(&self, event: &PatternMatchEvent);
    /// A watcher changed running/stopped/error state (`"strategy-status"`).
    fn strategy_status(&self, event: &StrategyStatusEvent);
    /// A per-instrument watcher error (`"strategy-error"`).
    fn strategy_error(&self, event: &StrategyErrorEvent);
    /// A pattern match's status changed — executed/expired/dismissed
    /// (`"match-status-update"`).
    fn match_status_update(&self, event: &MatchStatusUpdateEvent);
    /// A candle was processed; debug/telemetry tick (`"watcher-tick"`).
    fn watcher_tick(&self, event: &WatcherTickEvent);
    /// A live price tick from the OANDA stream (`"price-update"`).
    fn price_update(&self, event: &PriceUpdate);
    /// A price-stream error / reconnection notice (`"stream-error"`).
    fn stream_error(&self, event: &StreamError);
    /// Price-stream health changed (`"stream-health"`).
    fn stream_health(&self, event: &StreamHealthStatus);
}
