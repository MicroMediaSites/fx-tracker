//! CLI shim over [`wickd_core::events`] (moved to wickd-core by AGT-651 so
//! the desktop app injects the same calendars through
//! `ScriptedStrategy::for_host`). This module only adapts the core's
//! `String` errors to the CLI's `anyhow` conventions — sources, schema, and
//! precedence are documented on the core module.

use anyhow::Result;
use chrono::{DateTime, Utc};
use wickd_core::backtest::SurpriseCalendar;

/// Load the surprise calendar from `~/.wickd/calendar/`
/// (`wickd_core::events::calendar_dir`). Missing dir = empty feed;
/// malformed CSV = hard error naming the file.
pub fn load_surprise_calendar() -> Result<SurpriseCalendar> {
    wickd_core::events::load_surprise_calendar().map_err(anyhow::Error::msg)
}

/// Load the event calendar for `instrument`: the user file when present,
/// else the bundled schedule. Returns event times plus the source label
/// ("user" | "bundled") so runs can be self-describing.
pub fn load_for_instrument(instrument: &str) -> Result<(Vec<DateTime<Utc>>, &'static str)> {
    wickd_core::events::load_for_instrument(instrument).map_err(anyhow::Error::msg)
}
