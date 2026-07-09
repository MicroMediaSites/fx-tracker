//! Live surprise feed for surprise-conditioned strategies (AGT-632).
//!
//! Feeds the ABI v4 script accessors `surprise_z()` / `surprise_z_for()` /
//! `surprise_hours_ago()` / `surprise_hours_ago_for()` (see
//! `crates/wickd/STRATEGY_ABI.md`). The calendar is a directory of manual
//! monthly ForexFactory CSV exports (header:
//! `date,time,currency,event,impact,actual,forecast,previous`) — the same
//! format as the research corpus in `fx-tracker/data/calendar/`. Dropping a
//! new or re-exported month into the directory (including backfilled
//! `actual` values) updates the feed **without rebuilding wickd**: a fresh
//! process reads the directory at strategy construction, and a long-lived
//! process (the watcher) picks changes up via [`SurpriseCalendar::maybe_refresh`],
//! which re-scans file fingerprints at most once per refresh interval
//! (default 60s) from the candle path.
//!
//! ## Z-score methodology (pre-committed stats — no lookahead)
//!
//! A release's surprise is `actual - forecast`, z-scored per event series —
//! keyed on `(event, currency)` — against that series' mean and population
//! standard deviation computed **only from releases before the frozen
//! discovery cutoff** (2025-01-01T00:00:00Z), requiring at least
//! [`STATS_MIN_SAMPLES`] discovery releases and a non-zero deviation. This
//! mirrors `wickd-lab/surprise_study.py` (STUDY-003) and the H-015
//! registration: the stats are committed once per series, so a z-score read
//! at candle time never encodes information from after the cutoff beyond the
//! release's own published numbers. Series that cannot be scored (too few
//! discovery samples, or zero variance) are invisible to the accessors.
//!
//! ## Forecast-but-no-actual (and other unscoreable rows)
//!
//! A release whose `actual` is not yet published (or not parseable) is **not
//! a surprise yet** — it is skipped entirely, and the accessors report the
//! most recent *scored* release instead. Once the re-exported CSV backfills
//! the `actual`, the release becomes visible with its z-score on the next
//! (re)load. Scripts that must not act on stale surprises should gate on
//! `surprise_hours_ago()` (it always refers to the same release the z came
//! from).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;
use tracing::warn;

/// Expected header of a ForexFactory monthly CSV export. A file whose header
/// differs is a hard error naming the file — wrong format is user error, not
/// a data gap.
pub const FF_CSV_HEADER: &str = "date,time,currency,event,impact,actual,forecast,previous";

/// Minimum number of discovery-period releases a series needs before its
/// surprises can be z-scored (mirrors STUDY-003 / `surprise_study.py`).
pub const STATS_MIN_SAMPLES: usize = 8;

/// Frozen discovery cutoff for the per-series z-stats: 2025-01-01T00:00:00Z,
/// the end of the wickd-lab discovery window. Releases at or after this
/// instant never contribute to the stats (they are still scored *against*
/// them).
pub const STATS_CUTOFF_UNIX: i64 = 1_735_689_600;

/// Default wall-clock throttle for [`SurpriseCalendar::maybe_refresh`].
pub const DEFAULT_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Impact rank for filtering: high=3, medium=2, low=1, anything else
/// (holiday, unknown) = 0 — never matches a `min_impact` threshold.
pub fn impact_rank(impact: &str) -> u8 {
    if impact.eq_ignore_ascii_case("high") {
        3
    } else if impact.eq_ignore_ascii_case("medium") {
        2
    } else if impact.eq_ignore_ascii_case("low") {
        1
    } else {
        0
    }
}

/// Parse a script-facing `min_impact` label into a rank threshold. Unknown
/// labels return `None` — the accessors then match nothing and return their
/// sentinel, consistent with the ABI's "typos degrade to inert" convention.
pub fn min_impact_rank(min_impact: &str) -> Option<u8> {
    match impact_rank(min_impact) {
        0 => None,
        r => Some(r),
    }
}

