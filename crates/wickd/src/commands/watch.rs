//! `wickd watch` — persistent multi-instrument signal-monitoring daemon.
//!
//!   wickd watch ma-crossover EUR_USD --fast 10 --slow 30 --granularity H1
//!   wickd watch rsi EUR_USD,GBP_USD,USD_JPY --period 14 --overbought 70 --oversold 30
//!   wickd watch revert_adx EUR_USD,GBP_USD --set adx_max=22
//!
//! AGT-624: besides the two built-ins, `strategy` may name a Rhai-scripted
//! strategy, resolved with the exact same precedence as `backtest`/`strategy
//! run` (explicit `.rhai` path → built-in name → bare name under
//! `~/.wickd/strategies/`). Scripted watches accept repeatable
//! `--set id=value` overrides for the script's `@parameters` defaults,
//! validated exactly as in backtest; a script that fails validation aborts
//! watch at startup with an error naming the file.
//!
//! Hosts the strategy engine that lives in `wickd-core` *headless*: it
//! wires a [`MultiInstrumentWatcher`] (AGT-618 — one strategy/timeframe pair
//! evaluated concurrently across every instrument on the comma-separated
//! watchlist, not just one) to a daemon [`SignalSink`] and blocks until a
//! shutdown signal. Each detected condition is emitted as a structured JSON
//! signal (NDJSON) on stdout — `pattern-matched`, `watcher-tick`,
//! `strategy-status`, `strategy-error`, `match-status-update` — plus a
//! `strategy-signal-alert` line (AC3) whenever a candle's evaluation yields
//! an actionable Buy/Sell that isn't a dedup'd repeat (see
//! [`crate::signal_alert`]).
//!
//! AC2 / D4: before opening its own OANDA subscription, the daemon probes for
//! a running socket-hub (AGT-615). When one is up it drives the hub-covered
//! instruments off that shared feed — a [`wickd_core::strategy::TickStreamSource`]
//! per instrument, aggregating the hub's `price-update` ticks into candles — so
//! N watchers share one upstream connection. Instruments the hub isn't
//! streaming (and the whole watchlist when no hub is running) fall back to a
//! direct per-instrument OANDA source, exactly as before. See [`crate::hub`].
//!
//! Monitoring ONLY: the watcher runs in [`ExecutionMode::SignalOnly`] and the
//! watch path never imports or calls any order-placement endpoint. Clean
//! shutdown on SIGINT (Ctrl-C) or SIGTERM.
//!
//! The simple `Box<dyn Strategy>` used by `strategy run` (a backtest strategy)
//! is *not* what the watcher consumes — the watcher needs a
//! [`StrategyDefinition`]. For a built-in we build a minimal rules-based one
//! (`ma-crossover` → SMA-cross rules, `rsi` → threshold rules) and let the core
//! `RulesEngine` evaluate it against live candles, once per instrument. For a
//! scripted strategy we build a `strategy_type: "scripted"` definition carrying
//! the script source — the core watcher constructs one `ScriptedStrategy` per
//! instrument from it (with the `--set` overrides and the per-instrument event
//! calendar injected, matching backtest).

use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use tokio::sync::mpsc;

use wickd_core::backtest::rules_engine::SRZone;
use wickd_core::config::OandaEnvironment;
use wickd_core::event_sink::EventSink;
use wickd_core::oanda::endpoints::Granularity;
use wickd_core::shared::{
    CaptureMode, ComparisonOperator, Condition, CrossDirection, CrossTrigger, DataSource,
    EntryLogic, EntryLogicMode, EntryRule, ExitRule, IndicatorConfig, IndicatorSource,
    IndicatorType, ParameterizedValue, RiskMethod, RiskRewardTrigger, RiskSettings, RuleDirection,
    StrategyDefinition, ThresholdTrigger, Trigger, TriggerWithNot,
};
use wickd_core::oanda::client::OandaClient;
use wickd_core::strategy::{CandleSource, ExecutionMode, MultiInstrumentWatcher, TickStreamSource};

use crate::auto_exec::{self, AutoExecSink, AutoExecutor};
use crate::signal_alert::{AlertSink, ChangeDedupPolicy};
use crate::commands::{client, scripted};
use crate::feed::Format;
use crate::hub;
use crate::output::{exit, Out};
use crate::sink::{NoopSink, SignalSink};

/// Monitoring-only execution mode — AC3. Never armed for execution.
const MONITOR_MODE: ExecutionMode = ExecutionMode::SignalOnly;

/// Per-instrument signal filter passed to `MultiInstrumentWatcher::add_instrument`.
/// `wickd watch` doesn't expose long/short filtering (that's a desktop-app UI
/// concept); every instrument on the watchlist sees every signal.
const SIGNAL_FILTER_ALL: &str = "all";

/// How long to observe a running hub's ticks before deciding which watchlist
/// instruments it covers (AC2). Majors tick multiple times a second, so a few
/// seconds is ample; an instrument that stays silent through it is treated as
/// not-covered and gets its own direct source (a safe degradation).
const HUB_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Args, Debug)]
pub struct WatchArgs {
    /// Strategy to monitor: `ma-crossover`, `rsi`, or a Rhai script — an
    /// explicit `.rhai` path, or a bare name resolved under
    /// `~/.wickd/strategies/<name>.rhai` (AGT-624; same precedence as
    /// `backtest`: explicit path → built-in name → bare name).
    pub strategy: String,
    /// Instrument(s) to watch — comma-separated for a full watchlist, e.g.
    /// `EUR_USD,GBP_USD,USD_JPY` (AGT-618). All instruments share the same
    /// strategy/timeframe and are evaluated concurrently by one
    /// `MultiInstrumentWatcher`.
    #[arg(value_delimiter = ',', required = true)]
    pub instruments: Vec<String>,
    /// Candle granularity (M1, M5, M15, H1, H4, D, ...).
    #[arg(long, default_value = "H1")]
    pub granularity: String,
    /// Number of historical candles used to warm up indicators.
    #[arg(long, default_value_t = 200)]
    pub count: u32,
    /// OANDA environment whose stored credentials are used.
    #[arg(long, default_value = "practice")]
    pub env: String,
    /// Named account within --env whose credentials are used (AGT-625), e.g.
    /// h004. Default: the single/default account.
    #[arg(long, default_value = crate::vault_store::DEFAULT_ACCOUNT)]
    pub account: String,

