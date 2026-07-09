//! Price streaming commands — hub-first since AGT-652.
//!
//! Same command names and Tauri events as the old in-app streamer
//! (`subscribe_to_prices` / `unsubscribe_from_prices`, emitting
//! `price-update` / `stream-error` / `stream-health`), but the ticks now come
//! from the wickd stream hub via [`crate::hub_stream`]: attach to a running
//! `wickd stream`, degrade uncovered instruments to a direct subscription, or
//! host the hub when nothing else is streaming.

use tauri::State;

use crate::commands::trading::is_valid_instrument;
use candlesight_lib::hub_stream::{Cmd, HubStreamSnapshot, HubStreamState};

/// Subscribe to price updates for an instrument (refcounted per instrument).
#[tauri::command]
pub async fn subscribe_to_prices(
    instrument: String,
    hub: State<'_, HubStreamState>,
) -> Result<(), String> {
    if !is_valid_instrument(&instrument) {
        return Err(format!("Invalid instrument format: {}", instrument));
    }
    hub.send(Cmd::Subscribe(instrument))
}

/// Unsubscribe from price updates for an instrument.
#[tauri::command]
pub async fn unsubscribe_from_prices(
    instrument: String,
    hub: State<'_, HubStreamState>,
) -> Result<(), String> {
    hub.send(Cmd::Unsubscribe(instrument))
}

/// Where prices are flowing from (hub client / hub host / direct fallbacks) —
/// the watcher window's stream indicator.
#[tauri::command]
pub fn hub_stream_status(hub: State<'_, HubStreamState>) -> HubStreamSnapshot {
    hub.snapshot()
}
