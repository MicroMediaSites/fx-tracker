//! `wickd` — headless, agent-first CLI for OANDA trading.
//!
//! The CLI is the *hands and eyes*: it pipes clean OANDA data in (candles,
//! instruments, live prices) and sends orders out. The *brain* — backtesting,
//! pattern-matching, strategy logic — lives in the agent wrapping this binary,
//! which reasons over the JSON the CLI emits. There is no strategy engine here
//! by design.
//!
//! Output is JSON by default (NDJSON for `stream`) so an agent can parse it
//! directly; `--pretty` switches to indented JSON.
//!
//! Verbs:
//!   login        store OANDA credentials (API key in the OS keychain)
//!   logout       remove stored credentials
//!   candles      historical OHLC (+ optional indicators) for an instrument
//!   instruments  list tradeable instruments
//!   trade        account / positions / orders / place / close
//!   stream       live prices → JSON-lines (coming next)

mod alert;
/// Shim: the durable alert queue moved to wickd-core (AGT-652) so the desktop
/// app reads the same store. Same paths, same wire format.
mod alert_queue {
    pub use wickd_core::alert_queue::*;
}
mod audit;
mod auto_exec;
mod baseline;
mod commands;
mod convert;
mod dashboard;
mod events;
mod feed;
mod fs_perms;
/// Shim: the socket-hub client (probe/attach/partition) moved to
/// wickd-core::hub_client (AGT-652) so the desktop app shares it.
mod hub {
    pub use wickd_core::hub_client::*;
}
mod output;
/// Shim: the pending-proposal store moved to wickd-core (AGT-652).
mod pending {
    pub use wickd_core::pending::*;
}
mod prompt;
mod risk;
mod signal_alert;
mod sink;
mod spread_stats;
/// Shim: the socket-hub server moved to wickd-core (AGT-652) so the desktop
/// app can host a hub when no CLI stream is running.
mod stream_hub {
    pub use wickd_core::stream_hub::*;
}
mod vault_store;
mod watchlist;

use clap::{Parser, Subcommand};

use commands::alert::AlertArgs;
use commands::approve::ApproveArgs;
use commands::audit::AuditArgs;
use commands::backtest::BacktestArgs;
use commands::candles::CandlesArgs;
use commands::dashboard::DashboardArgs;
use commands::instruments::InstrumentsArgs;
use commands::login::{LoginArgs, LogoutArgs};
use commands::pending::PendingArgs;
use commands::queue::QueueArgs;
use commands::strategy::StrategyArgs;
use commands::stream::StreamArgs;
use commands::trade::TradeArgs;
use commands::view::ViewArgs;
use commands::watch::WatchArgs;
use output::Out;

#[derive(Parser)]
#[command(
    name = "wickd",
    version,
    about = "Agent-first OANDA CLI: data in, orders out — your agent supplies the strategy"
)]
struct Cli {
    /// Emit human-readable (indented) JSON instead of compact JSON.
    #[arg(long, global = true)]
    pretty: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Store OANDA credentials (API key in the OS keychain).
    Login(LoginArgs),
    /// Remove stored OANDA credentials.
    Logout(LogoutArgs),
    /// Historical OHLC candles (+ optional indicators) for an instrument.
    Candles(CandlesArgs),
    /// List tradeable instruments.
    Instruments(InstrumentsArgs),
    /// Account, positions, orders, and order execution.
    Trade(TradeArgs),
    /// Live prices → JSON-lines event stream.
    Stream(StreamArgs),
    /// Live terminal watchlist (bid/ask/spread) over a running `wickd stream`.
    Dashboard(DashboardArgs),
    /// List built-in strategies, or run one over candles → JSON signals.
    Strategy(StrategyArgs),
    /// Backtest a strategy over historical candles → JSON metrics + trades.
    Backtest(BacktestArgs),
    /// Open an on-demand ui-leaf view (FX ticket or live signal watcher); headless otherwise.
    View(ViewArgs),
    /// Monitor a strategy against live candles → JSON signal daemon (never trades).
    Watch(WatchArgs),
    /// Read the append-only audit log of execution decisions → JSON.
    Audit(AuditArgs),
    /// List signals awaiting approval (from `wickd watch --semi-auto`) → JSON.
    Pending(PendingArgs),
    /// Approve a pending signal and run its order through the guarded path.
    Approve(ApproveArgs),
    /// Price-level alerts: add|list|remove, or `run` to watch a live feed and fire them.
    Alert(AlertArgs),
    /// Poll the durable alert queue, or promote a strategy-signal alert into a pending proposal.
    Queue(QueueArgs),
}

#[tokio::main]
async fn main() {
    // Rust ignores SIGPIPE before `main`, so piping our JSON into a reader
    // that closes early (`head`, `jq -e`, a pager quit mid-scroll) turns
    // EPIPE into a stdout panic + backtrace. Restore the conventional Unix
    // disposition first thing — before anything writes to stdout — so a
    // closed pipe kills the process quietly (exit status 141), like every
    // other CLI. This is safe for our sockets: Rust std / tokio (and the
    // TLS stack layered over them) write with MSG_NOSIGNAL on Linux and set
    // SO_NOSIGPIPE on macOS, so only the stdout/stderr pipes are affected.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();
    let out = Out::new(cli.pretty);

    match cli.command {
        Command::Login(args) => commands::login::run(args, out).await,
        Command::Logout(args) => commands::login::logout(args, out),
        Command::Candles(args) => commands::candles::run(args, out).await,
        Command::Instruments(args) => commands::instruments::run(args, out).await,
        Command::Trade(args) => commands::trade::run(args, out).await,
        Command::Stream(args) => commands::stream::run(args, out).await,
        Command::Dashboard(args) => commands::dashboard::run(args, out).await,
        Command::Strategy(args) => commands::strategy::run(args, out).await,
        Command::Backtest(args) => commands::backtest::run(args, out).await,
        Command::View(args) => commands::view::run(args, out).await,
        Command::Watch(args) => commands::watch::run(args, out).await,
        Command::Audit(args) => commands::audit::run(args, out).await,
        Command::Pending(args) => commands::pending::run(args, out).await,
        Command::Approve(args) => commands::approve::run(args, out).await,
        Command::Alert(args) => commands::alert::run(args, out).await,
        Command::Queue(args) => commands::queue::run(args, out).await,
    }
}