/// One scored release: a calendar row that had a parseable actual AND
/// forecast, in a series with committed discovery stats.
#[derive(Debug, Clone, PartialEq)]
pub struct SurpriseRelease {
    /// Release time as Unix seconds (UTC).
    pub time_unix: i64,
    /// ISO currency the release prices (USD, EUR, ...).
    pub currency: String,
    /// Impact rank (see [`impact_rank`]).
    pub impact: u8,
    /// Signed z-score of `actual - forecast` against the series' frozen
    /// discovery stats.
    pub z: f64,
}

/// Most recent scored release at or before `t` that passes the filters:
/// `impact >= min_impact`, and — when `currency` is given — that exact
/// currency (case-insensitive), otherwise any of `legs` (the instrument's
/// two currency legs). `releases` must be sorted by `time_unix` ascending.
pub fn latest_scored<'a>(
    releases: &'a [SurpriseRelease],
    t: i64,
    min_impact: u8,
    legs: &[String],
    currency: Option<&str>,
) -> Option<&'a SurpriseRelease> {
    let idx = releases.partition_point(|r| r.time_unix <= t);
    releases[..idx].iter().rev().find(|r| {
        r.impact >= min_impact
            && match currency {
                Some(c) => r.currency.eq_ignore_ascii_case(c),
                None => legs.iter().any(|l| l.eq_ignore_ascii_case(&r.currency)),
            }
    })
}

/// Parse a ForexFactory numeric cell: optional `<`/`>` qualifiers, optional
/// `%`/`K`/`M`/`B`/`T` suffix. Empty or non-numeric cells (unpublished
/// actuals, "Tentative", ...) return `None`. Mirrors
/// `surprise_study.py::parse_num` — `%` is treated as a plain unit (no /100)
/// so surprises stay in the units forecasts are quoted in.
pub fn parse_ff_number(raw: &str) -> Option<f64> {
    let s: String = raw.trim().chars().filter(|c| *c != '<' && *c != '>').collect();
    if s.is_empty() {
        return None;
    }
    let (num, mult) = match s.chars().last() {
        Some('%') => (&s[..s.len() - 1], 1.0),
        Some('K') => (&s[..s.len() - 1], 1e3),
        Some('M') => (&s[..s.len() - 1], 1e6),
        Some('B') => (&s[..s.len() - 1], 1e9),
        Some('T') => (&s[..s.len() - 1], 1e12),
        _ => (s.as_str(), 1.0),
    };
    let v: f64 = num.parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    Some(v * mult)
}

/// A parsed calendar row before scoring.
struct RawRelease {
    time_unix: i64,
    currency: String,
    event: String,
    impact: u8,
    /// `actual - forecast` when both parsed; `None` = unscoreable (e.g. a
    /// forecast with no published actual yet).
    surprise: Option<f64>,
}

/// File fingerprint used to detect calendar-directory changes cheaply:
/// path + modification time + length per `*.csv`, sorted by path.
type Fingerprint = Vec<(PathBuf, Option<std::time::SystemTime>, u64)>;

/// The updatable surprise calendar: releases scored against frozen discovery
/// stats, plus enough state to notice when the backing CSV directory changes.
///
/// A missing directory is an *empty* calendar, not an error — the watcher may
/// start before the first CSV is dropped, and `maybe_refresh` will pick the
/// directory up once it appears.
///
/// `Clone` is cheap (the release set is behind an `Arc`); clones refresh
/// independently, which is how a multi-instrument watcher gives each
/// per-instrument strategy its own feed from one load.
#[derive(Clone)]
pub struct SurpriseCalendar {
    dir: PathBuf,
    releases: Arc<Vec<SurpriseRelease>>,
    fingerprint: Fingerprint,
    last_check: Instant,
    refresh_interval: Duration,
}

