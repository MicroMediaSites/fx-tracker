//! The earnings-calendar store (`~/.wickd/earnings/YYYY-MM-DD.json`): who
//! reports on a day, before or after the market, and what is expected.
//!
//! One file per day, written whole and atomically (tmp + rename), so a
//! reader never sees a half-written day. The store sits beside the
//! economic-calendar store ([`crate::calendar_store`]) and is read by
//! `wickd earnings upcoming` and the TUI; `wickd earnings sync` is the only
//! writer.
//!
//! ## Source
//!
//! Nasdaq's public earnings calendar ([`NASDAQ_EARNINGS_URL`]), one request
//! per day, no key. It is **unofficial**: nothing promises it stays up or
//! keeps its shape. So:
//!
//! - every field is optional on the way in and a value that doesn't parse
//!   (`"N/A"`, an empty string) is simply left out ([`normalize`]);
//! - a day whose fetch fails keeps the file it already has ([`sync_days`]);
//! - a fetch that returns no rows never replaces a stored day that has rows
//!   (an empty answer from an unofficial endpoint is more likely a glitch
//!   than every company cancelling).
//!
//! The feed carries expectations only (forecast EPS, last year's EPS), never
//! the reported result, so there is nothing to preserve across re-fetches.
//!
//! Text from the feed is untrusted and ends up on a terminal: symbols are
//! restricted to ticker characters and names are stripped of control
//! characters and capped ([`clean_symbol`], [`clean_text`]).

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, Datelike, NaiveDate, Utc, Weekday};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::paths::wickd_data_home;

/// Nasdaq's public earnings calendar; `?date=YYYY-MM-DD` selects the day.
pub const NASDAQ_EARNINGS_URL: &str = "https://api.nasdaq.com/api/calendar/earnings";

/// Recorded in each day file so a reader can tell where the rows came from.
pub const SOURCE: &str = "nasdaq";

/// The most response body read from the feed. A full day is ~20 KB in
/// earnings season; anything near this is not an earnings calendar.
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

/// The most rows kept from one day (the busiest days have a few hundred).
const MAX_ROWS: usize = 5_000;

const MAX_SYMBOL_CHARS: usize = 12;
const MAX_NAME_CHARS: usize = 80;

/// Pause between day requests in [`sync_days`]: the endpoint is free and
/// unofficial, so a week's sync is deliberately unhurried.
const REQUEST_GAP: Duration = Duration::from_millis(250);

/// `~/.wickd/earnings` (honours `WICKD_HOME`).
pub fn earnings_dir() -> Result<PathBuf, String> {
    Ok(wickd_data_home()?.join("earnings"))
}

/// When in the day a company reports. Ordered the way a day is read: before
/// the open, after the close, then the ones with no time given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Session {
    PreMarket,
    AfterHours,
    Unknown,
}

impl Session {
    fn from_feed(s: &str) -> Self {
        match s.trim() {
            "time-pre-market" => Session::PreMarket,
            "time-after-hours" => Session::AfterHours,
            _ => Session::Unknown,
        }
    }
}

/// One company's report on one day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EarningsRow {
    pub date: NaiveDate,
    pub symbol: String,
    pub name: String,
    pub session: Session,
    /// Consensus EPS forecast, when the feed gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eps_forecast: Option<Decimal>,
    /// How many analyst estimates the forecast averages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimates: Option<u32>,
    /// Market value in dollars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market_cap: Option<Decimal>,
    /// EPS reported for the same quarter a year ago.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_year_eps: Option<Decimal>,
    /// The quarter being reported, as the feed words it (`Oct/2026`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fiscal_quarter: String,
}

/// One stored day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayFile {
    pub date: NaiveDate,
    pub fetched_at: DateTime<Utc>,
    pub source: String,
    pub rows: Vec<EarningsRow>,
}

/// The US market's calendar date at `now` (US/Eastern), which is the date
/// the feed and the store are keyed by. Late evening in UTC is still the
/// same trading day.
pub fn market_date(now: DateTime<Utc>) -> NaiveDate {
    (now + chrono::Duration::hours(crate::equities::eastern_offset_hours(now))).date_naive()
}

// ── the feed ────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
struct RawResponse {
    #[serde(default)]
    data: Option<RawData>,
}

#[derive(Debug, Default, Deserialize)]
struct RawData {
    #[serde(default)]
    rows: Option<Vec<RawRow>>,
}

