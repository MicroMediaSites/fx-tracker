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
//! ## What the feed really carries
//!
//! Checked against the live feed (2026-11-17): for a company's report it
//! holds the **press release**, titled plainly ("NVIDIA Reports Financial
//! Results for Third Quarter Fiscal 2027"), not a wire headline with the
//! numbers in it. The numbers are in the release's opening paragraphs. So
//! for the best-ranked item the release's text is read too ([`lede`]): its
//! first paragraphs that state a figure are kept as plain text and shown
//! under the title. These are the company's own words and numbers; nothing
//! is computed from them.
//!
//! Headline and release text is untrusted and ends up on a terminal. A
//! release arrives as HTML: only its text is ever kept ([`plain_paragraphs`]
//! drops every tag and the contents of scripts and styles), cleaned of
//! control characters and capped. A symbol becomes part of a file name, so
//! only a plain ticker is accepted.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::earnings_results::results_symbol;
use crate::earnings_store::{clean_text, Session};
use crate::equities::alpaca::{AlpacaClient, NewsItem};
use crate::equities::eastern_offset_hours;

/// The most headlines kept for one report.
pub const MAX_PICKED: usize = 3;

/// The longest a stored headline runs.
const MAX_HEADLINE_CHARS: usize = 240;

/// The most stored links run (anything longer isn't a link worth keeping).
const MAX_URL_CHARS: usize = 400;

/// The most paragraphs of a release kept under its title.
pub const MAX_LEDE_PARAGRAPHS: usize = 3;

/// The longest a kept paragraph runs.
const MAX_PARAGRAPH_CHARS: usize = 420;

/// How far into a release the figures are looked for: they are in the
/// opening, and the tables further down are not prose.
const LEDE_SCAN_PARAGRAPHS: usize = 25;

/// The most of a release's markup that is read at all.
const MAX_CONTENT_BYTES: usize = 400_000;

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
    /// The release's opening paragraphs that state figures, as plain text.
    /// Empty when the feed carried no text for it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lede: Vec<String>,
}

impl Headline {
    /// Whether it carries a link to open (the source article).
    pub fn has_link(&self) -> bool {
        self.url.as_deref().is_some_and(|u| !u.trim().is_empty())
    }
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

impl HeadlinesFile {
    /// Whether any picked headline came with the release's figures.
    pub fn has_lede(&self) -> bool {
        self.picked.iter().any(|h| !h.lede.is_empty())
    }
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

/// The text of an HTML fragment, paragraph by paragraph: every tag is
/// dropped, the contents of `<script>` and `<style>` with them, block
/// elements become paragraph breaks, the common entities are decoded, and
/// each paragraph is cleaned of control characters, collapsed to one line
/// and capped. Plain text passes through unchanged (one paragraph per
/// line). Nothing of the markup survives, so the result is safe to draw.
pub fn plain_paragraphs(html: &str) -> Vec<String> {
    // Read a bounded amount, cut on a character boundary.
    let mut end = html.len().min(MAX_CONTENT_BYTES);
    while !html.is_char_boundary(end) {
        end -= 1;
    }
    let html = &html[..end];

    const BLOCKS: &[&str] = &[
        "p",
        "br",
        "div",
        "li",
        "tr",
        "table",
        "ul",
        "ol",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "section",
        "article",
        "blockquote",
        "hr",
    ];
    let mut text = String::with_capacity(html.len() / 2);
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        text.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('>') else {
            // An unclosed tag: nothing after it is text.
            rest = "";
            break;
        };
        let tag = after[..close].trim().to_ascii_lowercase();
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        rest = &after[close + 1..];
        if (name == "script" || name == "style") && !tag.starts_with('/') {
            // Skip to the end of the element; if it never ends, to the end.
            let lower = rest.to_ascii_lowercase();
            rest = match lower.find(&format!("</{name}")) {
                Some(at) => &rest[at..],
                None => "",
            };
        } else if BLOCKS.contains(&name.as_str()) {
            text.push('\n');
        }
    }
    text.push_str(rest);

