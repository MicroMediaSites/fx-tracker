//! The earnings slate (`~/.wickd/slate.json`): the short list of companies
//! picked ahead of their earnings reports, each with why it was picked and
//! what is expected, written **before** it reports.
//!
//! The slate is for watching, then reacting. Nothing here places a trade,
//! and an entry is never a reason to buy before a report: the play starts
//! after the report, on how the stock reacts
//! ([`wickd_core::earnings_reaction`]).
//!
//! ## Writers
//!
//! The CLI (usually an agent, during the evening prep) and the TUI both
//! write this file, so every change goes through
//! [`crate::lists::locked_update`]: an exclusive lock, a fresh read, and an
//! atomic replace.
//!
//! ```json
//! {
//!   "version": 1,
//!   "entries": [
//!     {"symbol": "WMT", "date": "2026-11-19", "session": "pre_market",
//!      "why": "largest retailer; reads the consumer for everything else",
//!      "expect": "EPS 0.64 expected.\nGood: beat and a raised year.\nBad: a cut.",
//!      "added": "2026-11-18T03:00:00Z"}
//!   ]
//! }
//! ```
//!
//! Text is kept as written apart from control characters (it is drawn on a
//! terminal): `why` is one line, `expect` keeps its line breaks.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use wickd_core::earnings_store::{self, Session};

use crate::lists::{invalid, locked_update, normalize_symbol};

/// The most a `why` holds.
pub const MAX_WHY_CHARS: usize = 200;
/// The most an `expect` holds.
pub const MAX_EXPECT_CHARS: usize = 4_000;
/// The most entries a slate holds: it is a short list by design.
pub const MAX_ENTRIES: usize = 200;
/// Entries whose report is older than this are left out of listings (they
/// stay in the file until removed).
pub const KEEP_DAYS: i64 = 14;
/// How far ahead the earnings store is searched for a symbol's next report.
const LOOKAHEAD_DAYS: i64 = 60;

/// One company on the slate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub symbol: String,
    /// The report's market date.
    pub date: NaiveDate,
    pub session: Session,
    /// Why it was picked, one line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// What is expected, free text, written before the report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<String>,
    pub added: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SlateFile {
    #[serde(default = "version")]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

fn version() -> u32 {
    1
}

pub fn slate_path() -> Result<PathBuf> {
    Ok(wickd_core::paths::wickd_data_home()
        .map_err(|e| anyhow!(e))?
        .join("slate.json"))
}

/// Read the slate (an absent file is an empty slate).
pub fn load_at(path: &Path) -> Result<SlateFile> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("{} is not valid slate JSON", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SlateFile {
            version: 1,
            ..Default::default()
        }),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Apply `change` to the slate under an exclusive lock and save atomically.
pub fn update_at<T>(path: &Path, change: impl FnOnce(&mut SlateFile) -> Result<T>) -> Result<T> {
    locked_update(path, load_at, change)
}

/// Fields to write onto an entry: `None` leaves a field as it is, and for
/// the two text fields `Some("")` clears it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Change {
    pub why: Option<String>,
    pub expect: Option<String>,
    pub date: Option<NaiveDate>,
    pub session: Option<Session>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Check and clean the text: control characters go (`expect` keeps its
    /// line breaks), and text over the limits is refused, not cut, so
    /// nothing is lost without the writer knowing.
    pub fn cleaned(mut self) -> Result<Self> {
        if let Some(w) = self.why.take() {
            let w = earnings_store::clean_text(&w, usize::MAX);
            if w.chars().count() > MAX_WHY_CHARS {
                return Err(invalid(format!(
                    "--why is {} characters; the most is {MAX_WHY_CHARS} (it is one line: put the detail in --expect)",
                    w.chars().count()
                )));
            }
            self.why = Some(w);
        }
        if let Some(e) = self.expect.take() {
            let e: String = e
                .replace("\r\n", "\n")
                .chars()
                .map(|c| if c == '\t' || c == '\r' { ' ' } else { c })
                .filter(|c| *c == '\n' || !c.is_control())
                .collect();
            let e = e.trim().to_string();
            if e.chars().count() > MAX_EXPECT_CHARS {
                return Err(invalid(format!(
                    "--expect is {} characters; the most is {MAX_EXPECT_CHARS}",
                    e.chars().count()
                )));
            }
            self.expect = Some(e);
        }
        Ok(self)
    }

    fn apply(&self, e: &mut Entry) {
        let text = |v: &Option<String>, to: &mut Option<String>| {
            if let Some(v) = v {
                *to = (!v.is_empty()).then(|| v.clone());
            }
        };
        text(&self.why, &mut e.why);
        text(&self.expect, &mut e.expect);
        if let Some(d) = self.date {
            e.date = d;
        }
        if let Some(s) = self.session {
            e.session = s;
        }
    }
}

/// A ticker as the slate stores it: the `wickd list` normalization, and
/// only what a US ticker can be (forex pairs don't report earnings).
pub fn slate_symbol(s: &str) -> Result<String> {
    let sym = normalize_symbol(s);
    earnings_store::clean_symbol(&sym)
        .filter(|c| *c == sym && !sym.contains('_'))
        .ok_or_else(|| invalid(format!("'{s}' is not a stock ticker")))
}

/// A symbol's next stored report from `from` on: its date and session.
/// `None` when the earnings store has no report for it in the next
/// [`LOOKAHEAD_DAYS`] days (not fetched, or it isn't reporting).
pub fn next_report(
    earn_dir: &Path,
    symbol: &str,
    from: NaiveDate,
) -> Result<Option<(NaiveDate, Session)>> {
    let days = earnings_store::read_days(earn_dir, from, from + Duration::days(LOOKAHEAD_DAYS))
        .map_err(|e| anyhow!(e))?;
    Ok(days.into_iter().find_map(|d| {
        d.rows
            .iter()
            .find(|r| r.symbol == symbol)
            .map(|r| (d.date, r.session))
    }))
}

/// What an [`add`] did.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct Added {
    pub added: Vec<String>,
    /// Already on the slate: kept in place, given fields replaced.
    pub updated: Vec<String>,
}