impl SurpriseCalendar {
    /// Load every `*.csv` in `dir` (sorted by file name), compute the frozen
    /// discovery stats, and score the releases. Unreadable files and
    /// wrong-header files are hard errors naming the file; rows with
    /// missing/unparseable numbers are normal data gaps and simply skipped.
    pub fn load_dir(dir: &Path) -> Result<Self, String> {
        let fingerprint = scan_dir(dir).map_err(|e| format!("scanning calendar dir {}: {e}", dir.display()))?;
        let releases = parse_and_score(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            releases: Arc::new(releases),
            fingerprint,
            last_check: Instant::now(),
            refresh_interval: DEFAULT_REFRESH_INTERVAL,
        })
    }

    /// Scored releases, sorted by time ascending. The `Arc` is swapped (never
    /// mutated) on reload, so holders of a clone see a consistent snapshot.
    pub fn releases(&self) -> Arc<Vec<SurpriseRelease>> {
        Arc::clone(&self.releases)
    }

    /// Override the wall-clock refresh throttle (default 60s). Mainly for
    /// tests and unusual cadences; `Duration::ZERO` re-checks on every call.
    pub fn set_refresh_interval(&mut self, interval: Duration) {
        self.refresh_interval = interval;
    }

    /// Re-scan the directory if the refresh interval has elapsed, and reload
    /// when any `*.csv` was added, removed, or modified. Returns `true` when
    /// the release set was actually reloaded (callers should then re-stage
    /// their snapshot). A reload failure keeps the previous data and warns —
    /// a half-written CSV drop must not kill a running watcher.
    pub fn maybe_refresh(&mut self) -> bool {
        if self.last_check.elapsed() < self.refresh_interval {
            return false;
        }
        self.last_check = Instant::now();
        let fresh = match scan_dir(&self.dir) {
            Ok(fp) => fp,
            Err(e) => {
                warn!(dir = %self.dir.display(), error = %e, "surprise calendar: directory scan failed; keeping previous data");
                return false;
            }
        };
        if fresh == self.fingerprint {
            return false;
        }
        match parse_and_score(&self.dir) {
            Ok(releases) => {
                self.releases = Arc::new(releases);
                self.fingerprint = fresh;
                true
            }
            Err(e) => {
                // Deliberately do NOT update the fingerprint: the next check
                // retries (and re-warns) until the drop parses.
                warn!(dir = %self.dir.display(), error = %e, "surprise calendar: reload failed; keeping previous data");
                false
            }
        }
    }
}

/// Fingerprint the `*.csv` files in `dir`. A missing directory fingerprints
/// as empty (same as an empty directory).
fn scan_dir(dir: &Path) -> Result<Fingerprint, String> {
    let mut out: Fingerprint = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.to_string()),
    };
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("csv") {
            continue;
        }
        let meta = entry.metadata().map_err(|e| e.to_string())?;
        out.push((path, meta.modified().ok(), meta.len()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Parse every `*.csv` in `dir`, compute the per-series discovery stats, and
/// return the scored releases sorted by time ascending.
fn parse_and_score(dir: &Path) -> Result<Vec<SurpriseRelease>, String> {
    let mut paths: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("csv"))
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(format!("reading calendar dir {}: {e}", dir.display())),
    };
    paths.sort();

    let mut raw: Vec<RawRelease> = Vec::new();
    for path in &paths {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("reading calendar file {}: {e}", path.display()))?;
        parse_csv(&contents, path, &mut raw)?;
    }

    // Frozen discovery stats per (event, currency) series: mean + population
    // stdev over pre-cutoff surprises, min sample count, non-zero deviation.
    let mut disc: HashMap<(String, String), Vec<f64>> = HashMap::new();
    for r in &raw {
        if let Some(s) = r.surprise {
            if r.time_unix < STATS_CUTOFF_UNIX {
                disc.entry((r.event.clone(), r.currency.clone())).or_default().push(s);
            }
        }
    }
    let stats: HashMap<(String, String), (f64, f64)> = disc
        .into_iter()
        .filter(|(_, v)| v.len() >= STATS_MIN_SAMPLES)
        .filter_map(|(k, v)| {
            let n = v.len() as f64;
            let mean = v.iter().sum::<f64>() / n;
            let sd = (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt();
            (sd > 0.0).then_some((k, (mean, sd)))
        })
        .collect();

    let mut out: Vec<SurpriseRelease> = raw
        .into_iter()
        .filter_map(|r| {
            let s = r.surprise?;
            let (mean, sd) = stats.get(&(r.event, r.currency.clone()))?;
            Some(SurpriseRelease {
                time_unix: r.time_unix,
                currency: r.currency,
                impact: r.impact,
                z: (s - mean) / sd,
            })
        })
        .collect();
    out.sort_by_key(|r| r.time_unix);
    Ok(out)
}

