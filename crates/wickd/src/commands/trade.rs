//! `wickd trade` — account state and order execution.
//!
//!   wickd trade account
//!   wickd trade positions
//!   wickd trade orders
//!   wickd trade place --instrument EUR_USD --units -1000 --sl 1.0850 --tp 1.0950
//!   wickd trade place --instrument EUR_USD --units 1000 --strategy ma-crossover
//!   wickd trade place --instrument EUR_USD --units 1000 --live   # prompts for a TTY keystroke
//!   wickd trade close --instrument EUR_USD --side long
//!
//! ## Execution safety: paper by default, `--live` to arm
//!
//! Order-submitting verbs (`place`, `close`) are **paper / dry-run by default**.
//! Without `--live` they compute the would-be order and emit it as JSON
//! (`"mode":"paper","submitted":false`) — they never contact OANDA's
//! order-submission endpoints and never require vault credentials. This is the
//! default for both the CLI and any daemon-driven execution.
//!
//! Real orders are submitted **only** when `--live` is passed. `--live` is the
//! single, canonical way to arm real submission. Arming a live submit requires
//! a **human keystroke on an interactive TTY** (AGT-613): the operator is
//! prompted and must type `yes`. The `--yes` trade-arming flag and piped /
//! redirected stdin can NOT satisfy this — a non-interactive context (agent,
//! CI, pipe) FAILS CLOSED and no order is placed.
//!
//! ## Non-interactive auto arming — PRACTICE ONLY (AGT-626)
//!
//! Autonomous trading (trust-ladder Stage 2) needs to arm a live submit without
//! a TTY. `--auto` (or the programmatic `execute_place_auto` / `execute_close_auto`
//! entry points) does exactly that — but it is **deliberately relaxed for the
//! practice environment only**. On `--env practice` a `--live --auto` submit skips
//! the TTY keystroke; on `--env live` it FAILS CLOSED, so autonomy can never fire
//! a real-money order. Live keeps every existing gate — `--auto` is not a second
//! way to arm live, only a practice-only way to arm without a human at the
//! terminal. `--auto` still requires `--live`; on its own it is still paper. The
//! full guarded contract (fatal pre-submit audit row → credential resolve → risk
//! caps → OANDA submit → outcome classification → terminal audit row) is
//! identical on the auto path — only the arming gate differs.
//!
//! `--env practice|live` is orthogonal: it selects WHICH OANDA account/endpoint
//! a *live* order targets (default `practice`). It does **not** arm submission
//! on its own — `--env live` without `--live` is still paper. So the matrix is:
//!
//!   (no `--live`)        → paper: emit the would-be order, never submit
//!   `--live` (+ confirm) → live:  submit a real order to the `--env` account
//!
//! There is deliberately no second way to arm: `--live` is it.

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand, ValueEnum};

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;

use wickd_core::config::OandaEnvironment;
use wickd_core::models::Trade;
use wickd_core::oanda::endpoints;
use wickd_core::oanda::types::{
    EntryOptions, EntryOrderRequest, EntryOrderType, OandaAccount, OrderCreateResponse, TimeInForce,
    TriggerCondition,
};

use crate::audit;
use crate::baseline;
use crate::commands::client;
use crate::output::{exit, Out};
use crate::prompt;
use crate::risk;
use crate::vault_store::{self, env_str, DEFAULT_ACCOUNT};

#[derive(Args, Debug)]
pub struct TradeArgs {
    /// OANDA account/endpoint a *live* order targets (practice|live). This does
    /// NOT arm submission — pass --live for that. Default: practice.
    #[arg(long, default_value = "practice", global = true)]
    pub env: String,
    /// Named account within --env whose credentials are used (AGT-625), e.g.
    /// h004. Default: the single/default account.
    #[arg(long, default_value = crate::vault_store::DEFAULT_ACCOUNT, global = true)]
    pub account: String,
    #[command(subcommand)]
    cmd: TradeCmd,
}

#[derive(Subcommand, Debug)]
enum TradeCmd {
    /// Account summary (balance, NAV, margin, open counts).
    Account,
    /// Open positions.
    Positions,
    /// Pending orders.
    Orders,
    /// Place a market order. Negative --units = short. Paper by default; --live to submit.
    Place(PlaceArgs),
    /// Close an open position (fully). Paper by default; --live to submit.
    Close(CloseArgs),
    /// Account performance since its recorded baseline: realized/unrealized P&L,
    /// NAV vs baseline, and the closed-trade list from OANDA (AGT-631).
    Report(ReportArgs),
    /// One-line performance summary for EVERY configured account in --env over
    /// a rolling recent window. Ignores --account (it spans all of them).
    Glance(GlanceArgs),
    /// Record or inspect an account's performance baseline (AGT-631).
    Baseline(BaselineArgs),
}

#[derive(Args, Debug)]
struct ReportArgs {
    /// How many recent closed trades to pull from OANDA before filtering to the
    /// ones closed since the baseline. Default 500 — comfortably covers a
    /// quarter's paper trades. Raise it if the report window predates them.
    #[arg(long, default_value_t = 500)]
    limit: u32,
}

#[derive(Args, Debug)]
struct GlanceArgs {
    /// Rolling window, in days back from now, that realized P&L is summed over.
    #[arg(long, default_value_t = 7)]
    days: u32,
    /// Exact window start — an ISO date (YYYY-MM-DD) or RFC3339 instant.
    /// Overrides --days. This exists because "today" is not a whole number of
    /// days back: the desktop app passes its viewer's local midnight, which
    /// the CLI cannot infer (it has no idea what timezone the reader is in).
    #[arg(long)]
    since: Option<String>,
    /// How many recent closed trades to pull per account before filtering to
    /// the window. Default 200 — the glance is a summary, not an audit; raise
    /// it for a high-frequency account whose window truncates.
    #[arg(long, default_value_t = 200)]
    limit: u32,
}

#[derive(Args, Debug)]
struct BaselineArgs {
    #[command(subcommand)]
    cmd: BaselineCmd,
}

#[derive(Subcommand, Debug)]
enum BaselineCmd {
    /// Record a new baseline for --account. Supersedes the prior one; the prior
    /// is kept in history. With no --balance, the account's current OANDA
    /// balance is fetched and used.
    Set(BaselineSetArgs),
    /// Show the account's current (latest) baseline, or null if none.
    Show,
    /// Show the account's full baseline history (newest first).
    History,
}

#[derive(Args, Debug)]
struct BaselineSetArgs {
    /// Starting balance to record (exact string, OANDA precision). Omit to fetch
    /// the account's current balance from OANDA (requires credentials).
    #[arg(long)]
    balance: Option<String>,
    /// The instant the balance is as-of: an ISO date (YYYY-MM-DD) or full
    /// RFC3339. Defaults to now — closed trades after this count toward the
    /// report. Backdate it to an account's true start if baselining after the
    /// fact.
    #[arg(long)]
    date: Option<String>,
    /// Account currency to record (e.g. USD). Ignored when --balance is omitted
    /// (the OANDA-fetched currency is used instead). Default: USD.
    #[arg(long)]
    currency: Option<String>,
}

#[derive(Args, Debug)]
struct PlaceArgs {
    #[arg(long)]
    instrument: String,
    /// Units to trade; negative for a short.
    #[arg(long, allow_hyphen_values = true)]
    units: i64,
    /// Entry order kind: market (immediate, FOK), or a resting limit/stop that
    /// waits at --price. Default: market.
    #[arg(long = "type", value_enum, default_value_t = EntryKind::Market)]
    order_type: EntryKind,
    /// Trigger price for a limit/stop entry (required for --type limit|stop;
    /// ignored for market). Formatted to the instrument's precision.
    #[arg(long, required_if_eq_any = [("order_type", "limit"), ("order_type", "stop")])]
    price: Option<String>,
    /// Time-in-force for a limit/stop entry. Default: gtc (rests until
    /// cancelled). Market entries are always FOK regardless of this flag.
    #[arg(long, value_enum)]
    tif: Option<CliTif>,
    /// Good-till-date (RFC3339) for a --tif gtd order.
    #[arg(long)]
    gtd_time: Option<String>,
    /// Worst fill price bound for a stop entry (slippage guard).
    #[arg(long)]
    price_bound: Option<String>,
    /// Which book side triggers a limit/stop entry. Default: OANDA default.
    #[arg(long, value_enum)]
    trigger: Option<CliTrigger>,
    /// Stop-loss price (string, OANDA precision).
    #[arg(long)]
    sl: Option<String>,
    /// Take-profit price.
    #[arg(long)]
    tp: Option<String>,
    /// Strategy to attribute this order to (AGT-630). Carried to OANDA as the
    /// order's clientExtensions tag and recorded in the audit ledger's
    /// strategy column. Optional — a manual order without it is unattributed.
    #[arg(long)]
    strategy: Option<String>,
    /// Arm REAL order submission. Without it, the order is simulated (paper)
    /// and emitted as JSON without ever contacting OANDA.
    #[arg(long)]
    live: bool,
    /// Retained for compatibility; does NOT arm a live submit. A live order
    /// requires an interactive TTY keystroke (AGT-613) — --yes cannot supply it.
    #[arg(long)]
    yes: bool,
    /// Arm a NON-INTERACTIVE live submit for autonomous PRACTICE trading
    /// (AGT-626, trust-ladder Stage 2). Only meaningful with --live; on the
    /// practice env it replaces the TTY keystroke so an agent/daemon can submit.
    /// On the LIVE env it FAILS CLOSED — live always requires the interactive
    /// keystroke, never --auto.
    #[arg(long)]
    auto: bool,
}

/// Entry order kind selected on the CLI (AGT-612, AC1). Market is the immediate
/// FOK path; Limit/Stop are resting orders that wait at a trigger price. All
/// three flow through the SINGLE guarded [`execute_place`]/[`place_confirmed`]
/// sequence — the kind only changes which `/orders` body is POSTed, not the
/// caps/audit guarantees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub(crate) enum EntryKind {
    Market,
    Limit,
    Stop,
}

impl EntryKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            EntryKind::Market => "market",
            EntryKind::Limit => "limit",
            EntryKind::Stop => "stop",
        }
    }
}

/// CLI surface for [`TimeInForce`] (only the values meaningful on an entry
/// order). Kept local so the core type needn't depend on clap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
enum CliTif {
    Gtc,
    Gtd,
    Gfd,
    Fok,
    Ioc,
}

impl From<CliTif> for TimeInForce {
    fn from(t: CliTif) -> Self {
        match t {
            CliTif::Gtc => TimeInForce::GTC,
            CliTif::Gtd => TimeInForce::GTD,
            CliTif::Gfd => TimeInForce::GFD,
            CliTif::Fok => TimeInForce::FOK,
            CliTif::Ioc => TimeInForce::IOC,
        }
    }
}

/// CLI surface for [`TriggerCondition`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
enum CliTrigger {
    Default,
    Inverse,
    Bid,
    Ask,
    Mid,
}

impl From<CliTrigger> for TriggerCondition {
    fn from(t: CliTrigger) -> Self {
        match t {
            CliTrigger::Default => TriggerCondition::Default,
            CliTrigger::Inverse => TriggerCondition::Inverse,
            CliTrigger::Bid => TriggerCondition::Bid,
            CliTrigger::Ask => TriggerCondition::Ask,
            CliTrigger::Mid => TriggerCondition::Mid,
        }
    }
}

/// The full entry specification threaded through the ONE guarded place path
/// (AGT-612, AC1). Bundles the order kind with the price/TIF/bound/trigger a
/// resting limit/stop needs, plus the SL/TP that apply to any kind — so caps
/// and the audit ledger apply identically to market, limit, and stop entries.
#[derive(Clone, Debug)]
pub(crate) struct EntryPlan {
    pub kind: EntryKind,
    pub price: Option<String>,
    pub tif: Option<TimeInForce>,
    pub gtd_time: Option<String>,
    pub price_bound: Option<String>,
    pub trigger: Option<TriggerCondition>,
    pub sl: Option<String>,
    pub tp: Option<String>,
    /// Strategy the order is attributed to (AGT-630): the pending signal's
    /// strategy on the `approve` path, `--strategy` on a manual place, `None`
    /// when unattributed. Flows to OANDA clientExtensions AND the audit
    /// ledger's strategy column.
    pub strategy: Option<String>,
}

impl EntryPlan {
    /// A plain market entry (the only kind `approve` builds in Stage 1): no
    /// price/TIF/bound/trigger, just the optional SL/TP carried from a signal.
    pub(crate) fn market(sl: Option<String>, tp: Option<String>) -> Self {
        Self {
            kind: EntryKind::Market,
            price: None,
            tif: None,
            gtd_time: None,
            price_bound: None,
            trigger: None,
            sl,
            tp,
            strategy: None,
        }
    }

    /// Attribute the plan to a strategy (AGT-630). `approve` uses this to carry
    /// the pending signal's strategy into the guarded place path.
    pub(crate) fn with_strategy(mut self, strategy: Option<String>) -> Self {
        self.strategy = strategy;
        self
    }
}

#[derive(Args, Debug)]
struct CloseArgs {
    #[arg(long)]
    instrument: String,
    /// Which side to close: long | short.
    #[arg(long)]
    side: String,
    /// Arm a REAL position close. Without it, the close is simulated (paper)
    /// and emitted as JSON without ever contacting OANDA.
    #[arg(long)]
    live: bool,
    /// Retained for compatibility; does NOT arm a live submit. A live close
    /// requires an interactive TTY keystroke (AGT-613) — --yes cannot supply it.
    #[arg(long)]
    yes: bool,
    /// Arm a NON-INTERACTIVE live close for autonomous PRACTICE trading
    /// (AGT-626). Only meaningful with --live; practice only — a --auto close
    /// against the LIVE env FAILS CLOSED, exactly like a --auto place.
    #[arg(long)]
    auto: bool,
}

