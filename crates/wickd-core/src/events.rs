//! Economic-calendar feeds for event- and surprise-gated strategies
//! (#290 event proximity, AGT-632 surprise z-scores).
//!
//! Feeds the ABI v3 script accessors `hours_since_event()` /
//! `hours_until_event()` and resolves the ABI v4 surprise-calendar directory
//! (see `crates/wickd/STRATEGY_ABI.md`). Lives in wickd-core (moved from the
//! CLI by AGT-651) so *every* host — `wickd backtest`/`strategy run`, the
//! live watcher, and the desktop app — injects the same calendars through
//! [`ScriptedStrategy::for_host`](crate::backtest::ScriptedStrategy::for_host)
//! instead of each host wiring (or forgetting to wire) them separately.
//!
//! ## Sources, in precedence order
//!
//! 1. `~/.wickd/events.json` — user-supplied, same schema as the bundled
//!    file. A corrupt user file is a hard error (the user asked for it);
//!    a missing one is not.
//! 2. The bundled calendar (`assets/events.json`, compiled in): FOMC, ECB,
//!    and BoE rate decisions plus US NFP and CPI releases, 2022-01 →
//!    2026-06, from published schedules (the file's `sources` field cites
//!    them). Event-gated strategies work out of the box within that span;
//!    candles outside it simply see a growing `hours_since_event()` (or -1
//!    before the first event) — extend via the user file.
//!
//! ## Schema
//!
//! ```json
//! { "events": [ { "time_utc": "2022-01-26T19:00:00Z",
//!                 "currency": "USD", "type": "rate_decision", "name": "FOMC" } ] }
//! ```
//!
//! Events are filtered to the instrument's two currency legs (EUR_USD keeps
//! EUR and USD events, drops GBP), so a BoE decision never gates a EUR_USD
//! strategy.
//!
//! Errors are `String` (wickd-core convention for the backtest surface);
//! the CLI shim (`crates/wickd/src/events.rs`) wraps them in `anyhow`.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::backtest::SurpriseCalendar;
use crate::paths::wickd_data_home;

/// Bundled default calendar — see module docs for coverage and provenance.
const BUNDLED_EVENTS: &str = include_str!("../assets/events.json");

#[derive(Debug, Deserialize)]
struct EventsFile {
    #[serde(default)]
    events: Vec<EventRecord>,
}

#[derive(Debug, Deserialize)]
struct EventRecord {
    /// RFC3339 event time (UTC).
    time_utc: String,
    /// ISO currency the event prices (USD, EUR, GBP, ...).
    currency: String,
}

/// Path to the user-supplied calendar (`~/.wickd/events.json`).
pub fn events_path() -> Result<PathBuf, String> {
    Ok(wickd_data_home()?.join("events.json"))
}

/// Directory of manual monthly ForexFactory CSV exports
/// (`~/.wickd/calendar/*.csv`, header
/// `date,time,currency,event,impact,actual,forecast,previous`) feeding the
/// ABI v4 surprise accessors (`surprise_z()` and siblings). Dropping a new
/// or re-exported month here — including backfilled `actual` values —
/// updates the feed without rebuilding wickd. Seed it from the research
/// corpus: `cp <fx-tracker>/data/calendar/*.csv ~/.wickd/calendar/`.
pub fn calendar_dir() -> Result<PathBuf, String> {
    Ok(wickd_data_home()?.join("calendar"))
}

/// Load the surprise calendar from [`calendar_dir`]. A missing directory is
/// an empty feed (the accessors return their sentinels) — the engine works
/// before the first CSV drop. A malformed CSV is a hard error naming the
/// file, matching the events.json rule: the user asked for it.
pub fn load_surprise_calendar() -> Result<SurpriseCalendar, String> {
    let dir = calendar_dir()?;
    SurpriseCalendar::load_dir(&dir)
        .map_err(|e| format!("loading surprise calendar from {}: {e}", dir.display()))
}