    decode_entities(&text)
        .split('\n')
        .map(|p| clean_text(p, MAX_PARAGRAPH_CHARS))
        .filter(|p| !p.is_empty())
        .collect()
}

/// Decode the entities a press release uses. An entity that isn't known is
/// left as it is written; a numeric one that isn't a printable character
/// becomes a space.
fn decode_entities(s: &str) -> String {
    const NAMED: &[(&str, &str)] = &[
        ("amp", "&"),
        ("lt", "<"),
        ("gt", ">"),
        ("quot", "\""),
        ("apos", "'"),
        ("nbsp", " "),
        ("reg", "®"),
        ("trade", "™"),
        ("copy", "©"),
        ("mdash", "—"),
        ("ndash", "–"),
        ("rsquo", "’"),
        ("lsquo", "‘"),
        ("rdquo", "”"),
        ("ldquo", "“"),
        ("hellip", "…"),
        ("cent", "¢"),
        ("euro", "€"),
        ("pound", "£"),
    ];
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let decoded = after
            .char_indices()
            .take(10)
            .find(|(_, c)| *c == ';')
            .and_then(|(semi, _)| {
                let name = &after[..semi];
                let ch = if let Some(hex) = name.strip_prefix("#x").or(name.strip_prefix("#X")) {
                    u32::from_str_radix(hex, 16).ok().map(numeric)
                } else if let Some(dec) = name.strip_prefix('#') {
                    dec.parse::<u32>().ok().map(numeric)
                } else {
                    NAMED
                        .iter()
                        .find(|(n, _)| *n == name)
                        .map(|(_, to)| to.to_string())
                };
                ch.map(|c| (c, semi + 1))
            });
        match decoded {
            Some((ch, used)) => {
                out.push_str(&ch);
                rest = &after[used..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A numeric character reference as text: the character when it is a
/// printable one (a non-breaking space as a space), else a space.
fn numeric(code: u32) -> String {
    match char::from_u32(code) {
        Some('\u{a0}') => " ".into(),
        Some(c) if !c.is_control() => c.to_string(),
        _ => " ".into(),
    }
}

/// The opening paragraphs of a release that state figures: among its first
/// [`LEDE_SCAN_PARAGRAPHS`], those of some length that carry a number with
/// a dollar sign or a percent, at most [`MAX_LEDE_PARAGRAPHS`]. From the
/// release's `content` when there is any, else its `summary`.
pub fn lede(content: &str, summary: &str) -> Vec<String> {
    let source = if content.trim().is_empty() {
        summary
    } else {
        content
    };
    plain_paragraphs(source)
        .into_iter()
        .take(LEDE_SCAN_PARAGRAPHS)
        .filter(|p| {
            p.chars().count() >= 40
                && (p.contains('$') || p.contains('%'))
                && p.chars().any(|c| c.is_ascii_digit())
        })
        .take(MAX_LEDE_PARAGRAPHS)
        .collect()
}

/// The items worth showing, best first: scored above zero, in the report's
/// own language (a translated duplicate of a release is skipped), the
/// earlier of equals first.
fn ranked(items: &[NewsItem]) -> Vec<&NewsItem> {
    let translated = |n: &NewsItem| {
        n.url.as_deref().is_some_and(|u| {
            ["/fr/", "/es/", "/de/", "/ja/", "/zh/"]
                .iter()
                .any(|l| u.contains(l))
        })
    };
    let mut scored: Vec<(u32, &NewsItem)> = items
        .iter()
        .filter(|n| !translated(n))
        .map(|n| (score(&n.headline), n))
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.created_at.cmp(&b.1.created_at)));
    scored.into_iter().map(|(_, n)| n).collect()
}

/// The item whose text is worth reading: the best-ranked one.
pub fn best(items: &[NewsItem]) -> Option<&NewsItem> {
    ranked(items).into_iter().next()
}

/// The headlines among `items` that state a report's results: the
/// highest-scoring first (the earlier of equals), at most [`MAX_PICKED`],
/// cleaned for storing and drawing. Items that score zero are left out.
pub fn pick(items: &[NewsItem]) -> Vec<Headline> {
    pick_with(items, &HashMap::new())
}

/// [`pick`], with the release text read for some items (`ledes`, by item
/// id) attached to their headlines.
pub fn pick_with(items: &[NewsItem], ledes: &HashMap<i64, Vec<String>>) -> Vec<Headline> {
    let scored: Vec<(u32, &NewsItem)> = ranked(items).into_iter().map(|n| (0, n)).collect();
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
                lede: ledes.get(&n.id).cloned().unwrap_or_default(),
            })
        })
        .take(MAX_PICKED)
        .collect()
}

/// What [`fetch_report`] read from the feed for one report.
#[derive(Debug, Default)]
pub struct Fetched {
    /// Every headline in the window, oldest first.
    pub items: Vec<NewsItem>,
    /// The release text read, by item id (at most the best item's).
    pub ledes: HashMap<i64, Vec<String>>,
    /// For the best item, how much text the feed carried: characters of
    /// content and of summary. `None` when its body wasn't fetched. Kept so
    /// a live check can tell "no text in the feed" from "text, but no
    /// figures found in it".
    pub body_chars: Option<(usize, usize)>,
}

