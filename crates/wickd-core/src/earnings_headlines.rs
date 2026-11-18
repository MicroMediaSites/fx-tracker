//! The headlines that state an earnings report's results, the same day
//! (`~/.wickd/earnings/headlines/SYM_YYYY-MM-DD.json`).
//!
//! The reported-figures table ([`crate::earnings_results`]) runs a day or
//! two behind a report. News headlines don't: a wire headline with the EPS,
//! the sales figure and the beat or miss is usually out within minutes. So
//! this module takes the headlines published right after a report
//! ([`window`]), keeps the few that state its results ([`pick`]), and stores
//! them per report so they can be read later without a network.
//!
//! Picking is by what a headline **says**, not by a fixed format: the
//! figures line (EPS, sales or revenue, a beat or a miss against an
//! estimate) ranks highest, guidance lines next, and previews, analyst
//! notes and unrelated news are left out. A headline is shown as published;
//! nothing is extracted from it or computed, so a wrong pick is a visible
//! wrong headline, never a wrong number.
//!
//! Headline text is untrusted and ends up on a terminal: it is cleaned of
//! control characters and capped before it is stored. A symbol becomes part
//! of a file name, so only a plain ticker is accepted.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::earnings_results::results_symbol;
use crate::earnings_store::{clean_text, Session};
use crate::equities::alpaca::NewsItem;
use crate::equities::eastern_offset_hours;

/// The most headlines kept for one report.
pub const MAX_PICKED: usize = 3;

/// The longest a stored headline runs.
const MAX_HEADLINE_CHARS: usize = 240;

/// The most stored links run (anything longer isn't a link worth keeping).
const MAX_URL_CHARS: usize = 400;

/// `<earnings dir>/headlines`.
pub fn headlines_dir(earnings_dir: &Path) -> PathBuf {
    earnings_dir.join("headlines")
}

/// One stored headline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Headline {
    pub time: DateTime<Utc>,
    pub headline: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// The headlines stored for one report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadlinesFile {
    pub symbol: String,
    /// The report's market date.
    pub date: NaiveDate,
    pub fetched_at: DateTime<Utc>,
    /// How many headlines the window held: zero means the feed had nothing
    /// yet, not that nothing matched.
    pub scanned: usize,
    /// The headlines that state the results, best first.
    pub picked: Vec<Headline>,
}

/// `hour:00` US/Eastern on `date`, as an instant.
fn eastern(date: NaiveDate, hour: u32) -> Option<DateTime<Utc>> {
    let guess = date.and_hms_opt(hour, 0, 0)?.and_utc();
    Some(guess - Duration::hours(eastern_offset_hours(guess)))
}

/// The stretch of time to read headlines from for a report on `date` in
/// `session`: from just before the report could be out, for about a day.
///
/// - before the open: from 4:00 AM Eastern that day (pre-market opens) to
///   midnight;
/// - after the close: from the 4:00 PM Eastern close to noon the next day;
/// - no time given: the whole of both, 4:00 AM to noon the next day.
pub fn window(date: NaiveDate, session: Session) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let next = date.succ_opt()?;
    Some(match session {
        Session::PreMarket => (eastern(date, 4)?, eastern(next, 0)?),
        Session::AfterHours => (eastern(date, 16)?, eastern(next, 12)?),
        Session::Unknown => (eastern(date, 4)?, eastern(next, 12)?),
    })
}

