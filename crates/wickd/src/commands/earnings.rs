//! `wickd earnings` — the earnings-calendar store (`~/.wickd/earnings/`).
//!
//!   wickd earnings sync                  # fetch today + the next 7 days
//!   wickd earnings sync --days 14
//!   wickd earnings upcoming              # stored rows, next 7 days
//!   wickd earnings upcoming --days 2 --min-cap 10000000000
//!   wickd earnings upcoming --lists      # only what's on the watchlist or a list
//!   wickd earnings upcoming --symbols NVDA,WMT
//!
//! `sync` is the only network path: one request per weekday to Nasdaq's
//! public earnings calendar (no key; unofficial — see
//! `wickd_core::earnings_store`). A day that fails keeps its stored file and
//! is reported in the JSON; the command fails only when every day failed.
//! `upcoming` reads the store and nothing else.
//!
//! Days are US market dates (US/Eastern), so "today" late in the evening is
//! still the trading day that just closed.
//!
//! `wickd earnings slate …` (see [`super::slate`]) is the watch list built on
//! top of this store.
//!
//!   wickd earnings results sync NVDA HD      # what they reported (EPS vs consensus)
//!   wickd earnings results sync --slate      # every company on the slate
//!   wickd earnings results show NVDA         # stored, no network
//!
//! Reported figures come from a second unofficial Nasdaq table, one request
//! per company, and can lag a report by a day (see
//! `wickd_core::earnings_results`).

use std::collections::HashSet;

use chrono::{Duration, NaiveDate, Utc};
use clap::{Args, Subcommand};
use rust_decimal::Decimal;

use wickd_core::earnings_results::{
    read_results, results_dir, results_symbol, sync_results, NASDAQ_SURPRISE_URL,
};
use wickd_core::earnings_store::{
    earnings_dir, http_client, market_date, read_days, sort_rows, sync_days, weekdays_from,
    DayFile, EarningsRow, NASDAQ_EARNINGS_URL, SOURCE,
};

use crate::output::{exit, Out};

/// The furthest ahead either verb looks. The feed thins out beyond a few
/// weeks, and a typo shouldn't fire hundreds of requests.
const MAX_DAYS: u32 = 60;

#[derive(Args, Debug)]
pub struct EarningsArgs {
    #[command(subcommand)]
    command: EarningsCommand,
}

#[derive(Subcommand, Debug)]
enum EarningsCommand {
    /// Fetch Nasdaq's public earnings calendar for today and the days after
    /// it into the store (`~/.wickd/earnings/YYYY-MM-DD.json`) → JSON stats
    /// per day. The source is unofficial.
    Sync(SyncArgs),
    /// List stored earnings from today on → JSON (no network).
    Upcoming(UpcomingArgs),
    /// The slate: companies watched through their reports, each with why and
    /// what is expected: add | set | rm | ls (`ls --reactions` adds how the
    /// stock reacted).
    Slate(super::slate::SlateArgs),
    /// What companies actually reported (EPS against the consensus): `sync`
    /// fetches Nasdaq's per-company table into the store, `show` prints
    /// what is stored. The source is unofficial and can lag a report by a
    /// day.
    Results(ResultsArgs),
}

#[derive(Args, Debug)]
struct ResultsArgs {
    #[command(subcommand)]
    command: ResultsCommand,
}

#[derive(Subcommand, Debug)]
enum ResultsCommand {
    /// Fetch and store the reported figures for these companies → JSON per
    /// company.
    Sync(ResultsSyncArgs),
    /// Print the stored reported figures for these companies → JSON (no
    /// network).
    Show(ResultsShowArgs),
}

#[derive(Args, Debug)]
struct ResultsSyncArgs {
    /// Stock tickers (commas work too).
    symbols: Vec<String>,

    /// Also every company on the slate.
    #[arg(long)]
    slate: bool,

    /// URL template override, `{symbol}` replaced by the ticker (testing).
    #[arg(long, default_value = NASDAQ_SURPRISE_URL, hide = true)]
    url: String,
}

#[derive(Args, Debug)]
struct ResultsShowArgs {
    /// Stock tickers (commas work too).
    #[arg(required = true)]
    symbols: Vec<String>,
}

#[derive(Args, Debug)]
struct SyncArgs {
    /// Calendar days after today to fetch (weekends are skipped; max 60).
    #[arg(long, default_value_t = 7)]
    days: u32,

