//! What a company actually reported: EPS against the consensus, quarter by
//! quarter (`~/.wickd/earnings/results/SYM.json`).
//!
//! The earnings calendar ([`crate::earnings_store`]) only carries what is
//! expected. This is the other half: once a company has reported, its
//! reported EPS, the consensus it was measured against, and the surprise.
//!
//! ## Source
//!
//! Nasdaq's per-company earnings-surprise table ([`NASDAQ_SURPRISE_URL`]),
//! one request per company, no key. Like the calendar it is **unofficial**,
//! and it has a property worth knowing: **it can lag a report by a day or
//! so**. A company that reported this morning may still show last quarter
//! as its newest row. So a result is only ever matched to a report by its
//! date ([`ResultsFile::for_report`]); the newest row is never assumed to
//! be the report in question.
//!
//! The same rules as the calendar store apply: every field is optional on
//! the way in, a fetch that fails or comes back empty keeps what is stored
//! ([`apply_fetch`]), and each file is written whole and atomically.
//!
//! A symbol becomes part of a URL and a file name, so only a plain ticker
//! is accepted for either ([`results_symbol`]).

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::earnings_store::{clean_text, parse_money, SOURCE};

/// Nasdaq's earnings-surprise table for one company; `{symbol}` is replaced
/// by the ticker.
pub const NASDAQ_SURPRISE_URL: &str =
    "https://api.nasdaq.com/api/company/{symbol}/earnings-surprise";

/// The most response body read. The table is about 1 KB.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// The most quarters kept (the feed gives four).
const MAX_ROWS: usize = 40;

/// Pause between company requests in [`sync_results`].
const REQUEST_GAP: Duration = Duration::from_millis(250);

/// How far a row's "date reported" may sit from the calendar's report date
/// and still be the same report (the two feeds can disagree by a day).
const MATCH_DAYS: i64 = 1;

/// `<earnings dir>/results`.
pub fn results_dir(earnings_dir: &Path) -> PathBuf {
    earnings_dir.join("results")
}

/// A ticker that is safe to put in a URL path and a file name: letters,
/// digits, `.` and `-` only, not starting with a dot, at most 12
/// characters. `None` for anything else.
pub fn results_symbol(s: &str) -> Option<String> {
    let t = s.trim().to_uppercase();
    let ok = !t.is_empty()
        && t.chars().count() <= 12
        && !t.starts_with('.')
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
    ok.then_some(t)
}

/// One reported quarter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultRow {
    /// The quarter, as the feed words it (`Oct 2026`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fiscal_quarter: String,
    /// The day it was reported.
    pub reported: NaiveDate,
    /// Reported EPS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eps: Option<Decimal>,
    /// The consensus EPS it was measured against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consensus: Option<Decimal>,
    /// The surprise, in percent of the consensus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surprise_pct: Option<Decimal>,
}

/// One company's stored results, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultsFile {
    pub symbol: String,
    pub fetched_at: DateTime<Utc>,
    pub source: String,
    pub rows: Vec<ResultRow>,
}

impl ResultsFile {
    /// The row for the report on `date`: the one reported within a day of
    /// it. `None` when the feed hasn't caught up with that report yet.
    pub fn for_report(&self, date: NaiveDate) -> Option<&ResultRow> {
        self.rows
            .iter()
            .filter(|r| (r.reported - date).num_days().abs() <= MATCH_DAYS)
            .min_by_key(|r| (r.reported - date).num_days().abs())
    }
}

// ── the feed ────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
struct RawResponse {
    #[serde(default)]
    data: Option<RawData>,
}

#[derive(Debug, Default, Deserialize)]
struct RawData {
    #[serde(default, rename = "earningsSurpriseTable")]
    table: Option<RawTable>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTable {
    #[serde(default)]
    rows: Option<Vec<RawRow>>,
}

/// A feed row, read loosely: the feed sends `eps` as a number and the
/// others as strings, and that could change.
#[derive(Debug, Default, Deserialize)]
struct RawRow {
    #[serde(default, rename = "fiscalQtrEnd")]
    fiscal_quarter: serde_json::Value,
    #[serde(default, rename = "dateReported")]
    reported: serde_json::Value,
    #[serde(default)]
    eps: serde_json::Value,
    #[serde(default, rename = "consensusForecast")]
    consensus: serde_json::Value,
    #[serde(default, rename = "percentageSurprise")]
    surprise: serde_json::Value,
}

fn text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// A plain or signed decimal (`2.22`, `-0.4`, `6.22`), or a dollar figure.
/// `N/A` and anything else that isn't a number is `None`.
fn number(v: &serde_json::Value) -> Option<Decimal> {
    let t = text(v);
    let t = t.trim();
    Decimal::from_str(t).ok().or_else(|| parse_money(t))
}

/// The feed's `M/D/YYYY`.
fn feed_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%m/%d/%Y").ok()
}