/// Load the event calendar for `instrument`: the user file when present,
/// else the bundled schedule. Returns the sorted-by-parse event times plus
/// the source label ("user" | "bundled") so runs can be self-describing.
pub fn load_for_instrument(instrument: &str) -> Result<(Vec<DateTime<Utc>>, &'static str), String> {
    let path = events_path()?;
    if path.is_file() {
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading event calendar {}: {e}", path.display()))?;
        let events = parse_filtered(&raw, instrument)
            .map_err(|e| format!("parsing event calendar {}: {e}", path.display()))?;
        Ok((events, "user"))
    } else {
        let events = parse_filtered(BUNDLED_EVENTS, instrument)
            .map_err(|e| format!("parsing the bundled event calendar (a wickd build bug): {e}"))?;
        Ok((events, "bundled"))
    }
}

/// Parse a calendar file and keep only events priced in one of the
/// instrument's currency legs. Pure; time order is the caller's concern
/// (`ScriptedStrategy::set_event_calendar` sorts).
fn parse_filtered(raw: &str, instrument: &str) -> Result<Vec<DateTime<Utc>>, String> {
    let file: EventsFile = serde_json::from_str(raw)
        .map_err(|e| format!("event calendar is not valid JSON for the schema: {e}"))?;
    let legs: Vec<&str> = instrument.split('_').collect();
    let mut out = Vec::with_capacity(file.events.len());
    for e in &file.events {
        if !legs.iter().any(|l| l.eq_ignore_ascii_case(&e.currency)) {
            continue;
        }
        let t = DateTime::parse_from_rfc3339(&e.time_utc)
            .map_err(|_| format!("invalid event time_utc '{}'", e.time_utc))?
            .with_timezone(&Utc);
        out.push(t);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_calendar_parses_and_filters_by_instrument_legs() {
        let eur_usd = parse_filtered(BUNDLED_EVENTS, "EUR_USD").unwrap();
        let gbp_usd = parse_filtered(BUNDLED_EVENTS, "GBP_USD").unwrap();
        let usd_only = parse_filtered(BUNDLED_EVENTS, "USD_JPY").unwrap();

        // The bundle spans 2022→2026 with USD (FOMC/NFP/CPI), EUR (ECB), and
        // GBP (BoE) events — every pair keeps its USD leg, so none is empty.
        assert!(!eur_usd.is_empty() && !gbp_usd.is_empty() && !usd_only.is_empty());
        // EUR_USD keeps ECB events and GBP_USD keeps BoE events that a
        // USD-only pair drops — the leg filter is real. (ECB and BoE may
        // hold equally often, so compare against the USD-only baseline
        // rather than each other.)
        assert!(eur_usd.len() > usd_only.len());
        assert!(gbp_usd.len() > usd_only.len());

        // Sanity: a known FOMC decision (2022-01-26 19:00 UTC) is present for
        // any USD pair.
        let fomc = DateTime::parse_from_rfc3339("2022-01-26T19:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(usd_only.contains(&fomc));
    }

    #[test]
    fn filter_is_case_insensitive_on_currency_legs() {
        let raw = r#"{"events":[
            {"time_utc":"2024-01-01T13:30:00Z","currency":"usd","name":"NFP"},
            {"time_utc":"2024-01-02T13:30:00Z","currency":"GBP","name":"BoE"}
        ]}"#;
        let got = parse_filtered(raw, "EUR_USD").unwrap();
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn malformed_calendar_is_a_clear_error() {
        assert!(parse_filtered("not json", "EUR_USD").is_err());
        let bad_time = r#"{"events":[{"time_utc":"tomorrow","currency":"USD"}]}"#;
        let msg = parse_filtered(bad_time, "EUR_USD").err().unwrap();
        assert!(msg.contains("invalid event time_utc"), "message was: {msg}");
    }

    #[test]
    fn empty_and_missing_events_key_are_fine() {
        assert!(parse_filtered(r#"{"events":[]}"#, "EUR_USD").unwrap().is_empty());
        assert!(parse_filtered(r#"{}"#, "EUR_USD").unwrap().is_empty());
    }
}