/// Execution mode: simulate (paper) or submit (live).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Paper,
    Live,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::Paper => "paper",
            Mode::Live => "live",
        }
    }
}

/// The single arming decision: `--live` selects live submission, anything else
/// is paper. Pure and side-effect-free so it can be unit-tested directly. Shared
/// by `trade place` and `approve` (AGT-599) so a signal approval inherits the
/// exact same paper-by-default arming.
pub(crate) fn execution_mode(live: bool) -> Mode {
    if live {
        Mode::Live
    } else {
        Mode::Paper
    }
}

pub async fn run(args: TradeArgs, out: Out) -> ! {
    let result = dispatch(args, out).await;
    match result {
        Ok(v) => {
            out.ok(&v);
            std::process::exit(exit::OK);
        }
        Err(e) => {
            let msg = format!("{e:#}");
            out.fail(execution_exit_code(&msg), "trade_failed", msg);
        }
    }
}

/// Classify an execution error message into a stable exit code. Shared by
/// `trade` and `approve` so a signal-approval order is categorized identically
/// to a direct `trade place`. The `risk cap` token (AGT-595) routes risk-cap
/// rejections to `exit::VALIDATION` — keep that match.
pub(crate) fn execution_exit_code(msg: &str) -> i32 {
    if msg.contains("keychain") || msg.contains("credentials") {
        exit::AUTH
    } else if msg.contains("confirm")
        || msg.contains("--yes")
        || msg.contains("side")
        // AGT-595: risk-cap rejections (all carry the "risk cap" token).
        || msg.contains("risk cap")
        // AGT-599: an unknown/already-consumed pending signal is a validation error.
        || msg.contains("pending signal")
        // AGT-612: a limit/stop entry with no --price is a validation error.
        || msg.contains("requires --price")
        // AGT-631: a report/baseline op with no recorded baseline is a
        // validation error (nothing to compute against yet).
        || msg.contains("no baseline")
    {
        exit::VALIDATION
    } else {
        exit::OANDA
    }
}

async fn dispatch(args: TradeArgs, _out: Out) -> Result<serde_json::Value> {
    // Parse the target environment without unlocking the vault — paper mode
    // (the default for place/close) must not require credentials.
    let env = OandaEnvironment::from_str(&args.env).map_err(|e| anyhow!(e.to_string()))?;
    match args.cmd {
        TradeCmd::Account => {
            let (_, oanda) = client::resolve(&args.env, &args.account)?;
            let account = endpoints::get_account(&oanda)
                .await
                .context("OANDA account fetch failed")?;
            Ok(serde_json::json!({ "account": account }))
        }
        TradeCmd::Positions => {
            let (_, oanda) = client::resolve(&args.env, &args.account)?;
            let positions = endpoints::get_positions(&oanda)
                .await
                .context("OANDA positions fetch failed")?;
            Ok(serde_json::json!({ "count": positions.len(), "positions": positions }))
        }
        TradeCmd::Orders => {
            let (_, oanda) = client::resolve(&args.env, &args.account)?;
            let orders = endpoints::get_orders(&oanda)
                .await
                .context("OANDA orders fetch failed")?;
            Ok(serde_json::json!({ "count": orders.len(), "orders": orders }))
        }
        TradeCmd::Place(p) => place(env, &args.account, p).await,
        TradeCmd::Close(c) => close(env, &args.account, c).await,
        TradeCmd::Report(r) => report(env, &args.env, &args.account, r).await,
        TradeCmd::Glance(g) => glance(env, &args.env, g).await,
        TradeCmd::Baseline(b) => baseline_cmd(env, &args.env, &args.account, b).await,
    }
}

/// Live-arming gate (AGT-613). Only reached once `--live` has selected live
/// submission (the paper path returns before this), so it always guards a real
/// OANDA order against either the practice or live account.
///
/// A live submission requires a **human keystroke on an interactive TTY**:
/// - AC1: the `--yes` trade-arming flag (and piped / redirected stdin) can NO
///   LONGER satisfy this confirm. An agent that cannot type at a terminal must
///   not be able to arm a live order, so `yes` is intentionally ignored here.
/// - AC2: a non-interactive context (no TTY) FAILS CLOSED — it bails before any
///   audit write, credential unlock, or network submit, so no order is placed.
/// - AC3: with a TTY, the operator must answer the prompt with `yes` to proceed.
fn confirm_live(env: OandaEnvironment, _yes: bool, what: &str) -> Result<()> {
    confirm_live_gate(prompt::is_interactive(), what, || {
        prompt::line(&format!(
            "Confirm LIVE {what} on the {} account? type 'yes': ",
            env_str(env)
        ))
    })
}

/// Pure decision core of [`confirm_live`], split out so the fail-closed
/// (AC1/AC2) and affirmative-answer (AC3) branches are unit-testable without a
/// real TTY. `interactive` stands in for [`prompt::is_interactive`] and
/// `read_answer` for the TTY read. `read_answer` is NEVER invoked when
/// `interactive` is false — a live arm can therefore never silently fall back to
/// reading a piped/redirected answer; no TTY means an immediate fail-closed.
fn confirm_live_gate(
    interactive: bool,
    what: &str,
    read_answer: impl FnOnce() -> Result<String>,
) -> Result<()> {
    if !interactive {
        bail!(
            "refusing to arm live {what}: a live submit requires an interactive \
             TTY confirmation — --yes and piped input cannot arm it"
        );
    }
    if read_answer()?.trim() != "yes" {
        bail!("live {what} not confirmed");
    }
    Ok(())
}

/// How a live submission is armed (AGT-626). Consulted ONLY on the live path —
/// the paper default returns before any arming happens — so this decides *how* a
/// real OANDA order gets authorized, never *whether* a submit is live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Arming {
    /// The default, human-in-the-loop arming (AGT-613): a live submit requires a
    /// keystroke on an interactive TTY. `yes`/piped input cannot satisfy it. This
    /// is what every existing CLI path uses, unchanged.
    Interactive { yes: bool },
    /// Non-interactive auto arming for autonomous PRACTICE trading (trust-ladder
    /// Stage 2). Valid ONLY for the practice environment: it replaces the TTY
    /// keystroke so an agent/daemon can submit a practice order. A live-env
    /// submit under this arming FAILS CLOSED (see [`arm_auto_practice`]) — autonomy
    /// is practice-only and can never fire a live order.
    AutoPractice,
}

/// The single live-arming gate (AGT-626): dispatch on how the caller armed the
/// submit. Interactive arming keeps the exact AGT-613 TTY contract; auto arming
/// is the new non-interactive practice-only path. Reached only once `--live` has
/// selected a live submit, so it always guards a real OANDA order.
fn arm_live(env: OandaEnvironment, arming: Arming, what: &str) -> Result<()> {
    match arming {
        Arming::Interactive { yes } => confirm_live(env, yes, what),
        Arming::AutoPractice => arm_auto_practice(env, what),
    }
}

/// AGT-626 auto-arming gate (practice-env only). Permits a non-interactive
/// practice submit; REFUSES (fail-closed) for the live environment. This is the
/// deliberate, safety-critical relaxation of the AGT-613 TTY gate: autonomy is
/// wanted on practice accounts (Stage 2), but live keeps every existing gate —
/// a live order still requires an interactive TTY keystroke, never `--auto`.
/// Pure and side-effect-free so both branches are unit-testable.
fn arm_auto_practice(env: OandaEnvironment, what: &str) -> Result<()> {
    match env {
        OandaEnvironment::Practice => Ok(()),
        OandaEnvironment::Live => bail!(
            "refusing to auto-arm live {what}: non-interactive auto execution is \
             permitted for the practice environment only — a live order still \
             requires an interactive TTY confirmation (--auto cannot arm live)"
        ),
    }
}

/// AGT-610 (AC2): classify a live fill by comparing filled vs requested units.
/// A market order is FOK today so a mismatch shouldn't normally happen, but
/// OANDA's fill transaction only ever reports what actually filled — treat
/// anything short of the full requested size (or an unparseable units string)
/// as `partial` rather than silently reporting it as a full `filled`. Pure and
/// unit-tested directly.
pub(crate) fn fill_outcome(filled_units: &str, requested_units: i64) -> &'static str {
    match filled_units.trim().parse::<i64>() {
        Ok(filled) if filled.abs() == requested_units.abs() => "filled",
        _ => "partial",
    }
}

/// AGT-612 (AC3): the four terminal outcomes an entry-order submission can
/// reach. The string forms (`filled`/`partial`/`resting`/`rejected`) are the
/// canonical `outcome` discriminator AGT-610 introduced — reused verbatim so
/// the audit ledger and `approve.rs` share one vocabulary across market and
/// limit/stop entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryOutcome {
    Filled,
    Partial,
    Rested,
    Rejected,
}

impl EntryOutcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            EntryOutcome::Filled => "filled",
            EntryOutcome::Partial => "partial",
            EntryOutcome::Rested => "resting",
            EntryOutcome::Rejected => "rejected",
        }
    }
}

/// AGT-612 (AC3): classify an OANDA `/orders` response into exactly one of the
/// four terminal outcomes. Precedence, most-conclusive first:
///   1. an `orderFillTransaction` → the order executed; `Filled` vs `Partial`
///      by comparing filled units to the requested size (via [`fill_outcome`],
///      the single source AGT-610 established for that distinction). This wins
///      even when an `orderCreateTransaction` is also present (OANDA returns
///      both on an immediate fill of a limit/stop).
///   2. an `orderRejectTransaction` (a hard reject — OANDA refused to create the
///      order) OR an `orderCancelTransaction` (created then cancelled) →
///      `Rejected`.
///   3. an `orderCreateTransaction` with none of the above → `Rested`: OANDA
///      accepted the order and it is working (a limit/stop awaiting its trigger).
///   4. nothing recognizable → `Rejected`, the fail-safe: an un-acknowledged
///      order must never be reported as `Rested` (which would consume its
///      pending signal — AC5) or as `Filled`.
/// Pure and unit-tested directly against synthetic responses.
pub(crate) fn classify_entry(response: &OrderCreateResponse, requested_units: i64) -> EntryOutcome {
    if let Some(fill) = &response.order_fill_transaction {
        return if fill_outcome(&fill.units, requested_units) == "filled" {
            EntryOutcome::Filled
        } else {
            EntryOutcome::Partial
        };
    }
    if response.order_reject_transaction.is_some() || response.order_cancel_transaction.is_some() {
        return EntryOutcome::Rejected;
    }
    if response.order_create_transaction.is_some() {
        return EntryOutcome::Rested;
    }
    EntryOutcome::Rejected
}

/// The human-readable cause to record for a `Rejected` outcome: prefer a hard
/// reject's reason, then a cancel's reason, then a generic label.
fn rejection_reason(response: &OrderCreateResponse) -> String {
    if let Some(reject) = &response.order_reject_transaction {
        return reject.cause();
    }
    if let Some(cancel) = &response.order_cancel_transaction {
        return cancel.reason.clone();
    }
    "order rejected (no acknowledgement from OANDA)".to_string()
}

/// AGT-610 (AC1): terminal audit write for a live submit call that itself
/// returned an error (e.g. a hard OANDA 400 reject) rather than a response to
/// inspect for a fill/cancel. Before this existed, that error just propagated
/// straight through `?` and NO terminal row was ever written — the pre-submit
/// "attempt" row (AGT-596) was left stuck forever since audit rows are
/// append-only/immutable. Takes the audit-db path explicitly (mirroring the
/// `_at` convention in `audit.rs`/`pending.rs`) so this exact function is
/// directly testable against a throwaway store; `execute_place`/`close` call
/// it with the real default path resolved via `audit::audit_path()`.
fn record_submit_terminal_error(
    audit_db: impl AsRef<std::path::Path>,
    action: &str,
    env: OandaEnvironment,
    instrument: &str,
    units: Option<i64>,
    sl: &Option<String>,
    tp: &Option<String>,
    strategy: &Option<String>,
    err: &anyhow::Error,
) {
    let mut entry = audit::AuditEntry::now(action, Mode::Live.as_str(), "error")
        .env(env_str(env))
        .instrument(instrument)
        .sl(sl.clone())
        .tp(tp.clone())
        .strategy(strategy.clone())
        .detail(Some(format!("{err:#}")));
    if let Some(units) = units {
        entry = entry.units(units);
    }
    audit::record_decision_at(audit_db, entry);
}

/// Best-effort wrapper: resolve the default audit db path and record the
/// submit-error terminal row there. A failure to resolve the path (e.g. no
/// home dir) is swallowed — consistent with [`audit::record_decision`]'s own
/// fire-and-forget contract for post-submission outcome rows.
fn record_submit_terminal_error_default(
    action: &str,
    env: OandaEnvironment,
    instrument: &str,
    units: Option<i64>,
    sl: &Option<String>,
    tp: &Option<String>,
    strategy: &Option<String>,
    err: &anyhow::Error,
) {
    if let Ok(path) = audit::audit_path() {
        record_submit_terminal_error(path, action, env, instrument, units, sl, tp, strategy, err);
    } else {
        eprintln!("warning: audit log write failed: could not resolve audit db path");
    }
}

