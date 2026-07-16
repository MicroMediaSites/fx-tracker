//! Tauri command over the CLI's spread-history store (`~/.wickd/spreads.db`).
//!
//! Read-only: the wickd CLI owns the database and does all the sampling; the
//! app only grades its live spread bar against the history. See
//! `candlesight_lib::spread_stats` for the reader and provenance notes.

use candlesight_lib::spread_stats::{list_all, SpreadStatsRow};

/// All instruments' historical spread statistics. Returns an empty list when
/// the CLI has never sampled on this machine (the UI falls back to purple
/// "no history" coloring).
#[tauri::command]
pub fn get_spread_stats() -> Result<Vec<SpreadStatsRow>, String> {
    list_all()
}