    /// Feed URL override (testing / a future alternate source).
    #[arg(long, default_value = NASDAQ_EARNINGS_URL)]
    url: String,
}

#[derive(Args, Debug)]
struct UpcomingArgs {
    /// Calendar days after today to include (max 60).
    #[arg(long, default_value_t = 7)]
    days: u32,

    /// Only these symbols, comma-separated.
    #[arg(long, value_delimiter = ',')]
    symbols: Vec<String>,

    /// Only companies worth at least this many dollars (rows with no market
    /// value are left out).
    #[arg(long)]
    min_cap: Option<Decimal>,

    /// Only symbols on the watchlist or any `wickd list` (added to
    /// `--symbols` when both are given).
    #[arg(long)]
    lists: bool,
}

pub async fn run(args: EarningsArgs, out: Out) -> ! {
    match args.command {
        EarningsCommand::Sync(a) => sync(a, out).await,
        EarningsCommand::Upcoming(a) => upcoming(a, out),
        EarningsCommand::Slate(a) => super::slate::run(a, out).await,
        EarningsCommand::Results(a) => match a.command {
            ResultsCommand::Sync(a) => results_sync(a, out).await,
            ResultsCommand::Show(a) => results_show(a, out),
        },
    }
}

/// The tickers typed on the command line: split on commas, checked,
/// de-duplicated, in order. A symbol that isn't a plain ticker is refused.
fn result_tickers(raw: &[String], out: Out) -> Vec<String> {
    let mut syms: Vec<String> = Vec::new();
    for s in raw.iter().flat_map(|s| s.split(',')).filter(|s| !s.trim().is_empty()) {
        match results_symbol(s) {
            Some(sym) if !syms.contains(&sym) => syms.push(sym),
            Some(_) => {}
            None => out.fail(
                exit::VALIDATION,
                "earnings_bad_symbol",
                format!("'{s}' is not a stock ticker"),
            ),
        }
    }
    syms
}

async fn results_sync(args: ResultsSyncArgs, out: Out) -> ! {
    let mut symbols = result_tickers(&args.symbols, out);
    if args.slate {
        let slate = crate::slate::slate_path().and_then(|p| crate::slate::load_at(&p));
        match slate {
            Ok(file) => {
                for e in file.entries {
                    if let Some(sym) = results_symbol(&e.symbol).filter(|s| !symbols.contains(s)) {
                        symbols.push(sym);
                    }
                }
            }
            Err(e) => out.fail(exit::GENERIC, "earnings_slate_unreadable", format!("{e:#}")),
        }
    }
    if symbols.is_empty() {
        out.fail(
            exit::VALIDATION,
            "earnings_no_symbols",
            "give at least one stock ticker, or --slate for every company on the slate",
        );
    }
    let dir = match earnings_dir() {
        Ok(d) => results_dir(&d),
        Err(e) => out.fail(exit::GENERIC, "earnings_dir_failed", e),
    };
    let client = match http_client() {
        Ok(c) => c,
        Err(e) => out.fail(exit::GENERIC, "earnings_client_failed", e),
    };
    let outcomes = sync_results(&client, &args.url, &dir, &symbols).await;
    let failed = outcomes.iter().filter(|o| !o.ok()).count();
    if every_day_failed(outcomes.len(), failed) {
        let first = outcomes
            .first()
            .and_then(|o| o.error.clone())
            .unwrap_or_else(|| "unknown error".into());
        out.fail(
            exit::GENERIC,
            "earnings_results_failed",
            format!("no company's results could be fetched ({failed} tried); stored results are unchanged. First error: {first}"),
        );
    }
    out.ok(&serde_json::json!({
        "source": SOURCE,
        "dir": dir.display().to_string(),
        "requested": outcomes.len(),
        "failed": failed,
        "results": outcomes,
    }));
    std::process::exit(exit::OK);
}

fn results_show(args: ResultsShowArgs, out: Out) -> ! {
    let symbols = result_tickers(&args.symbols, out);
    let dir = match earnings_dir() {
        Ok(d) => results_dir(&d),
        Err(e) => out.fail(exit::GENERIC, "earnings_dir_failed", e),
    };
    let mut stored = serde_json::Map::new();
    for sym in &symbols {
        match read_results(&dir, sym) {
            // `null` = nothing stored for it (run `results sync`).
            Ok(file) => {
                stored.insert(sym.clone(), serde_json::to_value(file).unwrap_or_default());
            }
            Err(e) => out.fail(exit::GENERIC, "earnings_results_unreadable", e),
        }
    }
    out.ok(&serde_json::json!({ "results": stored }));
    std::process::exit(exit::OK);
}