/// Build the would-be order payload for paper mode. Pure: no network, no
/// credentials — just the intended order the live path *would* submit. Reflects
/// the entry kind and (for limit/stop) the trigger price/TIF so a dry run shows
/// exactly what a live run would POST (AGT-612).
fn build_paper_order(env: OandaEnvironment, instrument: &str, units: i64, plan: &EntryPlan) -> serde_json::Value {
    let side = if units < 0 { "short" } else { "long" };
    serde_json::json!({
        "ok": true,
        "mode": Mode::Paper.as_str(),
        "submitted": false,
        "environment": env_str(env),
        "instrument": instrument,
        "units": units,
        "side": side,
        "type": plan.kind.as_str(),
        "price": plan.price,
        // Serde-serialized (the OANDA wire form, e.g. "GTC"), not a debug repr,
        // so the paper payload is a stable contract for an agent to parse.
        "tif": plan.tif.and_then(|t| serde_json::to_value(t).ok()),
        "sl": plan.sl,
        "tp": plan.tp,
        "strategy": plan.strategy,
    })
}

/// Build the would-be close payload for paper mode. Pure: no network.
fn build_paper_close(env: OandaEnvironment, instrument: &str, side: &str) -> serde_json::Value {
    serde_json::json!({
        "ok": true,
        "mode": Mode::Paper.as_str(),
        "submitted": false,
        "environment": env_str(env),
        "instrument": instrument,
        "side": side,
    })
}

/// `trade place`: a thin adapter over the shared guarded execution path.
/// Behavior is identical to before the AGT-599 refactor — all the logic now
/// lives in [`execute_place`], which `approve` also calls.
async fn place(env: OandaEnvironment, account: &str, p: PlaceArgs) -> Result<serde_json::Value> {
    let plan = EntryPlan {
        kind: p.order_type,
        price: p.price,
        tif: p.tif.map(Into::into),
        gtd_time: p.gtd_time,
        price_bound: p.price_bound,
        trigger: p.trigger.map(Into::into),
        sl: p.sl,
        tp: p.tp,
        strategy: p.strategy,
    };
    // AGT-626: `--auto` arms a non-interactive practice submit; otherwise the
    // default interactive (TTY-keystroke) arming applies. Both flow through the
    // one guarded sequence — `--auto` only changes the arming gate, nothing else.
    if p.auto {
        execute_place_auto(env, account, &p.instrument, p.units, plan, p.live).await
    } else {
        execute_place(env, account, &p.instrument, p.units, plan, p.live, p.yes).await
    }
}

/// The interactive (human-in-the-loop) guarded place entry point, shared by
/// `trade place` and `approve` (AGT-599). Paper by default; `--live` arms a real
/// submission gated on a TTY keystroke (AGT-613). Signature and behavior are
/// unchanged (AGT-626 leaves the live path byte-for-byte identical) — it simply
/// delegates to [`execute_place_armed`] with interactive arming.
pub(crate) async fn execute_place(
    env: OandaEnvironment,
    account: &str,
    instrument: &str,
    units: i64,
    plan: EntryPlan,
    live: bool,
    yes: bool,
) -> Result<serde_json::Value> {
    execute_place_armed(env, account, instrument, units, plan, live, Arming::Interactive { yes })
        .await
}

/// AGT-626: the programmatic NON-INTERACTIVE place entry point for autonomous
/// practice trading (trust-ladder Stage 2). Identical to [`execute_place`] but
/// armed with [`Arming::AutoPractice`], so a `--live` submit needs no TTY —
/// PROVIDED `env` is practice. A live-env submit under this arming fails closed
/// (see [`arm_auto_practice`]), so autonomy can never place a live order. It runs
/// the exact same guarded sequence, so it inherits the full audit + caps
/// contract for free — the only relaxation is the arming gate.
/// Broker-truth check for the auto path's duplicate-entry guard: does the
/// account ACTUALLY hold an open position on `instrument`? The executor's
/// in-memory map can hold a corpse — a position the broker already closed
/// via stop-loss/take-profit — and a corpse must not veto real entries
/// (observed 2026-07-17: every stopped-out M1 instrument was entry-poisoned
/// until restart). One targeted fetch on the conflict path only.
pub(crate) async fn position_open_at_broker(
    env: OandaEnvironment,
    account: &str,
    instrument: &str,
) -> Result<bool> {
    let (_, client) = client::resolve(crate::vault_store::env_str(env), account)?;
    let positions = wickd_core::oanda::endpoints::get_open_positions(&client).await?;
    Ok(positions.iter().any(|p| p.instrument == instrument && !p.is_flat()))
}

pub(crate) async fn execute_place_auto(
    env: OandaEnvironment,
    account: &str,
    instrument: &str,
    units: i64,
    plan: EntryPlan,
    live: bool,
) -> Result<serde_json::Value> {
    execute_place_armed(env, account, instrument, units, plan, live, Arming::AutoPractice).await
}

/// The single guarded place sequence, shared by every place entry point.
/// Paper by default; `--live` (with the given [`Arming`]) arms a real submission
/// through the arming gate → `audit::record_required` (fatal pre-submit) →
/// `risk::enforce_live_place` (caps) → OANDA submit → `audit::record_decision`
/// → `risk::record_fill`. This is the ONLY path that turns intent (a CLI flag or
/// an approved signal) into a live OANDA order, so any new caller inherits the
/// full arming + caps + audit guarantees for free. `arming` selects HOW the
/// live submit is authorized (interactive TTY vs practice auto) — it does not
/// change the audit/caps ordering, which is identical on every arming.
async fn execute_place_armed(
    env: OandaEnvironment,
    account: &str,
    instrument: &str,
    units: i64,
    plan: EntryPlan,
    live: bool,
    arming: Arming,
) -> Result<serde_json::Value> {
    // AGT-612 (AC2): a resting limit/stop entry is meaningless without a trigger
    // price. Reject a structurally-invalid request UP FRONT — before arming,
    // credentials, or an audit "attempt" row — so a malformed CLI call never
    // leaves a stuck ledger row. The "requires --price" token routes to
    // exit::VALIDATION (see `execution_exit_code`).
    if matches!(plan.kind, EntryKind::Limit | EntryKind::Stop) && plan.price.is_none() {
        bail!("a {} entry order requires --price", plan.kind.as_str());
    }

    // Paper (default): emit the would-be order, never submit, never need creds.
    if execution_mode(live) == Mode::Paper {
        audit::record_decision(
            audit::AuditEntry::now("place", Mode::Paper.as_str(), "not_submitted")
                .env(env_str(env))
                .instrument(instrument)
                .units(units)
                .sl(plan.sl.clone())
                .tp(plan.tp.clone())
                .strategy(plan.strategy.clone())
                .detail(Some(format!("type={}", plan.kind.as_str()))),
        );
        return Ok(build_paper_order(env, instrument, units, &plan));
    }

    // Live: run the arming gate BEFORE unlocking the vault or touching the
    // network. Interactive arming (AGT-613) demands a human TTY keystroke and
    // fails closed in any non-interactive context. Auto arming (AGT-626) permits
    // a non-interactive PRACTICE submit but fails closed for the live env. Either
    // way, a refusal bails here — before any audit write, credential unlock, or
    // submit — so no order is placed.
    arm_live(env, arming, "order")?;
    place_confirmed(env, account, instrument, units, plan).await
}

/// The post-confirmation live place sequence (AGT-613 split): everything after
/// the [`confirm_live`] arming gate. PRECONDITION: `confirm_live` has already
/// passed (a human keystroke on an interactive TTY). Kept as its own function so
/// the AGT-610 audit-before-submit ordering invariants stay unit-testable
/// without a TTY — those tests drive this directly, while the live arming gate
/// is tested via [`confirm_live_gate`]. Order: `audit::record_required` (fatal
/// pre-submit) → `client::resolve` (creds) → `risk::enforce_live_place` (caps) →
/// OANDA submit → terminal audit row.
async fn place_confirmed(
    env: OandaEnvironment,
    account: &str,
    instrument: &str,
    units: i64,
    plan: EntryPlan,
) -> Result<serde_json::Value> {
    let sl = plan.sl.clone();
    let tp = plan.tp.clone();

    // Audit the attempt BEFORE submitting: a failed audit write aborts the live
    // order so OANDA can never receive an order that left no ledger row (AC1).
    audit::record_required(
        &audit::AuditEntry::now("place", Mode::Live.as_str(), "attempt")
            .env(env_str(env))
            .instrument(instrument)
            .units(units)
            .sl(sl.clone())
            .tp(tp.clone())
            .strategy(plan.strategy.clone())
            .detail(Some(format!("type={}", plan.kind.as_str()))),
    )?;
    let (_, oanda) = client::resolve(env_str(env), account)?;
    // AGT-595: enforce position-risk caps before submitting — identically for
    // every kind (AC1). Rejects (no submission) if size / max-open / daily-loss
    // kill-switch is breached.
    risk::enforce_live_place(&oanda, units).await?;

    // AGT-612 (AC1/AC2): branch ONLY on which `/orders` body to POST — market
    // (immediate FOK) vs a resting limit/stop entry. Everything around this call
    // (arming, caps, audit, classification) is shared, so this is not a parallel
    // path.
    let submit = match plan.kind {
        EntryKind::Market => {
            // AGT-630 (AC1): the strategy (when known) rides to OANDA as the
            // order's clientExtensions, so the broker's transaction record
            // itself carries the attribution.
            endpoints::place_market_order_attributed(
                &oanda,
                instrument,
                units,
                sl.as_deref(),
                tp.as_deref(),
                plan.strategy.as_deref(),
            )
            .await
        }
        EntryKind::Limit | EntryKind::Stop => {
            let kind = if plan.kind == EntryKind::Limit {
                EntryOrderType::Limit
            } else {
                EntryOrderType::Stop
            };
            // Guaranteed Some by the up-front validation in `execute_place`.
            let price = plan.price.as_deref().unwrap_or_default();
            let opts = EntryOptions {
                time_in_force: plan.tif,
                gtd_time: plan.gtd_time.clone(),
                price_bound: plan.price_bound.clone(),
                trigger_condition: plan.trigger,
                stop_loss: sl.clone(),
                take_profit: tp.clone(),
                strategy: plan.strategy.clone(),
            };
            let request = EntryOrderRequest::new(kind, instrument, units, price, &opts);
            endpoints::place_entry_order(&oanda, &request).await
        }
    };

    let response = match submit.context("OANDA order placement failed") {
        Ok(r) => r,
        Err(err) => {
            // AGT-610 (AC1): a hard reject from the submit call itself (e.g. an
            // OANDA 400 with an errorMessage/errorCode body) must still land a
            // terminal audit row — otherwise the pre-submit "attempt" row
            // written above is left stuck forever with no outcome, since audit
            // rows are append-only/immutable and never get updated in place.
            record_submit_terminal_error_default(
                "place",
                env,
                instrument,
                Some(units),
                &sl,
                &tp,
                &plan.strategy,
                &err,
            );
            return Err(err);
        }
    };

    // AGT-612 (AC3/AC4): classify the response into ONE of four terminal
    // outcomes, then write the audit row with that TRUE outcome BEFORE returning
    // on every path.
    let env_name = env_str(env);
    let outcome = classify_entry(&response, units);
    match outcome {
        EntryOutcome::Filled | EntryOutcome::Partial => {
            // The classifier only returns Filled/Partial when a fill txn exists.
            // Propagate the anomaly as a structured error rather than panicking:
            // this is the live order path, so a surprising OANDA response (or a
            // future change that loosens `classify_entry`'s invariant) must not
            // drop the process while live positions are open.
            let fill = response.order_fill_transaction.ok_or_else(|| {
                anyhow!("BUG: classifier returned {outcome:?} but no fill txn in OANDA response")
            })?;
            audit::record_decision(
                audit::AuditEntry::now("place", Mode::Live.as_str(), outcome.as_str())
                    .env(env_name)
                    .instrument(instrument)
                    .units(units)
                    .sl(sl.clone())
                    .tp(tp.clone())
                    .strategy(plan.strategy.clone())
                    .detail(Some(format!("fill_id={} realized_pl={}", fill.id, fill.pl))),
            );
            // AGT-595: fold this fill's realized P&L into the daily kill-switch.
            risk::record_fill(&fill.pl);
            Ok(serde_json::json!({
                "ok": true,
                "mode": Mode::Live.as_str(),
                "submitted": true,
                "filled": outcome == EntryOutcome::Filled,
                "outcome": outcome.as_str(),
                "type": plan.kind.as_str(),
                "environment": env_name,
                "instrument": fill.instrument,
                "units": fill.units,
                "requested_units": units,
                "price": fill.price,
                "realized_pl": fill.pl,
                "trade_id": fill.trade_opened.map(|t| t.trade_id),
                "fill_id": fill.id,
                "time": fill.time,
                "strategy": plan.strategy,
            }))
        }
        EntryOutcome::Rejected => {
            // AGT-612 (AC3): a hard `orderRejectTransaction` OR a
            // created-then-cancelled order. Both are terminal rejections.
            let reason = rejection_reason(&response);
            audit::record_decision(
                audit::AuditEntry::now("place", Mode::Live.as_str(), outcome.as_str())
                    .env(env_name)
                    .instrument(instrument)
                    .units(units)
                    .sl(sl.clone())
                    .tp(tp.clone())
                    .strategy(plan.strategy.clone())
                    .detail(Some(format!("reason={reason}"))),
            );
            Ok(serde_json::json!({
                "ok": false,
                "mode": Mode::Live.as_str(),
                "submitted": true,
                "filled": false,
                "outcome": outcome.as_str(),
                "type": plan.kind.as_str(),
                "environment": env_name,
                "instrument": instrument,
                "reason": reason,
                "strategy": plan.strategy,
            }))
        }
        EntryOutcome::Rested => {
            // AGT-612 (AC3/AC5): accepted-but-resting — ok:true because OANDA
            // accepted the order; it just hasn't filled yet (a limit/stop
            // awaiting its trigger). `outcome:"resting"` is what approve.rs reads
            // to consume the pending signal (so it can't be re-approved) WITHOUT
            // treating this as a rejection.
            let order_id = response
                .order_create_transaction
                .as_ref()
                .map(|t| t.id.clone());
            audit::record_decision(
                audit::AuditEntry::now("place", Mode::Live.as_str(), outcome.as_str())
                    .env(env_name)
                    .instrument(instrument)
                    .units(units)
                    .sl(sl.clone())
                    .tp(tp.clone())
                    .strategy(plan.strategy.clone())
                    .detail(Some(format!(
                        "order accepted, resting (not yet filled); type={} order_id={}",
                        plan.kind.as_str(),
                        order_id.as_deref().unwrap_or("?")
                    ))),
            );
            Ok(serde_json::json!({
                "ok": true,
                "mode": Mode::Live.as_str(),
                "submitted": true,
                "filled": false,
                "outcome": outcome.as_str(),
                "type": plan.kind.as_str(),
                "environment": env_name,
                "instrument": instrument,
                "price": plan.price,
                "order_id": order_id,
                "reason": "order accepted, resting (not yet filled)",
                "strategy": plan.strategy,
            }))
        }
    }
}