/// Turn the feed's body into rows, newest first. A row with no usable date
/// is dropped (it can't be matched to a report). A body that isn't the
/// feed's JSON is an error; a company with no table is an empty result.
pub fn normalize(symbol: &str, body: &[u8]) -> Result<Vec<ResultRow>, String> {
    let raw: RawResponse = serde_json::from_slice(body)
        .map_err(|e| format!("earnings results for {symbol} are not the expected JSON: {e}"))?;
    let Some(data) = raw.data else {
        return Err(format!(
            "earnings results for {symbol}: no data (the request was rejected, or the symbol isn't known)"
        ));
    };
    let mut rows: Vec<ResultRow> = data
        .table
        .and_then(|t| t.rows)
        .unwrap_or_default()
        .into_iter()
        .take(MAX_ROWS)
        .filter_map(|r| {
            Some(ResultRow {
                fiscal_quarter: clean_text(&text(&r.fiscal_quarter), 16),
                reported: feed_date(&text(&r.reported))?,
                eps: number(&r.eps),
                consensus: number(&r.consensus),
                surprise_pct: number(&r.surprise),
            })
        })
        .collect();
    rows.sort_by(|a, b| b.reported.cmp(&a.reported));
    rows.dedup_by(|a, b| a.reported == b.reported);
    Ok(rows)
}

/// Fetch and normalize one company's results. `url_template` is
/// [`NASDAQ_SURPRISE_URL`] outside tests.
pub async fn fetch_results(
    client: &reqwest::Client,
    url_template: &str,
    symbol: &str,
) -> Result<Vec<ResultRow>, String> {
    let sym = results_symbol(symbol).ok_or_else(|| format!("'{symbol}' is not a stock ticker"))?;
    let url = url_template.replace("{symbol}", &sym);
    let mut resp = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("fetching results for {sym}: {}", e.without_url()))?;
    if !resp.status().is_success() {
        return Err(format!(
            "fetching results for {sym}: HTTP {}",
            resp.status()
        ));
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("reading results for {sym}: {}", e.without_url()))?
    {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(format!(
                "results for {sym} are larger than 1 MB; not reading them"
            ));
        }
        body.extend_from_slice(&chunk);
    }
    normalize(&sym, &body)
}

// ── the store ───────────────────────────────────────────────────────────

fn file_path(dir: &Path, symbol: &str) -> Option<PathBuf> {
    results_symbol(symbol).map(|s| dir.join(format!("{s}.json")))
}

/// Write one company's results, replacing what was there, atomically.
pub fn write_results(dir: &Path, file: &ResultsFile) -> Result<(), String> {
    let path = file_path(dir, &file.symbol)
        .ok_or_else(|| format!("'{}' is not a stock ticker", file.symbol))?;
    fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let tmp = path.with_extension("json.tmp");
    let bytes =
        serde_json::to_vec_pretty(file).map_err(|e| format!("encoding {}: {e}", path.display()))?;
    fs::write(&tmp, bytes).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path)
        .map_err(|e| format!("renaming {} → {}: {e}", tmp.display(), path.display()))
}

/// Read one company's stored results; `None` when there is no file (or the
/// symbol isn't a ticker).
pub fn read_results(dir: &Path, symbol: &str) -> Result<Option<ResultsFile>, String> {
    let Some(path) = file_path(dir, symbol) else {
        return Ok(None);
    };
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("{} is not a valid results file: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("reading {}: {e}", path.display())),
    }
}

/// What happened to one company in a sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResultOutcome {
    pub symbol: String,
    /// Quarters now stored for it, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<usize>,
    /// The newest stored quarter's report date, when known: a reader can
    /// tell at a glance whether the feed has caught up with a report.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub newest: Option<NaiveDate>,
    pub written: bool,
    /// The fetch returned nothing and stored rows were kept.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub kept_stored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ResultOutcome {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }
}