fn check_days(days: u32, out: Out) {
    if days > MAX_DAYS {
        out.fail(
            exit::VALIDATION,
            "earnings_bad_days",
            format!("--days {days} is too far ahead (the most is {MAX_DAYS})"),
        );
    }
}

async fn sync(args: SyncArgs, out: Out) -> ! {
    check_days(args.days, out);
    let dir = match earnings_dir() {
        Ok(d) => d,
        Err(e) => out.fail(exit::GENERIC, "earnings_dir_failed", e),
    };
    let client = match http_client() {
        Ok(c) => c,
        Err(e) => out.fail(exit::GENERIC, "earnings_client_failed", e),
    };
    let dates = weekdays_from(market_date(Utc::now()), args.days);
    let outcomes = sync_days(&client, &args.url, &dir, &dates).await;
    let failed = outcomes.iter().filter(|o| !o.ok()).count();
    if every_day_failed(outcomes.len(), failed) {
        let first = outcomes
            .first()
            .and_then(|o| o.error.clone())
            .unwrap_or_else(|| "unknown error".into());
        out.fail(
            exit::GENERIC,
            "earnings_sync_failed",
            format!("no day could be fetched ({failed} tried); stored days are unchanged. First error: {first}"),
        );
    }
    out.ok(&serde_json::json!({
        "source": SOURCE,
        "dir": dir.display().to_string(),
        "requested": outcomes.len(),
        "failed": failed,
        "days": outcomes,
    }));
    std::process::exit(exit::OK);
}

/// The sync as a whole failed only if it tried at least one day and none
/// worked. Some days failing is a normal partial result.
fn every_day_failed(requested: usize, failed: usize) -> bool {
    requested > 0 && failed == requested
}

fn upcoming(args: UpcomingArgs, out: Out) -> ! {
    check_days(args.days, out);
    let dir = match earnings_dir() {
        Ok(d) => d,
        Err(e) => out.fail(exit::GENERIC, "earnings_dir_failed", e),
    };
    let from = market_date(Utc::now());
    let to = from + Duration::days(i64::from(args.days));
    let days = match read_days(&dir, from, to) {
        Ok(d) => d,
        Err(e) => out.fail(exit::GENERIC, "earnings_read_failed", e),
    };

    let mut only: Option<HashSet<String>> = None;
    if !args.symbols.is_empty() {
        only.get_or_insert_default().extend(
            args.symbols
                .iter()
                .map(|s| crate::lists::normalize_symbol(s)),
        );
    }
    if args.lists {
        match list_symbols() {
            Ok(s) => only.get_or_insert_default().extend(s),
            Err(e) => out.fail(exit::GENERIC, "earnings_lists_failed", format!("{e:#}")),
        }
    }

    let stored = stored_days(&days, &weekdays_from(from, args.days));
    let rows = select(days, only.as_ref(), args.min_cap);
    out.ok(&serde_json::json!({
        "count": rows.len(),
        "from": from.to_string(),
        "to": to.to_string(),
        "stored": stored,
        "rows": rows,
    }));
    std::process::exit(exit::OK);
}

/// Every symbol on the watchlist or in any `wickd list`.
pub fn list_symbols() -> anyhow::Result<HashSet<String>> {
    let mut out: HashSet<String> = crate::tui::store::load_watchlist()?.into_iter().collect();
    let lists = crate::lists::load_at(&crate::lists::lists_path()?)?;
    out.extend(lists.lists.into_values().flatten().map(|e| e.symbol));
    Ok(out)
}

/// The rows of `days` that pass the filters, in store order.
fn select(
    days: Vec<DayFile>,
    only: Option<&HashSet<String>>,
    min_cap: Option<Decimal>,
) -> Vec<EarningsRow> {
    let mut rows: Vec<EarningsRow> = days
        .into_iter()
        .flat_map(|d| d.rows)
        .filter(|r| only.is_none_or(|s| s.contains(&r.symbol)))
        .filter(|r| min_cap.is_none_or(|m| r.market_cap.is_some_and(|c| c >= m)))
        .collect();
    sort_rows(&mut rows);
    rows
}