/// How strongly a headline reads as "these are the results" (0 = it
/// doesn't). Words are matched whole and without regard to case.
///
/// - **Figures** (highest): it names EPS, or sales / revenue, and has a
///   dollar figure; more so with a beat, a miss or an estimate.
/// - **Guidance**: it sees, guides, raises, cuts or reaffirms an outlook.
/// - **Not results**: previews ("ahead of", "what to expect"), analyst
///   actions ("price target", "downgrades") and the like score zero even
///   when they mention earnings.
pub fn score(headline: &str) -> u32 {
    let lower = headline.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !(c.is_alphanumeric() || c == '$' || c == '%'))
        .filter(|w| !w.is_empty())
        .collect();
    let has = |w: &str| words.contains(&w);
    let any = |ws: &[&str]| ws.iter().any(|w| has(w));
    let phrase = |p: &str| lower.contains(p);

    // What is said before a report, or about it by someone else.
    const NOT_RESULTS: &[&str] = &[
        "ahead of",
        "what to expect",
        "preview",
        "to report",
        "will report",
        "set to report",
        "price target",
        "analyst",
        "downgrade",
        "upgrade",
        "initiates",
        "options",
        "why ",
        "what's going on",
        "whats going on",
        "stock is",
        "shares are",
        "expected to",
    ];
    if NOT_RESULTS.iter().any(|p| phrase(p)) {
        return 0;
    }

    let money = lower.contains('$');
    let eps = has("eps") || phrase("earnings per share");
    let top_line = any(&["sales", "revenue", "revenues"]);
    let versus = any(&[
        "beat",
        "beats",
        "miss",
        "misses",
        "missed",
        "tops",
        "topped",
        "estimate",
        "estimates",
        "consensus",
        "inline",
    ]) || phrase("in line")
        || phrase("in-line");
    // `Q3`, `quarter`, `quarterly`, `FY`, `FY27`, "full year".
    let fiscal_year = |w: &&str| {
        w.strip_prefix("fy")
            .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
    };
    let quarter = any(&["q1", "q2", "q3", "q4", "quarter", "quarterly"])
        || words.iter().any(fiscal_year)
        || phrase("full year")
        || phrase("full-year");
    let guidance = any(&[
        "guidance",
        "outlook",
        "sees",
        "guides",
        "forecast",
        "forecasts",
        "raises",
        "lowers",
        "cuts",
        "reaffirms",
        "affirms",
        "expects",
    ]);
    let results_word = has("results") || has("reports") || has("earnings");

    let mut s = 0;
    if eps && money {
        s += 60;
    } else if eps {
        s += 30;
    }
    if top_line && money {
        s += 30;
    }
    if versus && (eps || top_line) {
        s += 30;
    }
    if guidance && (money || quarter) {
        s += 35;
    }
    if quarter && (results_word || eps || top_line) {
        s += 10;
    }
    s
}

/// The headlines among `items` that state a report's results: the
/// highest-scoring first (the earlier of equals), at most [`MAX_PICKED`],
/// cleaned for storing and drawing. Items that score zero are left out.
pub fn pick(items: &[NewsItem]) -> Vec<Headline> {
    let mut scored: Vec<(u32, &NewsItem)> = items
        .iter()
        .map(|n| (score(&n.headline), n))
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.created_at.cmp(&b.1.created_at)));
    let mut seen: Vec<String> = Vec::new();
    scored
        .into_iter()
        .filter_map(|(_, n)| {
            let headline = clean_text(&n.headline, MAX_HEADLINE_CHARS);
            // The same headline often runs twice (an update, a repost).
            let key = headline.to_lowercase();
            if headline.is_empty() || seen.contains(&key) {
                return None;
            }
            seen.push(key);
            Some(Headline {
                time: n.created_at,
                headline,
                source: clean_text(&n.source, 40),
                url: n
                    .url
                    .as_deref()
                    .filter(|u| u.starts_with("https://") && u.chars().count() <= MAX_URL_CHARS)
                    .filter(|u| !u.chars().any(|c| c.is_control() || c.is_whitespace()))
                    .map(str::to_string),
            })
        })
        .take(MAX_PICKED)
        .collect()
}

// ── the store ───────────────────────────────────────────────────────────

fn file_path(dir: &Path, symbol: &str, date: NaiveDate) -> Option<PathBuf> {
    results_symbol(symbol).map(|s| dir.join(format!("{s}_{}.json", date.format("%Y-%m-%d"))))
}

/// Write one report's headlines, replacing what was there, atomically.
pub fn write_headlines(dir: &Path, file: &HeadlinesFile) -> Result<(), String> {
    let path = file_path(dir, &file.symbol, file.date)
        .ok_or_else(|| format!("'{}' is not a stock ticker", file.symbol))?;
    fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let tmp = path.with_extension("json.tmp");
    let bytes =
        serde_json::to_vec_pretty(file).map_err(|e| format!("encoding {}: {e}", path.display()))?;
    fs::write(&tmp, bytes).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path)
        .map_err(|e| format!("renaming {} → {}: {e}", tmp.display(), path.display()))
}

/// Read one report's stored headlines; `None` when there is no file (or the
/// symbol isn't a ticker).
pub fn read_headlines(
    dir: &Path,
    symbol: &str,
    date: NaiveDate,
) -> Result<Option<HeadlinesFile>, String> {
    let Some(path) = file_path(dir, symbol, date) else {
        return Ok(None);
    };
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("{} is not a valid headlines file: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("reading {}: {e}", path.display())),
    }
}