/// Store what one company's fetch returned, applying the keep-on-failure
/// and keep-on-empty rules.
pub fn apply_fetch(
    dir: &Path,
    symbol: &str,
    fetched: Result<Vec<ResultRow>, String>,
    now: DateTime<Utc>,
) -> ResultOutcome {
    let stored = || read_results(dir, symbol).ok().flatten();
    let summary = |f: Option<ResultsFile>| {
        (
            f.as_ref().map(|f| f.rows.len()),
            f.and_then(|f| f.rows.first().map(|r| r.reported)),
        )
    };
    let failed = |error: String| {
        let (rows, newest) = summary(stored());
        ResultOutcome {
            symbol: symbol.to_string(),
            rows,
            newest,
            written: false,
            kept_stored: false,
            error: Some(error),
        }
    };
    let rows = match fetched {
        Ok(r) => r,
        Err(e) => return failed(e),
    };
    if rows.is_empty() {
        if let Some(f) = stored().filter(|f| !f.rows.is_empty()) {
            let (rows, newest) = summary(Some(f));
            return ResultOutcome {
                symbol: symbol.to_string(),
                rows,
                newest,
                written: false,
                kept_stored: true,
                error: None,
            };
        }
    }
    let file = ResultsFile {
        symbol: symbol.to_string(),
        fetched_at: now,
        source: SOURCE.to_string(),
        rows,
    };
    match write_results(dir, &file) {
        Ok(()) => {
            let (rows, newest) = summary(Some(file));
            ResultOutcome {
                symbol: symbol.to_string(),
                rows,
                newest,
                written: true,
                kept_stored: false,
                error: None,
            }
        }
        Err(e) => failed(e),
    }
}

