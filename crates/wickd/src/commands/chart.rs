//! `wickd chart context SYM [--tf 1d] [--bars N]` — what the TUI chart shows
//! for a symbol, as JSON (wickd-learn D-21): the newest N bars and every
//! indicator value the chart draws at each bar (Ichimoku; 50/200-day
//! averages on daily bars). Values only, no rule verdicts (D-20).
//!
//!   wickd chart context AAPL --tf 1d --bars 30
//!   wickd chart context EUR_USD --tf 1h --bars 30
//!
//! History comes through the chart's own loaders (Alpaca for stocks, OANDA
//! practice for `EUR_USD`-style pairs) and values from the chart's own
//! builder (`tui::context::build`). Prices are display f64 like the chart;
//! nothing here trades.

use chrono::Utc;
use clap::{Args, Subcommand};

use wickd_core::equities::alpaca::{AlpacaClient, QuoteFeed};

use crate::alpaca_creds;
use crate::output::{exit, Out};
use crate::tui::context::{self, Timeframe};
use crate::tui::fx;

#[derive(Args, Debug)]
pub struct ChartArgs {
    #[command(subcommand)]
    pub command: ChartCommand,
}

#[derive(Subcommand, Debug)]
pub enum ChartCommand {
    /// A symbol's chart as numbers: bars + every indicator the chart draws → JSON.
    Context(ContextArgs),
}

#[derive(Args, Debug)]
pub struct ContextArgs {
    /// Stock ticker (AAPL) or forex pair (EUR_USD or EUR/USD).
    pub symbol: String,
    /// Bar size, one per chart range: 5m, 15m, 1h, 1d, 1w, 1mo.
    #[arg(long, default_value = "1d")]
    pub tf: String,
    /// How many of the newest bars to return (1..=1000).
    #[arg(long, default_value_t = 60)]
    pub bars: usize,
    /// Include pre/post-market bars on intraday stock timeframes (the
    /// chart's `v e`); pairs always show every bar.
    #[arg(long)]
    pub extended: bool,
}

pub const MAX_BARS: usize = 1000;

pub async fn run(args: ChartArgs, out: Out) -> ! {
    match args.command {
        ChartCommand::Context(a) => run_context(a, out).await,
    }
}

async fn run_context(args: ContextArgs, out: Out) -> ! {
    let symbol = match crate::info::validate(&args.symbol) {
        Ok(_) => crate::lists::normalize_symbol(&args.symbol),
        Err(e) => out.fail(exit::VALIDATION, "invalid_symbol", format!("{e:#}")),
    };
    let Some(tf) = Timeframe::parse(&args.tf) else {
        out.fail(
            exit::VALIDATION,
            "invalid_timeframe",
            format!("unknown --tf '{}' (use {})", args.tf, Timeframe::labels()),
        );
    };
    if args.bars == 0 || args.bars > MAX_BARS {
        out.fail(
            exit::VALIDATION,
            "invalid_bars",
            format!("--bars must be 1..={MAX_BARS}"),
        );
    }
    let now = Utc::now();
    let history = if fx::is_fx(&symbol) {
        let client = match crate::tui::oanda_client() {
            Ok(c) => c,
            Err(e) => out.fail(exit::AUTH, "no_oanda_keys", e),
        };
        fx::history(&client, &symbol, tf.range, now)
            .await
            .map_err(|e| (exit::OANDA, "history_failed", e))
    } else {
        let creds = match alpaca_creds::resolve() {
            Ok(Some((c, _))) => c,
            Ok(None) => out.fail(
                exit::AUTH,
                "no_alpaca_keys",
                "no Alpaca keys configured — run `wickd alpaca login`",
            ),
            Err(e) => out.fail(exit::AUTH, "alpaca_keys_unreadable", format!("{e:#}")),
        };
        let client = AlpacaClient::new(creds, QuoteFeed::Iex);
        crate::tui::stock_history(&client, &symbol, tf.range, now)
            .await
            .map_err(|e| (exit::GENERIC, "history_failed", e))
    };
    let (warm, vis) = match history {
        Ok(h) => h,
        Err((code, kind, msg)) => out.fail(code, kind, msg),
    };
    let ctx = context::build(&symbol, tf.range, &warm, &vis, args.extended, args.bars);
    if ctx.bars.is_empty() {
        out.fail(
            exit::GENERIC,
            "no_data",
            format!("no {} bars for {symbol}", tf.label),
        );
    }
    out.ok(&ctx);
    std::process::exit(exit::OK);
}
