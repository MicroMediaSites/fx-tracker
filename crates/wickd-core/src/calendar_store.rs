//! Read + merge access to the economic-calendar CSV store
//! (`~/.wickd/calendar/YYYY-MM.csv`, header [`FF_CSV_HEADER`]).
//!
//! The store is the single calendar source shared by the strategy engine
//! (event blackout ABI v3, surprise accessors ABI v4 — see
//! [`crate::backtest::surprise`]) and the desktop app's Economic Calendar UI.
//! This module adds:
//!
//! - a row-preserving reader ([`read_range`]) that returns full CSV rows
//!   (the surprise loader is deliberately lossy — it only keeps what the
//!   z-score accessors need),
//! - the ForexFactory weekly-feed fetch + normalize
//!   ([`fetch_feed`] / [`normalize_feed`]), and
//! - the merge that folds a fetched week into the monthly files
//!   ([`merge_into_store`]).
//!
//! Conventions (all inherited from the existing store + parser):
//! - `date`/`time` columns are **naive UTC** (`surprise.rs` parses them with
//!   `NaiveDateTime` and stamps `and_utc()`); the FF feed publishes
//!   ET-offset RFC3339 instants, converted here at normalize time.
//! - Event titles may contain commas: the first 3 and last 4 columns are
//!   fixed, the middle is the title (same positional rule as the parser).
//! - The weekly feed carries NO `actual` column, so the merge is
//!   feed-authoritative for scheduling fields (time shifts, forecast
//!   revisions) but always preserves a non-empty stored `actual`.
//!
//! The merge rewrites whole monthly files atomically (tmp + rename) so a
//! concurrent reader (`SurpriseCalendar::maybe_refresh` fingerprints and
//! re-reads the directory) never observes a half-written file.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::backtest::surprise::FF_CSV_HEADER;

/// Default ForexFactory weekly feed (this week, rolls on Sunday ET). The
/// matching `nextweek` variant does not exist (404), so store coverage
/// beyond the current week comes from re-running the sync as weeks roll.
pub const FF_WEEKLY_FEED_URL: &str = "https://nfs.faireconomy.media/ff_calendar_thisweek.json";

/// One calendar row, exactly as stored (strings preserved verbatim so a
/// read-modify-write round-trip is lossless).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CalendarEvent {
    /// Event date, `YYYY-MM-DD` (UTC).
    pub date: String,
    /// Event time, `HH:MM` (UTC).
    pub time: String,
    /// ISO currency the event prices (USD, EUR, ...).
    pub currency: String,
    /// Event title (may contain commas).
    pub event: String,
    /// `high` | `medium` | `low` | `holiday` (lowercase in the store).
    pub impact: String,
    /// Released value, empty until the release happens (the weekly feed
    /// never carries this — merges preserve it).
    pub actual: String,
    /// Consensus forecast (may be empty).
    pub forecast: String,
    /// Prior release value (may be empty).
    pub previous: String,
}

impl CalendarEvent {
    /// Release instant as Unix seconds, when the date/time parse.
    pub fn time_unix(&self) -> Option<i64> {
        NaiveDateTime::parse_from_str(&format!("{} {}", self.date, self.time), "%Y-%m-%d %H:%M")
            .ok()
            .map(|dt| dt.and_utc().timestamp())
    }

    fn csv_line(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{}",
            self.date,
            self.time,
            self.currency,
            self.event,
            self.impact,
            self.actual,
            self.forecast,
            self.previous
        )
    }

    /// Identity for actual-preservation across time shifts: an event is
    /// "the same release" if date + currency + title match, even when FF
    /// re-times it (tentative → scheduled).
    fn release_key(&self) -> (String, String, String) {
        (self.date.clone(), self.currency.clone(), self.event.clone())
    }
}