/// Fetch each of `symbols` in turn and store it. One company failing never
/// stops the others.
pub async fn sync_results(
    client: &reqwest::Client,
    url_template: &str,
    dir: &Path,
    symbols: &[String],
) -> Vec<ResultOutcome> {
    let mut out = Vec::with_capacity(symbols.len());
    for (i, symbol) in symbols.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(REQUEST_GAP).await;
        }
        let fetched = fetch_results(client, url_template, symbol).await;
        out.push(apply_fetch(dir, symbol, fetched, Utc::now()));
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

    /// NVDA's table as captured on 2026-11-17 (the evening it reported: the
    /// newest row is still the quarter before), plus odd shapes.
    const FIXTURE: &str = r#"{"data":{"symbol":"NVDA","chart":[],"earningsSurpriseTable":{"headers":{"eps":"Earnings Per Share*"},"rows":[
      {"fiscalQtrEnd":"Jul 2026","dateReported":"8/26/2026","eps":2.22,"consensusForecast":"2.09","percentageSurprise":"6.22"},
      {"fiscalQtrEnd":"Apr 2026","dateReported":"5/20/2026","eps":1.87,"consensusForecast":"1.7","percentageSurprise":"10"},
      {"fiscalQtrEnd":"Jan 2026","dateReported":"2/25/2026","eps":"-0.4","consensusForecast":"N/A","percentageSurprise":null},
      {"fiscalQtrEnd":"Oct 2025","dateReported":"not a date","eps":1.3,"consensusForecast":"1.25","percentageSurprise":"4"},
      {"fiscalQtrEnd":"Bad\u001b[31m Quarter","dateReported":"11/19/2025","eps":null}
    ]}},"status":{"rCode":200}}"#;

    #[test]
    fn normalize_reads_numbers_and_strings_and_drops_undated_rows() {
        let rows = normalize("NVDA", FIXTURE.as_bytes()).unwrap();
        assert_eq!(
            rows.len(),
            4,
            "the row with no date can't be matched to a report"
        );
        assert_eq!(
            rows[0],
            ResultRow {
                fiscal_quarter: "Jul 2026".into(),
                reported: d("2026-08-26"),
                eps: Some(dec!(2.22)),
                consensus: Some(dec!(2.09)),
                surprise_pct: Some(dec!(6.22)),
            }
        );
        // A loss as a string, a missing consensus, a null surprise.
        assert_eq!(
            (rows[2].eps, rows[2].consensus, rows[2].surprise_pct),
            (Some(dec!(-0.4)), None, None)
        );
        // Newest first, and text made safe to draw.
        assert_eq!(rows[3].reported, d("2025-11-19"));
        assert_eq!(rows[3].fiscal_quarter, "Bad[31m Quarter");
        assert_eq!(rows[3].eps, None);
    }

    #[test]
    fn normalize_tells_no_table_from_a_rejected_request() {
        let none =
            br#"{"data":{"symbol":"ZZZ","earningsSurpriseTable":null},"status":{"rCode":200}}"#;
        assert_eq!(normalize("ZZZ", none), Ok(vec![]));
        let rejected = br#"{"data":null,"status":{"rCode":400}}"#;
        assert!(normalize("ZZZ", rejected).unwrap_err().contains("ZZZ"));
        assert!(normalize("ZZZ", b"<html>").is_err());
    }

    #[test]
    fn a_result_is_matched_to_a_report_by_date_never_by_being_newest() {
        let file = ResultsFile {
            symbol: "NVDA".into(),
            fetched_at: Utc::now(),
            source: SOURCE.into(),
            rows: normalize("NVDA", FIXTURE.as_bytes()).unwrap(),
        };
        // The evening NVDA reported, the feed still ends at August: no match.
        assert_eq!(file.for_report(d("2026-11-17")), None);
        // The August report matches on the day, and a day either side.
        for day in ["2026-08-26", "2026-08-25", "2026-08-27"] {
            assert_eq!(
                file.for_report(d(day)).map(|r| r.eps),
                Some(Some(dec!(2.22))),
                "{day}"
            );
        }
        assert_eq!(
            file.for_report(d("2026-08-28")),
            None,
            "two days off is a different event"
        );
    }

    #[test]
    fn only_a_plain_ticker_reaches_a_url_or_a_file_name() {
        assert_eq!(results_symbol(" nvda "), Some("NVDA".into()));
        assert_eq!(results_symbol("brk.b"), Some("BRK.B".into()));
        assert_eq!(results_symbol("BF-B"), Some("BF-B".into()));
        for bad in [
            "",
            "..",
            ".HIDDEN",
            "A/B",
            "../X",
            "A B",
            "X?y=1",
            "TOOLONGSYMBOL1",
            "A^B",
        ] {
            assert_eq!(results_symbol(bad), None, "{bad:?}");
        }
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read_results(tmp.path(), "../etc/passwd"), Ok(None));
        let bad = ResultsFile {
            symbol: "../x".into(),
            fetched_at: Utc::now(),
            source: SOURCE.into(),
            rows: vec![],
        };
        assert!(write_results(tmp.path(), &bad).is_err());
        assert_eq!(
            fs::read_dir(tmp.path()).unwrap().count(),
            0,
            "nothing was written"
        );
    }

    #[test]
    fn a_failed_or_empty_fetch_keeps_the_stored_results() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = results_dir(tmp.path());
        let now: DateTime<Utc> = "2026-11-18T03:00:00Z".parse().unwrap();
        let rows = normalize("NVDA", FIXTURE.as_bytes()).unwrap();

        let o = apply_fetch(&dir, "NVDA", Err("HTTP 503".into()), now);
        assert_eq!(
            (o.ok(), o.written, o.rows, o.newest),
            (false, false, None, None)
        );
        assert_eq!(read_results(&dir, "NVDA"), Ok(None));

        let o = apply_fetch(&dir, "NVDA", Ok(rows.clone()), now);
        assert_eq!(
            (o.ok(), o.written, o.rows, o.newest),
            (true, true, Some(4), Some(d("2026-08-26")))
        );
        let later = now + chrono::Duration::hours(12);
        let o = apply_fetch(&dir, "NVDA", Err("timeout".into()), later);
        assert_eq!((o.ok(), o.written, o.rows), (false, false, Some(4)));
        let o = apply_fetch(&dir, "NVDA", Ok(vec![]), later);
        assert_eq!(
            (o.ok(), o.written, o.kept_stored, o.rows),
            (true, false, true, Some(4))
        );
        assert_eq!(read_results(&dir, "NVDA").unwrap().unwrap().fetched_at, now);

        // The feed catches up: the new quarter replaces the file.
        let mut caught_up = rows.clone();
        caught_up.insert(
            0,
            ResultRow {
                fiscal_quarter: "Oct 2026".into(),
                reported: d("2026-11-17"),
                eps: Some(dec!(2.68)),
                consensus: Some(dec!(2.49)),
                surprise_pct: Some(dec!(7.63)),
            },
        );
        let o = apply_fetch(&dir, "NVDA", Ok(caught_up), later);
        assert_eq!(
            (o.written, o.rows, o.newest),
            (true, Some(5), Some(d("2026-11-17")))
        );
        let file = read_results(&dir, "NVDA").unwrap().unwrap();
        assert_eq!(
            file.for_report(d("2026-11-17")).unwrap().eps,
            Some(dec!(2.68))
        );
        // No temp file is left, and a file that isn't one is an error that names it.
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["NVDA.json"]);
        fs::write(dir.join("HD.json"), b"not json").unwrap();
        assert!(read_results(&dir, "HD").unwrap_err().contains("HD.json"));
    }
}