/// A feed row. Every field is optional and read as loosely as possible: a
/// field that turns up as a number or `null` instead of a string is not a
/// reason to drop the day.
#[derive(Debug, Default, Deserialize)]
struct RawRow {
    #[serde(default)]
    symbol: serde_json::Value,
    #[serde(default)]
    name: serde_json::Value,
    #[serde(default)]
    time: serde_json::Value,
    #[serde(default, rename = "epsForecast")]
    eps_forecast: serde_json::Value,
    #[serde(default, rename = "noOfEsts")]
    estimates: serde_json::Value,
    #[serde(default, rename = "marketCap")]
    market_cap: serde_json::Value,
    #[serde(default, rename = "lastYearEPS")]
    last_year_eps: serde_json::Value,
    #[serde(default, rename = "fiscalQuarterEnding")]
    fiscal_quarter: serde_json::Value,
}

fn text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// A dollar figure as the feed writes it: `$2.49`, `$3,762,974,100,000`,
/// `($0.12)` or `-$0.12` for a loss. `N/A`, an empty string and anything
/// else that isn't a number is `None`.
pub fn parse_money(s: &str) -> Option<Decimal> {
    let t = s.trim();
    let (t, negative) = match t.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        Some(inner) => (inner.trim(), true),
        None => match t.strip_prefix('-') {
            Some(rest) => (rest.trim(), true),
            None => (t, false),
        },
    };
    let digits: String = t.chars().filter(|c| *c != '$' && *c != ',').collect();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let d = Decimal::from_str(&digits).ok()?;
    Some(if negative { -d } else { d })
}

/// A ticker as stored: trimmed, upper-cased, and only letters, digits,
/// `.`, `-`, `/` and `^`. Anything else (or nothing, or too long) is not a
/// ticker and the row is dropped.
pub fn clean_symbol(s: &str) -> Option<String> {
    let t = s.trim().to_uppercase();
    let ok = !t.is_empty()
        && t.chars().count() <= MAX_SYMBOL_CHARS
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '/' | '^'));
    ok.then_some(t)
}

/// Free text from the feed, safe to draw: control characters removed
/// (including escape sequences' ESC), whitespace collapsed, capped at `max`
/// characters.
pub fn clean_text(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect();
    flat.split(' ')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// Turn one day's feed body into rows for `date`: one per ticker (the first
/// wins), sorted by session, then market value (largest first), then
/// symbol. A body that isn't the feed's JSON is an error; a body with no
/// rows (a weekend, a holiday) is an empty day.
pub fn normalize(date: NaiveDate, body: &[u8]) -> Result<Vec<EarningsRow>, String> {
    let raw: RawResponse = serde_json::from_slice(body)
        .map_err(|e| format!("earnings feed for {date} is not the expected JSON: {e}"))?;
    let Some(data) = raw.data else {
        return Err(format!(
            "earnings feed for {date} has no data (the request was rejected)"
        ));
    };
    let mut seen = std::collections::HashSet::new();
    let mut rows: Vec<EarningsRow> = data
        .rows
        .unwrap_or_default()
        .into_iter()
        .take(MAX_ROWS)
        .filter_map(|r| {
            let symbol = clean_symbol(&text(&r.symbol))?;
            seen.insert(symbol.clone()).then(|| EarningsRow {
                date,
                symbol,
                name: clean_text(&text(&r.name), MAX_NAME_CHARS),
                session: Session::from_feed(&text(&r.time)),
                eps_forecast: parse_money(&text(&r.eps_forecast)),
                estimates: text(&r.estimates).trim().parse().ok(),
                market_cap: parse_money(&text(&r.market_cap)),
                last_year_eps: parse_money(&text(&r.last_year_eps)),
                fiscal_quarter: clean_text(&text(&r.fiscal_quarter), 16),
            })
        })
        .collect();
    sort_rows(&mut rows);
    Ok(rows)
}

/// Date, then session, then market value (largest first, unknown last),
/// then symbol.
pub fn sort_rows(rows: &mut [EarningsRow]) {
    rows.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then(a.session.cmp(&b.session))
            .then(b.market_cap.cmp(&a.market_cap))
            .then(a.symbol.cmp(&b.symbol))
    });
}

