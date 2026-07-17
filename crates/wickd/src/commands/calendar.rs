//! `wickd calendar` — the economic-calendar store (`~/.wickd/calendar/`).
//!
//!   wickd calendar sync                 # fetch the FF weekly feed, merge it in
//!   wickd calendar sync --dry-run       # show what would change, write nothing
//!   wickd calendar upcoming             # next 7 days, medium+high impact
//!   wickd calendar upcoming --days 3 --currencies USD,EUR --min-impact high
//!
//! One store, two consumers: the strategy engine (event blackout ABI v3,
//! surprise accessors ABI v4) and the desktop app's Economic Calendar UI both
//! read `~/.wickd/calendar/YYYY-MM.csv`. `sync` is the freshness verb — a
//! periodic one-shot for launchd (`com.openthink.wickd-calendar`, same shape
//! as the books collector): fetch this week's ForexFactory feed, merge, exit.
//!
//! The weekly feed carries schedule/forecast data but never `actual` values,
//! so the merge preserves stored actuals; backfilled history is untouched
//! (see `wickd_core::calendar_store` for the exact merge rules).
//!
//! NETWORK BOUNDARY: this command is the ONLY calendar network path. The
//! desktop app never fetches — it reads the store the same way the strategy
//! engine does (the offline-boot e2e specs assert zero non-localhost
//! requests, and the calendar feature must keep them green).

use chrono::{Duration, Utc};
use clap::{Args, Subcommand};

use wickd_core::backtest::surprise::{impact_rank, min_impact_rank};
use wickd_core::calendar_store::{
    fetch_feed, merge_into_store, normalize_feed, read_range, FF_WEEKLY_FEED_URL,
};
use wickd_core::events::calendar_dir;

use crate::output::{exit, Out};

#[derive(Args, Debug)]
pub struct CalendarArgs {
    #[command(subcommand)]
    command: CalendarCommand,
}

#[derive(Subcommand, Debug)]
enum CalendarCommand {
    /// Fetch the ForexFactory weekly feed and merge it into the store
    /// (`~/.wickd/calendar/YYYY-MM.csv`) → JSON merge stats.
    Sync(SyncArgs),
    /// List upcoming events from the store → JSON (no network).
    Upcoming(UpcomingArgs),
}

#[derive(Args, Debug)]
struct SyncArgs {
    /// Fetch and report merge stats without writing any file.
    #[arg(long)]
    dry_run: bool,

    /// Feed URL override (testing / a future alternate source).
    #[arg(long, default_value = FF_WEEKLY_FEED_URL)]
    url: String,
}

#[derive(Args, Debug)]
struct UpcomingArgs {
    /// Days ahead to include (from now, UTC).
    #[arg(long, default_value_t = 7)]
    days: u32,

    /// Currencies to include, comma-separated (e.g. `USD,EUR`). Default: all.
    #[arg(long, value_delimiter = ',')]
    currencies: Vec<String>,

    /// Minimum impact: `low`, `medium`, or `high`.
    #[arg(long, default_value = "medium")]
    min_impact: String,
}

pub async fn run(args: CalendarArgs, out: Out) -> ! {
    match args.command {
        CalendarCommand::Sync(a) => sync(a, out).await,
        CalendarCommand::Upcoming(a) => upcoming(a, out),
    }
}

async fn sync(args: SyncArgs, out: Out) -> ! {
    let dir = match calendar_dir() {
        Ok(d) => d,
        Err(e) => out.fail(exit::GENERIC, "calendar_dir_failed", e),
    };
    let feed = match fetch_feed(&args.url).await {
        Ok(f) => f,
        Err(e) => out.fail(exit::GENERIC, "calendar_fetch_failed", e),
    };
    let rows = normalize_feed(&feed);

    if args.dry_run {
        let window_from = rows.iter().map(|r| r.date.clone()).min();
        let window_to = rows.iter().map(|r| r.date.clone()).max();
        out.ok(&serde_json::json!({
            "dry_run": true,
            "fetched": feed.len(),
            "normalized": rows.len(),
            "window_from": window_from,
            "window_to": window_to,
            "dir": dir.display().to_string(),
        }));
        std::process::exit(exit::OK);
    }

    let fetched = feed.len();
    let normalized = rows.len();
    match merge_into_store(&dir, rows) {
        Ok(stats) => {
            out.ok(&serde_json::json!({
                "fetched": fetched,
                "normalized": normalized,
                "added": stats.added,
                "updated": stats.updated,
                "kept_unmatched": stats.kept_unmatched,
                "files_touched": stats.files_touched,
                "window_from": stats.window_from,
                "window_to": stats.window_to,
                "dir": dir.display().to_string(),
            }));
            std::process::exit(exit::OK);
        }
        Err(e) => out.fail(exit::GENERIC, "calendar_merge_failed", e),
    }
}

fn upcoming(args: UpcomingArgs, out: Out) -> ! {
    let Some(min_rank) = min_impact_rank(&args.min_impact) else {
        out.fail(
            exit::VALIDATION,
            "calendar_bad_min_impact",
            format!("unknown --min-impact '{}' (use low|medium|high)", args.min_impact),
        );
    };
    let dir = match calendar_dir() {
        Ok(d) => d,
        Err(e) => out.fail(exit::GENERIC, "calendar_dir_failed", e),
    };

    let now = Utc::now();
    let from = now.date_naive();
    let to = (now + Duration::days(args.days as i64)).date_naive();
    let currencies: Vec<String> = args.currencies.iter().map(|c| c.trim().to_uppercase()).collect();

    let rows = match read_range(&dir, from, to) {
        Ok(r) => r,
        Err(e) => out.fail(exit::GENERIC, "calendar_read_failed", e),
    };

    let now_unix = now.timestamp();
    let events: Vec<_> = rows
        .into_iter()
        .filter(|r| {
            impact_rank(&r.impact) >= min_rank
                && (currencies.is_empty() || currencies.contains(&r.currency.to_uppercase()))
                && r.time_unix().is_some_and(|t| t >= now_unix)
        })
        .collect();

    let next_high_impact = events
        .iter()
        .find(|r| impact_rank(&r.impact) == 3)
        .map(|r| {
            serde_json::json!({
                "date": r.date, "time": r.time, "currency": r.currency, "event": r.event,
                "seconds_until": r.time_unix().map(|t| t - now_unix),
            })
        });

    out.ok(&serde_json::json!({
        "count": events.len(),
        "from": from.to_string(),
        "to": to.to_string(),
        "min_impact": args.min_impact,
        "next_high_impact": next_high_impact,
        "events": events,
    }));
    std::process::exit(exit::OK);
}
