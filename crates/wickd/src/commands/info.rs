//! `wickd info <SYM>` — what a stock or forex pair is, as JSON (the same card
//! the TUI shows). See [`crate::info`] for the sources.
//!
//!   wickd info QQQ
//!   wickd info eur/usd
//!   wickd info NVDA --refresh     # ignore the 30-day cache

use clap::Args;

use crate::info;
use crate::output::{exit, Out};

#[derive(Args, Debug)]
pub struct InfoArgs {
    /// Stock symbol (`QQQ`) or forex pair (`EUR_USD`, `eur/usd`).
    pub symbol: String,
    /// Refetch instead of using the cached card.
    #[arg(long)]
    pub refresh: bool,
}

pub async fn run(args: InfoArgs, out: Out) -> ! {
    if info::normalize(&args.symbol).is_empty() {
        out.fail(exit::VALIDATION, "invalid_symbol", "symbol is empty");
    }
    match info::fetch(&args.symbol, args.refresh).await {
        Ok(card) => {
            out.ok(&card);
            std::process::exit(exit::OK);
        }
        Err(e) => out.fail(exit::GENERIC, "info_failed", format!("{e:#}")),
    }
}