/// `trade close`: a thin adapter over the shared guarded close path. AGT-626:
/// `--auto` selects the non-interactive practice arming; otherwise the default
/// interactive (TTY-keystroke) arming applies.
async fn close(env: OandaEnvironment, account: &str, c: CloseArgs) -> Result<serde_json::Value> {
    if c.auto {
        execute_close_auto(env, account, &c.instrument, &c.side, c.live).await
    } else {
        execute_close_armed(
            env,
            account,
            &c.instrument,
            &c.side,
            c.live,
            Arming::Interactive { yes: c.yes },
        )
        .await
    }
}

/// AGT-626: the programmatic NON-INTERACTIVE close entry point for autonomous
/// practice trading. Identical to the interactive close but armed with
/// [`Arming::AutoPractice`], so a `--live` close needs no TTY on the practice env
/// and fails closed on the live env. Runs the exact same guarded sequence, so the
/// daily-loss kill-switch (AGT-595) is enforced identically (AC3).
pub(crate) async fn execute_close_auto(
    env: OandaEnvironment,
    account: &str,
    instrument: &str,
    side: &str,
    live: bool,
) -> Result<serde_json::Value> {
    execute_close_armed(env, account, instrument, side, live, Arming::AutoPractice).await
}

/// The single guarded close sequence, shared by every close entry point. Paper by
/// default; `--live` (with the given [`Arming`]) arms a real close through the
/// arming gate → `audit::record_required` (fatal pre-submit) →
/// `risk::enforce_live_close` (kill-switch) → OANDA close → terminal audit row.
/// `arming` only selects HOW the live close is authorized — the audit/kill-switch
/// ordering is identical on every arming (AC3).
async fn execute_close_armed(
    env: OandaEnvironment,
    account: &str,
    instrument: &str,
    side: &str,
    live: bool,
    arming: Arming,
) -> Result<serde_json::Value> {
    let is_long = match side.to_lowercase().as_str() {
        "long" => true,
        "short" => false,
        other => bail!("invalid --side '{other}' (expected 'long' or 'short')"),
    };

    // Paper (default): emit the would-be close, never submit, never need creds.
    if execution_mode(live) == Mode::Paper {
        audit::record_decision(
            audit::AuditEntry::now("close", Mode::Paper.as_str(), "not_submitted")
                .env(env_str(env))
                .instrument(instrument)
                .detail(Some(format!("side={side}"))),
        );
        return Ok(build_paper_close(env, instrument, side));
    }

    // Live: run the arming gate before unlocking the vault or touching the
    // network. Interactive arming needs a TTY keystroke; auto arming (AGT-626)
    // permits a non-interactive practice close but fails closed for live.
    arm_live(env, arming, "position close")?;
    // Audit the attempt BEFORE submitting (AC1): fatal write — no live close
    // reaches OANDA without a ledger row already on disk.
    audit::record_required(
        &audit::AuditEntry::now("close", Mode::Live.as_str(), "attempt")
            .env(env_str(env))
            .instrument(instrument)
            .detail(Some(format!("side={side}"))),
    )?;
    let (_, oanda) = client::resolve(env_str(env), account)?;
    // AGT-595: a close is still live execution — a tripped daily-loss
    // kill-switch halts it too (size/max-open caps don't apply to a close).
    risk::enforce_live_close(&oanda).await?;
    let response = match endpoints::close_position(&oanda, instrument, is_long)
        .await
        .context("OANDA position close failed")
    {
        Ok(r) => r,
        Err(err) => {
            // AGT-610 (AC1): same fix as `execute_place` — a hard reject from
            // the close submit call must still land a terminal audit row so
            // the pre-submit "attempt" row above doesn't get stuck forever.
            record_submit_terminal_error_default(
                "close",
                env,
                instrument,
                None,
                &None,
                &None,
                &None,
                &err,
            );
            return Err(err);
        }
    };

    audit::record_decision(
        audit::AuditEntry::now(
            "close",
            Mode::Live.as_str(),
            if response.long_order_fill_transaction.is_some()
                || response.short_order_fill_transaction.is_some()
            {
                "filled"
            } else {
                "no_fill"
            },
        )
        .env(env_str(env))
        .instrument(instrument)
        .detail(Some(format!("side={side}"))),
    );

    let env_name = env_str(env);
    let fill = response
        .long_order_fill_transaction
        .or(response.short_order_fill_transaction);
    match fill {
        Some(f) => {
            // AGT-595: record the close's realized P&L for the kill-switch.
            risk::record_fill(&f.pl);
            Ok(serde_json::json!({
            "ok": true,
            "mode": Mode::Live.as_str(),
            "submitted": true,
            "closed": true,
            "environment": env_name,
            "instrument": f.instrument,
            "side": side,
            "units": f.units,
            "price": f.price,
            "realized_pl": f.pl,
            "fill_id": f.id,
            "time": f.time,
            }))
        }
        None => Ok(serde_json::json!({
            "ok": true,
            "mode": Mode::Live.as_str(),
            "submitted": true,
            "closed": false,
            "environment": env_name,
            "instrument": instrument,
            "side": side,
            "note": "no fill transaction (position may have been empty)",
        })),
    }
}

// ── `trade report` + `trade baseline` (AGT-631) ─────────────────────────────

/// Parse an OANDA money string to `Decimal`, defaulting to zero on an empty or
/// unparseable value — the same forgiving posture the core `Trade` conversion
/// takes, so a stray field never crashes a read-only report.
fn parse_dec(s: &str) -> Decimal {
    s.trim().parse().unwrap_or(Decimal::ZERO)
}

/// Render a `Decimal` as an exact JSON string. Money and metrics cross the JSON
/// boundary as strings across this codebase (see the audit ledger / backtest
/// metrics) so an agent parses exact decimals, never lossy floats.
fn dec_str(d: Decimal) -> String {
    d.to_string()
}

/// Parse a `--date` argument: an ISO date (`YYYY-MM-DD`, read as 00:00:00 UTC)
/// or a full RFC3339 instant. Pure and unit-tested.
fn parse_baseline_date(input: &str) -> Result<DateTime<Utc>> {
    let s = input.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = d.and_hms_opt(0, 0, 0).expect("00:00:00 is always valid");
        return Ok(DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc));
    }
    bail!("invalid --date '{input}': use an ISO date (YYYY-MM-DD) or an RFC3339 instant")
}

/// The OANDA-reported account state the report reconciles against. Parsed once
/// from the account fetch so the pure [`build_report`] needn't know OANDA's
/// string wire format.
#[derive(Debug, Clone)]
struct AccountSnapshot {
    balance: Decimal,
    nav: Decimal,
    unrealized_pl: Decimal,
    currency: String,
    open_trade_count: i32,
}

impl AccountSnapshot {
    fn from_oanda(a: &OandaAccount) -> Self {
        Self {
            balance: parse_dec(&a.balance),
            nav: parse_dec(&a.nav),
            unrealized_pl: parse_dec(&a.unrealized_pl),
            currency: a.currency.clone(),
            open_trade_count: a.open_trade_count,
        }
    }
}

/// Keep only trades closed at/after `since` (the baseline instant), newest
/// first. A CLOSED trade always has a `close_time`; anything missing one is
/// excluded (it has no realized P&L to attribute to the window). Pure.
fn closed_since(mut trades: Vec<Trade>, since: DateTime<Utc>) -> Vec<Trade> {
    trades.retain(|t| t.close_time.map(|ct| ct >= since).unwrap_or(false));
    trades.sort_by(|a, b| b.close_time.cmp(&a.close_time));
    trades
}

/// Build the report JSON from the recorded baseline, the OANDA account
/// snapshot, and the closed trades since the baseline. Pure (no network) so the
/// realized sum, NAV-vs-baseline, per-strategy grouping, and reconciliation
/// residual are unit-tested directly.
fn build_report(
    account: &str,
    b: &baseline::Baseline,
    snap: &AccountSnapshot,
    closed: &[Trade],
) -> serde_json::Value {
    let baseline_balance = parse_dec(&b.balance);

    // Realized P&L since baseline = sum of the closed trades' realized P&L.
    let realized: Decimal = closed.iter().map(|t| t.realized_pl).sum();
    let unrealized = snap.unrealized_pl;
    let nav = snap.nav;
    let nav_change = nav - baseline_balance;

    // Reconciliation (AC3): the headline `nav` is OANDA's own account NAV, so
    // the report reconciles with the broker by construction. As a documented
    // sanity check we also reconstruct NAV from the baseline + the P&L we
    // account for; the residual should be ~0 net of financing/fees (and any
    // trades that predate the baseline / fetch window).
    let reconstructed_nav = baseline_balance + realized + unrealized;
    let residual = nav - reconstructed_nav;

    // Per-strategy realized attribution: AGT-630's clientExtensions tag, echoed
    // by OANDA onto each closed trade. Manual/legacy trades → "unattributed".
    let mut grouped: std::collections::BTreeMap<String, (Decimal, u64)> =
        std::collections::BTreeMap::new();
    for t in closed {
        let key = t
            .strategy
            .clone()
            .unwrap_or_else(|| "unattributed".to_string());
        let e = grouped.entry(key).or_insert((Decimal::ZERO, 0));
        e.0 += t.realized_pl;
        e.1 += 1;
    }
    let by_strategy: serde_json::Map<String, serde_json::Value> = grouped
        .into_iter()
        .map(|(k, (pl, n))| {
            (
                k,
                serde_json::json!({ "realized_pl": dec_str(pl), "trades": n }),
            )
        })
        .collect();

    let closed_trades: Vec<serde_json::Value> = closed
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "instrument": t.instrument,
                "units": dec_str(t.units),
                "open_time": t.open_time.to_rfc3339(),
                "close_time": t.close_time.map(|ct| ct.to_rfc3339()),
                "realized_pl": dec_str(t.realized_pl),
                "strategy": t.strategy,
            })
        })
        .collect();

    serde_json::json!({
        "account": account,
        "environment": b.environment,
        "currency": snap.currency,
        "baseline": {
            "balance": b.balance,
            "date": b.baseline_date,
            "recorded_at": b.recorded_at,
        },
        "realized_pl_since_baseline": dec_str(realized),
        "unrealized_pl": dec_str(unrealized),
        "nav": dec_str(nav),
        "balance": dec_str(snap.balance),
        "nav_vs_baseline": dec_str(nav_change),
        "open_trade_count": snap.open_trade_count,
        "closed_trade_count": closed.len(),
        "by_strategy": by_strategy,
        "reconciliation": {
            "oanda_nav": dec_str(nav),
            "reconstructed_nav": dec_str(reconstructed_nav),
            "residual": dec_str(residual),
            "note": "reconstructed_nav = baseline + realized_pl_since_baseline + \
                     unrealized_pl; residual should be ~0 net of financing/fees. A \
                     large residual means funds moved or trades predate the \
                     baseline/fetch window. The headline nav is OANDA's own account NAV.",
        },
        "closed_trades": closed_trades,
    })
}

/// `wickd trade report --account <name>` (AGT-631, AC2/AC3). Loads the account's
/// latest baseline, fetches its live OANDA account + closed-trade history, and
/// emits the performance JSON. Fails (validation) if no baseline exists yet.
async fn report(
    env: OandaEnvironment,
    env_raw: &str,
    account: &str,
    r: ReportArgs,
) -> Result<serde_json::Value> {
    let conn = baseline::open()?;
    let base = baseline::latest(&conn, account)?.ok_or_else(|| {
        anyhow!(
            "no baseline recorded for {} account '{}' — run \
             `wickd trade baseline set --env {} --account {} --balance <amount>`",
            env_str(env),
            account,
            env_raw,
            account
        )
    })?;
    let since = parse_baseline_date(&base.baseline_date)
        .context("stored baseline_date is not a valid date")?;

    let (_, oanda) = client::resolve(env_raw, account)?;
    let acct = endpoints::get_account(&oanda)
        .await
        .context("OANDA account fetch failed")?;
    let snap = AccountSnapshot::from_oanda(&acct);

    let closed = endpoints::get_trade_history(&oanda, Some(r.limit), None)
        .await
        .context("OANDA closed-trade history fetch failed")?;
    let closed = closed_since(closed, since);

    Ok(build_report(account, &base, &snap, &closed))
}

/// Resolve the glance window's start instant: an explicit `--since` if given,
/// otherwise `days` back from `now`. Pure and unit-tested.
///
/// `--since` wins over `--days` rather than erroring on both, because `--days`
/// carries a clap default and is therefore always "set" — there is no way to
/// tell "the user asked for 7" from "nobody passed anything".
fn glance_window(
    now: DateTime<Utc>,
    days: u32,
    since: Option<&str>,
) -> Result<DateTime<Utc>> {
    match since {
        Some(s) => parse_baseline_date(s).context("invalid --since"),
        None => Ok(now - chrono::Duration::days(days as i64)),
    }
}