/// The HTTP client for the feed: a timeout, TLS 1.2 at least, and an honest
/// client name.
pub fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .user_agent(concat!("wickd/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("building the HTTP client: {e}"))
}

/// Fetch and normalize one day. `base_url` is [`NASDAQ_EARNINGS_URL`]
/// outside tests.
pub async fn fetch_day(
    client: &reqwest::Client,
    base_url: &str,
    date: NaiveDate,
) -> Result<Vec<EarningsRow>, String> {
    let mut resp = client
        .get(base_url)
        .query(&[("date", date.format("%Y-%m-%d").to_string())])
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("fetching earnings for {date}: {}", e.without_url()))?;
    if !resp.status().is_success() {
        return Err(format!(
            "fetching earnings for {date}: HTTP {}",
            resp.status()
        ));
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("reading earnings for {date}: {}", e.without_url()))?
    {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(format!(
                "earnings feed for {date} is larger than {} MB; not reading it",
                MAX_BODY_BYTES / (1024 * 1024)
            ));
        }
        body.extend_from_slice(&chunk);
    }
    normalize(date, &body)
}

// ── the store ───────────────────────────────────────────────────────────

fn day_path(dir: &Path, date: NaiveDate) -> PathBuf {
    dir.join(format!("{}.json", date.format("%Y-%m-%d")))
}

/// Write one day, replacing what was there, atomically.
pub fn write_day(dir: &Path, day: &DayFile) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let path = day_path(dir, day.date);
    let tmp = dir.join(format!(".{}.json.tmp", day.date.format("%Y-%m-%d")));
    let bytes =
        serde_json::to_vec_pretty(day).map_err(|e| format!("encoding {}: {e}", path.display()))?;
    fs::write(&tmp, bytes).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path)
        .map_err(|e| format!("renaming {} → {}: {e}", tmp.display(), path.display()))
}

/// Read one stored day; `None` when there is no file for it.
pub fn read_day(dir: &Path, date: NaiveDate) -> Result<Option<DayFile>, String> {
    let path = day_path(dir, date);
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("{} is not a valid earnings day file: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("reading {}: {e}", path.display())),
    }
}

/// The stored days from `from` to `to` inclusive, oldest first. A day with
/// no file is skipped (it is "not fetched yet", not an error).
pub fn read_days(dir: &Path, from: NaiveDate, to: NaiveDate) -> Result<Vec<DayFile>, String> {
    let mut out = Vec::new();
    let mut d = from;
    while d <= to {
        if let Some(day) = read_day(dir, d)? {
            out.push(day);
        }
        let Some(next) = d.succ_opt() else { break };
        d = next;
    }
    Ok(out)
}

/// Every stored row from `from` to `to` inclusive, in [`sort_rows`] order.
pub fn read_range(dir: &Path, from: NaiveDate, to: NaiveDate) -> Result<Vec<EarningsRow>, String> {
    let mut rows: Vec<EarningsRow> = read_days(dir, from, to)?
        .into_iter()
        .flat_map(|d| d.rows)
        .collect();
    sort_rows(&mut rows);
    Ok(rows)
}

/// `from` and the `days` calendar days after it, without Saturdays and
/// Sundays (nobody reports then, and the feed answers with no rows).
pub fn weekdays_from(from: NaiveDate, days: u32) -> Vec<NaiveDate> {
    (0..=days as i64)
        .filter_map(|i| from.checked_add_signed(chrono::Duration::days(i)))
        .filter(|d| !matches!(d.weekday(), Weekday::Sat | Weekday::Sun))
        .collect()
}

/// What happened to one day in a sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DayOutcome {
    pub date: NaiveDate,
    /// Rows now stored for the day (after this sync), when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<usize>,
    /// The day's file was written by this sync.
    pub written: bool,
    /// The fetch returned no rows and the stored day had some, so the
    /// stored day was kept.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub kept_stored: bool,
    /// Why the day wasn't fetched or written; the stored file is untouched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl DayOutcome {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }
}

