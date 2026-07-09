//! `TauriEventSink` — the desktop app's implementation of
//! [`wickd_core::EventSink`].
//!
//! The trading core (watchers, price streamer) was made transport-agnostic by
//! emitting through `EventSink` instead of a Tauri `AppHandle`. This sink
//! restores the exact previous desktop behavior: every event is re-emitted as
//! the identically named Tauri event the React frontend already listens for,
//! and pattern matches additionally fire the OS notification (previously sent
//! inline by the watchers).

use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use wickd_core::event_sink::EventSink;
use wickd_core::oanda::streaming::{PriceUpdate, StreamError, StreamHealthStatus};
use wickd_core::strategy::{
    MatchStatusUpdateEvent, PatternMatchEvent, StrategyErrorEvent, StrategyStatusEvent,
    WatcherTickEvent,
};

use crate::notifications::{send_pattern_match_notification, NotificationClickedPayload};

/// Bridges core events back onto the Tauri event bus (and OS notifications).
pub struct TauriEventSink {
    app: AppHandle,
}

impl TauriEventSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    /// Convenience constructor returning the `Arc<dyn EventSink>` the core
    /// watcher/streamer entry points expect.
    pub fn arc(app: AppHandle) -> Arc<dyn EventSink> {
        Arc::new(Self::new(app))
    }
}

impl EventSink for TauriEventSink {
    fn pattern_matched(&self, event: &PatternMatchEvent) {
        let _ = self.app.emit("pattern-matched", event);

        // Fire the OS notification (this used to live inline in the watchers'
        // `emit_signal`; the mapping is reconstructed verbatim from the event).
        let pm = &event.pattern_match;
        let payload = NotificationClickedPayload {
            match_id: pm.id.clone(),
            instrument: pm.instrument.clone(),
            timeframe: event.timeframe.clone(),
            strategy_id: pm.config_id.clone(),
            strategy_name: event.strategy_name.clone(),
            direction: pm
                .direction
                .as_ref()
                .map(|d| format!("{:?}", d))
                .unwrap_or_else(|| "Exit".to_string()),
            entry_price: pm
                .entry_price
                .map(|p| p.to_string())
                .unwrap_or_else(|| "N/A".to_string()),
            stop_loss: pm
                .stop_loss
                .map(|p| p.to_string())
                .unwrap_or_else(|| "N/A".to_string()),
            take_profit: pm
                .take_profit
                .map(|p| p.to_string())
                .unwrap_or_else(|| "N/A".to_string()),
            match_time: pm.created_at.timestamp_millis(),
        };
        send_pattern_match_notification(self.app.clone(), payload);
    }

    fn strategy_status(&self, event: &StrategyStatusEvent) {
        let _ = self.app.emit("strategy-status", event);
    }

    fn strategy_error(&self, event: &StrategyErrorEvent) {
        let _ = self.app.emit("strategy-error", event);
    }

    fn match_status_update(&self, event: &MatchStatusUpdateEvent) {
        let _ = self.app.emit("match-status-update", event);
    }

    fn watcher_tick(&self, event: &WatcherTickEvent) {
        let _ = self.app.emit("watcher-tick", event);
    }

    fn price_update(&self, event: &PriceUpdate) {
        let _ = self.app.emit("price-update", event);
    }

    fn stream_error(&self, event: &StreamError) {
        let _ = self.app.emit("stream-error", event);
    }

    fn stream_health(&self, event: &StreamHealthStatus) {
        let _ = self.app.emit("stream-health", event);
    }
}