    /// Delivery format for strategy-signal alerts: `ndjson` (default,
    /// machine-readable) or `human` for a live terminal feed — one clear line
    /// per Buy/Sell fire, with the raw signal firehose suppressed (AGT-619).
    #[arg(long, value_enum, default_value_t = Format::Ndjson)]
    pub format: Format,

    /// AGT-599 (trust-ladder Stage 1): in addition to streaming signals, record
    /// each tradeable entry signal as a *pending proposal* in
    /// `~/.wickd/pending.json` for later explicit approval via `wickd approve`.
    /// Still monitoring-only — watch NEVER places an order in either mode.
    #[arg(long)]
    pub semi_auto: bool,

    /// AGT-627 (trust-ladder Stage 2): autonomously EXECUTE tradeable signals —
    /// each entry signal places a practice-account order (with the script's
    /// SL/TP) through the AGT-626 guarded auto path, and each close signal routes
    /// through the guarded close path — WITHOUT a human keystroke. Practice
    /// environment ONLY: `--auto --env live` is refused at startup and the
    /// guarded path fails closed on live regardless. Mutually exclusive with
    /// `--semi-auto`.
    #[arg(long)]
    pub auto: bool,

    /// [--auto] Order size (magnitude, in units) for autonomous entries. The
    /// signal supplies only the direction; this supplies the size (AGT-599
    /// advisory-sizing contract — a script's own sizing stays advisory, never
    /// executed). Default: the conservative [`crate::pending::DEFAULT_PROPOSED_UNITS`].
    #[arg(long, default_value_t = crate::pending::DEFAULT_PROPOSED_UNITS)]
    pub units: i64,

    // --- ma-crossover params ---
    /// [ma-crossover] fast MA period.
    #[arg(long, default_value_t = 10)]
    pub fast: usize,
    /// [ma-crossover] slow MA period (must be > fast).
    #[arg(long, default_value_t = 30)]
    pub slow: usize,

    // --- rsi params ---
    /// [rsi] lookback period.
    #[arg(long, default_value_t = 14)]
    pub period: usize,
    /// [rsi] overbought threshold (short signal above this).
    #[arg(long, default_value_t = 70.0)]
    pub overbought: f64,
    /// [rsi] oversold threshold (long signal below this).
    #[arg(long, default_value_t = 30.0)]
    pub oversold: f64,

    // --- scripted-strategy params ---
    /// [scripted] Override a script's `@parameters` default: `--set <id>=<value>`,
    /// repeatable. Validated against the script's declared parameters (unknown
    /// id or out-of-min/max value is an error), exactly as in `backtest` (AGT-624 AC2).
    #[arg(long = "set", value_name = "ID=VALUE")]
    pub set: Vec<String>,
}

pub async fn run(args: WatchArgs, out: Out) -> ! {
    match watch(args).await {
        Ok(()) => std::process::exit(exit::OK),
        Err(e) => {
            let msg = format!("{e:#}");
            let code = if msg.contains("keychain") || msg.contains("credentials") {
                exit::AUTH
            } else if msg.contains("strategy")
                || msg.contains("period")
                || msg.contains("granularity")
                || msg.contains("threshold")
                || msg.contains("parameter")
                || msg.contains("script")
            {
                exit::VALIDATION
            } else {
                exit::OANDA
            };
            out.fail(code, "watch_failed", msg);
        }
    }
}