/// Fetch a report's headlines, and the text of the best one: the window's
/// headlines first (titles only), then one tightly drawn request for the
/// best item's body. A body that can't be fetched is not an error: the
/// headlines still stand without it.
pub async fn fetch_report(
    client: &AlpacaClient,
    symbol: &str,
    date: NaiveDate,
    session: Session,
    now: DateTime<Utc>,
) -> Result<Fetched, String> {
    let (start, end) =
        window(date, session).ok_or_else(|| format!("no time window for {symbol} on {date}"))?;
    let symbols = [symbol.to_string()];
    let items = client
        .news_window(&symbols, start, end.min(now), 50)
        .await
        .map_err(|e| e.to_string())?;
    let mut ledes = HashMap::new();
    let mut body_chars = None;
    if let Some(top) = best(&items) {
        let around = Duration::seconds(1);
        let bodies = client
            .news_bodies(
                &symbols,
                top.created_at - around,
                top.created_at + around,
                5,
            )
            .await
            .unwrap_or_default();
        if let Some(body) = bodies.iter().find(|b| b.id == top.id) {
            body_chars = Some((body.content.chars().count(), body.summary.chars().count()));
            let text = lede(&body.content, &body.summary);
            if !text.is_empty() {
                ledes.insert(top.id, text);
            }
        }
    }
    Ok(Fetched {
        items,
        ledes,
        body_chars,
    })
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
    store_fetch_with(dir, symbol, date, items, &HashMap::new(), now)
}

