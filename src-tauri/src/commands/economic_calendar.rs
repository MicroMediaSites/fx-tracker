//! Tauri command over the economic-calendar CSV store (`~/.wickd/calendar/`).
//!
//! Read-only and OFFLINE: the wickd CLI owns freshness (`wickd calendar
//! sync`, periodic via the `com.openthink.wickd-calendar` launchd job); the
//! app only reads whatever the store holds. No network — the offline-boot
//! e2e specs (zero non-localhost requests) must stay green. A missing or
//! stale store degrades to an empty/partial list, never an error dialog.

use serde::Serialize;
use wickd_core::calendar_store::{read_range, CalendarEvent};
use wickd_core::events::calendar_dir;

/// One event row for the UI: the stored CSV row plus the parsed release
/// instant so the frontend never re-implements the store's UTC convention.
#[derive(Debug, Serialize)]
pub struct EconomicCalendarEvent {
    pub date: String,
    pub time: String,
    /// Release instant as Unix seconds (UTC); None for rows whose
    /// date/time don't parse (kept out of the response entirely).
    pub time_unix: i64,
    pub currency: String,
    pub event: String,
    pub impact: String,
    pub actual: String,
    pub forecast: String,
    pub previous: String,
}

impl EconomicCalendarEvent {
    fn from_row(row: CalendarEvent) -> Option<Self> {
        let time_unix = row.time_unix()?;
        Some(Self {
            date: row.date,
            time: row.time,
            time_unix,
            currency: row.currency,
            event: row.event,
            impact: row.impact,
            actual: row.actual,
            forecast: row.forecast,
            previous: row.previous,
        })
    }
}

/// Calendar rows for `[now - days_back, now + days_ahead]` (UTC days),
/// sorted by release time. Filtering (impact, currencies) is the
/// frontend's job — a week of events is small and client-side filter
/// chips need the full set anyway.
#[tauri::command]
pub fn get_economic_calendar(days_back: u32, days_ahead: u32) -> Result<Vec<EconomicCalendarEvent>, String> {
    let dir = calendar_dir()?;
    let now = chrono::Utc::now();
    let from = (now - chrono::Duration::days(days_back as i64)).date_naive();
    let to = (now + chrono::Duration::days(days_ahead as i64)).date_naive();
    let rows = read_range(&dir, from, to)?;
    Ok(rows.into_iter().filter_map(EconomicCalendarEvent::from_row).collect())
}