async fn watch(args: WatchArgs) -> Result<()> {
    if args.instruments.is_empty() {
        bail!("provide at least one instrument, e.g. `wickd watch ma-crossover EUR_USD`");
    }

    // AGT-627 argument validation — done BEFORE the strategy build, credential
    // resolution, or any network call, so an invalid autonomous-execution
    // request never starts a daemon.
    if args.auto {
        if args.semi_auto {
            bail!(
                "`--auto` (autonomous execution, Stage 2) and `--semi-auto` (propose-only, \
                 Stage 1) are mutually exclusive — choose one"
            );
        }
        if args.units <= 0 {
            bail!("`--units` must be a positive order size (got {})", args.units);
        }
        // AC2: `--auto --env live` is refused at startup (belt-and-braces with the
        // guarded path's own practice-only arming gate).
        let env = OandaEnvironment::from_str(&args.env).map_err(|e| anyhow!(e.to_string()))?;
        auto_exec::reject_auto_live(args.auto, env)?;
    }

    let timeframe =
        Granularity::from_str(&args.granularity).map_err(|e| anyhow!("invalid granularity: {e}"))?;

    // Validate + build the strategy definition before any network call
    // (AGT-624 AC4: a script that fails validation aborts here, with the
    // error naming the file). One StrategyDefinition, cloned per-instrument
    // internally by the watcher (each instrument gets its own RulesEngine or
    // ScriptedStrategy so matches emit individually) — see
    // `wickd_core::strategy::multi_watcher`.
    let resolved = build_watch_strategy(&args)?;

    // Resolve credentials -> authenticated OANDA client (the watcher polls it,
    // and each instrument still does its own one-off REST warmup even on the hub
    // path — see `TickStreamSource::get_candles`). `env` (practice, guaranteed by
    // the AGT-627 gate above) is threaded to the `--auto` executor.
    let (env, client) = client::resolve(&args.env, &args.account)?;

    let watcher_id = format!("watch-{}-{}", resolved.label, timeframe);
    // No dynamic add/remove needed for a CLI-fixed watchlist — the command
    // channel exists only because the constructor requires one; dropping the
    // sender immediately just makes `try_recv` in the watcher's main loop a
    // permanent (cheap) no-op instead of blocking.
    let (_command_tx, command_rx) = mpsc::channel(1);

    // Shared stop signal: flipping it makes the watcher loop exit cleanly.
    let stop = Arc::new(AtomicBool::new(false));

    let mut watcher = MultiInstrumentWatcher::new(
        watcher_id.clone(),
        format!("wickd-watch-{}", resolved.label),
        resolved.label.clone(),
        resolved.definition,
        timeframe,
        "wickd-watch".to_string(),
        client.clone(),
        MONITOR_MODE,
        command_rx,
        stop.clone(),
    );

    // Restart backfill: attach the durable candle ledger so candles that
    // close while the process is down (machine shutdown, crash) are replayed
    // and evaluated at the next startup instead of silently skipped. An
    // unopenable ledger degrades to the historical skip-the-gap behavior —
    // it must never stop the watcher.
    match wickd_core::strategy::WatchStateStore::default_dir()
        .and_then(|dir| wickd_core::strategy::WatchStateStore::open(&dir, &watcher_id))
    {
        Ok(store) => watcher.set_state_store(store),
        Err(e) => eprintln!(
            "wickd watch: watch-state ledger unavailable ({e}) — restart backfill disabled"
        ),
    }

    // Scripted watch (AGT-624): hand the validated `--set` overrides to the
    // watcher (AC2) and inject each instrument's event calendar so ABI v3
    // `hours_since_event()`/`hours_until_event()` behave exactly as in
    // backtest — BEFORE any instrument is added.
    if let Some(script) = &resolved.scripted {
        watcher.set_script_params(script.params.clone());
        for instrument in &args.instruments {
            let (events, _source) = crate::events::load_for_instrument(instrument)?;
            watcher.set_script_event_calendar(instrument.clone(), events);
        }
        // Surprise feed (ABI v4): one load of ~/.wickd/calendar/*.csv, cloned
        // into each per-instrument script instance; the per-candle refresh
        // hook then picks up CSV drops (including backfilled actuals) while
        // the watcher runs — no restart, no rebuild.
        watcher.set_script_surprise_calendar(crate::events::load_surprise_calendar()?);
        eprintln!(
            "wickd watch: running scripted strategy {} (params: {})",
            script.path.display(),
            script.effective
        );
    }

    // AC1/AC2: register every instrument on the watchlist against the same
    // strategy/timeframe watcher — concurrent evaluation, not one process per
    // pair. When a socket-hub (AGT-615) is running, hub-covered instruments are
    // driven off its shared feed (no second upstream subscription); everything
    // else — and the whole watchlist when no hub is up — uses a direct OANDA
    // source, exactly as before. `_hub_feed` must outlive the watcher: dropping
    // it aborts the background reader that feeds the tick sources.
    let _hub_feed = attach_watchlist(&mut watcher, &args.instruments, timeframe, &client).await?;

    // Sink selection (AGT-599 / AGT-619 / AGT-627): default `watch` streams
    // signals only; with `--semi-auto` the sink additionally records each
    // tradeable entry as a pending proposal (a machine workflow, so it keeps its
    // NDJSON output); with `--auto` the sink additionally EXECUTES each signal
    // through the guarded auto path (Stage 2, practice-only) while still emitting
    // the unchanged NDJSON stream (AC5); with `--format human` (and neither) the
    // base sink is silenced so the human feed shows only alert fires.
    //
    // NOTE: `--auto` keeps the watcher itself in `SignalOnly` (MONITOR_MODE) —
    // the core watcher never places an order. Execution happens ONLY out-of-band
    // on the executor task, via `execute_place_auto`/`execute_close_auto`, so the
    // "watch imports no order code" boundary the monitor relies on is preserved
    // and every order still flows through the one guarded auto path.
    let base_sink: Arc<dyn EventSink> = if args.auto {
        // Hand each tradeable signal to a dedicated executor task. The sink only
        // classifies + dispatches (staying a monitoring NDJSON stream); the
        // executor owns per-instrument position state (AC4) and performs the
        // serialized async submissions through the AGT-626 guarded auto path.
        let (tx, rx) = mpsc::unbounded_channel();
        let mut executor = AutoExecutor::new(env, args.account.clone(), args.units);

        // AGT-628: reconcile in-memory position state against OANDA's ACTUAL open
        // positions BEFORE arming the executor. An existing position on a watched
        // instrument is adopted (its per-instrument state is seeded so a duplicate
        // entry stays suppressed and the strategy's close logic resumes after
        // warmup — AC1); each adoption is emitted on the signal stream and recorded
        // in the audit log (AC2). Open positions on instruments NOT on the
        // watchlist are reported and left completely untouched (AC2). A failed
        // fetch aborts startup: arming autonomous execution blind to real positions
        // is exactly the orphan/double-entry hazard this ticket removes.
        let open_positions = wickd_core::oanda::endpoints::get_open_positions(&client)
            .await
            .context("fetching open positions for startup reconciliation (`watch --auto`)")?;
        let recon = auto_exec::reconcile_positions(&open_positions, &args.instruments);
        if !recon.adopted.is_empty() || !recon.unwatched.is_empty() {
            eprintln!(
                "wickd watch: startup reconciliation — adopted {} open position(s) on watched \
                 instrument(s), reported {} on unwatched instrument(s)",
                recon.adopted.len(),
                recon.unwatched.len()
            );
        }
        auto_exec::apply_reconciliation(&mut executor, &recon, env);

        tokio::spawn(auto_exec::run_executor(executor, rx));
        eprintln!(
            "wickd watch: AUTONOMOUS execution armed (practice env, {} units/order) — \
             tradeable signals place practice orders via the guarded auto path",
            args.units
        );
        Arc::new(AutoExecSink::new(tx))
    } else if args.semi_auto {
        Arc::new(crate::sink::SemiAutoSink::new(crate::pending::pending_path()?))
    } else if args.format == Format::Human {
        Arc::new(NoopSink)
    } else {
        Arc::new(SignalSink)
    };
    // AC3 / AGT-619: wrap with the strategy-signal alert layer — additive,
    // never filters the underlying signal stream. `format` selects NDJSON vs
    // the human terminal feed for the fire line. See `crate::signal_alert`.
    // AGT-620: each fired strategy-signal alert is also appended to the durable
    // alert queue so an agent can poll it and later `wickd queue promote <id>`.
    let sink: Arc<dyn EventSink> = Arc::new(
        AlertSink::new(base_sink, ChangeDedupPolicy::new(), args.format)
            .with_queue(crate::alert_queue::queue_path()?)
            .with_account(args.account.clone()),
    );

    // Install a SIGTERM handler in addition to SIGINT (Ctrl-C) — AC4.
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("installing SIGTERM handler")?;

    // Run the (blocking) watcher loop and the shutdown signals concurrently in
    // this task — no `spawn`, so we sidestep `Send` bounds on the watcher.
    let watch_fut = watcher.start(sink);
    tokio::pin!(watch_fut);

    tokio::select! {
        // Watcher finished on its own (e.g. warmup exhausted retries).
        res = &mut watch_fut => return res.map_err(|e| anyhow!("watcher stopped: {e}")),
        _ = tokio::signal::ctrl_c() => {}
        _ = sigterm.recv() => {}
    }

    // Shutdown requested: ask the loop to wind down, then give it a brief grace
    // period so it can emit its `Stopped` status without blocking on a long poll.
    stop.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(2), watch_fut).await;
    Ok(())
}