/// Store what a fetch for one report returned. A fetch that picked nothing
/// never replaces stored headlines (a feed that is briefly empty shouldn't
/// lose what was already read); otherwise the new file replaces the old.
/// Returns what is stored afterwards.
pub fn store_fetch(
    dir: &Path,
    symbol: &str,
    date: NaiveDate,
    items: &[NewsItem],
    now: DateTime<Utc>,
) -> Result<HeadlinesFile, String> {
    let picked = pick(items);
    if picked.is_empty() {
        if let Some(stored) = read_headlines(dir, symbol, date)?.filter(|f| !f.picked.is_empty()) {
            return Ok(stored);
        }
    }
    let file = HeadlinesFile {
        symbol: results_symbol(symbol)
            .ok_or_else(|| format!("'{symbol}' is not a stock ticker"))?,
        date,
        fetched_at: now,
        scanned: items.len(),
        picked,
    };
    write_headlines(dir, &file)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn item(id: i64, time: &str, headline: &str) -> NewsItem {
        NewsItem {
            id,
            headline: headline.into(),
            summary: String::new(),
            created_at: at(time),
            symbols: vec!["NVDA".into()],
            source: "benzinga".into(),
            url: Some(format!("https://example.com/news/{id}")),
        }
    }

    // Typical wire-headline shapes. These are not captures from the feed:
    // the picking keys on what a headline says so that it doesn't depend on
    // one exact form.
    const FIGURES: &str =
        "NVIDIA Q3 Adj. EPS $2.68 Beats $2.47 Estimate, Sales $115.80B Beat $109.15B Estimate";
    const GUIDANCE: &str = "NVIDIA Sees Q4 Sales $130.00B +/- 3% vs $125.30B Est";
    const MISS: &str = "Home Depot Q3 Adjusted EPS $3.74 Misses $3.84 Estimate, Sales $41.35B Beat $41.10B Estimate";

    #[test]
    fn a_figures_headline_scores_highest_then_guidance_and_chatter_scores_nothing() {
        assert!(
            score(FIGURES) > score(GUIDANCE),
            "{} vs {}",
            score(FIGURES),
            score(GUIDANCE)
        );
        assert!(score(MISS) >= 100);
        assert!(score(GUIDANCE) > 0);
        assert!(
            score("Target Reports Third Quarter Earnings") > 0,
            "plain but on topic"
        );
        assert!(score("Walmart Q3 Revenue $187.5B, Up 5% YoY") > 0);
        assert!(score("Intuit Raises FY27 Guidance") > 0);
        for chatter in [
            "What To Expect From NVIDIA Earnings",
            "NVIDIA To Report Q3 Earnings After The Close: Here's A Preview",
            "Analyst Raises NVIDIA Price Target To $220 After Q3 EPS Beat",
            "Wedbush Maintains Outperform on NVIDIA, Raises Price Target to $210",
            "Why NVIDIA Shares Are Rising After Hours",
            "NVIDIA Options Traders Price An 8.5% Move Ahead Of Earnings",
            "NVIDIA Board Chairman Confirms Huang Will Not Be Reinstated",
            "Stocks Slide As Software Leads Tech Lower",
            "",
        ] {
            assert_eq!(score(chatter), 0, "{chatter:?}");
        }
        // Words are matched whole: "steps" is not "eps", "missile" not "miss".
        assert_eq!(score("Company Steps Up Missile Production"), 0);
    }

    #[test]
    fn pick_keeps_the_results_headlines_best_first_and_cleans_them() {
        let items = vec![
            item(
                1,
                "2026-11-17T21:05:00Z",
                "Why NVIDIA Shares Are Rising After Hours",
            ),
            item(2, "2026-11-17T21:21:00Z", FIGURES),
            item(3, "2026-11-17T21:22:00Z", GUIDANCE),
            // The same figures line again, as an update.
            item(4, "2026-11-17T21:30:00Z", FIGURES),
            item(
                5,
                "2026-11-17T21:40:00Z",
                "NVIDIA Raises Quarterly Outlook, Expects Q4 Gross Margin 71%-72%",
            ),
            item(
                6,
                "2026-11-17T21:50:00Z",
                "NVIDIA Reports\u{1b}[31m Third Quarter\tResults",
            ),
            item(
                7,
                "2026-11-18T13:00:00Z",
                "Analyst Maintains Buy On NVIDIA After Q3 EPS Beat",
            ),
        ];
        let got = pick(&items);
        assert_eq!(got.len(), MAX_PICKED);
        assert_eq!(got[0].headline, FIGURES);
        assert_eq!(
            got[0].time,
            at("2026-11-17T21:21:00Z"),
            "the first of the two copies"
        );
        assert_eq!(got[1].headline, GUIDANCE);
        assert!(got[2]
            .headline
            .starts_with("NVIDIA Raises Quarterly Outlook"));
        assert!(got.iter().all(|h| h.source == "benzinga"));
        assert_eq!(got[0].url.as_deref(), Some("https://example.com/news/2"));
        // Nothing but chatter: nothing picked.
        assert!(pick(&items[..1]).is_empty());
        assert!(pick(&[]).is_empty());
        // Text is made safe to draw, and a link that isn't one is dropped.
        let mut odd = item(
            8,
            "2026-11-17T21:50:00Z",
            "NVIDIA Q3 EPS\u{1b}[31m $2.68\tBeats Estimate",
        );
        odd.url = Some("javascript:alert(1)".into());
        odd.source = "bad\u{7}source".into();
        let got = pick(&[odd]);
        assert_eq!(got[0].headline, "NVIDIA Q3 EPS[31m $2.68 Beats Estimate");
        assert_eq!(
            (got[0].url.clone(), got[0].source.as_str()),
            (None, "badsource")
        );
    }

    #[test]
    fn the_window_starts_when_the_report_can_be_out() {
        // Winter (EST = UTC-5). Before the open: 4 AM Eastern to midnight.
        assert_eq!(
            window(d("2026-11-17"), Session::PreMarket),
            Some((at("2026-11-17T09:00:00Z"), at("2026-11-18T05:00:00Z")))
        );
        // After the close: 4 PM Eastern to noon the next day.
        assert_eq!(
            window(d("2026-11-17"), Session::AfterHours),
            Some((at("2026-11-17T21:00:00Z"), at("2026-11-18T17:00:00Z")))
        );
        // No time given: both.
        assert_eq!(
            window(d("2026-11-17"), Session::Unknown),
            Some((at("2026-11-17T09:00:00Z"), at("2026-11-18T17:00:00Z")))
        );
        // Summer (EDT = UTC-4).
        assert_eq!(
            window(d("2026-07-28"), Session::AfterHours),
            Some((at("2026-07-28T20:00:00Z"), at("2026-07-29T16:00:00Z")))
        );
    }

    #[test]
    fn a_report_s_headlines_round_trip_and_an_empty_fetch_keeps_them() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = headlines_dir(tmp.path());
        let date = d("2026-11-17");
        let now = at("2026-11-17T22:00:00Z");
        assert_eq!(read_headlines(&dir, "NVDA", date), Ok(None));

        // The feed has nothing yet: stored as "scanned 0, nothing picked".
        let none = store_fetch(&dir, "NVDA", date, &[], now).unwrap();
        assert_eq!((none.scanned, none.picked.len()), (0, 0));
        assert_eq!(
            read_headlines(&dir, "NVDA", date)
                .unwrap()
                .unwrap()
                .fetched_at,
            now
        );

        // Then the results headlines arrive.
        let items = vec![
            item(
                1,
                "2026-11-17T21:05:00Z",
                "Why NVIDIA Shares Are Rising After Hours",
            ),
            item(2, "2026-11-17T21:21:00Z", FIGURES),
        ];
        let later = at("2026-11-17T22:30:00Z");
        let got = store_fetch(&dir, "nvda", date, &items, later).unwrap();
        assert_eq!(
            (got.symbol.as_str(), got.scanned, got.picked.len()),
            ("NVDA", 2, 1)
        );
        assert_eq!(read_headlines(&dir, "NVDA", date).unwrap().unwrap(), got);

        // A later fetch that picks nothing (a feed hiccup) keeps them.
        let kept =
            store_fetch(&dir, "NVDA", date, &items[..1], at("2026-11-18T03:00:00Z")).unwrap();
        assert_eq!(kept, got);
        // Another report of the same company is a different file.
        assert_eq!(read_headlines(&dir, "NVDA", d("2026-08-26")), Ok(None));
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["NVDA_2026-11-17.json"]);
        // Only a plain ticker reaches a file name.
        assert!(store_fetch(&dir, "../x", date, &items, now).is_err());
        assert_eq!(read_headlines(&dir, "../x", date), Ok(None));
        // A file that isn't one is an error that names it.
        fs::write(dir.join("HD_2026-11-17.json"), b"not json").unwrap();
        assert!(read_headlines(&dir, "HD", date)
            .unwrap_err()
            .contains("HD_2026-11-17.json"));
    }
}