/// Merge outcome, JSON-shaped for the CLI.
#[derive(Debug, Default, Serialize)]
pub struct MergeStats {
    /// Rows added that had no release-key match in the store.
    pub added: usize,
    /// Feed rows that replaced a stored row (schedule/forecast refresh);
    /// stored non-empty `actual` values are carried over.
    pub updated: usize,
    /// Stored rows inside the feed window with no feed counterpart, kept
    /// as-is (FF occasionally drops rows; deleting data the strategy layer
    /// may have scored would be worse than a little noise).
    pub kept_unmatched: usize,
    /// Monthly files rewritten.
    pub files_touched: Vec<String>,
    /// Feed window (inclusive dates, UTC).
    pub window_from: String,
    pub window_to: String,
}

/// One event as served by the FF weekly JSON feed.
#[derive(Debug, Deserialize)]
pub struct FeedEvent {
    pub title: String,
    /// ISO currency (the feed calls it `country`).
    pub country: String,
    /// RFC3339 with the feed's local offset (ET).
    pub date: String,
    /// `High` | `Medium` | `Low` | `Holiday`.
    pub impact: String,
    #[serde(default)]
    pub forecast: String,
    #[serde(default)]
    pub previous: String,
}

/// Fetch the weekly feed. Network errors are strings so callers on the CLI
/// path can map them onto their exit-code taxonomy.
pub async fn fetch_feed(url: &str) -> Result<Vec<FeedEvent>, String> {
    let resp = reqwest::get(url)
        .await
        .map_err(|e| format!("fetching {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("fetching {url}: HTTP {}", resp.status()));
    }
    resp.json::<Vec<FeedEvent>>()
        .await
        .map_err(|e| format!("parsing feed JSON from {url}: {e}"))
}

/// Normalize feed events to store rows: ET-offset instants → naive UTC
/// date/time, impact lowercased, `actual` empty. Rows whose timestamp
/// doesn't parse are skipped (mirrors the reader's "data gap, not user
/// error" rule for All-Day/tentative rows).
pub fn normalize_feed(feed: &[FeedEvent]) -> Vec<CalendarEvent> {
    feed.iter()
        .filter_map(|f| {
            let utc: DateTime<Utc> = DateTime::parse_from_rfc3339(&f.date).ok()?.with_timezone(&Utc);
            Some(CalendarEvent {
                date: utc.format("%Y-%m-%d").to_string(),
                time: utc.format("%H:%M").to_string(),
                currency: f.country.trim().to_string(),
                event: f.title.trim().to_string(),
                impact: f.impact.trim().to_lowercase(),
                actual: String::new(),
                forecast: f.forecast.trim().to_string(),
                previous: f.previous.trim().to_string(),
            })
        })
        .collect()
}

/// Parse one monthly CSV into full rows. Same positional comma rule and
/// same skip-don't-fail row handling as the surprise parser; the strict
/// header check is a hard error naming the file (the user asked for it).
pub fn parse_month(contents: &str, path: &Path) -> Result<Vec<CalendarEvent>, String> {
    let mut lines = contents.lines();
    let header = lines.next().unwrap_or("").trim_start_matches('\u{feff}').trim();
    if header != FF_CSV_HEADER {
        return Err(format!(
            "calendar file {} has an unexpected header (expected '{FF_CSV_HEADER}', got '{header}')",
            path.display()
        ));
    }
    let mut rows = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 8 {
            continue;
        }
        let n = parts.len();
        rows.push(CalendarEvent {
            date: parts[0].to_string(),
            time: parts[1].to_string(),
            currency: parts[2].to_string(),
            event: parts[3..n - 4].join(","),
            impact: parts[n - 4].to_string(),
            actual: parts[n - 3].to_string(),
            forecast: parts[n - 2].to_string(),
            previous: parts[n - 1].to_string(),
        });
    }
    Ok(rows)
}

