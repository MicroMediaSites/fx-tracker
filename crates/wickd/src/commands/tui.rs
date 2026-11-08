//! `wickd tui` — terminal stock ticker: watchlist, symbol search, and price
//! history drill-down (US equities via Alpaca market data).
//!
//!   wickd alpaca login          # once: store free Alpaca paper-account keys
//!   wickd tui                   # IEX real-time quotes (free plan)
//!   wickd tui --feed sip        # consolidated real-time quotes (paid plan)
//!   wickd tui --refresh 5       # poll quotes every 5s
//!
//! The watchlist lives in `~/.wickd/tui.json`. This is the one interactive
//! (non-JSON) verb; startup errors still come back as the usual JSON error
//! envelope so agents can branch on them.

use std::time::Duration;

use clap::Args;

use wickd_core::equities::alpaca::{AlpacaClient, QuoteFeed};

use crate::alpaca_creds;
use crate::output::{exit, Out};
use crate::tui;

#[derive(Args, Debug)]
pub struct TuiArgs {
    /// Quote feed: `iex` (free, real-time, IEX exchange only), `sip` (paid,
    /// real-time, all exchanges) or `delayed-sip` (free, all exchanges, 15
    /// minutes behind). History is always consolidated.
    #[arg(long, default_value = "iex")]
    pub feed: String,
    /// Seconds between quote refreshes (minimum 2).
    #[arg(long, default_value_t = 15)]
    pub refresh: u64,
}

pub async fn run(args: TuiArgs, out: Out) -> ! {
    let Some(feed) = QuoteFeed::parse(&args.feed) else {
        out.fail(
            exit::VALIDATION,
            "invalid_feed",
            format!(
                "unknown --feed '{}' (use iex, sip or delayed-sip)",
                args.feed
            ),
        );
    };
    if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        out.fail(
            exit::VALIDATION,
            "not_a_terminal",
            "wickd tui is interactive — run it in a terminal",
        );
    }
    let creds = match alpaca_creds::resolve() {
        Ok(Some((c, _))) => c,
        Ok(None) => out.fail(
            exit::AUTH,
            "no_alpaca_keys",
            "no Alpaca keys configured — run `wickd alpaca login` (free paper-account keys work)",
        ),
        Err(e) => out.fail(exit::AUTH, "alpaca_keys_unreadable", format!("{e:#}")),
    };
    let watchlist = match tui::store::load_watchlist() {
        Ok(w) => w,
        Err(e) => out.fail(exit::GENERIC, "watchlist_unreadable", format!("{e:#}")),
    };
    let opts = tui::Options {
        refresh: Duration::from_secs(args.refresh.max(2)),
    };
    match tui::run(AlpacaClient::new(creds, feed), watchlist, opts).await {
        Ok(()) => std::process::exit(exit::OK),
        Err(e) => out.fail(exit::GENERIC, "tui_failed", format!("{e:#}")),
    }
}
