//! `wickd earnings slate` — the short list of companies watched through
//! their earnings reports (`~/.wickd/slate.json`).
//!
//!   wickd earnings slate add WMT --why "reads the consumer" --expect "EPS 0.64 expected. …"
//!   wickd earnings slate add INTU ROST                  # dates come from the earnings store
//!   wickd earnings slate add XYZ --date 2026-12-02 --session post
//!   wickd earnings slate set WMT --expect-file notes.txt
//!   wickd earnings slate rm ROST
//!   wickd earnings slate ls                             # soonest report first (no network)
//!   wickd earnings slate ls --reactions                 # + how each reacted (Alpaca daily bars)
//!
//! The slate is for watching, then reacting: an entry records why a company
//! is worth watching and what is expected **before** it reports. It is never
//! a reason to buy ahead of a report.
//!
//! Only `ls --reactions` uses the network (one batched Alpaca request, the
//! same keys as `wickd quote`). Everything else reads and writes local files.

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;

use wickd_core::earnings_reaction::{reaction, Reaction, VOLUME_SESSIONS};
use wickd_core::earnings_store::{earnings_dir, market_date, Session};
use wickd_core::equities::alpaca::{AlpacaClient, AlpacaError, QuoteFeed};
use wickd_core::equities::Bar;

use crate::alpaca_creds;
use crate::lists::ValidationError;
use crate::output::{exit, Out};
use crate::slate::{self, Change, Entry};

#[derive(Args, Debug)]
pub struct SlateArgs {
    #[command(subcommand)]
    command: SlateCommand,
}

#[derive(Subcommand, Debug)]
enum SlateCommand {
    /// Put companies on the slate. The report date and session come from the
    /// earnings store unless given; re-adding keeps an entry's place and
    /// replaces only the fields given.
    Add(AddArgs),
    /// Change fields of an entry already on the slate. What isn't given is
    /// kept; an empty `--why` or `--expect` clears it.
    Set(SetArgs),
    /// Take companies off the slate.
    Rm(RmArgs),
    /// The slate, soonest report first → JSON. No network without
    /// `--reactions`.
    Ls(LsArgs),
}

/// When in the day a company reports, as typed on the command line.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum SessionArg {
    /// Before the open.
    Pre,
    /// After the close.
    Post,
    /// Not known.
    Unknown,
}

impl From<SessionArg> for Session {
    fn from(s: SessionArg) -> Self {
        match s {
            SessionArg::Pre => Session::PreMarket,
            SessionArg::Post => Session::AfterHours,
            SessionArg::Unknown => Session::Unknown,
        }
    }
}

#[derive(Args, Debug)]
struct Fields {
    /// Why it's worth watching, one line (max 200 characters).
    #[arg(long)]
    why: Option<String>,
    /// What is expected, written before the report: free text.
    #[arg(long, conflicts_with = "expect_file")]
    expect: Option<String>,
    /// Read the expectation from a file (`-` = stdin), for several lines.
    #[arg(long, value_name = "PATH|-")]
    expect_file: Option<String>,
    /// The report's date, when the earnings store doesn't have it (or has
    /// it wrong).
    #[arg(long, value_name = "YYYY-MM-DD")]
    date: Option<NaiveDate>,
    /// When in the day it reports.
    #[arg(long, value_enum)]
    session: Option<SessionArg>,
}

impl Fields {
    fn change(self) -> Result<Change> {
        let expect = match (self.expect, self.expect_file) {
            (Some(e), _) => Some(e),
            (None, Some(path)) => Some(read_text(&path)?),
            (None, None) => None,
        };
        Change {
            why: self.why,
            expect,
            date: self.date,
            session: self.session.map(Session::from),
        }
        .cleaned()
    }
}