/// Read all rows in `[from, to]` (inclusive, UTC dates) across the monthly
/// files that intersect the range. A missing directory or missing month is
/// an empty result, matching the surprise loader's "missing = empty feed".
pub fn read_range(dir: &Path, from: NaiveDate, to: NaiveDate) -> Result<Vec<CalendarEvent>, String> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    for month in months_in_range(from, to) {
        let path = dir.join(format!("{month}.csv"));
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        let rows = parse_month(&contents, &path)?;
        out.extend(rows.into_iter().filter(|r| {
            r.date.as_str() >= from.format("%Y-%m-%d").to_string().as_str()
                && r.date.as_str() <= to.format("%Y-%m-%d").to_string().as_str()
        }));
    }
    out.sort_by(|a, b| a.date.cmp(&b.date).then(a.time.cmp(&b.time)));
    Ok(out)
}

/// Past releases of one event series (same currency + exact title), newest
/// first, scanning monthly files backward from `before` until `limit` rows
/// are found or `max_months` months have been searched. FF titles are
/// stable across occurrences ("Core CPI m/m" every month), so exact title
/// match IS the series identity — the same key the merge uses.
pub fn read_series_history(
    dir: &Path,
    currency: &str,
    event: &str,
    before: NaiveDate,
    limit: usize,
    max_months: u32,
) -> Result<Vec<CalendarEvent>, String> {
    let mut out: Vec<CalendarEvent> = Vec::new();
    if !dir.is_dir() || limit == 0 {
        return Ok(out);
    }
    let before_str = before.format("%Y-%m-%d").to_string();
    let (mut y, mut m) = (before.year(), before.month());
    for _ in 0..max_months {
        let path = dir.join(format!("{y:04}-{m:02}.csv"));
        if let Ok(contents) = fs::read_to_string(&path) {
            let mut rows: Vec<CalendarEvent> = parse_month(&contents, &path)?
                .into_iter()
                .filter(|r| r.currency == currency && r.event == event && r.date < before_str)
                .collect();
            rows.sort_by(|a, b| b.date.cmp(&a.date).then(b.time.cmp(&a.time)));
            out.extend(rows);
            if out.len() >= limit {
                out.truncate(limit);
                break;
            }
        }
        m -= 1;
        if m == 0 {
            m = 12;
            y -= 1;
        }
    }
    Ok(out)
}

/// `YYYY-MM` labels for every month whose file could hold rows in range.
fn months_in_range(from: NaiveDate, to: NaiveDate) -> Vec<String> {
    let mut months = Vec::new();
    let (mut y, mut m) = (from.year(), from.month());
    loop {
        months.push(format!("{y:04}-{m:02}"));
        if (y, m) >= (to.year(), to.month()) {
            break;
        }
        m += 1;
        if m > 12 {
            m = 1;
            y += 1;
        }
    }
    months
}