/// Parse one CSV file's contents into `raw`. Hard error on a wrong header;
/// rows with the wrong column count or an unparseable date/time are skipped
/// (data gaps, not user error).
fn parse_csv(contents: &str, path: &Path, raw: &mut Vec<RawRelease>) -> Result<(), String> {
    let mut lines = contents.lines();
    let header = lines.next().unwrap_or("").trim_start_matches('\u{feff}').trim();
    if header != FF_CSV_HEADER {
        return Err(format!(
            "calendar file {} has an unexpected header (expected '{FF_CSV_HEADER}', got '{header}')",
            path.display()
        ));
    }
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 8 {
            continue;
        }
        // An event name containing commas would split into >8 parts: the
        // first 3 and last 4 columns are fixed, the middle is the event.
        let n = parts.len();
        let (date, time, currency) = (parts[0], parts[1], parts[2]);
        let event = parts[3..n - 4].join(",");
        let (impact, actual, forecast) = (parts[n - 4], parts[n - 3], parts[n - 2]);
        let Ok(dt) = NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%Y-%m-%d %H:%M") else {
            continue; // "All Day" / tentative rows carry no usable timestamp
        };
        let surprise = match (parse_ff_number(actual), parse_ff_number(forecast)) {
            (Some(a), Some(f)) => Some(a - f),
            _ => None,
        };
        raw.push(RawRelease {
            time_unix: dt.and_utc().timestamp(),
            currency: currency.trim().to_string(),
            event,
            impact: impact_rank(impact.trim()),
            surprise,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- parse_ff_number ----

    #[test]
    fn parse_ff_number_handles_ff_export_formats() {
        assert_eq!(parse_ff_number("54.8"), Some(54.8));
        assert_eq!(parse_ff_number("-1.1%"), Some(-1.1));
        assert_eq!(parse_ff_number("<0.5%"), Some(0.5));
        assert_eq!(parse_ff_number(">1.2B"), Some(1.2e9));
        assert_eq!(parse_ff_number("2.5K"), Some(2500.0));
        assert_eq!(parse_ff_number("-3M"), Some(-3e6));
        assert_eq!(parse_ff_number("0.1T"), Some(1e11));
        assert_eq!(parse_ff_number(" 7 "), Some(7.0));
        assert_eq!(parse_ff_number(""), None);
        assert_eq!(parse_ff_number("Tentative"), None);
        assert_eq!(parse_ff_number("%"), None);
    }

    // ---- impact ranks ----

    #[test]
    fn impact_ranks_order_and_unknowns() {
        assert!(impact_rank("high") > impact_rank("medium"));
        assert!(impact_rank("medium") > impact_rank("low"));
        assert_eq!(impact_rank("holiday"), 0);
        assert_eq!(impact_rank("HIGH"), 3);
        assert_eq!(min_impact_rank("low"), Some(1));
        assert_eq!(min_impact_rank("banana"), None);
    }

    // ---- corpus fixtures ----

    /// Build a discovery-period CSV whose "CPI y/y,USD" series has 8
    /// surprises of [+1,-1,+1,-1,...] → mean 0, population stdev 1 — so a
    /// later release's z equals its raw surprise. Forecast fixed at 3.0.
    fn discovery_csv() -> String {
        let mut s = format!("{FF_CSV_HEADER}\n");
        for i in 0..8 {
            let actual = if i % 2 == 0 { 4.0 } else { 2.0 }; // surprise ±1
            s.push_str(&format!("2024-0{}-10,13:30,USD,CPI y/y,high,{actual}%,3.0%,3.0%\n", i + 1));
        }
        s
    }

    fn write_calendar(dir: &Path, name: &str, contents: &str) {
        std::fs::write(dir.join(name), contents).unwrap();
    }

    fn t(rfc3339: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(rfc3339).unwrap().timestamp()
    }

    #[test]
    fn z_scores_against_frozen_discovery_stats() {
        let dir = tempfile::tempdir().unwrap();
        write_calendar(dir.path(), "2024.csv", &discovery_csv());
        // A post-cutoff release: actual 5.0 vs forecast 3.0 → surprise +2 → z = +2.
        write_calendar(
            dir.path(),
            "2026-06.csv",
            &format!("{FF_CSV_HEADER}\n2026-06-10,13:30,USD,CPI y/y,high,5.0%,3.0%,3.1%\n"),
        );
        let cal = SurpriseCalendar::load_dir(dir.path()).unwrap();
        let releases = cal.releases();
        let legs = vec!["EUR".to_string(), "USD".to_string()];

        let r = latest_scored(&releases, t("2026-06-11T00:00:00Z"), 3, &legs, None).unwrap();
        assert_eq!(r.time_unix, t("2026-06-10T13:30:00Z"));
        assert!((r.z - 2.0).abs() < 1e-9, "z was {}", r.z);
        assert_eq!(r.currency, "USD");
        assert_eq!(r.impact, 3);
    }

    #[test]
    fn no_lookahead_release_is_invisible_before_its_time_and_visible_at_it() {
        let dir = tempfile::tempdir().unwrap();
        write_calendar(dir.path(), "2024.csv", &discovery_csv());
        write_calendar(
            dir.path(),
            "2026-06.csv",
            &format!("{FF_CSV_HEADER}\n2026-06-10,13:30,USD,CPI y/y,high,5.0%,3.0%,3.1%\n"),
        );
        let cal = SurpriseCalendar::load_dir(dir.path()).unwrap();
        let releases = cal.releases();
        let legs = vec!["EUR".to_string(), "USD".to_string()];
        let release_t = t("2026-06-10T13:30:00Z");

        // One second before the release: only the discovery releases exist,
        // and the most recent of those is 2024-08-10.
        let before = latest_scored(&releases, release_t - 1, 3, &legs, None).unwrap();
        assert_eq!(before.time_unix, t("2024-08-10T13:30:00Z"));
        // At the release instant it becomes the latest.
        let at = latest_scored(&releases, release_t, 3, &legs, None).unwrap();
        assert_eq!(at.time_unix, release_t);
        // Before ANY release: nothing.
        assert!(latest_scored(&releases, t("2021-01-01T00:00:00Z"), 3, &legs, None).is_none());
    }

    #[test]
    fn forecast_without_actual_is_skipped_until_backfilled() {
        let dir = tempfile::tempdir().unwrap();
        write_calendar(dir.path(), "2024.csv", &discovery_csv());
        // The newest release has a forecast but no actual yet — not a
        // surprise; the previous scored release must be reported instead.
        write_calendar(
            dir.path(),
            "2026-06.csv",
            &format!(
                "{FF_CSV_HEADER}\n\
                 2026-06-10,13:30,USD,CPI y/y,high,5.0%,3.0%,3.1%\n\
                 2026-07-10,13:30,USD,CPI y/y,high,,3.2%,5.0%\n"
            ),
        );
        let mut cal = SurpriseCalendar::load_dir(dir.path()).unwrap();
        let legs = vec!["EUR".to_string(), "USD".to_string()];
        let after_pending = t("2026-07-10T14:30:00Z");

        let releases = cal.releases();
        let r = latest_scored(&releases, after_pending, 3, &legs, None).unwrap();
        assert_eq!(r.time_unix, t("2026-06-10T13:30:00Z"), "pending release must be skipped");

        // Backfill the actual (a re-dropped export) → the release appears,
        // with z from the same frozen stats (surprise 4.2-3.2 = +1 → z = +1).
        write_calendar(
            dir.path(),
            "2026-06.csv",
            &format!(
                "{FF_CSV_HEADER}\n\
                 2026-06-10,13:30,USD,CPI y/y,high,5.0%,3.0%,3.1%\n\
                 2026-07-10,13:30,USD,CPI y/y,high,4.2%,3.2%,5.0%\n"
            ),
        );
        cal.set_refresh_interval(Duration::ZERO);
        assert!(cal.maybe_refresh(), "modified CSV must trigger a reload");
        let releases = cal.releases();
        let r = latest_scored(&releases, after_pending, 3, &legs, None).unwrap();
        assert_eq!(r.time_unix, t("2026-07-10T13:30:00Z"));
        assert!((r.z - 1.0).abs() < 1e-9, "z was {}", r.z);
    }

    #[test]
    fn refresh_is_throttled_and_noop_when_nothing_changed() {
        let dir = tempfile::tempdir().unwrap();
        write_calendar(dir.path(), "2024.csv", &discovery_csv());
        let mut cal = SurpriseCalendar::load_dir(dir.path()).unwrap();
        // Default 60s throttle: an immediate check is skipped entirely.
        assert!(!cal.maybe_refresh());
        // Unthrottled but unchanged: scan runs, no reload.
        cal.set_refresh_interval(Duration::ZERO);
        assert!(!cal.maybe_refresh());
        // A new file appears → reload.
        write_calendar(
            dir.path(),
            "2026-06.csv",
            &format!("{FF_CSV_HEADER}\n2026-06-10,13:30,USD,CPI y/y,high,5.0%,3.0%,3.1%\n"),
        );
        assert!(cal.maybe_refresh());
        assert!(!cal.maybe_refresh(), "second check with no further change is a no-op");
    }

    #[test]
    fn series_below_min_samples_or_zero_variance_are_unscored() {
        let dir = tempfile::tempdir().unwrap();
        let mut csv = format!("{FF_CSV_HEADER}\n");
        // Only 7 discovery samples — below STATS_MIN_SAMPLES.
        for i in 0..7 {
            csv.push_str(&format!("2024-0{}-05,08:00,GBP,Thin Series m/m,high,1.{i}%,1.0%,1.0%\n", i + 1));
        }
        // 8 samples but identical surprises — zero variance.
        for i in 0..8 {
            csv.push_str(&format!("2024-0{}-06,08:00,GBP,Flat Series m/m,high,2.0%,1.0%,1.0%\n", i + 1));
        }
        csv.push_str("2026-06-05,08:00,GBP,Thin Series m/m,high,1.5%,1.0%,1.0%\n");
        csv.push_str("2026-06-06,08:00,GBP,Flat Series m/m,high,2.0%,1.0%,1.0%\n");
        write_calendar(dir.path(), "all.csv", &csv);

        let cal = SurpriseCalendar::load_dir(dir.path()).unwrap();
        assert!(cal.releases().is_empty(), "unscoreable series must produce no releases");
    }

    #[test]
    fn impact_currency_and_leg_filters_apply() {
        let dir = tempfile::tempdir().unwrap();
        let mut csv = discovery_csv();
        // A medium-impact EUR series with valid stats (same ±1 construction).
        for i in 0..8 {
            let actual = if i % 2 == 0 { 4.0 } else { 2.0 };
            csv.push_str(&format!("2024-0{}-12,09:00,EUR,PMI,medium,{actual},3.0,3.0\n", i + 1));
        }
        // A scored JPY release, most recent of all.
        for i in 0..8 {
            let actual = if i % 2 == 0 { 4.0 } else { 2.0 };
            csv.push_str(&format!("2024-0{}-14,01:30,JPY,Tankan,high,{actual},3.0,3.0\n", i + 1));
        }
        write_calendar(dir.path(), "all.csv", &csv);
        let cal = SurpriseCalendar::load_dir(dir.path()).unwrap();
        let releases = cal.releases();
        let legs = vec!["EUR".to_string(), "USD".to_string()];
        let after_all = t("2024-12-31T00:00:00Z");

        // Leg filter: the JPY release (latest overall, high impact) is not a leg
        // of EUR_USD — the latest high-impact leg release is the USD CPI.
        let r = latest_scored(&releases, after_all, 3, &legs, None).unwrap();
        assert_eq!(r.currency, "USD");
        // min_impact medium admits the EUR PMI (later in the month than CPI on the 10th).
        let r = latest_scored(&releases, after_all, 2, &legs, None).unwrap();
        assert_eq!(r.currency, "EUR");
        // Explicit currency override ignores legs.
        let r = latest_scored(&releases, after_all, 3, &legs, Some("JPY")).unwrap();
        assert_eq!(r.currency, "JPY");
        // Explicit currency + impact threshold that excludes everything.
        assert!(latest_scored(&releases, after_all, 3, &legs, Some("CHF")).is_none());
    }

    #[test]
    fn wrong_header_is_a_hard_error_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        write_calendar(dir.path(), "bad.csv", "when,what\n2026-01-01,stuff\n");
        let err = SurpriseCalendar::load_dir(dir.path()).err().unwrap();
        assert!(err.contains("bad.csv"), "error was: {err}");
        assert!(err.contains("unexpected header"), "error was: {err}");
    }

    #[test]
    fn missing_directory_is_an_empty_calendar_that_can_appear_later() {
        let parent = tempfile::tempdir().unwrap();
        let dir = parent.path().join("not-yet-created");
        let mut cal = SurpriseCalendar::load_dir(&dir).unwrap();
        assert!(cal.releases().is_empty());

        std::fs::create_dir_all(&dir).unwrap();
        write_calendar(&dir, "2024.csv", &discovery_csv());
        cal.set_refresh_interval(Duration::ZERO);
        assert!(cal.maybe_refresh(), "directory appearing must trigger a reload");
        assert!(!cal.releases().is_empty());
    }

    #[test]
    fn event_names_with_commas_survive_column_reassembly() {
        let dir = tempfile::tempdir().unwrap();
        let mut csv = format!("{FF_CSV_HEADER}\n");
        for i in 0..8 {
            let actual = if i % 2 == 0 { 4.0 } else { 2.0 };
            csv.push_str(&format!(
                "2024-0{}-10,13:30,USD,\"CPI, core, y/y\",high,{actual}%,3.0%,3.0%\n",
                i + 1
            ));
        }
        csv.push_str("2026-06-10,13:30,USD,\"CPI, core, y/y\",high,5.0%,3.0%,3.1%\n");
        write_calendar(dir.path(), "all.csv", &csv);
        let cal = SurpriseCalendar::load_dir(dir.path()).unwrap();
        let legs = vec!["EUR".to_string(), "USD".to_string()];
        let releases = cal.releases();
        let r = latest_scored(&releases, t("2026-06-11T00:00:00Z"), 3, &legs, None).unwrap();
        assert!((r.z - 2.0).abs() < 1e-9, "comma-in-event series must still score; z was {}", r.z);
    }
}