/// Read `--expect-file` (`-` = stdin), capped well above the stored limit so
/// a wrong path can't pull a huge file into memory.
fn read_text(path: &str) -> Result<String> {
    use std::io::Read;
    const CAP: u64 = 64 * 1024;
    let mut buf = String::new();
    let read = if path == "-" {
        std::io::stdin().take(CAP + 1).read_to_string(&mut buf)
    } else {
        std::fs::File::open(path).and_then(|f| f.take(CAP + 1).read_to_string(&mut buf))
    };
    read.map_err(|e| ValidationError(format!("--expect-file {path}: {e}")))?;
    if buf.len() as u64 > CAP {
        return Err(
            ValidationError(format!("--expect-file {path}: more than {} KB", CAP / 1024)).into(),
        );
    }
    Ok(buf)
}

#[derive(Args, Debug)]
struct AddArgs {
    /// Stock tickers (commas work too).
    #[arg(required = true)]
    symbols: Vec<String>,
    #[command(flatten)]
    fields: Fields,
}

#[derive(Args, Debug)]
struct SetArgs {
    symbol: String,
    #[command(flatten)]
    fields: Fields,
}

#[derive(Args, Debug)]
struct RmArgs {
    /// Stock tickers (commas work too).
    #[arg(required = true)]
    symbols: Vec<String>,
}

#[derive(Args, Debug)]
struct LsArgs {
    /// Also say how each stock reacted, once its reaction day has come
    /// (Alpaca daily bars; needs `wickd alpaca login`).
    #[arg(long)]
    reactions: bool,
    /// Include entries whose report is more than 14 days past.
    #[arg(long)]
    all: bool,
}

pub async fn run(args: SlateArgs, out: Out) -> ! {
    let result = match args.command {
        SlateCommand::Add(a) => add(a),
        SlateCommand::Set(a) => set(a),
        SlateCommand::Rm(a) => rm(a),
        SlateCommand::Ls(a) => ls(a, out).await,
    };
    match result {
        Ok(payload) => {
            out.ok(&payload);
            std::process::exit(exit::OK);
        }
        Err(e) => {
            let code = if e.downcast_ref::<ValidationError>().is_some() {
                exit::VALIDATION
            } else {
                exit::GENERIC
            };
            out.fail(code, "earnings_slate_failed", format!("{e:#}"))
        }
    }
}

/// The tickers typed on the command line, split on commas, checked, in
/// order, without repeats.
fn tickers(raw: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for s in raw
        .iter()
        .flat_map(|s| s.split(','))
        .filter(|s| !s.trim().is_empty())
    {
        let sym = slate::slate_symbol(s)?;
        if !out.contains(&sym) {
            out.push(sym);
        }
    }
    if out.is_empty() {
        return Err(ValidationError("give at least one stock ticker".into()).into());
    }
    Ok(out)
}

fn add(args: AddArgs) -> Result<serde_json::Value> {
    let symbols = tickers(&args.symbols)?;
    let change = args.fields.change()?;
    let earn_dir = earnings_dir().map_err(|e| anyhow!(e))?;
    let now = Utc::now();
    let today = market_date(now);
    let path = slate::slate_path()?;
    let (done, entries) = slate::update_at(&path, |f| {
        let done = slate::add(
            f,
            &symbols,
            &change,
            |sym| slate::next_report(&earn_dir, sym, today),
            now,
        )?;
        let entries: Vec<Entry> = f
            .entries
            .iter()
            .filter(|e| symbols.contains(&e.symbol))
            .cloned()
            .collect();
        Ok((done, entries))
    })?;
    Ok(serde_json::json!({ "added": done.added, "updated": done.updated, "entries": entries }))
}

fn set(args: SetArgs) -> Result<serde_json::Value> {
    let symbol = slate::slate_symbol(&args.symbol)?;
    let change = args.fields.change()?;
    if change.is_empty() {
        return Err(ValidationError(
            "nothing to change: give --why, --expect, --expect-file, --date or --session".into(),
        )
        .into());
    }
    let entry = slate::update_at(&slate::slate_path()?, |f| slate::set(f, &symbol, &change))?;
    Ok(serde_json::json!({ "entry": entry }))
}