/// Per weekday in the window: when it was fetched and how many rows it
/// holds, or `fetched_at: null` for a day the store has no file for — so a
/// reader can tell "nobody reports" from "not fetched yet".
fn stored_days(days: &[DayFile], weekdays: &[NaiveDate]) -> Vec<serde_json::Value> {
    weekdays
        .iter()
        .map(|date| match days.iter().find(|d| d.date == *date) {
            Some(d) => serde_json::json!({
                "date": date.to_string(),
                "fetched_at": d.fetched_at.to_rfc3339(),
                "rows": d.rows.len(),
            }),
            None => serde_json::json!({ "date": date.to_string(), "fetched_at": null, "rows": 0 }),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use wickd_core::earnings_store::Session;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn row(date: &str, symbol: &str, session: Session, cap: Option<Decimal>) -> EarningsRow {
        EarningsRow {
            date: d(date),
            symbol: symbol.into(),
            name: format!("{symbol} Inc."),
            session,
            eps_forecast: None,
            estimates: None,
            market_cap: cap,
            last_year_eps: None,
            fiscal_quarter: String::new(),
        }
    }

    fn day(date: &str, rows: Vec<EarningsRow>) -> DayFile {
        DayFile {
            date: d(date),
            fetched_at: "2026-11-18T01:00:00Z".parse().unwrap(),
            source: "nasdaq".into(),
            rows,
        }
    }

    fn days() -> Vec<DayFile> {
        vec![
            day(
                "2026-11-19",
                vec![row(
                    "2026-11-19",
                    "WMT",
                    Session::PreMarket,
                    Some(dec!(1000000000000)),
                )],
            ),
            day(
                "2026-11-18",
                vec![
                    row(
                        "2026-11-18",
                        "PANW",
                        Session::AfterHours,
                        Some(dec!(258000000000)),
                    ),
                    row(
                        "2026-11-18",
                        "TINY",
                        Session::PreMarket,
                        Some(dec!(40000000)),
                    ),
                    row(
                        "2026-11-18",
                        "TGT",
                        Session::PreMarket,
                        Some(dec!(72000000000)),
                    ),
                    row("2026-11-18", "NOCAP", Session::Unknown, None),
                ],
            ),
        ]
    }

    fn syms(rows: &[EarningsRow]) -> Vec<&str> {
        rows.iter().map(|r| r.symbol.as_str()).collect()
    }

    #[test]
    fn rows_come_out_by_date_then_session_then_size() {
        let rows = select(days(), None, None);
        assert_eq!(syms(&rows), ["TGT", "TINY", "PANW", "NOCAP", "WMT"]);
    }

    #[test]
    fn min_cap_drops_small_and_unknown_companies() {
        let rows = select(days(), None, Some(dec!(10000000000)));
        assert_eq!(syms(&rows), ["TGT", "PANW", "WMT"]);
    }

    #[test]
    fn a_symbol_set_keeps_only_its_members_and_combines_with_min_cap() {
        let only: HashSet<String> = ["TINY", "WMT", "ZZZ"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(syms(&select(days(), Some(&only), None)), ["TINY", "WMT"]);
        assert_eq!(
            syms(&select(days(), Some(&only), Some(dec!(1000000000)))),
            ["WMT"]
        );
        assert!(select(days(), Some(&HashSet::new()), None).is_empty());
    }

    #[test]
    fn stored_says_which_weekdays_were_never_fetched() {
        let got = stored_days(&days(), &weekdays_from(d("2026-11-18"), 2));
        assert_eq!(got.len(), 3);
        assert_eq!(
            (got[0]["date"].as_str(), got[0]["rows"].as_u64()),
            (Some("2026-11-18"), Some(4))
        );
        assert!(got[0]["fetched_at"].is_string());
        assert_eq!(got[1]["rows"].as_u64(), Some(1));
        assert_eq!(
            (got[2]["date"].as_str(), got[2]["fetched_at"].is_null()),
            (Some("2026-11-20"), true)
        );
    }

    #[test]
    fn the_sync_fails_only_when_every_day_failed() {
        assert!(every_day_failed(5, 5));
        assert!(!every_day_failed(5, 4));
        assert!(!every_day_failed(5, 0));
        // A weekend with --days 0 asks for nothing: not a failure.
        assert!(!every_day_failed(0, 0));
    }
}