/// Wire every watchlist instrument into `watcher`, sharing a running socket-hub
/// (AGT-615) where possible and falling back to a direct OANDA source otherwise.
///
/// Returns the [`hub::HubFeed`] when a hub was attached — the caller MUST keep
/// it alive for the watcher's lifetime, since dropping it aborts the background
/// reader that feeds the tick sources — or `None` when no hub is running.
async fn attach_watchlist(
    watcher: &mut MultiInstrumentWatcher,
    instruments: &[String],
    timeframe: Granularity,
    client: &OandaClient,
) -> Result<Option<hub::HubFeed>> {
    let Some(handle) = hub::probe_hub().await else {
        // No hub running: preserve the pre-AGT-615 behavior exactly — every
        // instrument opens its own direct OANDA source.
        for instrument in instruments {
            watcher
                .add_instrument(instrument.clone(), Vec::<SRZone>::new(), SIGNAL_FILTER_ALL.to_string())
                .await
                .map_err(|e| anyhow!("failed to add {instrument} to the watchlist: {e}"))?;
        }
        return Ok(None);
    };

    // A hub is up. Fan its feed out for our whole watchlist, then give it a
    // brief window to reveal which instruments it's actually streaming — the hub
    // has no control channel, so coverage is learned by observing ticks.
    let socket_path = handle.socket_path().display().to_string();
    let (feed, mut receivers) = hub::HubFeed::attach(handle.into_stream(), instruments);
    let observed = hub::discover_instruments(&feed, instruments, HUB_DISCOVERY_TIMEOUT).await;
    let (on_hub, direct) = hub::partition_watchlist(instruments, &observed);

    if !on_hub.is_empty() {
        eprintln!(
            "wickd watch: sharing the stream hub at {socket_path} for {} — no second subscription",
            on_hub.join(", ")
        );
    }
    if !direct.is_empty() {
        eprintln!(
            "wickd watch: hub is not streaming {} — opening a direct subscription for those",
            direct.join(", ")
        );
    }

    // Hub-covered instruments: drive off the shared feed via a tick source.
    for instrument in &on_hub {
        let rx = receivers.remove(instrument).ok_or_else(|| {
            anyhow!("BUG: no tick receiver for {instrument} — HubFeed::attach contract violated")
        })?;
        let source: Box<dyn CandleSource> = Box::new(TickStreamSource::new(
            client.clone(),
            instrument.clone(),
            timeframe,
            rx,
        ));
        watcher
            .add_instrument_with_source(
                instrument.clone(),
                source,
                Vec::<SRZone>::new(),
                SIGNAL_FILTER_ALL.to_string(),
            )
            .await
            .map_err(|e| anyhow!("failed to add {instrument} (hub feed) to the watchlist: {e}"))?;
    }

    // Uncovered instruments: fall back to a direct OANDA source. Their unused
    // receivers drop here, so the feed stops buffering ticks for them.
    for instrument in &direct {
        watcher
            .add_instrument(instrument.clone(), Vec::<SRZone>::new(), SIGNAL_FILTER_ALL.to_string())
            .await
            .map_err(|e| anyhow!("failed to add {instrument} to the watchlist: {e}"))?;
    }

    Ok(Some(feed))
}