/// [`store_fetch`], with release text (`ledes`, by item id). A fetch that
/// brings no release text never replaces a stored file that has some.
pub fn store_fetch_with(
    dir: &Path,
    symbol: &str,
    date: NaiveDate,
    items: &[NewsItem],
    ledes: &HashMap<i64, Vec<String>>,
    now: DateTime<Utc>,
) -> Result<HeadlinesFile, String> {
    let picked = pick_with(items, ledes);
    let stored = read_headlines(dir, symbol, date)?;
    if let Some(stored) = stored {
        let nothing_new = picked.is_empty() && !stored.picked.is_empty();
        let would_lose_text = stored.has_lede() && !picked.iter().any(|h| !h.lede.is_empty());
        if nothing_new || would_lose_text {
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
        assert!(got.iter().all(|h| h.lede.is_empty()));
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

    /// A release shaped like the ones the feed links to.
    const RELEASE: &str = r#"<html><head><style>p { color: red } .x::after { content: "$9.99 fake" }</style>
<script>var leak = "EPS $0.01 misses";</script></head><body>
<h1>The Home Depot Announces Third Quarter Fiscal 2026 Results</h1>
<p>ATLANTA, Nov. 17, 2026 /CNW/ -- The Home Depot&reg;, the world&#39;s largest home improvement retailer, today reported sales of $43.4&nbsp;billion for the third quarter of fiscal 2026, an increase of 4.9% from the third quarter of fiscal 2025.</p>
<p>Net earnings for the third quarter of fiscal 2026 were $3.7 billion, or $3.72 per diluted share, compared with net earnings of $3.6 billion, or $3.62 per diluted share, in the same period of fiscal 2025.</p>
<p>&ldquo;Our third quarter results were in line with our expectations,&rdquo; said the chair.</p>
<ul><li>Total sales growth of approximately 3.0%</li><li>Comparable sales growth to be slightly positive for the comparable 52-week period, about 1.0% to 2.0%</li></ul>
<p>A fourth paragraph with a figure, $1.0 billion of share repurchases, that is past the three kept.</p>
<table><tr><td>Net sales</td><td>$43,400</td></tr></table>
</body></html>"#;

    #[test]
    fn a_release_becomes_plain_paragraphs_and_no_markup_survives() {
        let paras = plain_paragraphs(RELEASE);
        assert!(
            paras.iter().all(|p| !p.contains('<') && !p.contains('>')),
            "{paras:?}"
        );
        // Script and style contents are gone, not just their tags.
        assert!(!paras
            .iter()
            .any(|p| p.contains("leak") || p.contains("fake") || p.contains("color")));
        assert_eq!(
            paras[0],
            "The Home Depot Announces Third Quarter Fiscal 2026 Results"
        );
        // Entities are decoded: ®, an apostrophe, a non-breaking space, quotes.
        assert!(paras[1]
            .starts_with("ATLANTA, Nov. 17, 2026 /CNW/ -- The Home Depot®, the world's largest"));
        assert!(paras[1].contains("sales of $43.4 billion"));
        assert!(paras
            .iter()
            .any(|p| p.starts_with("“Our third quarter results")));
        // List items and table rows are their own paragraphs.
        assert!(paras.contains(&"Total sales growth of approximately 3.0%".to_string()));

        // Plain text passes through, one paragraph per line.
        assert_eq!(plain_paragraphs("one\ntwo & three"), ["one", "two & three"]);
        // Hostile or broken markup yields text or nothing, never markup.
        assert_eq!(plain_paragraphs("<script>alert(1)"), Vec::<String>::new());
        assert_eq!(
            plain_paragraphs("ok <b onclick=\"x()\">bold</b> <img src=x"),
            ["ok bold"]
        );
        assert_eq!(
            plain_paragraphs("a &#27;[31m b &#x7; c &bogus; d &#65;"),
            ["a [31m b c &bogus; d A"]
        );
        assert_eq!(
            plain_paragraphs("&lt;script&gt; stays text"),
            ["<script> stays text"]
        );
        assert_eq!(plain_paragraphs(""), Vec::<String>::new());
        // A long paragraph is capped; a huge input is read only so far.
        let long = format!("<p>{}</p>", "x".repeat(5_000));
        assert_eq!(
            plain_paragraphs(&long)[0].chars().count(),
            MAX_PARAGRAPH_CHARS
        );
        let huge = format!("<p>first</p>{}", "<p>é</p>".repeat(100_000));
        assert!(plain_paragraphs(&huge).len() < 100_000);
    }

    #[test]
    fn the_lede_is_the_opening_paragraphs_that_state_figures() {
        let got = lede(RELEASE, "");
        assert_eq!(got.len(), MAX_LEDE_PARAGRAPHS);
        assert!(
            got[0].contains("today reported sales of $43.4 billion"),
            "{got:?}"
        );
        assert!(got[1].contains("$3.72 per diluted share"));
        // The quote has no figure, so the first guidance line is the third
        // kept; the paragraphs after it are past the limit.
        assert_eq!(
            got[2], "Total sales growth of approximately 3.0%",
            "{got:?}"
        );
        assert!(!got.iter().any(|p| p.contains("share repurchases")));
        // The summary is the fallback when there is no content.
        let summary = "NVIDIA today reported revenue of $115.8 billion for the third quarter, up 103% from a year ago.";
        assert_eq!(lede("  ", summary), [summary]);
        // No figures, no lede.
        assert!(lede(
            "<p>The company will host a conference call at 5 p.m. Eastern to discuss it.</p>",
            ""
        )
        .is_empty());
        assert!(lede("", "").is_empty());
    }

    #[test]
    fn release_text_is_attached_to_its_headline_and_translations_are_skipped() {
        let mut french = item(
            1,
            "2026-11-17T11:31:00Z",
            "The Home Depot Reports Third Quarter 2026 Financial Results",
        );
        french.url = Some("https://www.newswire.ca/fr/news-releases/home-depot-annonce".into());
        let english = item(
            2,
            "2026-11-17T12:31:00Z",
            "The Home Depot Reports Third Quarter Fiscal 2026 Results and Updates Guidance",
        );
        let other = item(
            3,
            "2026-11-17T11:31:00Z",
            "The Home Depot Announces Executive Leadership Changes",
        );
        let items = vec![french, english, other];
        assert_eq!(best(&items).map(|n| n.id), Some(2));
        let mut ledes = HashMap::new();
        ledes.insert(2, lede(RELEASE, ""));
        let got = pick_with(&items, &ledes);
        assert_eq!(
            got.len(),
            1,
            "the translation and the unrelated item are left out"
        );
        assert_eq!(got[0].lede.len(), 3);
        assert!(pick(&items)[0].lede.is_empty());

        // Stored with its text; a later fetch without text doesn't lose it.
        let tmp = tempfile::tempdir().unwrap();
        let dir = headlines_dir(tmp.path());
        let date = d("2026-11-17");
        let now = at("2026-11-17T22:00:00Z");
        // Titles only at first (what was stored before the text was read).
        let first = store_fetch(&dir, "HD", date, &items, now).unwrap();
        assert!(!first.has_lede());
        let with = store_fetch_with(&dir, "HD", date, &items, &ledes, now).unwrap();
        assert!(with.has_lede());
        let again = store_fetch(&dir, "HD", date, &items, at("2026-11-18T03:00:00Z")).unwrap();
        assert_eq!(again, with, "kept");
        // A file written before the field existed still loads.
        let old = r#"{"symbol":"HD","date":"2026-11-17","fetched_at":"2026-11-17T22:00:00Z","scanned":3,"picked":[{"time":"2026-11-17T12:31:00Z","headline":"x"}]}"#;
        let file: HeadlinesFile = serde_json::from_str(old).unwrap();
        assert!(!file.has_lede());
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