/// Group every configured account name in `env_cfg` by the OANDA account id it
/// resolves to, so accounts aliased to the same broker account are fetched once
/// and rendered as one row. (Matt's practice config has `default` and `tf-m30`
/// both pointing at `…-005`.)
///
/// The group's *primary* name is the first non-`default` name if there is one —
/// `tf-m30` says more about what the account is doing than `default` does. Names
/// that fail to resolve are skipped here and reported as errors by the caller.
/// Pure (no keychain, no network) and unit-tested.
fn group_accounts_by_id(
    env: OandaEnvironment,
    env_cfg: &vault_store::EnvConfig,
) -> Vec<(String, Vec<String>)> {
    let mut by_id: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for name in env_cfg.account_names() {
        if let Ok(id) = env_cfg.account_id_for(env, &name) {
            by_id.entry(id).or_default().push(name);
        }
    }
    let mut groups: Vec<(String, Vec<String>)> = by_id
        .into_values()
        .map(|mut names| {
            // Primary first, the rest in stable order behind it.
            if let Some(pos) = names.iter().position(|n| n != DEFAULT_ACCOUNT) {
                names.swap(0, pos);
                names[1..].sort();
            }
            (names[0].clone(), names)
        })
        .collect();
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    groups
}

/// Build one account's glance row from its OANDA snapshot and the trades closed
/// inside the window. Pure (no network) so the realized sum, win/loss tally, and
/// win rate are unit-tested directly.
///
/// Scratch trades (exactly zero realized P&L) count toward `trades` but are
/// excluded from the win-rate denominator — a break-even fill is neither a win
/// nor a loss, and folding it into either skews a low-count window.
fn build_glance_row(
    primary: &str,
    names: &[String],
    account_id: &str,
    snap: &AccountSnapshot,
    closed: &[Trade],
) -> serde_json::Value {
    let realized: Decimal = closed.iter().map(|t| t.realized_pl).sum();
    let wins = closed.iter().filter(|t| t.realized_pl > Decimal::ZERO).count();
    let losses = closed.iter().filter(|t| t.realized_pl < Decimal::ZERO).count();
    let decided = wins + losses;

    serde_json::json!({
        "account": primary,
        "names": names,
        "account_id": account_id,
        "currency": snap.currency,
        "nav": dec_str(snap.nav),
        "balance": dec_str(snap.balance),
        "unrealized_pl": dec_str(snap.unrealized_pl),
        "open_trade_count": snap.open_trade_count,
        "realized": dec_str(realized),
        "trades": closed.len(),
        "wins": wins,
        "losses": losses,
        // Null (not 0) when nothing decided in the window — the UI must render
        // "—", not a misleading 0%.
        "win_rate": if decided > 0 {
            serde_json::json!((wins as f64 / decided as f64 * 1000.0).round() / 1000.0)
        } else {
            serde_json::Value::Null
        },
        "error": serde_json::Value::Null,
    })
}

/// `wickd trade glance [--days N]`: a one-line performance summary for every
/// account configured in `--env`, over a rolling window ending now.
///
/// Deliberately different from `report`: no baseline is required (the window is
/// fixed and recent, not since-inception), and it spans all accounts rather than
/// one. That makes it the cheap "how is the whole ladder doing" surface the
/// desktop dashboard polls.
///
/// A per-account failure (revoked key, OANDA 5xx) is captured *into that row's
/// `error` field* rather than failing the command — one bad account must not
/// blank the whole panel.
async fn glance(env: OandaEnvironment, env_raw: &str, g: GlanceArgs) -> Result<serde_json::Value> {
    let cfg = vault_store::load()?;
    let env_cfg = match env {
        OandaEnvironment::Practice => cfg.practice,
        OandaEnvironment::Live => cfg.live,
    }
    .unwrap_or_default();

    let groups = group_accounts_by_id(env, &env_cfg);
    if groups.is_empty() {
        bail!(
            "no {env} credentials stored — run `wickd login --env {env}`",
            env = env_str(env)
        );
    }

    let now = Utc::now();
    let since = glance_window(now, g.days, g.since.as_deref())?;

    // Resolve credentials up front and serially: keychain reads are local and
    // fast, and keeping them off the worker threads avoids concurrent access to
    // the same keychain item. Only the network fetches fan out.
    let mut resolved = Vec::new();
    let mut rows: Vec<serde_json::Value> = Vec::new();
    for (primary, names) in groups {
        match client::resolve(env_raw, &primary) {
            Ok((_, oanda)) => resolved.push((primary, names, oanda)),
            Err(e) => rows.push(serde_json::json!({
                "account": primary,
                "names": names,
                "error": e.to_string(),
            })),
        }
    }

    let limit = g.limit;
    let mut set = tokio::task::JoinSet::new();
    for (primary, names, oanda) in resolved {
        set.spawn(async move {
            let account_id = oanda.account_id().to_string();
            // The two fetches are independent — run them concurrently so an
            // account costs one round trip of latency, not two. With the
            // per-account fan-out above, the whole glance is ~one round trip.
            let fetched = async {
                let (acct, closed) = tokio::join!(
                    endpoints::get_account(&oanda),
                    endpoints::get_trade_history(&oanda, Some(limit), None),
                );
                Ok::<_, anyhow::Error>((
                    acct.context("OANDA account fetch failed")?,
                    closed.context("OANDA closed-trade history fetch failed")?,
                ))
            }
            .await;

            match fetched {
                Ok((acct, closed)) => {
                    let snap = AccountSnapshot::from_oanda(&acct);
                    let closed = closed_since(closed, since);
                    build_glance_row(&primary, &names, &account_id, &snap, &closed)
                }
                Err(e) => serde_json::json!({
                    "account": primary,
                    "names": names,
                    "account_id": account_id,
                    "error": format!("{e:#}"),
                }),
            }
        });
    }
    while let Some(res) = set.join_next().await {
        rows.push(res.context("account fetch task panicked")?);
    }

    // Stable ordering regardless of which fetch finished first.
    rows.sort_by(|a, b| {
        a.get("account")
            .and_then(|v| v.as_str())
            .cmp(&b.get("account").and_then(|v| v.as_str()))
    });

    Ok(serde_json::json!({
        "environment": env_str(env),
        // Null when --since drove the window: reporting the unused --days
        // default alongside an explicit instant would misdescribe the result.
        // `since` below is always the authoritative window start.
        "days": if g.since.is_some() { serde_json::Value::Null } else { g.days.into() },
        "since": since.to_rfc3339(),
        "generated_at": now.to_rfc3339(),
        "count": rows.len(),
        "accounts": rows,
    }))
}

/// `wickd trade baseline …` (AGT-631, AC1): record or inspect an account's
/// performance baseline.
async fn baseline_cmd(
    env: OandaEnvironment,
    env_raw: &str,
    account: &str,
    b: BaselineArgs,
) -> Result<serde_json::Value> {
    match b.cmd {
        BaselineCmd::Set(s) => baseline_set(env, env_raw, account, s).await,
        BaselineCmd::Show => {
            let conn = baseline::open()?;
            let latest = baseline::latest(&conn, account)?;
            Ok(serde_json::json!({
                "account": account,
                "baseline": latest.map(|b| b.to_json()),
            }))
        }
        BaselineCmd::History => {
            let conn = baseline::open()?;
            let hist = baseline::history(&conn, account)?;
            Ok(serde_json::json!({
                "account": account,
                "count": hist.len(),
                "baselines": hist.iter().map(|b| b.to_json()).collect::<Vec<_>>(),
            }))
        }
    }
}