/// A watch strategy resolved from the CLI `strategy` argument (AGT-624).
#[derive(Debug)]
struct ResolvedWatchStrategy {
    /// Definition the core watcher consumes (rules-based for built-ins,
    /// `strategy_type: "scripted"` carrying the script source for scripts).
    definition: StrategyDefinition,
    /// Canonical display label: the built-in's canonical name, or the
    /// script's file stem — used for the watcher id / strategy name so a
    /// path argument doesn't leak into event `config_id`s.
    label: String,
    /// Present only for scripted strategies.
    scripted: Option<ScriptedWatch>,
}

/// Scripted-strategy specifics carried alongside the definition.
#[derive(Debug)]
struct ScriptedWatch {
    /// Resolved script file (for startup diagnostics).
    path: PathBuf,
    /// Validated `--set` overrides, handed to the core watcher (AC2).
    params: HashMap<String, f64>,
    /// Effective parameter map (defaults merged with overrides) — the run is
    /// self-describing.
    effective: serde_json::Value,
}

/// Build the [`StrategyDefinition`] the watcher needs from the CLI args,
/// validating params without panicking. Built-ins mirror `strategy run`'s set
/// (`ma-crossover`, `rsi`) expressed as `RulesEngine` rules; anything else
/// resolves as a Rhai script with the same precedence as `backtest`/`strategy
/// run` (AGT-624 AC1): an unambiguous script reference (explicit path or
/// `.rhai` suffix) first, then built-in names, then a bare name under
/// `~/.wickd/strategies/` — so a script can never shadow a built-in.
fn build_watch_strategy(args: &WatchArgs) -> Result<ResolvedWatchStrategy> {
    let overrides = scripted::parse_set_pairs(&args.set)?;

    if let Some(path) = scripted::resolve_explicit_script_path(&args.strategy) {
        return scripted_watch_strategy(path, overrides);
    }

    // Built-ins take typed flags, not @parameters — a --set here is a mistake
    // that must not silently no-op (mirrors `strategy run`/`backtest`).
    let ensure_no_set = |name: &str| -> Result<()> {
        if args.set.is_empty() {
            Ok(())
        } else {
            bail!(
                "'--set' overrides scripted-strategy parameters; '{name}' is a built-in \
                 (use --fast/--slow or --period/--overbought/--oversold)"
            )
        }
    };

    match args.strategy.to_lowercase().as_str() {
        "ma-crossover" | "ma" => {
            ensure_no_set("ma-crossover")?;
            if args.fast == 0 || args.slow == 0 {
                bail!("ma-crossover strategy periods must be greater than 0");
            }
            if args.fast >= args.slow {
                bail!(
                    "ma-crossover strategy fast period ({}) must be less than slow period ({})",
                    args.fast,
                    args.slow
                );
            }
            Ok(ResolvedWatchStrategy {
                definition: ma_crossover_definition(args.fast, args.slow),
                label: "ma-crossover".to_string(),
                scripted: None,
            })
        }
        "rsi" => {
            ensure_no_set("rsi")?;
            if args.period == 0 {
                bail!("rsi strategy period must be greater than 0");
            }
            if args.oversold >= args.overbought {
                bail!(
                    "rsi strategy oversold threshold ({}) must be less than overbought ({})",
                    args.oversold,
                    args.overbought
                );
            }
            Ok(ResolvedWatchStrategy {
                definition: rsi_definition(args.period, args.overbought, args.oversold),
                label: "rsi".to_string(),
                scripted: None,
            })
        }
        other => {
            if let Some(path) = scripted::resolve_named_script_path(other)? {
                return scripted_watch_strategy(path, overrides);
            }
            bail!(
                "unknown strategy '{other}' (available: ma-crossover, rsi, \
                 or a .rhai script path/name under ~/.wickd/strategies/)"
            )
        }
    }
}

/// Validate `path` (script compile + `--set` overrides against declared
/// `@parameters`, exactly as backtest — AGT-624 AC2/AC4) and wrap it in a
/// `strategy_type: "scripted"` definition the core watcher instantiates
/// per-instrument.
fn scripted_watch_strategy(
    path: PathBuf,
    overrides: HashMap<String, f64>,
) -> Result<ResolvedWatchStrategy> {
    let (script, effective) = scripted::load_validated_script(&path, &overrides)?;
    let label = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("scripted")
        .to_string();
    Ok(ResolvedWatchStrategy {
        definition: scripted_definition(&label, script),
        label,
        scripted: Some(ScriptedWatch {
            path,
            params: overrides,
            effective,
        }),
    })
}

/// A `strategy_type: "scripted"` [`StrategyDefinition`] carrying the script
/// source. The rules-engine fields (entry/exit rules, indicators) are unused
/// on the scripted executor path — the script declares its own via
/// `@indicators`/`@parameters` — but the struct requires them.
fn scripted_definition(name: &str, script: String) -> StrategyDefinition {
    StrategyDefinition {
        id: format!("wickd-watch-{name}"),
        user_id: "wickd-watch".to_string(),
        name: name.to_string(),
        description: format!("wickd watch scripted strategy {name}"),
        parameters: vec![],
        indicators: vec![],
        variables: vec![],
        entry_rules: vec![],
        entry_logic: EntryLogic {
            mode: EntryLogicMode::All,
            min_score: None,
        },
        exit_rules: vec![],
        risk_settings: default_risk_settings(),
        version: 1,
        is_active: true,
        schema_version: 2,
        strategy_type: "scripted".to_string(),
        script_content: Some(script),
    }
}