/// Add `symbols` to the slate. A new entry needs a report date: `change`'s
/// if given, else `resolve`'s (the earnings store's next report), else the
/// add is refused for that symbol. An entry already there keeps its place
/// and only takes the fields given (its date is re-resolved only when
/// `change` names one).
pub fn add(
    file: &mut SlateFile,
    symbols: &[String],
    change: &Change,
    mut resolve: impl FnMut(&str) -> Result<Option<(NaiveDate, Session)>>,
    now: DateTime<Utc>,
) -> Result<Added> {
    let mut out = Added::default();
    for sym in symbols {
        if let Some(e) = file.entries.iter_mut().find(|e| e.symbol == *sym) {
            change.apply(e);
            out.updated.push(sym.clone());
            continue;
        }
        let stored = resolve(sym)?;
        let Some(date) = change.date.or(stored.map(|s| s.0)) else {
            return Err(invalid(format!(
                "no stored earnings report for {sym} in the next {LOOKAHEAD_DAYS} days: run `wickd earnings sync --days {LOOKAHEAD_DAYS}`, or give the date with --date YYYY-MM-DD"
            )));
        };
        if file.entries.len() >= MAX_ENTRIES {
            return Err(invalid(format!(
                "the slate already holds {MAX_ENTRIES} entries; remove some first (`wickd earnings slate rm`)"
            )));
        }
        let mut e = Entry {
            symbol: sym.clone(),
            date,
            // A date given by hand without a session doesn't inherit the
            // stored report's session unless the dates agree.
            session: match stored {
                Some((d, s)) if d == date => s,
                _ => Session::Unknown,
            },
            why: None,
            expect: None,
            added: now,
        };
        change.apply(&mut e);
        file.entries.push(e);
        out.added.push(sym.clone());
    }
    Ok(out)
}

/// Change the given fields of one entry.
pub fn set(file: &mut SlateFile, symbol: &str, change: &Change) -> Result<Entry> {
    let e = file
        .entries
        .iter_mut()
        .find(|e| e.symbol == symbol)
        .ok_or_else(|| {
            invalid(format!(
                "{symbol} is not on the slate (see `wickd earnings slate ls`)"
            ))
        })?;
    change.apply(e);
    Ok(e.clone())
}