/// Record a new baseline for `account`. Supersedes the prior one (kept in
/// history). With no `--balance`, fetches the account's current OANDA balance.
async fn baseline_set(
    env: OandaEnvironment,
    env_raw: &str,
    account: &str,
    s: BaselineSetArgs,
) -> Result<serde_json::Value> {
    // Same name rule as login/keychain keys — never seed a baseline under a
    // malformed account name.
    crate::vault_store::validate_account_name(account)?;

    let baseline_date = match &s.date {
        Some(d) => parse_baseline_date(d)?.to_rfc3339(),
        None => Utc::now().to_rfc3339(),
    };

    // Explicit --balance is the offline path; otherwise fetch the account's
    // current balance (and currency) from OANDA.
    let (balance, currency) = match s.balance {
        Some(bal) => (bal, s.currency.or_else(|| Some("USD".to_string()))),
        None => {
            let (_, oanda) = client::resolve(env_raw, account)?;
            let acct = endpoints::get_account(&oanda)
                .await
                .context("OANDA account fetch failed (needed to seed the baseline balance)")?;
            (acct.balance.clone(), Some(acct.currency.clone()))
        }
    };

    let stored = baseline::record_at(
        baseline::baseline_path()?,
        baseline::Baseline::new(
            account,
            Some(env_str(env).to_string()),
            &balance,
            currency,
            baseline_date,
        ),
    )?;

    Ok(serde_json::json!({
        "ok": true,
        "recorded": stored.to_json(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wickd_core::models::TradeState;
    use rust_decimal_macros::dec;

    // ── AGT-631 report/baseline helpers + tests ────────────────────────────

    fn dt(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn closed_trade(
        id: &str,
        instrument: &str,
        units: Decimal,
        realized: Decimal,
        close: &str,
        strategy: Option<&str>,
    ) -> Trade {
        Trade {
            id: id.to_string(),
            instrument: instrument.to_string(),
            open_price: dec!(1.0),
            open_time: dt("2026-07-01T00:00:00Z"),
            units,
            realized_pl: realized,
            unrealized_pl: None,
            state: TradeState::Closed,
            close_time: Some(dt(close)),
            close_price: Some(dec!(1.0)),
            strategy: strategy.map(|s| s.to_string()),
        }
    }

    // ── trade glance (multi-account rolling window) ────────────────────────

    fn snap_10k() -> AccountSnapshot {
        AccountSnapshot {
            balance: dec!(10000),
            nav: dec!(10012),
            unrealized_pl: dec!(12),
            currency: "USD".into(),
            open_trade_count: 1,
        }
    }

    fn env_cfg(default_id: Option<&str>, named: &[(&str, &str)]) -> vault_store::EnvConfig {
        vault_store::EnvConfig {
            account_id: default_id.map(|s| s.to_string()),
            accounts: named
                .iter()
                .map(|(n, id)| {
                    (
                        n.to_string(),
                        vault_store::AccountConfig { account_id: id.to_string() },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn glance_row_sums_realized_and_tallies_wins() {
        let closed = vec![
            closed_trade("1", "EUR_USD", dec!(100), dec!(40), "2026-07-19T00:00:00Z", None),
            closed_trade("2", "EUR_USD", dec!(100), dec!(-10), "2026-07-18T00:00:00Z", None),
            closed_trade("3", "GBP_USD", dec!(100), dec!(17.2), "2026-07-17T00:00:00Z", None),
        ];
        let row = build_glance_row("h004", &["h004".into()], "101-1", &snap_10k(), &closed);

        assert_eq!(row["realized"], "47.2");
        assert_eq!(row["trades"], 3);
        assert_eq!(row["wins"], 2);
        assert_eq!(row["losses"], 1);
        assert_eq!(row["win_rate"], 0.667);
        assert_eq!(row["nav"], "10012");
        assert_eq!(row["unrealized_pl"], "12");
        assert!(row["error"].is_null());
    }

    #[test]
    fn glance_row_scratch_trades_are_neither_win_nor_loss() {
        let closed = vec![
            closed_trade("1", "EUR_USD", dec!(100), dec!(5), "2026-07-19T00:00:00Z", None),
            closed_trade("2", "EUR_USD", dec!(100), dec!(0), "2026-07-18T00:00:00Z", None),
        ];
        let row = build_glance_row("h004", &["h004".into()], "101-1", &snap_10k(), &closed);

        // Counted as a trade, excluded from the win-rate denominator: 1/1, not 1/2.
        assert_eq!(row["trades"], 2);
        assert_eq!(row["wins"], 1);
        assert_eq!(row["losses"], 0);
        assert_eq!(row["win_rate"], 1.0);
    }

    #[test]
    fn glance_row_empty_window_has_null_win_rate() {
        let row = build_glance_row("tf-h1", &["tf-h1".into()], "101-6", &snap_10k(), &[]);

        assert_eq!(row["trades"], 0);
        assert_eq!(row["realized"], "0");
        // Null, never 0 — the UI must render "—" for "nothing decided yet".
        assert!(row["win_rate"].is_null());
    }

    #[test]
    fn glance_window_defaults_to_days_back_from_now() {
        let now = dt("2026-07-20T15:00:00Z");
        let since = glance_window(now, 7, None).unwrap();

        assert_eq!(since, dt("2026-07-13T15:00:00Z"));
    }

    #[test]
    fn glance_window_since_overrides_days() {
        let now = dt("2026-07-20T15:00:00Z");
        // The app's "Today" view: local midnight, which is NOT a whole number
        // of days back from now and cannot be expressed with --days at all.
        let since = glance_window(now, 7, Some("2026-07-20T06:00:00Z")).unwrap();

        assert_eq!(since, dt("2026-07-20T06:00:00Z"));
    }

    #[test]
    fn glance_window_accepts_a_bare_iso_date() {
        let now = dt("2026-07-20T15:00:00Z");
        let since = glance_window(now, 7, Some("2026-07-20")).unwrap();

        assert_eq!(since, dt("2026-07-20T00:00:00Z"));
    }

    #[test]
    fn glance_window_rejects_a_malformed_since() {
        let now = dt("2026-07-20T15:00:00Z");
        let err = glance_window(now, 7, Some("yesterday")).unwrap_err();

        // Must not silently fall back to --days: a bad window would quietly
        // report the wrong period's P&L as if it were the one asked for.
        assert!(format!("{err:#}").contains("--since"), "unhelpful error: {err:#}");
    }

    #[test]
    fn glance_groups_aliases_of_the_same_oanda_account() {
        // Matt's real practice shape: `default` (v1 slot) and `tf-m30` both
        // resolve to …-005, so they are one broker account under two names.
        let cfg = env_cfg(
            Some("101-005"),
            &[("tf-m30", "101-005"), ("h004", "101-001")],
        );
        let groups = group_accounts_by_id(OandaEnvironment::Practice, &cfg);

        assert_eq!(groups.len(), 2, "…-005 must be fetched once, not twice");
        let m30 = groups.iter().find(|(p, _)| p == "tf-m30").expect("tf-m30 group");
        // The informative name wins the row label over the generic `default`.
        assert_eq!(m30.1, vec!["tf-m30".to_string(), "default".to_string()]);
        assert!(groups.iter().any(|(p, names)| p == "h004" && names == &["h004".to_string()]));
    }

    #[test]
    fn glance_groups_keep_default_when_it_is_the_only_name() {
        let cfg = env_cfg(Some("101-005"), &[]);
        let groups = group_accounts_by_id(OandaEnvironment::Practice, &cfg);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, "default");
    }

    fn baseline_10k() -> baseline::Baseline {
        baseline::Baseline {
            id: 1,
            account: "h004".into(),
            environment: Some("practice".into()),
            // Scale 0 so computed decimal strings stay trailing-zero-free in
            // asserts; production baselines keep OANDA's exact scale verbatim.
            balance: "10000".into(),
            currency: Some("USD".into()),
            baseline_date: "2026-07-05T00:00:00Z".into(),
            recorded_at: "2026-07-05T12:00:00Z".into(),
        }
    }

    #[test]
    fn parse_baseline_date_accepts_iso_date_and_rfc3339() {
        assert_eq!(
            parse_baseline_date("2026-07-05").unwrap(),
            dt("2026-07-05T00:00:00Z")
        );
        assert_eq!(
            parse_baseline_date("2026-07-05T14:30:00Z").unwrap(),
            dt("2026-07-05T14:30:00Z")
        );
        assert!(parse_baseline_date("nonsense").is_err());
    }

    #[test]
    fn closed_since_filters_and_orders_newest_first() {
        let since = dt("2026-07-05T00:00:00Z");
        let trades = vec![
            // before the baseline → excluded
            closed_trade("1", "EUR_USD", dec!(1000), dec!(5), "2026-07-04T00:00:00Z", None),
            closed_trade("2", "EUR_USD", dec!(1000), dec!(10), "2026-07-06T00:00:00Z", None),
            closed_trade("3", "GBP_USD", dec!(-500), dec!(-3), "2026-07-08T00:00:00Z", None),
        ];
        let out = closed_since(trades, since);
        // Only the two on/after the baseline survive, newest first.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, "3");
        assert_eq!(out[1].id, "2");
    }

    #[test]
    fn build_report_computes_pl_nav_and_reconciles() {
        // AC2/AC3: realized = sum of closed realized_pl; nav_vs_baseline =
        // nav − baseline; reconstructed nav = baseline + realized + unrealized,
        // so with financing-free synthetic data the residual is exactly zero.
        let base = baseline_10k();
        let snap = AccountSnapshot {
            balance: dec!(10015),
            nav: dec!(10020),
            unrealized_pl: dec!(5),
            currency: "USD".into(),
            open_trade_count: 1,
        };
        // Newest-first, as `closed_since` hands them to `build_report`.
        let closed = vec![
            closed_trade("3", "GBP_USD", dec!(-500), dec!(5), "2026-07-08T00:00:00Z", Some("h004-surprise")),
            closed_trade("2", "EUR_USD", dec!(1000), dec!(10), "2026-07-06T00:00:00Z", Some("h004-surprise")),
        ];
        let v = build_report("h004", &base, &snap, &closed);

        assert_eq!(v["account"], "h004");
        assert_eq!(v["realized_pl_since_baseline"], "15");
        assert_eq!(v["unrealized_pl"], "5");
        assert_eq!(v["nav"], "10020");
        assert_eq!(v["nav_vs_baseline"], "20");
        assert_eq!(v["closed_trade_count"], 2);
        // baseline 10000 + realized 15 + unrealized 5 = 10020 = nav → residual 0.
        assert_eq!(v["reconciliation"]["reconstructed_nav"], "10020");
        assert_eq!(v["reconciliation"]["residual"], "0");
        // Per-strategy attribution rolls both trades under the tag.
        assert_eq!(v["by_strategy"]["h004-surprise"]["realized_pl"], "15");
        assert_eq!(v["by_strategy"]["h004-surprise"]["trades"], 2);
        // Closed-trade list carries the required fields.
        let first = &v["closed_trades"][0];
        assert_eq!(first["id"], "3");
        assert_eq!(first["instrument"], "GBP_USD");
        assert_eq!(first["units"], "-500");
        assert_eq!(first["strategy"], "h004-surprise");
        assert!(first["open_time"].is_string() && first["close_time"].is_string());
    }

    #[test]
    fn build_report_buckets_unattributed_and_flags_residual() {
        // A manual trade with no strategy lands in "unattributed"; a nonzero
        // residual (financing/fees or pre-baseline funds) is surfaced, not hidden.
        let base = baseline_10k();
        let snap = AccountSnapshot {
            balance: dec!(10000),
            nav: dec!(10008), // 3 more than baseline+realized+unrealized (financing)
            unrealized_pl: dec!(0),
            currency: "USD".into(),
            open_trade_count: 0,
        };
        let closed = vec![closed_trade(
            "9", "EUR_USD", dec!(1000), dec!(5), "2026-07-06T00:00:00Z", None,
        )];
        let v = build_report("h004", &base, &snap, &closed);
        assert_eq!(v["by_strategy"]["unattributed"]["trades"], 1);
        assert_eq!(v["by_strategy"]["unattributed"]["realized_pl"], "5");
        // reconstructed = 10000 + 5 + 0 = 10005; residual = 10008 − 10005 = 3.
        assert_eq!(v["reconciliation"]["reconstructed_nav"], "10005");
        assert_eq!(v["reconciliation"]["residual"], "3");
    }

    fn place_args(units: i64, live: bool) -> PlaceArgs {
        PlaceArgs {
            instrument: "EUR_USD".to_string(),
            units,
            order_type: EntryKind::Market,
            price: None,
            tif: None,
            gtd_time: None,
            price_bound: None,
            trigger: None,
            sl: Some("1.0850".to_string()),
            tp: Some("1.0950".to_string()),
            strategy: None,
            live,
            yes: false,
            auto: false,
        }
    }

    #[test]
    fn default_is_paper_live_flag_arms() {
        assert_eq!(execution_mode(false), Mode::Paper);
        assert_eq!(execution_mode(true), Mode::Live);
    }

    #[test]
    fn paper_order_payload_is_a_dry_run() {
        // Default (no --live) builds the would-be order and never submits.
        let p = place_args(1000, false);
        assert_eq!(execution_mode(p.live), Mode::Paper);
        let plan = EntryPlan::market(p.sl.clone(), p.tp.clone());
        let v = build_paper_order(OandaEnvironment::Practice, &p.instrument, p.units, &plan);
        assert_eq!(v["mode"], "paper");
        assert_eq!(v["submitted"], false);
        assert_eq!(v["instrument"], "EUR_USD");
        assert_eq!(v["units"], 1000);
        assert_eq!(v["side"], "long");
        assert_eq!(v["sl"], "1.0850");
        assert_eq!(v["tp"], "1.0950");
        assert_eq!(v["environment"], "practice");
    }

    #[test]
    fn paper_order_infers_short_from_negative_units() {
        let p = place_args(-500, false);
        let plan = EntryPlan::market(p.sl.clone(), p.tp.clone());
        let v = build_paper_order(OandaEnvironment::Live, &p.instrument, p.units, &plan);
        assert_eq!(v["side"], "short");
        assert_eq!(v["units"], -500);
        // --env live without --live is still paper, targeting the live account.
        assert_eq!(v["environment"], "live");
        assert_eq!(v["submitted"], false);
    }

    #[test]
    fn live_flag_selects_live_mode() {
        let p = place_args(1000, true);
        assert_eq!(execution_mode(p.live), Mode::Live);
    }

    #[test]
    fn paper_close_is_a_dry_run() {
        let c = CloseArgs {
            instrument: "EUR_USD".to_string(),
            side: "long".to_string(),
            live: false,
            yes: false,
            auto: false,
        };
        assert_eq!(execution_mode(c.live), Mode::Paper);
        let v = build_paper_close(OandaEnvironment::Practice, &c.instrument, &c.side);
        assert_eq!(v["mode"], "paper");
        assert_eq!(v["submitted"], false);
        assert_eq!(v["instrument"], "EUR_USD");
        assert_eq!(v["side"], "long");
    }

    // AGT-613 rewrites the AGT-611 arming contract: a live submit now requires a
    // human keystroke on an interactive TTY. `--yes` and piped input can no
    // longer arm it, and a non-interactive context fails closed.

    #[test]
    fn live_arming_fails_closed_without_a_tty_even_with_yes() {
        // AC2: the real test harness has no TTY, so `confirm_live` must refuse —
        // regardless of which account the live order targets.
        assert!(confirm_live(OandaEnvironment::Practice, false, "order").is_err());
        assert!(confirm_live(OandaEnvironment::Live, false, "order").is_err());
        // AC1: `--yes` (yes == true) does NOT arm a live order in a non-TTY
        // context — the old "--yes arms it" short-circuit is gone.
        assert!(confirm_live(OandaEnvironment::Practice, true, "order").is_err());
        assert!(confirm_live(OandaEnvironment::Live, true, "order").is_err());
    }

    #[test]
    fn live_arming_gate_encodes_ac1_ac2_ac3() {
        // AC2: no TTY → fail closed, and the answer reader is NEVER consulted,
        // so a piped/redirected answer can never satisfy the confirm.
        let mut read_attempted = false;
        let res = confirm_live_gate(false, "order", || {
            read_attempted = true;
            Ok("yes".to_string())
        });
        assert!(res.is_err(), "no TTY must fail closed");
        assert!(
            !read_attempted,
            "non-interactive path must not read any answer (no piped fallback)"
        );

        // AC3: with a TTY, an affirmative `yes` keystroke proceeds...
        assert!(confirm_live_gate(true, "order", || Ok("yes".to_string())).is_ok());
        assert!(confirm_live_gate(true, "order", || Ok("  yes  ".to_string())).is_ok());
        // ...and anything else (or a decline) does not.
        assert!(confirm_live_gate(true, "order", || Ok("no".to_string())).is_err());
        assert!(confirm_live_gate(true, "order", || Ok(String::new())).is_err());
        assert!(confirm_live_gate(true, "order", || Ok("YES".to_string())).is_err());
    }

    // ------------------------------------------------------------------------
    // AGT-626: non-interactive auto arming — PRACTICE ONLY. The auto gate must
    // permit a practice submit without a TTY and FAIL CLOSED for the live env,
    // while the interactive (AGT-613) gate stays byte-for-byte unchanged.
    // ------------------------------------------------------------------------

    // AC1: the auto-arming gate permits the PRACTICE env (no TTY needed).
    // AC2: it FAILS CLOSED for the LIVE env — autonomy can never arm live.
    #[test]
    fn auto_arm_is_practice_only() {
        assert!(arm_auto_practice(OandaEnvironment::Practice, "order").is_ok());
        assert!(arm_auto_practice(OandaEnvironment::Practice, "position close").is_ok());

        let live_err = arm_auto_practice(OandaEnvironment::Live, "order").unwrap_err();
        let msg = format!("{live_err:#}");
        assert!(msg.contains("practice environment only"));
        assert!(msg.contains("--auto cannot arm live"));
        assert!(arm_auto_practice(OandaEnvironment::Live, "position close").is_err());
    }

    // The arming dispatcher wires the two modes correctly:
    //  - AutoPractice: practice ok, live fails closed (AC1/AC2).
    //  - Interactive: fails closed in this (no-TTY) test harness regardless of
    //    env, i.e. the AGT-613 contract is untouched by AGT-626 (AC2).
    #[test]
    fn arm_live_dispatches_interactive_vs_auto() {
        // Auto arming: practice proceeds, live refuses.
        assert!(arm_live(OandaEnvironment::Practice, Arming::AutoPractice, "order").is_ok());
        assert!(arm_live(OandaEnvironment::Live, Arming::AutoPractice, "order").is_err());

        // Interactive arming: no TTY in the harness → fail closed for BOTH envs,
        // and --yes cannot rescue it. Live behavior is byte-for-byte unchanged.
        assert!(
            arm_live(OandaEnvironment::Practice, Arming::Interactive { yes: false }, "order")
                .is_err()
        );
        assert!(
            arm_live(OandaEnvironment::Live, Arming::Interactive { yes: true }, "order").is_err()
        );
    }

    fn temp_audit_db() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let mut p = std::env::temp_dir();
        p.push(format!("wickd-trade-test-audit-{pid}-{nanos}-{n}.db"));
        p
    }

    // AGT-610 (AC2): a fill of fewer units than requested is a PARTIAL fill,
    // not a full one — this is the pure comparison `execute_place` uses to
    // label both the audit row and the JSON response. Before the fix, any
    // `order_fill_transaction` at all was unconditionally labeled "filled".
    #[test]
    fn fill_outcome_labels_partial_vs_full() {
        // Exact match (either side) -> filled.
        assert_eq!(fill_outcome("1000", 1000), "filled");
        assert_eq!(fill_outcome("-1000", -1000), "filled");
        // Fewer units filled than requested -> partial, NOT filled.
        assert_eq!(fill_outcome("600", 1000), "partial");
        assert_eq!(fill_outcome("-250", -1000), "partial");
        // Fail safe: an unparseable units string must never silently report a
        // full fill.
        assert_eq!(fill_outcome("not-a-number", 1000), "partial");
    }

    // ------------------------------------------------------------------------
    // AGT-612: 4-outcome entry-order classifier + limit/stop plumbing.
    // ------------------------------------------------------------------------

    fn response_from(json: serde_json::Value) -> OrderCreateResponse {
        serde_json::from_value(json).expect("synthetic OANDA response deserializes")
    }

    // AC3: an orderFillTransaction whose units MATCH the request → Filled — and
    // it wins even when an orderCreateTransaction is also present (immediate
    // fill of a limit/stop).
    #[test]
    fn classify_entry_filled() {
        let resp = response_from(serde_json::json!({
            "orderCreateTransaction": {"id":"1","time":"t","type":"LIMIT_ORDER","instrument":"EUR_USD","units":"1000","timeInForce":"GTC","positionFill":"DEFAULT"},
            "orderFillTransaction": {"id":"2","time":"t","type":"ORDER_FILL","instrument":"EUR_USD","units":"1000","price":"1.08500"},
            "lastTransactionID":"2"
        }));
        assert_eq!(classify_entry(&resp, 1000), EntryOutcome::Filled);
        assert_eq!(classify_entry(&resp, 1000).as_str(), "filled");
    }

    // AC3: an orderFillTransaction that filled FEWER units than requested →
    // Partial (never a full Filled).
    #[test]
    fn classify_entry_partial() {
        let resp = response_from(serde_json::json!({
            "orderFillTransaction": {"id":"2","time":"t","type":"ORDER_FILL","instrument":"EUR_USD","units":"600","price":"1.08500"},
            "lastTransactionID":"2"
        }));
        assert_eq!(classify_entry(&resp, 1000), EntryOutcome::Partial);
        assert_eq!(classify_entry(&resp, 1000).as_str(), "partial");
    }

    // AC3: only an orderCreateTransaction (no fill/cancel/reject) → Rested. This
    // is the resting limit/stop shape whose signal AC5 must consume.
    #[test]
    fn classify_entry_rested() {
        let resp = response_from(serde_json::json!({
            "orderCreateTransaction": {"id":"1","time":"t","type":"LIMIT_ORDER","instrument":"EUR_USD","units":"1000","timeInForce":"GTC","positionFill":"DEFAULT"},
            "lastTransactionID":"1"
        }));
        assert_eq!(classify_entry(&resp, 1000), EntryOutcome::Rested);
        assert_eq!(classify_entry(&resp, 1000).as_str(), "resting");
    }

    // AC3: an orderCancelTransaction (created then cancelled) → Rejected.
    #[test]
    fn classify_entry_rejected_via_cancel() {
        let resp = response_from(serde_json::json!({
            "orderCreateTransaction": {"id":"1","time":"t","type":"MARKET_ORDER","instrument":"EUR_USD","units":"1000","timeInForce":"FOK","positionFill":"DEFAULT"},
            "orderCancelTransaction": {"id":"2","time":"t","type":"ORDER_CANCEL","orderID":"1","reason":"MARKET_HALTED"},
            "lastTransactionID":"2"
        }));
        assert_eq!(classify_entry(&resp, 1000), EntryOutcome::Rejected);
        assert_eq!(rejection_reason(&resp), "MARKET_HALTED");
    }

    // AC3: a hard orderRejectTransaction (OANDA refused to create the order) →
    // Rejected, and the reject reason is surfaced.
    #[test]
    fn classify_entry_rejected_via_hard_reject() {
        let resp = response_from(serde_json::json!({
            "orderRejectTransaction": {"id":"9","time":"t","type":"LIMIT_ORDER_REJECT","rejectReason":"PRICE_PRECISION_EXCEEDED"},
            "lastTransactionID":"9"
        }));
        assert_eq!(classify_entry(&resp, 1000), EntryOutcome::Rejected);
        assert_eq!(rejection_reason(&resp), "PRICE_PRECISION_EXCEEDED");
    }

    // AC3 fail-safe: a response with no recognizable transaction is classified
    // Rejected — never Rested (which would consume a pending signal, AC5) or
    // Filled.
    #[test]
    fn classify_entry_anomalous_response_is_rejected() {
        let resp = response_from(serde_json::json!({ "lastTransactionID": "0" }));
        assert_eq!(classify_entry(&resp, 1000), EntryOutcome::Rejected);
    }

    // AC1/AC2: a limit/stop entry with no --price is rejected UP FRONT with a
    // validation-routed error, before any arming/audit/network. Runs with
    // live=true to prove the price check precedes even the TTY arming gate.
    #[tokio::test]
    async fn limit_without_price_is_a_validation_error() {
        let plan = EntryPlan {
            kind: EntryKind::Limit,
            price: None,
            tif: None,
            gtd_time: None,
            price_bound: None,
            trigger: None,
            sl: None,
            tp: None,
            strategy: None,
        };
        let err = execute_place(
            OandaEnvironment::Practice,
            crate::vault_store::DEFAULT_ACCOUNT,
            "EUR_USD",
            1000,
            plan,
            true,
            true,
        )
            .await
            .expect_err("a limit entry with no price must fail");
        let msg = format!("{err:#}");
        assert!(msg.contains("requires --price"));
        assert_eq!(execution_exit_code(&msg), crate::output::exit::VALIDATION);
    }

    // AC2: the paper (dry-run) payload reflects the limit kind + trigger price,
    // so a dry run shows exactly what a live run would POST.
    #[test]
    fn paper_limit_order_reflects_kind_and_price() {
        let plan = EntryPlan {
            kind: EntryKind::Limit,
            price: Some("1.07500".to_string()),
            tif: Some(TimeInForce::GTC),
            gtd_time: None,
            price_bound: None,
            trigger: None,
            sl: None,
            tp: None,
            strategy: None,
        };
        let v = build_paper_order(OandaEnvironment::Practice, "EUR_USD", 1000, &plan);
        assert_eq!(v["mode"], "paper");
        assert_eq!(v["submitted"], false);
        assert_eq!(v["type"], "limit");
        assert_eq!(v["price"], "1.07500");
        assert_eq!(v["tif"], "GTC");
    }

    // AGT-610 (AC1) regression: a hard reject from the OANDA submit call
    // itself (e.g. HTTP 400 with an errorMessage body) must still land a
    // terminal audit row. Before this fix, `execute_place`'s `?` on the
    // submit call propagated the error immediately and NOTHING further was
    // ever written — the pre-submit "attempt" row (AGT-596, still fatal and
    // unchanged here) was left stuck forever with no outcome/reason, since
    // audit rows are append-only and never updated in place. This test drives
    // the real `record_required_at`/`record_submit_terminal_error` write path
    // against a throwaway db and proves a second, terminal row now exists.
    #[test]
    fn hard_submit_error_still_writes_a_terminal_audit_row() {
        let path = temp_audit_db();

        // Seed the pre-submit "attempt" row exactly as `execute_place` does
        // via `audit::record_required` before ever calling OANDA.
        audit::record_required_at(
            &path,
            &audit::AuditEntry::now("place", Mode::Live.as_str(), "attempt")
                .env("practice")
                .instrument("EUR_USD")
                .units(1000),
        )
        .unwrap();

        let err = anyhow!("OANDA order placement failed: INSUFFICIENT_MARGIN");
        record_submit_terminal_error(
            &path,
            "place",
            OandaEnvironment::Practice,
            "EUR_USD",
            Some(1000),
            &None,
            &None,
            &Some("ma-crossover".to_string()),
            &err,
        );

        let conn = audit::open_at(&path).unwrap();
        let rows = audit::query(&conn, 10).unwrap();
        assert_eq!(rows.len(), 2, "expected the attempt row plus a terminal row");
        // Newest first: the terminal row must not be stuck at "attempt".
        assert_eq!(rows[0]["outcome"], "error");
        assert_ne!(rows[0]["outcome"], "attempt");
        assert!(rows[0]["detail"]
            .as_str()
            .unwrap()
            .contains("INSUFFICIENT_MARGIN"));
        // AGT-630 (AC2): the terminal error row keeps the strategy attribution.
        assert_eq!(rows[0]["strategy"], "ma-crossover");
        assert_eq!(rows[1]["outcome"], "attempt");

        let _ = std::fs::remove_file(&path);
    }

    // -----------------------------------------------------------------------
    // AGT-611: live market-order path verification (offline-automatable ACs).
    //
    // These tests drive the REAL `execute_place` path against a throwaway
    // `$HOME`, so they never touch the operator's real `~/.wickd/`. Key lever:
    // with no `config.json` under the temp HOME, `client::resolve` fails fast
    // with a "no ... credentials stored" error BEFORE any keychain read or
    // network call. That lets us prove the ordering of the guarded live
    // sequence (audit-write BEFORE OANDA submit) fully offline, with no OANDA
    // account. AC4's live fill, AC5's live max-open breach, and AC6's live
    // close/round-trip need a real practice account and are covered by the
    // human-run steps in `docs/d3-live-order-checklist.md`.
    // -----------------------------------------------------------------------

    /// Serializes the tests that override the process-global `$HOME`. No other
    /// test in this crate reads a `$HOME`-derived path, so this lock only needs
    /// to guard the HOME-overriding tests here against each other.
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A scoped `$HOME` override pointing at a fresh temp dir. Restores the
    /// previous value (and releases the lock) on drop, and `chmod`s the
    /// `.wickd` dir back to writable first so the temp tree can always be
    /// cleaned up even after the read-only (AC7) test.
    struct TempHome {
        _guard: std::sync::MutexGuard<'static, ()>,
        prev: Option<std::ffi::OsString>,
        dir: std::path::PathBuf,
    }

    impl TempHome {
        fn new() -> Self {
            let guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let pid = std::process::id();
            let mut dir = std::env::temp_dir();
            dir.push(format!("wickd-agt611-home-{pid}-{nanos}"));
            std::fs::create_dir_all(&dir).unwrap();
            let prev = std::env::var_os("HOME");
            std::env::set_var("HOME", &dir);
            Self { _guard: guard, prev, dir }
        }

        fn wickd_dir(&self) -> std::path::PathBuf {
            self.dir.join(".wickd")
        }

        fn audit_db(&self) -> std::path::PathBuf {
            self.wickd_dir().join("audit.db")
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let wickd = self.wickd_dir();
                if wickd.exists() {
                    let _ =
                        std::fs::set_permissions(&wickd, std::fs::Permissions::from_mode(0o700));
                }
            }
            match &self.prev {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// AC2: `trade place` WITHOUT `--live` emits `submitted:false` and makes no
    /// network call. Driven through the real `execute_place` against a temp
    /// `$HOME` with NO credentials configured — the paper path returns Ok
    /// without ever calling `client::resolve`, so the *absence* of any
    /// credential/network error is itself proof no OANDA call was attempted.
    #[tokio::test]
    async fn ac2_paper_place_submits_nothing_and_needs_no_network() {
        let home = TempHome::new();
        let v = execute_place(
            OandaEnvironment::Practice,
            crate::vault_store::DEFAULT_ACCOUNT,
            "EUR_USD",
            1000,
            EntryPlan::market(Some("1.0850".into()), Some("1.0950".into())),
            false, // no --live
            false,
        )
        .await
        .expect("paper place must succeed with no creds and no network");
        assert_eq!(v["mode"], "paper");
        assert_eq!(v["submitted"], false);
        assert_eq!(v["instrument"], "EUR_USD");
        // The paper decision was recorded to the (temp) audit store as
        // `not_submitted` — never `attempt`, which would imply the live path.
        let conn = audit::open_at(home.audit_db()).unwrap();
        let rows = audit::query(&conn, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["mode"], "paper");
        assert_eq!(rows[0]["outcome"], "not_submitted");
        drop(home);
    }

    /// AC3: an oversize order is rejected by the size cap BEFORE any OANDA
    /// network call. The size cap lives in the *pure* `risk::enforce_pre_trade`
    /// guard, which `enforce_live_place` runs before it ever fetches positions
    /// or submits — so the rejection is provably pre-network (no I/O at all),
    /// and its message routes to `exit::VALIDATION`.
    #[test]
    fn ac3_oversize_rejected_pre_network_and_routes_to_validation() {
        let caps = crate::risk::RiskCaps {
            max_position_size: Some(500),
            max_open_positions: None,
            daily_loss_limit: None,
        };
        // Pure guard: no OANDA client, no network — a 1000-unit order is
        // rejected outright against a 500 cap (magnitude also catches shorts).
        let rej = crate::risk::enforce_pre_trade(&caps, 1000, 0, rust_decimal::Decimal::ZERO)
            .unwrap_err();
        let msg = rej.message();
        assert!(msg.contains("risk cap"));
        assert!(msg.contains("exceeds the max position size"));
        assert!(crate::risk::enforce_pre_trade(&caps, -1000, 0, rust_decimal::Decimal::ZERO)
            .is_err());
        // Exactly at the cap is allowed (strict >).
        assert!(crate::risk::enforce_pre_trade(&caps, 500, 0, rust_decimal::Decimal::ZERO)
            .is_ok());
        // The classifier routes that message to a validation exit (not OANDA).
        assert_eq!(execution_exit_code(&msg), crate::output::exit::VALIDATION);
    }

    /// AC4 (ordering invariant): the audit row is written BEFORE the OANDA
    /// submit. Driven through the real `execute_place` live path against a
    /// WRITABLE temp `$HOME` with NO credentials — `record_required` writes the
    /// pre-submit `attempt` row, THEN `client::resolve` fails (no creds) before
    /// any network submit. An `attempt` row on disk PLUS a *credentials* error
    /// (not an OANDA/submit error) proves the audit write happened first. The
    /// actual live fill is a human-run practice step in the D3 checklist.
    #[tokio::test]
    async fn ac4_audit_attempt_row_written_before_submit() {
        let home = TempHome::new();
        // AGT-613: the live arming gate (`confirm_live`) needs a human TTY and is
        // covered separately; drive the post-confirm sequence directly to assert
        // the audit-before-submit ordering invariant this test owns.
        let err = place_confirmed(
            OandaEnvironment::Practice,
            crate::vault_store::DEFAULT_ACCOUNT,
            "EUR_USD",
            1000,
            EntryPlan::market(None, None),
        )
            .await
            .expect_err("live place must fail: no credentials in temp HOME");
        let msg = format!("{err:#}");
        // Failed at credential resolution — i.e. AFTER the audit write, and
        // still BEFORE any OANDA network submit.
        assert!(
            msg.contains("no practice credentials"),
            "expected a credential-resolution failure, got: {msg}"
        );
        // The pre-submit attempt row is already durably on disk.
        let conn = audit::open_at(home.audit_db()).unwrap();
        let rows = audit::query(&conn, 10).unwrap();
        assert_eq!(rows.len(), 1, "exactly the pre-submit attempt row");
        assert_eq!(rows[0]["mode"], "live");
        assert_eq!(rows[0]["action"], "place");
        assert_eq!(rows[0]["outcome"], "attempt");
        assert_eq!(rows[0]["units"], 1000);
        drop(home);
    }

    /// AGT-630 (AC1/AC2): a `--strategy` place carries the attribution into
    /// BOTH the emitted payload and the audit row's strategy column — driven
    /// through the real `execute_place` paper path against a temp `$HOME`.
    #[tokio::test]
    async fn strategy_flag_attributes_paper_place_and_audit_row() {
        let home = TempHome::new();
        let plan = EntryPlan::market(Some("1.0850".into()), Some("1.0950".into()))
            .with_strategy(Some("ma-crossover".to_string()));
        let v = execute_place(
            OandaEnvironment::Practice,
            crate::vault_store::DEFAULT_ACCOUNT,
            "EUR_USD",
            1000,
            plan,
            false,
            false,
        )
            .await
            .expect("paper place succeeds");
        assert_eq!(v["mode"], "paper");
        assert_eq!(v["strategy"], "ma-crossover");

        let conn = audit::open_at(home.audit_db()).unwrap();
        let rows = audit::query(&conn, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["strategy"], "ma-crossover");
        drop(home);
    }

    /// AGT-630: without a strategy, the place stays unattributed — a null
    /// strategy in the payload and a NULL audit column, exactly as before.
    #[tokio::test]
    async fn place_without_strategy_stays_unattributed() {
        let home = TempHome::new();
        let plan = EntryPlan::market(None, None);
        let v = execute_place(
            OandaEnvironment::Practice,
            crate::vault_store::DEFAULT_ACCOUNT,
            "EUR_USD",
            1000,
            plan,
            false,
            false,
        )
            .await
            .expect("paper place succeeds");
        assert_eq!(v["strategy"], serde_json::Value::Null);

        let conn = audit::open_at(home.audit_db()).unwrap();
        let rows = audit::query(&conn, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["strategy"], serde_json::Value::Null);
        drop(home);
    }

    /// AGT-630 (AC2): the live pre-submit "attempt" row carries the strategy
    /// too — driven through the real post-confirm sequence (`place_confirmed`)
    /// against a temp `$HOME` with no credentials, same lever as the AGT-611
    /// ordering tests: the attempt row lands, then credential resolution fails
    /// before any network call.
    #[tokio::test]
    async fn live_attempt_row_carries_strategy() {
        let home = TempHome::new();
        let plan = EntryPlan::market(None, None).with_strategy(Some("rsi-reversion".into()));
        let err = place_confirmed(
            OandaEnvironment::Practice,
            crate::vault_store::DEFAULT_ACCOUNT,
            "EUR_USD",
            1000,
            plan,
        )
            .await
            .expect_err("live place must fail: no credentials in temp HOME");
        assert!(format!("{err:#}").contains("no practice credentials"));

        let conn = audit::open_at(home.audit_db()).unwrap();
        let rows = audit::query(&conn, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["outcome"], "attempt");
        assert_eq!(rows[0]["strategy"], "rsi-reversion");
        drop(home);
    }

    /// AC7: making `~/.wickd/`'s audit store unwritable makes a live attempt
    /// abort with NO OANDA order — proving `record_required` is pre-submit
    /// fatal. With the temp `.wickd` dir read-only, `record_required` cannot
    /// open the audit db and returns Err, aborting the command BEFORE
    /// `client::resolve` (so no "credentials" error) and BEFORE any OANDA
    /// submit. Unix-only: it relies on directory permission bits.
    #[cfg(unix)]
    #[tokio::test]
    async fn ac7_unwritable_audit_store_aborts_before_any_oanda_call() {
        use std::os::unix::fs::PermissionsExt;
        let home = TempHome::new();
        let wickd = home.wickd_dir();
        std::fs::create_dir_all(&wickd).unwrap();
        // Read + execute, but NOT write: the audit db cannot be created.
        std::fs::set_permissions(&wickd, std::fs::Permissions::from_mode(0o500)).unwrap();

        // AGT-613: bypass the (TTY-only) arming gate and drive the post-confirm
        // sequence directly — this test owns the pre-submit fatal-audit ordering.
        let err = place_confirmed(
            OandaEnvironment::Practice,
            crate::vault_store::DEFAULT_ACCOUNT,
            "EUR_USD",
            1000,
            EntryPlan::market(None, None),
        )
            .await
            .expect_err("live place must abort on an unwritable audit store");
        let msg = format!("{err:#}");
        // The abort is the audit-store write failure itself...
        assert!(
            msg.contains("audit db") || msg.contains("audit"),
            "expected an audit-store write failure, got: {msg}"
        );
        // ...and it happened BEFORE credential resolution / any OANDA call.
        assert!(
            !msg.contains("credentials") && !msg.contains("keychain"),
            "abort must precede credential resolution, got: {msg}"
        );
        // No audit db was ever created in the read-only dir → no ledger row,
        // and (by the pre-submit ordering) no order ever reached OANDA.
        assert!(!home.audit_db().exists(), "no audit db should exist");
        drop(home);
    }

    // ------------------------------------------------------------------------
    // AGT-626: non-interactive auto execution end-to-end, driven through the
    // REAL `execute_place_auto` / `execute_close_auto` entry points against a
    // temp `$HOME` with NO credentials. Same lever as the AGT-611/613 tests:
    // with no config.json under the temp HOME, `client::resolve` fails fast with
    // "no practice credentials" AFTER the pre-submit audit row and BEFORE any
    // network/keychain access — so the whole guarded sequence is provable
    // offline, with no OANDA account and no TTY.
    // ------------------------------------------------------------------------

    /// AC1: the auto entry point permits a PRACTICE `--live` submit with NO TTY
    /// (the interactive gate would fail closed here), and preserves the full
    /// guarded contract: the fatal pre-submit `attempt` audit row is on disk,
    /// THEN credential resolution fails — i.e. audit-before-submit ordering and
    /// the credential-resolve → risk-enforcement sequence are unchanged. Proves
    /// the ONLY relaxation is the arming gate.
    #[tokio::test]
    async fn ac1_auto_place_arms_practice_without_a_tty_and_keeps_the_guarded_sequence() {
        let home = TempHome::new();
        let err = execute_place_auto(
            OandaEnvironment::Practice,
            crate::vault_store::DEFAULT_ACCOUNT,
            "EUR_USD",
            1000,
            EntryPlan::market(Some("1.0850".into()), Some("1.0950".into())),
            true, // --live: a real submit, armed non-interactively for practice
        )
        .await
        .expect_err("auto place must proceed past the arm gate, then fail: no creds");
        let msg = format!("{err:#}");
        // It got PAST the arming gate without a TTY (no arm-refusal message)...
        assert!(
            !msg.contains("interactive TTY") && !msg.contains("practice environment only"),
            "auto practice arming must not fail closed, got: {msg}"
        );
        // ...and failed at credential resolution — i.e. AFTER the audit write and
        // still BEFORE any OANDA network submit (identical to the live path).
        assert!(
            msg.contains("no practice credentials"),
            "expected a credential-resolution failure, got: {msg}"
        );
        // The pre-submit attempt row is durably on disk (fatal-audit-first held).
        let conn = audit::open_at(home.audit_db()).unwrap();
        let rows = audit::query(&conn, 10).unwrap();
        assert_eq!(rows.len(), 1, "exactly the pre-submit attempt row");
        assert_eq!(rows[0]["mode"], "live");
        assert_eq!(rows[0]["action"], "place");
        assert_eq!(rows[0]["outcome"], "attempt");
        assert_eq!(rows[0]["units"], 1000);
        drop(home);
    }

    /// AC2: a LIVE submission still fails closed under auto arming — `--auto`
    /// cannot arm a live order. The refusal happens at the arm gate, BEFORE any
    /// audit row is written, so no order is placed and no ledger row is left
    /// stuck. Live-env behavior is unchanged: autonomy is practice-only.
    #[tokio::test]
    async fn ac2_auto_place_fails_closed_for_the_live_env_with_no_audit_row() {
        let home = TempHome::new();
        let err = execute_place_auto(
            OandaEnvironment::Live,
            crate::vault_store::DEFAULT_ACCOUNT,
            "EUR_USD",
            1000,
            EntryPlan::market(None, None),
            true, // --live --auto against the LIVE env → must be refused
        )
        .await
        .expect_err("auto arming must fail closed for the live env");
        let msg = format!("{err:#}");
        assert!(msg.contains("practice environment only"), "got: {msg}");
        assert!(msg.contains("--auto cannot arm live"), "got: {msg}");
        // Fail-closed at the arm gate means NOTHING downstream ran: no credential
        // error, and — crucially — NO audit db/attempt row exists (the refusal
        // precedes `record_required`).
        assert!(
            !msg.contains("credentials") && !msg.contains("keychain"),
            "live auto refusal must precede credential resolution, got: {msg}"
        );
        assert!(!home.audit_db().exists(), "no audit db should exist for a refused live auto arm");
        drop(home);
    }

    /// AC1 (paper): `--auto` WITHOUT `--live` is still paper — it never submits
    /// and needs no network/creds, exactly like any other paper place. `--auto`
    /// only changes the live arming gate; it cannot itself turn a paper call live.
    #[tokio::test]
    async fn auto_without_live_is_still_paper() {
        let home = TempHome::new();
        let v = execute_place_auto(
            OandaEnvironment::Practice,
            crate::vault_store::DEFAULT_ACCOUNT,
            "EUR_USD",
            1000,
            EntryPlan::market(None, None),
            false, // no --live → paper regardless of auto arming
        )
        .await
        .expect("paper place must succeed with no creds and no network");
        assert_eq!(v["mode"], "paper");
        assert_eq!(v["submitted"], false);
        let conn = audit::open_at(home.audit_db()).unwrap();
        let rows = audit::query(&conn, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["outcome"], "not_submitted");
        drop(home);
    }

    /// AC3 (close): the auto CLOSE path enforces the daily-loss kill-switch
    /// identically. `enforce_live_close` (which trips the kill-switch) runs
    /// immediately AFTER credential resolution in the ONE shared close sequence
    /// `execute_close_auto` funnels into — so proving the auto close reaches the
    /// pre-submit audit row + credential-resolve boundary (the step just before
    /// the kill-switch) shows the guard is on the identical path, with no TTY.
    #[tokio::test]
    async fn ac3_auto_close_arms_practice_without_a_tty_and_keeps_the_guarded_sequence() {
        let home = TempHome::new();
        let err = execute_close_auto(
            OandaEnvironment::Practice,
            crate::vault_store::DEFAULT_ACCOUNT,
            "EUR_USD",
            "long",
            true, // --live close, armed non-interactively for practice
        )
        .await
        .expect_err("auto close must proceed past the arm gate, then fail: no creds");
        let msg = format!("{err:#}");
        assert!(
            !msg.contains("interactive TTY") && !msg.contains("practice environment only"),
            "auto practice close arming must not fail closed, got: {msg}"
        );
        assert!(msg.contains("no practice credentials"), "got: {msg}");
        let conn = audit::open_at(home.audit_db()).unwrap();
        let rows = audit::query(&conn, 10).unwrap();
        assert_eq!(rows.len(), 1, "exactly the pre-submit attempt row");
        assert_eq!(rows[0]["action"], "close");
        assert_eq!(rows[0]["outcome"], "attempt");
        drop(home);
    }

    /// AC2 (close): the auto CLOSE path also fails closed for the live env, with
    /// no audit row — symmetric with the place path.
    #[tokio::test]
    async fn ac2_auto_close_fails_closed_for_the_live_env_with_no_audit_row() {
        let home = TempHome::new();
        let err = execute_close_auto(
            OandaEnvironment::Live,
            crate::vault_store::DEFAULT_ACCOUNT,
            "EUR_USD",
            "long",
            true,
        )
        .await
        .expect_err("auto close arming must fail closed for the live env");
        let msg = format!("{err:#}");
        assert!(msg.contains("practice environment only"), "got: {msg}");
        assert!(!home.audit_db().exists(), "no audit db for a refused live auto close");
        drop(home);
    }
}