/// Store what one day's fetch returned, applying the keep-on-failure and
/// keep-on-empty rules. Split from [`sync_days`] so the rules are testable
/// without a network.
pub fn apply_fetch(
    dir: &Path,
    date: NaiveDate,
    fetched: Result<Vec<EarningsRow>, String>,
    now: DateTime<Utc>,
) -> DayOutcome {
    let stored_rows = || read_day(dir, date).ok().flatten().map(|d| d.rows.len());
    let rows = match fetched {
        Ok(r) => r,
        Err(e) => {
            return DayOutcome {
                date,
                rows: stored_rows(),
                written: false,
                kept_stored: false,
                error: Some(e),
            }
        }
    };
    if rows.is_empty() {
        if let Some(n) = stored_rows().filter(|n| *n > 0) {
            return DayOutcome {
                date,
                rows: Some(n),
                written: false,
                kept_stored: true,
                error: None,
            };
        }
    }
    let n = rows.len();
    let day = DayFile {
        date,
        fetched_at: now,
        source: SOURCE.to_string(),
        rows,
    };
    match write_day(dir, &day) {
        Ok(()) => DayOutcome {
            date,
            rows: Some(n),
            written: true,
            kept_stored: false,
            error: None,
        },
        Err(e) => DayOutcome {
            date,
            rows: stored_rows(),
            written: false,
            kept_stored: false,
            error: Some(e),
        },
    }
}

