//! `wickd books` — snapshot OANDA order/position books (client sentiment).
//!
//! Fetches the order book AND position book for each requested instrument —
//! the current snapshot by default, or a historical one via `--time` (OANDA
//! serves 20-minute-boundary snapshots back to ~2018). With `--store` each
//! snapshot is appended to `~/.wickd/books.db` (see [`crate::books`]); the
//! write is idempotent, so this is the verb the launchd collector job runs
//! on an interval.
//!
//! Output is a JSON summary per snapshot (aggregate long/short percentages,
//! bucket count, stored/skipped). `--full` includes the raw bucket arrays.

use anyhow::{Context, Result};
use clap::Args;

use wickd_core::oanda::endpoints;

use crate::books::{self, BookType};
use crate::commands::client;
use crate::output::{exit, Out};
use crate::vault_store;

/// The default collection basket: the gauntlet majors + the remaining
/// USD majors (wickd-lab CAMPAIGN-002 universe).
const DEFAULT_INSTRUMENTS: &str = "EUR_USD,GBP_USD,USD_JPY,USD_CHF,AUD_USD,USD_CAD,NZD_USD,EUR_GBP";

#[derive(Args, Debug)]
pub struct BooksArgs {
    /// Instruments to snapshot, comma-separated (e.g. `EUR_USD,GBP_USD`).
    #[arg(value_delimiter = ',', default_value = DEFAULT_INSTRUMENTS)]
    pub instruments: Vec<String>,

    /// Historical snapshot instant (RFC3339 on a 20-minute boundary, e.g.
    /// `2023-01-03T12:00:00Z`). Omit for the latest snapshot.
    #[arg(long)]
    pub time: Option<String>,

    /// Append the snapshots to the local books store (`~/.wickd/books.db`).
    /// Idempotent: snapshots already stored are skipped, not duplicated.
    #[arg(long)]
    pub store: bool,

    /// Include the raw bucket arrays in the JSON output (they are large).
    #[arg(long)]
    pub full: bool,

    /// OANDA environment whose stored credentials are used.
    #[arg(long, default_value = "practice")]
    pub env: String,

    /// Named account whose credentials are used (see `wickd login --account`).
    #[arg(long, default_value = vault_store::DEFAULT_ACCOUNT)]
    pub account: String,
}

pub async fn run(args: BooksArgs, out: Out) -> ! {
    let result: Result<serde_json::Value> = books_run(&args).await;
    match result {
        Ok(v) => {
            out.ok(&v);
            std::process::exit(exit::OK);
        }
        Err(e) => {
            let msg = format!("{e:#}");
            let code = if msg.contains("keychain") || msg.contains("credentials") {
                exit::AUTH
            } else {
                exit::OANDA
            };
            out.fail(code, "books_failed", msg);
        }
    }
}

async fn books_run(args: &BooksArgs) -> Result<serde_json::Value> {
    let (_env, client) = client::resolve(&args.env, &args.account)?;

    // One store handle for the whole run; only opened when writing.
    let conn = if args.store { Some(books::open()?) } else { None };

    let mut snapshots = Vec::new();
    let mut errors = Vec::new();
    let (mut stored, mut skipped) = (0u32, 0u32);

    for instrument in &args.instruments {
        for book_type in [BookType::Order, BookType::Position] {
            let fetched = match book_type {
                BookType::Order => {
                    endpoints::get_order_book(&client, instrument, args.time.as_deref()).await
                }
                BookType::Position => {
                    endpoints::get_position_book(&client, instrument, args.time.as_deref()).await
                }
            };
            // Per-book failures (an instrument without books, a missing
            // historical snapshot) are reported but do not abort the sweep —
            // a collector run should store everything it CAN get.
            let book = match fetched {
                Ok(b) => b,
                Err(e) => {
                    errors.push(serde_json::json!({
                        "instrument": instrument,
                        "book_type": book_type.as_str(),
                        "error": e.to_string(),
                    }));
                    continue;
                }
            };

            let summary = match &conn {
                Some(c) => {
                    let s = books::record(c, &book, book_type).with_context(|| {
                        format!("could not store {instrument} {} book", book_type.as_str())
                    })?;
                    if s.stored { stored += 1 } else { skipped += 1 }
                    s
                }
                None => books::summarize(&book, book_type)?,
            };

            let mut v = serde_json::to_value(&summary)?;
            if args.full {
                v["bucket_data"] = serde_json::to_value(&book.buckets)?;
            }
            if conn.is_none() {
                // Not a store run: `stored` would be misleading noise.
                if let Some(o) = v.as_object_mut() {
                    o.remove("stored");
                }
            }
            snapshots.push(v);
        }
    }

    let mut result = serde_json::json!({
        "snapshots": snapshots,
        "errors": errors,
    });
    if let Some(c) = &conn {
        result["store"] = serde_json::json!({
            "path": books::books_path()?.display().to_string(),
            "stored": stored,
            "skipped_existing": skipped,
            "total_rows": books::count(c)?,
        });
    }
    Ok(result)
}