/// Remove entries; returns the symbols that were there.
pub fn remove(file: &mut SlateFile, symbols: &[String]) -> Vec<String> {
    let mut gone = Vec::new();
    file.entries.retain(|e| {
        let hit = symbols.contains(&e.symbol);
        if hit {
            gone.push(e.symbol.clone());
        }
        !hit
    });
    gone
}

/// The slate for reading: soonest report first (then session, then symbol),
/// without entries whose report is more than [`KEEP_DAYS`] days before
/// `today` unless `all`.
pub fn ordered(file: &SlateFile, today: NaiveDate, all: bool) -> Vec<Entry> {
    let mut out: Vec<Entry> = file
        .entries
        .iter()
        .filter(|e| all || (today - e.date).num_days() <= KEEP_DAYS)
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then(a.session.cmp(&b.session))
            .then(a.symbol.cmp(&b.symbol))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wickd_core::earnings_store::{write_day, DayFile, EarningsRow};

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn now() -> DateTime<Utc> {
        "2026-11-18T03:00:00Z".parse().unwrap()
    }

    fn syms(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn why(s: &str) -> Change {
        Change {
            why: Some(s.into()),
            ..Default::default()
        }
    }

    /// A resolver that knows WMT (Thu before the open) and INTU (Thu after
    /// the close).
    fn known(sym: &str) -> Result<Option<(NaiveDate, Session)>> {
        Ok(match sym {
            "WMT" => Some((d("2026-11-19"), Session::PreMarket)),
            "INTU" => Some((d("2026-11-19"), Session::AfterHours)),
            _ => None,
        })
    }

    fn is_validation(e: &anyhow::Error) -> bool {
        e.downcast_ref::<crate::lists::ValidationError>().is_some()
    }

    #[test]
    fn add_takes_the_report_from_the_store_and_keeps_what_is_not_given() {
        let mut f = SlateFile::default();
        let a = add(
            &mut f,
            &syms(&["WMT", "INTU"]),
            &why("retail read"),
            known,
            now(),
        )
        .unwrap();
        assert_eq!((a.added, a.updated), (syms(&["WMT", "INTU"]), vec![]));
        assert_eq!(
            (
                f.entries[0].date,
                f.entries[0].session,
                f.entries[0].why.as_deref()
            ),
            (d("2026-11-19"), Session::PreMarket, Some("retail read"))
        );
        assert_eq!(f.entries[1].session, Session::AfterHours);
        assert_eq!(f.entries[0].added, now());

        // Re-adding with only an expectation: place, date, why and added kept.
        let later = now() + Duration::hours(5);
        let expect = Change {
            expect: Some("EPS 0.64 expected.\nGood: a raised year.".into()),
            ..Default::default()
        };
        let a = add(
            &mut f,
            &syms(&["WMT"]),
            &expect,
            |_| panic!("not resolved again"),
            later,
        )
        .unwrap();
        assert_eq!((a.added, a.updated), (vec![], syms(&["WMT"])));
        let wmt = &f.entries[0];
        assert_eq!(wmt.symbol, "WMT", "kept its place");
        assert_eq!(wmt.why.as_deref(), Some("retail read"));
        assert_eq!(
            wmt.expect.as_deref(),
            Some("EPS 0.64 expected.\nGood: a raised year.")
        );
        assert_eq!(wmt.added, now());
    }

    #[test]
    fn a_symbol_with_no_stored_report_needs_a_date() {
        let mut f = SlateFile::default();
        let err = add(&mut f, &syms(&["ZZZ"]), &Change::default(), known, now()).unwrap_err();
        assert!(is_validation(&err));
        let msg = err.to_string();
        assert!(
            msg.contains("ZZZ") && msg.contains("wickd earnings sync") && msg.contains("--date"),
            "{msg}"
        );
        assert!(f.entries.is_empty());
        // With a date it goes on, session not known.
        let dated = Change {
            date: Some(d("2026-12-02")),
            ..Default::default()
        };
        add(&mut f, &syms(&["ZZZ"]), &dated, known, now()).unwrap();
        assert_eq!(
            (f.entries[0].date, f.entries[0].session),
            (d("2026-12-02"), Session::Unknown)
        );
        // A hand-given date that disagrees with the store doesn't borrow the
        // store's session; one that agrees does; an explicit session wins.
        let other = Change {
            date: Some(d("2026-11-20")),
            ..Default::default()
        };
        add(&mut f, &syms(&["WMT"]), &other, known, now()).unwrap();
        assert_eq!(f.entries[1].session, Session::Unknown);
        let same = Change {
            date: Some(d("2026-11-19")),
            ..Default::default()
        };
        add(&mut f, &syms(&["INTU"]), &same, known, now()).unwrap();
        assert_eq!(f.entries[2].session, Session::AfterHours);
        let explicit = Change {
            date: Some(d("2026-11-20")),
            session: Some(Session::PreMarket),
            ..Default::default()
        };
        remove(&mut f, &syms(&["WMT"]));
        add(&mut f, &syms(&["WMT"]), &explicit, known, now()).unwrap();
        assert_eq!(f.entries.last().unwrap().session, Session::PreMarket);
    }

    #[test]
    fn set_changes_only_what_is_given_and_empty_text_clears() {
        let mut f = SlateFile::default();
        let both = Change {
            why: Some("why".into()),
            expect: Some("expect".into()),
            ..Default::default()
        };
        add(&mut f, &syms(&["WMT"]), &both, known, now()).unwrap();
        let e = set(
            &mut f,
            "WMT",
            &Change {
                session: Some(Session::AfterHours),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            (e.session, e.why.as_deref(), e.expect.as_deref(), e.date),
            (
                Session::AfterHours,
                Some("why"),
                Some("expect"),
                d("2026-11-19")
            )
        );
        let e = set(
            &mut f,
            "WMT",
            &Change {
                why: Some(String::new()),
                date: Some(d("2026-11-20")),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            (e.why, e.expect.as_deref(), e.date),
            (None, Some("expect"), d("2026-11-20"))
        );
        let err = set(&mut f, "NOPE", &Change::default()).unwrap_err();
        assert!(is_validation(&err) && err.to_string().contains("not on the slate"));
    }

    #[test]
    fn text_is_cleaned_and_overlong_text_is_refused_not_cut() {
        let c = Change {
            why: Some("  a\tb\u{1b}[31m\n c  ".into()),
            expect: Some("line one\r\nline\ttwo\u{7}\n\n  ".into()),
            ..Default::default()
        }
        .cleaned()
        .unwrap();
        assert_eq!(
            c.why.as_deref(),
            Some("a b[31m c"),
            "one line, no control characters"
        );
        assert_eq!(
            c.expect.as_deref(),
            Some("line one\nline two"),
            "line breaks kept"
        );
        let long = Change {
            why: Some("x".repeat(MAX_WHY_CHARS + 1)),
            ..Default::default()
        };
        let err = long.cleaned().unwrap_err();
        assert!(
            is_validation(&err) && err.to_string().contains("201 characters"),
            "{err}"
        );
        let long = Change {
            expect: Some("x".repeat(MAX_EXPECT_CHARS + 1)),
            ..Default::default()
        };
        assert!(is_validation(&long.cleaned().unwrap_err()));
        // Exactly at the limit is fine.
        assert!(Change {
            why: Some("x".repeat(MAX_WHY_CHARS)),
            ..Default::default()
        }
        .cleaned()
        .is_ok());
    }

    #[test]
    fn symbols_must_be_stock_tickers() {
        assert_eq!(slate_symbol(" wmt ").unwrap(), "WMT");
        assert_eq!(slate_symbol("brk.b").unwrap(), "BRK.B");
        for bad in ["", "EUR/USD", "EUR_USD", "A B", "TOOLONGSYMBOLX", "X;Y"] {
            let err = slate_symbol(bad).unwrap_err();
            assert!(is_validation(&err), "{bad:?}");
        }
    }

    #[test]
    fn listings_run_soonest_first_and_leave_out_old_reports() {
        let mut f = SlateFile::default();
        let resolve = |sym: &str| -> Result<Option<(NaiveDate, Session)>> {
            Ok(Some(match sym {
                "OLD" => (d("2026-11-02"), Session::PreMarket),
                "EDGE" => (d("2026-11-04"), Session::PreMarket),
                "LATE" => (d("2026-11-19"), Session::AfterHours),
                "EARLY" => (d("2026-11-19"), Session::PreMarket),
                _ => (d("2026-11-18"), Session::Unknown),
            }))
        };
        add(
            &mut f,
            &syms(&["LATE", "OLD", "EARLY", "TODAY", "EDGE"]),
            &Change::default(),
            resolve,
            now(),
        )
        .unwrap();
        let today = d("2026-11-18");
        let names = |v: Vec<Entry>| v.into_iter().map(|e| e.symbol).collect::<Vec<_>>();
        // EDGE reported exactly 14 days ago: still listed. OLD (16 days): not.
        assert_eq!(
            names(ordered(&f, today, false)),
            ["EDGE", "TODAY", "EARLY", "LATE"]
        );
        assert_eq!(
            names(ordered(&f, today, true)),
            ["OLD", "EDGE", "TODAY", "EARLY", "LATE"]
        );
        assert_eq!(f.entries.len(), 5, "nothing is deleted by listing");
        assert_eq!(remove(&mut f, &syms(&["OLD", "NOPE"])), syms(&["OLD"]));
        assert_eq!(f.entries.len(), 4);
    }

    #[test]
    fn the_file_round_trips_and_updates_are_locked_and_atomic() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested").join("slate.json");
        assert_eq!(
            load_at(&path).unwrap().entries,
            vec![],
            "no file is an empty slate"
        );
        let a = update_at(&path, |f| {
            add(f, &syms(&["WMT"]), &why("retail"), known, now())
        })
        .unwrap();
        assert_eq!(a.added, syms(&["WMT"]));
        let back = load_at(&path).unwrap();
        assert_eq!((back.version, back.entries.len()), (1, 1));
        assert_eq!(back.entries[0].why.as_deref(), Some("retail"));
        // A change that fails writes nothing.
        let err = update_at(&path, |f| {
            f.entries.clear();
            set(f, "NOPE", &Change::default())
        })
        .unwrap_err();
        assert!(is_validation(&err));
        assert_eq!(load_at(&path).unwrap().entries.len(), 1);
        // No temp file is left; the lock file is.
        let names: Vec<String> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!names.iter().any(|n| n.ends_with(".tmp")), "{names:?}");
        // A file that isn't a slate is an error that names it.
        std::fs::write(&path, b"not json").unwrap();
        assert!(load_at(&path)
            .unwrap_err()
            .to_string()
            .contains("slate.json"));
    }

    #[test]
    fn the_next_report_comes_from_the_earnings_store() {
        let tmp = tempfile::tempdir().unwrap();
        let row = |date: &str, symbol: &str, session| EarningsRow {
            date: d(date),
            symbol: symbol.into(),
            name: String::new(),
            session,
            eps_forecast: None,
            estimates: None,
            market_cap: None,
            last_year_eps: None,
            fiscal_quarter: String::new(),
        };
        for (date, rows) in [
            (
                "2026-11-17",
                vec![row("2026-11-17", "NVDA", Session::AfterHours)],
            ),
            (
                "2026-11-19",
                vec![
                    row("2026-11-19", "WMT", Session::PreMarket),
                    row("2026-11-19", "NVDA", Session::Unknown),
                ],
            ),
        ] {
            write_day(
                tmp.path(),
                &DayFile {
                    date: d(date),
                    fetched_at: now(),
                    source: "nasdaq".into(),
                    rows,
                },
            )
            .unwrap();
        }
        let from = d("2026-11-18");
        assert_eq!(
            next_report(tmp.path(), "WMT", from).unwrap(),
            Some((d("2026-11-19"), Session::PreMarket))
        );
        // Yesterday's report is behind us: the next one from today on.
        assert_eq!(
            next_report(tmp.path(), "NVDA", from).unwrap(),
            Some((d("2026-11-19"), Session::Unknown))
        );
        assert_eq!(
            next_report(tmp.path(), "NVDA", d("2026-11-17"))
                .unwrap()
                .unwrap()
                .0,
            d("2026-11-17")
        );
        assert_eq!(next_report(tmp.path(), "ZZZ", from).unwrap(), None);
        assert_eq!(
            next_report(&tmp.path().join("nope"), "WMT", from).unwrap(),
            None
        );
    }
}