fn rm(args: RmArgs) -> Result<serde_json::Value> {
    let symbols = tickers(&args.symbols)?;
    let removed = slate::update_at(&slate::slate_path()?, |f| Ok(slate::remove(f, &symbols)))?;
    let not_on_slate: Vec<&String> = symbols.iter().filter(|s| !removed.contains(s)).collect();
    Ok(serde_json::json!({ "removed": removed, "not_on_slate": not_on_slate }))
}

/// A slate entry with how the stock reacted, when that is known.
#[derive(Debug, Serialize)]
struct Listed {
    #[serde(flatten)]
    entry: Entry,
    #[serde(skip_serializing_if = "Option::is_none")]
    reaction: Option<Reaction>,
}

/// Each entry with its reaction from `bars` (none when its reaction day
/// hasn't traded or the bars don't cover it).
fn with_reactions(
    entries: Vec<Entry>,
    bars: &HashMap<String, Vec<Bar>>,
    now: DateTime<Utc>,
) -> Vec<Listed> {
    entries
        .into_iter()
        .map(|entry| {
            let reaction = bars
                .get(&entry.symbol)
                .and_then(|b| reaction(b, entry.date, entry.session, now));
            Listed { entry, reaction }
        })
        .collect()
}

/// How far back to ask for bars so the oldest report on the slate has its
/// prior close and a full volume average: the sessions needed, as calendar
/// days with room for weekends and holidays.
fn bars_start(entries: &[Entry], now: DateTime<Utc>) -> DateTime<Utc> {
    let oldest = entries
        .iter()
        .map(|e| e.date)
        .min()
        .unwrap_or_else(|| now.date_naive());
    let back = (VOLUME_SESSIONS as i64 + 2) * 7 / 5 + 10;
    oldest.and_hms_opt(0, 0, 0).map_or(now, |t| t.and_utc()) - Duration::days(back)
}