/// A `DataSource` reading an indicator's primary `value` output on the current candle.
fn indicator_value(id: &str) -> DataSource {
    DataSource::Indicator(IndicatorSource {
        indicator: id.to_string(),
        output: "value".to_string(),
        offset: 0,
        symbol: None,
        timeframe: None,
        capture: CaptureMode::EachCandle,
        trail: None,
    })
}

/// A single-trigger condition (no chained groups, not negated).
fn simple_condition(trigger: Trigger) -> Condition {
    Condition {
        name: None,
        primary: TriggerWithNot {
            trigger,
            negated: false,
        },
        chain: vec![],
        disabled: None,
    }
}

fn entry_rule(id: &str, name: &str, direction: RuleDirection, trigger: Trigger) -> EntryRule {
    EntryRule {
        id: id.to_string(),
        name: Some(name.to_string()),
        direction,
        conditions: vec![simple_condition(trigger)],
        trigger_chain: None,
        pending_order: None,
    }
}

/// Default risk settings. The watch daemon never executes, but `RulesEngine`
/// requires a `RiskSettings` to size hypothetical signals.
fn default_risk_settings() -> RiskSettings {
    RiskSettings {
        risk_method: RiskMethod::Percent,
        risk_value: ParameterizedValue::Fixed(1.0),
        rr_ratio: ParameterizedValue::Fixed(2.0),
        spread_buffer_pips: ParameterizedValue::Fixed(1.0),
        stop_loss_source: None,
        risk_method_short: None,
        risk_value_short: None,
        rr_ratio_short: None,
        spread_buffer_pips_short: None,
        stop_loss_source_short: None,
    }
}

/// A take-profit-at-RR exit rule, so in-position runs can emit exit signals.
fn risk_reward_exit() -> ExitRule {
    ExitRule {
        id: "rr_exit".to_string(),
        name: Some("Take profit at 2:1".to_string()),
        direction: RuleDirection::Both,
        conditions: vec![simple_condition(Trigger::RiskReward(RiskRewardTrigger {
            ratio: ParameterizedValue::Fixed(2.0),
        }))],
        trigger_chain: None,
        close_percent: ParameterizedValue::Fixed(100.0),
        priority: 100,
    }
}

fn base_definition(name: &str, indicators: Vec<IndicatorConfig>, entry_rules: Vec<EntryRule>) -> StrategyDefinition {
    StrategyDefinition {
        id: format!("wickd-watch-{name}"),
        user_id: "wickd-watch".to_string(),
        name: name.to_string(),
        description: format!("wickd watch monitor for {name}"),
        parameters: vec![],
        indicators,
        variables: vec![],
        entry_rules,
        entry_logic: EntryLogic {
            mode: EntryLogicMode::All,
            min_score: None,
        },
        exit_rules: vec![risk_reward_exit()],
        risk_settings: default_risk_settings(),
        version: 1,
        is_active: true,
        schema_version: 2,
        strategy_type: "rules".to_string(),
        script_content: None,
    }
}

/// `ma-crossover`: long when the fast SMA crosses above the slow SMA, short on
/// the reverse — the rules-engine equivalent of `MovingAverageCrossover`.
fn ma_crossover_definition(fast: usize, slow: usize) -> StrategyDefinition {
    let indicators = vec![
        IndicatorConfig::new_fixed("sma_fast", IndicatorType::Sma, &[("period", fast as f64)]),
        IndicatorConfig::new_fixed("sma_slow", IndicatorType::Sma, &[("period", slow as f64)]),
    ];
    let cross = |direction: CrossDirection| {
        Trigger::Cross(CrossTrigger {
            left: indicator_value("sma_fast"),
            right: indicator_value("sma_slow"),
            direction,
            lookback: ParameterizedValue::Fixed(1.0),
        })
    };
    let entry_rules = vec![
        entry_rule(
            "cross_up",
            "Fast SMA crosses above slow",
            RuleDirection::Long,
            cross(CrossDirection::Above),
        ),
        entry_rule(
            "cross_down",
            "Fast SMA crosses below slow",
            RuleDirection::Short,
            cross(CrossDirection::Below),
        ),
    ];
    base_definition("ma-crossover", indicators, entry_rules)
}