/// Merge normalized feed rows into the store under `dir`.
///
/// The feed is authoritative for its own window (min..max feed date): a
/// stored row inside the window with the same release key is replaced by
/// the feed row (schedule shifts and forecast revisions land), carrying
/// over a non-empty stored `actual`. Stored window rows with no feed
/// counterpart are kept ([`MergeStats::kept_unmatched`]). Rows outside the
/// window — and other months entirely — are untouched.
pub fn merge_into_store(dir: &Path, feed_rows: Vec<CalendarEvent>) -> Result<MergeStats, String> {
    let mut stats = MergeStats::default();
    let Some(window_from) = feed_rows.iter().map(|r| r.date.clone()).min() else {
        return Ok(stats); // empty feed: nothing to do
    };
    let window_to = feed_rows
        .iter()
        .map(|r| r.date.clone())
        .max()
        .expect("non-empty");
    stats.window_from = window_from.clone();
    stats.window_to = window_to.clone();

    fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;

    // Group feed rows by monthly file.
    let mut by_month: HashMap<String, Vec<CalendarEvent>> = HashMap::new();
    for row in feed_rows {
        by_month.entry(row.date[..7].to_string()).or_default().push(row);
    }

    let mut months: Vec<String> = by_month.keys().cloned().collect();
    months.sort();
    for month in months {
        let feed_month = by_month.remove(&month).expect("keyed by month");
        let path = dir.join(format!("{month}.csv"));
        let existing = match fs::read_to_string(&path) {
            Ok(contents) => parse_month(&contents, &path)?,
            Err(_) => Vec::new(),
        };

        // Split stored rows: outside the feed window they pass through
        // untouched; inside, they either donate their `actual` to a feed
        // replacement or survive as unmatched.
        let (in_window, outside): (Vec<_>, Vec<_>) = existing
            .into_iter()
            .partition(|r| r.date >= window_from && r.date <= window_to);

        let mut stored: HashMap<(String, String, String), Vec<CalendarEvent>> = HashMap::new();
        for row in in_window {
            stored.entry(row.release_key()).or_default().push(row);
        }

        let mut merged = outside;
        for feed_row in feed_month {
            match stored.get_mut(&feed_row.release_key()).and_then(|v| {
                // Prefer the exact-time twin (two same-titled releases on
                // one day stay distinct); fall back to the first leftover.
                let i = v.iter().position(|r| r.time == feed_row.time).unwrap_or(0);
                if v.is_empty() { None } else { Some(v.remove(i)) }
            }) {
                Some(prior) => {
                    stats.updated += 1;
                    let mut row = feed_row;
                    if !prior.actual.is_empty() {
                        row.actual = prior.actual;
                    }
                    merged.push(row);
                }
                None => {
                    stats.added += 1;
                    merged.push(feed_row);
                }
            }
        }
        // Stored window rows the feed no longer lists.
        for leftovers in stored.into_values() {
            stats.kept_unmatched += leftovers.len();
            merged.extend(leftovers);
        }

        merged.sort_by(|a, b| {
            a.date
                .cmp(&b.date)
                .then(a.time.cmp(&b.time))
                .then(a.currency.cmp(&b.currency))
                .then(a.event.cmp(&b.event))
        });

        let mut contents = String::with_capacity(merged.len() * 64 + FF_CSV_HEADER.len() + 1);
        contents.push_str(FF_CSV_HEADER);
        contents.push('\n');
        for row in &merged {
            contents.push_str(&row.csv_line());
            contents.push('\n');
        }

        // Atomic within the same directory so the fingerprinting reader
        // never sees a torn file.
        let tmp = dir.join(format!(".{month}.csv.tmp"));
        fs::write(&tmp, &contents).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
        fs::rename(&tmp, &path).map_err(|e| format!("renaming {} → {}: {e}", tmp.display(), path.display()))?;
        stats.files_touched.push(format!("{month}.csv"));
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(date: &str, time: &str, currency: &str, event: &str, impact: &str, actual: &str, forecast: &str) -> CalendarEvent {
        CalendarEvent {
            date: date.into(),
            time: time.into(),
            currency: currency.into(),
            event: event.into(),
            impact: impact.into(),
            actual: actual.into(),
            forecast: forecast.into(),
            previous: String::new(),
        }
    }

    #[test]
    fn normalize_converts_et_offsets_to_naive_utc() {
        let feed = vec![FeedEvent {
            title: "Core CPI m/m".into(),
            country: "USD".into(),
            date: "2026-07-14T08:30:00-04:00".into(),
            impact: "High".into(),
            forecast: "0.3%".into(),
            previous: "0.2%".into(),
        }];
        let rows = normalize_feed(&feed);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].date, "2026-07-14");
        assert_eq!(rows[0].time, "12:30"); // 08:30 ET = 12:30 UTC
        assert_eq!(rows[0].impact, "high");
        assert_eq!(rows[0].actual, "");
    }

    #[test]
    fn normalize_skips_unparseable_timestamps() {
        let feed = vec![FeedEvent {
            title: "Tentative Thing".into(),
            country: "EUR".into(),
            date: "All Day".into(),
            impact: "Low".into(),
            forecast: String::new(),
            previous: String::new(),
        }];
        assert!(normalize_feed(&feed).is_empty());
    }

    #[test]
    fn merge_adds_new_rows_and_writes_sorted_atomic_file() {
        let dir = tempfile::tempdir().unwrap();
        let stats = merge_into_store(
            dir.path(),
            vec![
                ev("2026-07-15", "12:30", "USD", "Core CPI m/m", "high", "", "0.3%"),
                ev("2026-07-14", "01:30", "AUD", "Employment Change", "high", "", "20.1K"),
            ],
        )
        .unwrap();
        assert_eq!(stats.added, 2);
        assert_eq!(stats.updated, 0);
        let contents = std::fs::read_to_string(dir.path().join("2026-07.csv")).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines[0], FF_CSV_HEADER);
        assert!(lines[1].starts_with("2026-07-14,01:30,AUD"), "sorted by date/time: {lines:?}");
        assert!(std::fs::read_dir(dir.path()).unwrap().count() == 1, "no tmp file left behind");
    }

    #[test]
    fn merge_updates_forecast_but_preserves_stored_actual() {
        let dir = tempfile::tempdir().unwrap();
        merge_into_store(
            dir.path(),
            vec![ev("2026-07-14", "12:30", "USD", "Core CPI m/m", "high", "0.4%", "0.2%")],
        )
        .unwrap();
        // Feed refresh re-lists the event with a revised forecast and no actual.
        let stats = merge_into_store(
            dir.path(),
            vec![ev("2026-07-14", "12:30", "USD", "Core CPI m/m", "high", "", "0.3%")],
        )
        .unwrap();
        assert_eq!(stats.updated, 1);
        let rows = read_range(
            dir.path(),
            NaiveDate::from_ymd_opt(2026, 7, 14).unwrap(),
            NaiveDate::from_ymd_opt(2026, 7, 14).unwrap(),
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].forecast, "0.3%", "feed forecast wins");
        assert_eq!(rows[0].actual, "0.4%", "stored actual survives");
    }

    #[test]
    fn merge_replaces_time_shifted_rows_instead_of_duplicating() {
        let dir = tempfile::tempdir().unwrap();
        merge_into_store(
            dir.path(),
            vec![ev("2026-07-16", "13:00", "USD", "FOMC Member Speaks", "low", "", "")],
        )
        .unwrap();
        // FF re-times the speech within the same day.
        let stats = merge_into_store(
            dir.path(),
            vec![ev("2026-07-16", "15:30", "USD", "FOMC Member Speaks", "low", "", "")],
        )
        .unwrap();
        assert_eq!(stats.updated, 1, "time shift is an update, not a duplicate");
        let rows = read_range(
            dir.path(),
            NaiveDate::from_ymd_opt(2026, 7, 16).unwrap(),
            NaiveDate::from_ymd_opt(2026, 7, 16).unwrap(),
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].time, "15:30");
    }

    #[test]
    fn merge_leaves_rows_outside_the_feed_window_untouched() {
        let dir = tempfile::tempdir().unwrap();
        // Backfilled row early in the month, outside the feed's window.
        merge_into_store(
            dir.path(),
            vec![ev("2026-07-01", "00:30", "JPY", "Final Manufacturing PMI", "low", "54.8", "54.9")],
        )
        .unwrap();
        let stats = merge_into_store(
            dir.path(),
            vec![ev("2026-07-14", "12:30", "USD", "Core CPI m/m", "high", "", "0.3%")],
        )
        .unwrap();
        assert_eq!(stats.added, 1);
        assert_eq!(stats.kept_unmatched, 0, "out-of-window rows aren't 'unmatched', they're untouched");
        let rows = read_range(
            dir.path(),
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].actual, "54.8", "backfilled actual untouched");
    }

    #[test]
    fn merge_keeps_stored_window_rows_the_feed_dropped() {
        let dir = tempfile::tempdir().unwrap();
        merge_into_store(
            dir.path(),
            vec![
                ev("2026-07-14", "12:30", "USD", "Core CPI m/m", "high", "", "0.3%"),
                ev("2026-07-14", "14:00", "USD", "Crude Oil Inventories", "low", "", ""),
            ],
        )
        .unwrap();
        // Refresh no longer lists the inventories row but covers the date.
        let stats = merge_into_store(
            dir.path(),
            vec![ev("2026-07-14", "12:30", "USD", "Core CPI m/m", "high", "", "0.3%")],
        )
        .unwrap();
        assert_eq!(stats.kept_unmatched, 1);
        let rows = read_range(
            dir.path(),
            NaiveDate::from_ymd_opt(2026, 7, 14).unwrap(),
            NaiveDate::from_ymd_opt(2026, 7, 14).unwrap(),
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn merge_spanning_two_months_touches_both_files() {
        let dir = tempfile::tempdir().unwrap();
        let stats = merge_into_store(
            dir.path(),
            vec![
                ev("2026-07-31", "12:30", "USD", "Core PCE Price Index m/m", "high", "", "0.2%"),
                ev("2026-08-01", "01:45", "CNY", "RatingDog Manufacturing PMI", "low", "", "50.1"),
            ],
        )
        .unwrap();
        assert_eq!(stats.files_touched, vec!["2026-07.csv", "2026-08.csv"]);
    }

    #[test]
    fn read_range_round_trips_comma_titles() {
        let dir = tempfile::tempdir().unwrap();
        merge_into_store(
            dir.path(),
            vec![ev("2026-07-14", "12:30", "GBP", "MPC Member Pill, Ramsden Speak", "medium", "", "")],
        )
        .unwrap();
        let rows = read_range(
            dir.path(),
            NaiveDate::from_ymd_opt(2026, 7, 14).unwrap(),
            NaiveDate::from_ymd_opt(2026, 7, 14).unwrap(),
        )
        .unwrap();
        assert_eq!(rows[0].event, "MPC Member Pill, Ramsden Speak");
    }

    #[test]
    fn series_history_walks_backward_across_months_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        merge_into_store(
            dir.path(),
            vec![
                ev("2026-05-13", "12:30", "USD", "Core CPI m/m", "high", "0.2%", "0.3%"),
                ev("2026-06-10", "12:30", "USD", "Core CPI m/m", "high", "0.3%", "0.3%"),
                ev("2026-06-10", "12:30", "USD", "CPI m/m", "high", "0.2%", "0.2%"), // different series
                ev("2026-07-14", "12:30", "USD", "Core CPI m/m", "high", "0.4%", "0.2%"),
            ],
        )
        .unwrap();
        let hist = read_series_history(
            dir.path(),
            "USD",
            "Core CPI m/m",
            NaiveDate::from_ymd_opt(2026, 7, 17).unwrap(),
            10,
            12,
        )
        .unwrap();
        let dates: Vec<&str> = hist.iter().map(|r| r.date.as_str()).collect();
        assert_eq!(dates, vec!["2026-07-14", "2026-06-10", "2026-05-13"]);
        assert!(hist.iter().all(|r| r.event == "Core CPI m/m"));
    }

    #[test]
    fn series_history_respects_limit_and_excludes_the_before_date_row() {
        let dir = tempfile::tempdir().unwrap();
        merge_into_store(
            dir.path(),
            vec![
                ev("2026-07-01", "12:30", "USD", "Core CPI m/m", "high", "0.2%", "0.2%"),
                ev("2026-07-14", "12:30", "USD", "Core CPI m/m", "high", "0.4%", "0.2%"),
                ev("2026-07-17", "12:30", "USD", "Core CPI m/m", "high", "", "0.3%"), // "today": excluded
            ],
        )
        .unwrap();
        let hist = read_series_history(
            dir.path(),
            "USD",
            "Core CPI m/m",
            NaiveDate::from_ymd_opt(2026, 7, 17).unwrap(),
            1,
            12,
        )
        .unwrap();
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].date, "2026-07-14");
    }

    #[test]
    fn read_range_missing_dir_is_empty() {
        let rows = read_range(
            Path::new("/nonexistent/calendar"),
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
        )
        .unwrap();
        assert!(rows.is_empty());
    }
}