async fn ls(args: LsArgs, out: Out) -> Result<serde_json::Value> {
    let file = slate::load_at(&slate::slate_path()?)?;
    let now = Utc::now();
    let entries = slate::ordered(&file, market_date(now), args.all);
    if !args.reactions {
        return Ok(serde_json::json!({ "count": entries.len(), "entries": entries }));
    }
    // Only entries whose report day has come can have reacted.
    let due: Vec<String> = entries
        .iter()
        .filter(|e| e.date <= market_date(now))
        .map(|e| e.symbol.clone())
        .collect();
    let bars = if due.is_empty() {
        HashMap::new()
    } else {
        let creds = match alpaca_creds::resolve() {
            Ok(Some((c, _))) => c,
            Ok(None) => out.fail(
                exit::AUTH,
                "no_alpaca_keys",
                "no Alpaca keys configured — run `wickd alpaca login` (or leave out --reactions)",
            ),
            Err(e) => out.fail(exit::AUTH, "alpaca_keys_unreadable", format!("{e:#}")),
        };
        let client = AlpacaClient::new(creds, QuoteFeed::Iex);
        match client
            .bars_on(&due, "1Day", bars_start(&entries, now), "sip")
            .await
        {
            Ok(map) => map,
            Err(AlpacaError::Auth(_)) => out.fail(
                exit::AUTH,
                "alpaca_auth",
                "Alpaca rejected the keys — run `wickd alpaca login`",
            ),
            Err(e) => out.fail(exit::GENERIC, "earnings_slate_bars_failed", format!("{e}")),
        }
    };
    let no_bars: Vec<&String> = due.iter().filter(|s| !bars.contains_key(*s)).collect();
    let listed = with_reactions(entries, &bars, now);
    Ok(serde_json::json!({
        "count": listed.len(),
        "as_of": now,
        "entries": listed,
        "no_bars": no_bars,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn entry(symbol: &str, date: &str, session: Session) -> Entry {
        Entry {
            symbol: symbol.into(),
            date: d(date),
            session,
            why: None,
            expect: None,
            added: "2026-11-15T03:00:00Z".parse().unwrap(),
        }
    }

    fn bar(date: &str, open: f64, close: f64, volume: f64) -> Bar {
        Bar {
            time: format!("{date}T05:00:00Z").parse().unwrap(),
            open,
            high: open.max(close) + 1.0,
            low: open.min(close) - 1.0,
            close,
            volume,
            vwap: None,
        }
    }

    #[test]
    fn tickers_are_split_checked_and_deduplicated() {
        let got = tickers(&["wmt,intu".into(), " ROST ".into(), "WMT".into()]).unwrap();
        assert_eq!(got, ["WMT", "INTU", "ROST"]);
        for bad in [
            vec!["EUR_USD".to_string()],
            vec![",".to_string()],
            vec!["A B".to_string()],
        ] {
            let err = tickers(&bad).unwrap_err();
            assert!(err.downcast_ref::<ValidationError>().is_some(), "{bad:?}");
        }
    }

    #[test]
    fn reactions_are_attached_only_where_the_bars_reach() {
        let now: DateTime<Utc> = "2026-11-18T15:00:00Z".parse().unwrap();
        let mut bars = HashMap::new();
        bars.insert(
            "HD".to_string(),
            vec![
                bar("2026-11-16", 348.0, 347.5, 6_000_000.0),
                bar("2026-11-17", 335.0, 339.0, 9_000_000.0),
            ],
        );
        let listed = with_reactions(
            vec![
                entry("HD", "2026-11-17", Session::PreMarket),
                // Reports tomorrow: bars exist, but there's nothing to react to yet.
                entry("WMT", "2026-11-19", Session::PreMarket),
                // No bars came back for it.
                entry("ZZZ", "2026-11-17", Session::PreMarket),
            ],
            &bars,
            now,
        );
        let hd = listed[0].reaction.as_ref().unwrap();
        assert_eq!(hd.day, d("2026-11-17"));
        assert!((hd.gap_pct - (335.0 - 347.5) / 347.5 * 100.0).abs() < 1e-9);
        assert!(hd.complete);
        assert!(listed[1].reaction.is_none() && listed[2].reaction.is_none());
        // The JSON carries the entry's own fields at the top level.
        let json = serde_json::to_value(&listed[0]).unwrap();
        assert_eq!(json["symbol"], "HD");
        assert_eq!(json["session"], "pre_market");
        assert!(json["reaction"]["gap_pct"].is_number());
        assert!(serde_json::to_value(&listed[1])
            .unwrap()
            .get("reaction")
            .is_none());
    }

    #[test]
    fn bars_are_asked_for_far_enough_back_for_the_oldest_report() {
        let now: DateTime<Utc> = "2026-11-18T15:00:00Z".parse().unwrap();
        let entries = vec![
            entry("A", "2026-11-17", Session::PreMarket),
            entry("B", "2026-11-05", Session::AfterHours),
        ];
        let start = bars_start(&entries, now);
        // 22 sessions before Nov 5 is early October; the start is before that.
        assert!(start.date_naive() <= d("2026-10-06"), "{start}");
        assert!(
            start.date_naive() >= d("2026-09-15"),
            "not absurdly far: {start}"
        );
        // An empty slate asks from about six weeks back, not from the epoch.
        assert!((now - bars_start(&[], now)).num_days() < 60);
    }

    #[test]
    fn session_words_map_to_sessions() {
        assert_eq!(Session::from(SessionArg::Pre), Session::PreMarket);
        assert_eq!(Session::from(SessionArg::Post), Session::AfterHours);
        assert_eq!(Session::from(SessionArg::Unknown), Session::Unknown);
    }
}