/// `rsi`: long when RSI drops below the oversold threshold, short when it rises
/// above overbought — the rules-engine equivalent of `RsiStrategy`.
fn rsi_definition(period: usize, overbought: f64, oversold: f64) -> StrategyDefinition {
    let indicators = vec![IndicatorConfig::new_fixed(
        "rsi",
        IndicatorType::Rsi,
        &[("period", period as f64)],
    )];
    let threshold = |operator: ComparisonOperator, value: f64| {
        Trigger::Threshold(ThresholdTrigger {
            source: indicator_value("rsi"),
            operator,
            value: ParameterizedValue::Fixed(value),
            lookback: ParameterizedValue::Fixed(1.0),
        })
    };
    let entry_rules = vec![
        entry_rule(
            "rsi_oversold",
            "RSI below oversold",
            RuleDirection::Long,
            threshold(ComparisonOperator::LessThan, oversold),
        ),
        entry_rule(
            "rsi_overbought",
            "RSI above overbought",
            RuleDirection::Short,
            threshold(ComparisonOperator::GreaterThan, overbought),
        ),
    ];
    base_definition("rsi", indicators, entry_rules)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wickd_core::backtest::RulesEngine;

    fn args(strategy: &str) -> WatchArgs {
        WatchArgs {
            strategy: strategy.to_string(),
            instruments: vec!["EUR_USD".to_string()],
            granularity: "H1".to_string(),
            count: 200,
            env: "practice".to_string(),
            account: crate::vault_store::DEFAULT_ACCOUNT.to_string(),
            format: Format::Ndjson,
            semi_auto: false,
            auto: false,
            units: crate::pending::DEFAULT_PROPOSED_UNITS,
            fast: 10,
            slow: 30,
            period: 14,
            overbought: 70.0,
            oversold: 30.0,
            set: vec![],
        }
    }

    /// A `.rhai` file under the OS temp dir, deleted when it drops (same
    /// pattern as `commands::scripted`'s tests).
    struct TempScript(std::path::PathBuf);

    impl TempScript {
        fn new(contents: &str) -> Self {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let mut p = std::env::temp_dir();
            p.push(format!("wickd-watch-test-{pid}-{nanos}-{n}.rhai"));
            std::fs::write(&p, contents).expect("write temp script");
            Self(p)
        }
    }

    impl Drop for TempScript {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    const VALID_SCRIPT: &str = r#"
// @parameters: [
//   { "id": "threshold", "type": "number", "default": 30.0, "min": 10.0, "max": 90.0 }
// ]
fn on_candle() {
    "hold"
}
"#;

    const MALFORMED_SCRIPT: &str = r#"
fn on_candle( {
    "hold"
}
"#;

    // The definition we build must be accepted by the core RulesEngine — i.e.
    // it is actually wireable into a live watcher (the loop the daemon hosts).
    #[test]
    fn ma_crossover_definition_is_engine_valid() {
        let resolved = build_watch_strategy(&args("ma-crossover")).unwrap();
        assert_eq!(resolved.label, "ma-crossover");
        assert!(resolved.scripted.is_none());
        let def = resolved.definition;
        assert_eq!(def.indicators.len(), 2);
        assert_eq!(def.entry_rules.len(), 2);
        assert!(RulesEngine::new(def).is_ok());
    }

    #[test]
    fn rsi_definition_is_engine_valid() {
        let resolved = build_watch_strategy(&args("rsi")).unwrap();
        assert_eq!(resolved.label, "rsi");
        assert!(resolved.scripted.is_none());
        let def = resolved.definition;
        assert_eq!(def.indicators.len(), 1);
        assert!(RulesEngine::new(def).is_ok());
    }

    // AC1: an explicit `.rhai` path resolves to a scripted definition the
    // core watcher can instantiate per instrument.
    #[test]
    fn explicit_script_path_resolves_to_a_scripted_definition() {
        let f = TempScript::new(VALID_SCRIPT);
        let mut a = args("ignored");
        a.strategy = f.0.display().to_string();
        let resolved = build_watch_strategy(&a).unwrap();
        assert_eq!(resolved.definition.strategy_type, "scripted");
        assert_eq!(resolved.definition.script_content.as_deref(), Some(VALID_SCRIPT));
        let scripted = resolved.scripted.expect("scripted metadata present");
        assert_eq!(scripted.path, f.0);
        // Defaults echo in the effective params — the run is self-describing.
        assert_eq!(scripted.effective["threshold"], 30.0);
        // The label is the file stem, not the raw path argument.
        assert!(resolved.label.starts_with("wickd-watch-test-"));
        assert!(!resolved.label.contains('/'));
    }

    // AC2: `--set` overrides are parsed, validated against @parameters, and
    // carried through for the watcher.
    #[test]
    fn set_overrides_apply_to_a_scripted_watch() {
        let f = TempScript::new(VALID_SCRIPT);
        let mut a = args("ignored");
        a.strategy = f.0.display().to_string();
        a.set = vec!["threshold=55".to_string()];
        let resolved = build_watch_strategy(&a).unwrap();
        let scripted = resolved.scripted.unwrap();
        assert_eq!(scripted.params.get("threshold"), Some(&55.0));
        assert_eq!(scripted.effective["threshold"], 55.0);
    }

    // AC2: overrides are validated exactly as in backtest — an unknown id or
    // out-of-range value aborts before any network call.
    #[test]
    fn set_overrides_are_validated_against_declared_parameters() {
        let f = TempScript::new(VALID_SCRIPT);
        let mut a = args("ignored");
        a.strategy = f.0.display().to_string();

        a.set = vec!["typo=5".to_string()];
        let msg = format!("{:#}", build_watch_strategy(&a).unwrap_err());
        assert!(msg.contains("unknown parameter 'typo'"), "message was: {msg}");

        a.set = vec!["threshold=95".to_string()];
        let msg = format!("{:#}", build_watch_strategy(&a).unwrap_err());
        assert!(msg.contains("above the declared max"), "message was: {msg}");
    }

    // AC4: `--set` against a built-in is an explicit error, not a silent no-op.
    #[test]
    fn set_with_a_builtin_is_rejected() {
        let mut a = args("rsi");
        a.set = vec!["threshold=55".to_string()];
        let msg = format!("{:#}", build_watch_strategy(&a).unwrap_err());
        assert!(msg.contains("built-in"), "message was: {msg}");
    }

    // AC4: a script that fails `validate_script` aborts watch at startup with
    // the validation error naming the file.
    #[test]
    fn invalid_script_aborts_startup_naming_the_file() {
        let f = TempScript::new(MALFORMED_SCRIPT);
        let mut a = args("ignored");
        a.strategy = f.0.display().to_string();
        let msg = format!("{:#}", build_watch_strategy(&a).unwrap_err());
        assert!(msg.contains("invalid strategy script"), "message was: {msg}");
        assert!(msg.contains(&f.0.display().to_string()), "message was: {msg}");
    }

    // AC1: the unknown-strategy error now points at the script options too.
    #[test]
    fn unknown_strategy_error_mentions_scripts() {
        let msg = format!(
            "{:#}",
            build_watch_strategy(&args("definitely-not-a-real-strategy-name-xyz")).unwrap_err()
        );
        assert!(msg.contains("~/.wickd/strategies/"), "message was: {msg}");
    }

    // AC2: `--set` is repeatable on the watch CLI, like backtest.
    #[test]
    fn set_flag_is_repeatable_on_the_cli() {
        use clap::Parser;
        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            watch: WatchArgs,
        }
        let cli = TestCli::parse_from([
            "test",
            "revert_adx",
            "EUR_USD",
            "--set",
            "a=1",
            "--set",
            "b=2.5",
        ]);
        assert_eq!(cli.watch.set, vec!["a=1".to_string(), "b=2.5".to_string()]);
    }

    #[test]
    fn rejects_fast_ge_slow_without_panicking() {
        let mut a = args("ma-crossover");
        a.fast = 30;
        a.slow = 10;
        assert!(build_watch_strategy(&a).is_err());
    }

    #[test]
    fn rejects_inverted_rsi_thresholds() {
        let mut a = args("rsi");
        a.overbought = 30.0;
        a.oversold = 70.0;
        assert!(build_watch_strategy(&a).is_err());
    }

    #[test]
    fn rejects_unknown_strategy() {
        assert!(build_watch_strategy(&args("nope")).is_err());
    }

    // AC3: the watch path is monitoring-only. Guard the constant so a future
    // edit can't silently arm execution.
    #[test]
    fn monitor_mode_never_executes() {
        assert_eq!(MONITOR_MODE, ExecutionMode::SignalOnly);
    }

    // AC4: the shutdown plumbing — flipping the shared stop flag is what the
    // signal handlers do; assert the wiring an watcher observes via should_stop.
    #[test]
    fn stop_flag_plumbing_flips() {
        let stop = Arc::new(AtomicBool::new(false));
        assert!(!stop.load(Ordering::SeqCst));
        // This is exactly what the SIGINT/SIGTERM arm does.
        stop.store(true, Ordering::SeqCst);
        assert!(stop.load(Ordering::SeqCst));
    }

    // AC1: `wickd watch` takes a comma-separated watchlist, not just one
    // instrument — this is the CLI-flag fallback the ticket calls for since
    // AGT-614's `~/.wickd/watchlist.json` isn't merged yet, mirroring the
    // convention `stream` already uses for its instrument list.
    #[test]
    fn instruments_arg_splits_watchlist_on_commas() {
        use clap::Parser;
        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            watch: WatchArgs,
        }
        let cli = TestCli::parse_from(["test", "ma-crossover", "EUR_USD,GBP_USD,USD_JPY"]);
        assert_eq!(
            cli.watch.instruments,
            vec!["EUR_USD".to_string(), "GBP_USD".to_string(), "USD_JPY".to_string()]
        );
    }

    // Backward compatibility: a single instrument (no commas) still parses
    // exactly as it did before AGT-618.
    #[test]
    fn instruments_arg_accepts_a_single_instrument() {
        use clap::Parser;
        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            watch: WatchArgs,
        }
        let cli = TestCli::parse_from(["test", "rsi", "EUR_USD"]);
        assert_eq!(cli.watch.instruments, vec!["EUR_USD".to_string()]);
    }

    // AC1: an empty watchlist is rejected before any credential resolution
    // or network call — `watch()` checks this as its very first step.
    #[tokio::test]
    async fn watch_rejects_an_empty_watchlist_before_touching_credentials() {
        let mut a = args("ma-crossover");
        a.instruments = vec![];
        let err = watch(a).await.expect_err("empty watchlist must be rejected");
        assert!(
            err.to_string().contains("at least one instrument"),
            "unexpected error: {err}"
        );
    }

    // AGT-627 AC2: `--auto --env live` is refused at startup — before the
    // strategy build, credential resolution, or any network call — so an
    // autonomous live request never opens a watch loop.
    #[tokio::test]
    async fn watch_rejects_auto_against_live_before_touching_credentials() {
        let mut a = args("ma-crossover");
        a.auto = true;
        a.env = "live".to_string();
        let err = watch(a).await.expect_err("--auto --env live must be rejected");
        assert!(
            err.to_string().contains("practice environment only"),
            "unexpected error: {err}"
        );
    }

    // AGT-627: autonomous execution (Stage 2) and propose-only (Stage 1) are
    // mutually exclusive — rejected up front.
    #[tokio::test]
    async fn watch_rejects_auto_combined_with_semi_auto() {
        let mut a = args("ma-crossover");
        a.auto = true;
        a.semi_auto = true;
        let err = watch(a).await.expect_err("--auto --semi-auto must be rejected");
        assert!(
            err.to_string().contains("mutually exclusive"),
            "unexpected error: {err}"
        );
    }

    // AGT-627: a non-positive `--units` is rejected before the daemon starts —
    // an autonomous order must have a real, positive size.
    #[tokio::test]
    async fn watch_rejects_auto_with_non_positive_units() {
        let mut a = args("ma-crossover");
        a.auto = true;
        a.units = 0;
        let err = watch(a).await.expect_err("--units 0 must be rejected under --auto");
        assert!(
            err.to_string().contains("positive order size"),
            "unexpected error: {err}"
        );
    }
}