/// Fetch each of `dates` in turn and store it. One day failing never stops
/// the others.
pub async fn sync_days(
    client: &reqwest::Client,
    base_url: &str,
    dir: &Path,
    dates: &[NaiveDate],
) -> Vec<DayOutcome> {
    let mut out = Vec::with_capacity(dates.len());
    for (i, date) in dates.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(REQUEST_GAP).await;
        }
        let fetched = fetch_day(client, base_url, *date).await;
        out.push(apply_fetch(dir, *date, fetched, Utc::now()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// Rows captured from the feed on 2026-11-17 (trimmed), plus the odd
    /// shapes it has been seen to send and a few it might.
    const FIXTURE: &str = r#"{"data":{"asOf":"Wed, Nov 18, 2026","headers":{"time":"Time"},"rows":[
      {"lastYearRptDt":"11/19/2025","lastYearEPS":"$1.28","time":"time-pre-market","symbol":"TJX","name":"TJX Companies, Inc. (The)","marketCap":"$144,195,600,000","fiscalQuarterEnding":"Oct/2026","epsForecast":"$1.33","noOfEsts":"7"},
      {"lastYearRptDt":"11/19/2025","lastYearEPS":"$0.53","time":"time-after-hours","symbol":"PANW","name":"Palo Alto Networks, Inc.","marketCap":"$258,701,689,000","fiscalQuarterEnding":"Oct/2026","epsForecast":"$0.52","noOfEsts":"16"},
      {"lastYearRptDt":"11/19/2025","lastYearEPS":"$3.06","time":"time-pre-market","symbol":"LOW","name":"Lowe's Companies, Inc.","marketCap":"$126,197,883,000","fiscalQuarterEnding":"Oct/2026","epsForecast":"$2.86","noOfEsts":"11"},
      {"lastYearRptDt":"N/A","lastYearEPS":"N/A","time":"time-not-supplied","symbol":"BHP","name":"BHP Group Limited","marketCap":"$195,077,603,000","fiscalQuarterEnding":"Sep/2026","epsForecast":"","noOfEsts":"4"},
      {"lastYearEPS":"($0.12)","time":"time-after-hours","symbol":"loss","name":"  Loss\tMaker\u001b[31m  Corp  ","marketCap":"N/A","epsForecast":"-$0.05","noOfEsts":"N/A"},
      {"time":"time-pre-market","symbol":"TJX","name":"A second TJX row","marketCap":"$1"},
      {"time":"time-pre-market","symbol":"BAD SYMBOL;rm","name":"Not a ticker"},
      {"time":"time-pre-market","symbol":"","name":"No ticker"},
      {"time":null,"symbol":"NUM","name":null,"marketCap":12345,"epsForecast":null,"noOfEsts":3}
    ]},"message":null,"status":{"rCode":200}}"#;

    #[test]
    fn money_reads_dollars_commas_and_losses_and_refuses_the_rest() {
        assert_eq!(parse_money("$2.49"), Some(dec!(2.49)));
        assert_eq!(parse_money("$3,762,974,100,000"), Some(dec!(3762974100000)));
        assert_eq!(parse_money("($0.12)"), Some(dec!(-0.12)));
        assert_eq!(parse_money("-$0.05"), Some(dec!(-0.05)));
        assert_eq!(parse_money(" 12345 "), Some(dec!(12345)));
        for bad in ["", "N/A", "$", "--", "1e9", "$1.2.3", "abc"] {
            assert_eq!(parse_money(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn normalize_keeps_what_parses_and_drops_what_is_not_a_ticker() {
        let rows = normalize(d("2026-11-18"), FIXTURE.as_bytes()).unwrap();
        let syms: Vec<&str> = rows.iter().map(|r| r.symbol.as_str()).collect();
        // Before the open (largest first), after the close (LOSS has no
        // value, so it sorts last), then the ones with no time given.
        assert_eq!(syms, ["TJX", "LOW", "PANW", "LOSS", "BHP", "NUM"]);
        let tjx = &rows[0];
        assert_eq!(
            tjx.name, "TJX Companies, Inc. (The)",
            "the first TJX row wins"
        );
        assert_eq!(
            (
                tjx.session,
                tjx.eps_forecast,
                tjx.estimates,
                tjx.market_cap,
                tjx.last_year_eps
            ),
            (
                Session::PreMarket,
                Some(dec!(1.33)),
                Some(7),
                Some(dec!(144195600000)),
                Some(dec!(1.28))
            )
        );
        assert_eq!(tjx.fiscal_quarter, "Oct/2026");
        let bhp = rows.iter().find(|r| r.symbol == "BHP").unwrap();
        assert_eq!(
            (bhp.session, bhp.eps_forecast, bhp.last_year_eps),
            (Session::Unknown, None, None)
        );
        let loss = rows.iter().find(|r| r.symbol == "LOSS").unwrap();
        assert_eq!(
            loss.name, "Loss Maker[31m Corp",
            "control characters are gone"
        );
        assert_eq!(
            (
                loss.eps_forecast,
                loss.last_year_eps,
                loss.market_cap,
                loss.estimates
            ),
            (Some(dec!(-0.05)), Some(dec!(-0.12)), None, None)
        );
        let num = rows.iter().find(|r| r.symbol == "NUM").unwrap();
        assert_eq!(
            (num.market_cap, num.estimates, num.name.as_str()),
            (Some(dec!(12345)), Some(3), "")
        );
    }

    #[test]
    fn normalize_tells_an_empty_day_from_a_rejected_request() {
        let weekend = br#"{"data":{"asOf":"Sat, Nov 21, 2026","headers":null,"rows":null},"status":{"rCode":200}}"#;
        assert_eq!(normalize(d("2026-11-21"), weekend), Ok(vec![]));
        let rejected = br#"{"data":null,"status":{"rCode":400}}"#;
        assert!(normalize(d("2026-11-21"), rejected)
            .unwrap_err()
            .contains("rejected"));
        assert!(normalize(d("2026-11-21"), b"<html>blocked</html>").is_err());
    }

    #[test]
    fn symbols_and_names_are_made_safe() {
        assert_eq!(clean_symbol(" brk.b "), Some("BRK.B".into()));
        assert_eq!(clean_symbol("BF-B"), Some("BF-B".into()));
        for bad in ["", "   ", "A B", "TOOLONGSYMBOL1", "X\u{1b}[0m", "A;B"] {
            assert_eq!(clean_symbol(bad), None, "{bad:?}");
        }
        assert_eq!(clean_text("a\n\tb\u{7}  c", 80), "a b c");
        assert_eq!(clean_text("abcdef", 3), "abc");
    }

    #[test]
    fn a_day_round_trips_and_the_range_reader_skips_missing_days() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("earnings");
        assert_eq!(read_day(&dir, d("2026-11-18")), Ok(None), "no folder yet");
        assert_eq!(
            read_range(&dir, d("2026-11-16"), d("2026-11-20")),
            Ok(vec![])
        );

        let rows = normalize(d("2026-11-18"), FIXTURE.as_bytes()).unwrap();
        let day = DayFile {
            date: d("2026-11-18"),
            fetched_at: "2026-11-18T01:00:00Z".parse().unwrap(),
            source: SOURCE.into(),
            rows: rows.clone(),
        };
        write_day(&dir, &day).unwrap();
        assert_eq!(read_day(&dir, d("2026-11-18")), Ok(Some(day)));
        // A later day, written first in time order to prove the reader sorts.
        let later = EarningsRow {
            date: d("2026-11-19"),
            symbol: "WMT".into(),
            name: "Walmart Inc.".into(),
            session: Session::PreMarket,
            eps_forecast: Some(dec!(0.63)),
            estimates: None,
            market_cap: Some(dec!(1000000000000)),
            last_year_eps: None,
            fiscal_quarter: String::new(),
        };
        write_day(
            &dir,
            &DayFile {
                date: d("2026-11-19"),
                fetched_at: Utc::now(),
                source: SOURCE.into(),
                rows: vec![later.clone()],
            },
        )
        .unwrap();
        let got = read_range(&dir, d("2026-11-16"), d("2026-11-20")).unwrap();
        assert_eq!(got.len(), rows.len() + 1);
        assert_eq!(got.last(), Some(&later));
        assert_eq!(
            read_range(&dir, d("2026-11-19"), d("2026-11-19")),
            Ok(vec![later])
        );
        // No temp file is left behind.
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
        // A file that isn't a day file is an error that names it.
        fs::write(dir.join("2026-11-17.json"), b"not json").unwrap();
        let err = read_range(&dir, d("2026-11-16"), d("2026-11-20")).unwrap_err();
        assert!(err.contains("2026-11-17.json"), "{err}");
    }

    #[test]
    fn the_market_date_is_the_eastern_one() {
        // 01:30 UTC on the 18th is 20:30 on the 17th in New York (EST).
        assert_eq!(
            market_date("2026-11-18T01:30:00Z".parse().unwrap()),
            d("2026-11-17")
        );
        assert_eq!(
            market_date("2026-11-18T05:00:00Z".parse().unwrap()),
            d("2026-11-18")
        );
        // Summer time: midnight Eastern is 04:00 UTC.
        assert_eq!(
            market_date("2026-07-02T03:59:00Z".parse().unwrap()),
            d("2026-07-01")
        );
        assert_eq!(
            market_date("2026-07-02T04:00:00Z".parse().unwrap()),
            d("2026-07-02")
        );
    }

    #[test]
    fn weekdays_skip_the_weekend() {
        let days = weekdays_from(d("2026-11-19"), 5); // Thu .. Tue
        let got: Vec<String> = days.iter().map(|x| x.to_string()).collect();
        assert_eq!(
            got,
            ["2026-11-19", "2026-11-20", "2026-11-23", "2026-11-24"]
        );
        assert_eq!(
            weekdays_from(d("2026-11-21"), 0),
            vec![],
            "a Saturday alone is nothing"
        );
    }

    #[test]
    fn a_failed_or_empty_fetch_keeps_the_stored_day() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let date = d("2026-11-18");
        let now: DateTime<Utc> = "2026-11-18T01:00:00Z".parse().unwrap();
        let rows = normalize(date, FIXTURE.as_bytes()).unwrap();

        // Nothing stored, fetch fails: reported, nothing written.
        let o = apply_fetch(dir, date, Err("HTTP 503".into()), now);
        assert_eq!((o.ok(), o.written, o.rows), (false, false, None));
        assert_eq!(read_day(dir, date), Ok(None));

        // First good fetch is written.
        let o = apply_fetch(dir, date, Ok(rows.clone()), now);
        assert_eq!(
            (o.ok(), o.written, o.rows, o.kept_stored),
            (true, true, Some(6), false)
        );

        // A later failure leaves it alone and says how many rows are stored.
        let later = now + chrono::Duration::hours(6);
        let o = apply_fetch(dir, date, Err("timeout".into()), later);
        assert_eq!((o.ok(), o.written, o.rows), (false, false, Some(6)));
        assert_eq!(read_day(dir, date).unwrap().unwrap().fetched_at, now);

        // So does an empty answer.
        let o = apply_fetch(dir, date, Ok(vec![]), later);
        assert_eq!(
            (o.ok(), o.written, o.rows, o.kept_stored),
            (true, false, Some(6), true)
        );
        assert_eq!(read_day(dir, date).unwrap().unwrap().rows, rows);

        // A good fetch replaces it.
        let o = apply_fetch(dir, date, Ok(rows[..2].to_vec()), later);
        assert_eq!((o.written, o.rows), (true, Some(2)));
        assert_eq!(read_day(dir, date).unwrap().unwrap().fetched_at, later);

        // An empty day with nothing stored is written as empty (a holiday is
        // then "fetched, nothing reports", not "never fetched").
        let hol = d("2026-11-26");
        let o = apply_fetch(dir, hol, Ok(vec![]), now);
        assert_eq!((o.written, o.rows, o.kept_stored), (true, Some(0), false));
        assert_eq!(read_day(dir, hol).unwrap().unwrap().rows, vec![]);
    }
}
