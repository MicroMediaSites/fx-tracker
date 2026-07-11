//! `wickd backtest` — run a backtest via `wickd-core` over OANDA candles.
//!
//!   wickd backtest ma-crossover EUR_USD --fast 10 --slow 30 --granularity H1 --count 500
//!   wickd backtest rsi EUR_USD --period 14 --overbought 70 --oversold 30 --count 1000
//!   wickd backtest ma-crossover EUR_USD --from 2024-01-01T00:00:00Z --to 2024-02-01T00:00:00Z
//!   wickd backtest ./my-strategy.rhai EUR_USD --granularity H1 --count 500
//!
//! Wraps `wickd-core`'s `BacktestEngine` behind a JSON-first CLI verb,
//! mirroring `wickd strategy run` for strategy selection. The engine simulates
//! trading the chosen strategy over historical candles and reports performance
//! metrics, the equity curve, and the simulated trades. JSON by default;
//! structured, non-panicking errors.
//!
//! `strategy` also accepts a Rhai script — see `commands::strategy` and
//! `commands::scripted` for the shared resolution rules (explicit `.rhai`
//! path, or a bare name under `~/.wickd/strategies/`, with built-in names
//! keeping precedence).

use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use rust_decimal::Decimal;
use serde_json::{json, Value};

use wickd_core::backtest::costs;
use wickd_core::backtest::strategy::ExtendedSignal;
use wickd_core::backtest::{
    BacktestConfig, BacktestEngine, MovingAverageCrossover, RsiStrategy, Signal, Strategy,
};
use wickd_core::models::Candle;
use wickd_core::oanda::endpoints::{self, Granularity};

use crate::commands::{client, scripted, walk_forward};
use crate::vault_store;
use crate::output::{exit, Out};

#[derive(Args, Debug)]
pub struct BacktestArgs {
    /// Strategy to backtest: a built-in name (`ma-crossover`, `rsi`), a path to
    /// a `.rhai` script, or a bare name resolved under `~/.wickd/strategies/`.
    pub strategy: String,
    /// Instrument, e.g. EUR_USD.
    pub instrument: String,
    /// Candle granularity (M1, M5, M15, H1, H4, D, ...).
    #[arg(long, default_value = "H1")]
    pub granularity: String,
    /// Recent candle count (max 5000). Ignored when both --from and --to are set.
    #[arg(long, default_value_t = 500)]
    pub count: u32,
    /// Start of the date range (RFC3339, e.g. 2024-01-01T00:00:00Z).
    #[arg(long)]
    pub from: Option<String>,
    /// End of the date range (RFC3339). Requires --from.
    #[arg(long)]
    pub to: Option<String>,
    /// OANDA environment whose stored credentials are used.
    #[arg(long, default_value = "practice")]
    pub env: String,

    // --- backtest config overrides (defaults from BacktestConfig::default) ---
    /// Starting account balance.
    #[arg(long)]
    pub balance: Option<String>,
    /// Position size in units.
    #[arg(long)]
    pub position_size: Option<String>,
    /// Half-spread in pips charged per fill side (round-trip cost = 2×
    /// this value). Defaults to a per-instrument table of typical OANDA
    /// spreads; pass 0 for costless mid-to-mid fills.
    #[arg(long)]
    pub spread_pips: Option<String>,
    /// Indicator warmup: fetch this many candles BEFORE --from and feed them
    /// to the strategy without trading them (excluded from metrics/equity).
    /// Lets slow indicators (e.g. SMA-200) be warm at the window start
    /// instead of consuming the tested span. Requires --from and --to.
    #[arg(long, default_value_t = 0)]
    pub warmup: usize,

    // --- walk-forward mode ---
    /// Run walk-forward analysis: split the range into sequential in-sample /
    /// out-of-sample windows and re-optimize parameters per window.
    #[arg(long)]
    pub walk_forward: bool,
    /// [walk-forward] in-sample (training) window size in candles.
    #[arg(long, default_value_t = 250)]
    pub is_size: usize,
    /// [walk-forward] out-of-sample (test) window size in candles.
    #[arg(long, default_value_t = 50)]
    pub oos_size: usize,
    /// [walk-forward] window roll step in candles (defaults to --oos-size).
    #[arg(long)]
    pub wf_step: Option<usize>,
    /// [walk-forward] anchored mode: expanding in-sample window from a fixed start.
    #[arg(long)]
    pub anchored: bool,
    /// [walk-forward] cap on parameter combinations searched per window.
    #[arg(long, default_value_t = 512)]
    pub max_combos: usize,

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
    /// [rsi] overbought threshold.
    #[arg(long, default_value = "70")]
    pub overbought: String,
    /// [rsi] oversold threshold.
    #[arg(long, default_value = "30")]
    pub oversold: String,

    // --- scripted-strategy params ---
    /// [scripted] Override a script's `@parameters` default: `--set <id>=<value>`,
    /// repeatable. Validated against the script's declared parameters (unknown
    /// id or out-of-min/max value is an error). Effective values are echoed in
    /// the JSON result.
    #[arg(long = "set", value_name = "ID=VALUE")]
    pub set: Vec<String>,
}

pub async fn run(args: BacktestArgs, out: Out) -> ! {
    match run_backtest(args).await {
        Ok(value) => {
            out.ok(&value);
            std::process::exit(exit::OK);
        }
        Err(e) => {
            let msg = format!("{e:#}");
            let code = if msg.contains("keychain") || msg.contains("credentials") {
                exit::AUTH
            } else if msg.contains("strategy")
                || msg.contains("period")
                || msg.contains("granularity")
                || msg.contains("threshold")
                || msg.contains("balance")
                || msg.contains("position size")
                || msg.contains("walk-forward")
                || msg.contains("window")
                || msg.contains("parameter")
            {
                exit::VALIDATION
            } else {
                exit::OANDA
            };
            out.fail(code, "backtest_failed", msg);
        }
    }
}

/// Build a boxed strategy from the CLI name + params, validating without panicking.
/// (`MovingAverageCrossover::new` asserts `fast < slow`, so guard it here.)
///
/// Mirrors `commands::strategy::build_strategy`'s built-in matching (duplicated
/// for the skeleton so the two verbs stay independently editable — see that
/// module's comment). Scripted-strategy resolution is shared via
/// `commands::scripted` so both verbs apply the same precedence: an explicit
/// `.rhai` path first, then built-in names, then `~/.wickd/strategies/<name>.rhai`.
fn build_strategy(args: &BacktestArgs) -> Result<(Box<dyn Strategy>, Option<Value>)> {
    let overrides = scripted::parse_set_pairs(&args.set)?;

    if let Some(path) = scripted::resolve_explicit_script_path(&args.strategy) {
        let (strategy, effective) =
            scripted::load_scripted_strategy(&path, &args.instrument, &overrides)?;
        return Ok((strategy, Some(effective)));
    }

    // Built-ins take typed flags, not @parameters — a --set here is a mistake
    // that must not silently no-op.
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
                bail!("ma-crossover periods must be greater than 0");
            }
            if args.fast >= args.slow {
                bail!(
                    "ma-crossover fast period ({}) must be less than slow period ({})",
                    args.fast,
                    args.slow
                );
            }
            Ok((Box::new(MovingAverageCrossover::new(args.fast, args.slow)), None))
        }
        "rsi" => {
            ensure_no_set("rsi")?;
            if args.period == 0 {
                bail!("rsi period must be greater than 0");
            }
            let overbought = Decimal::from_str(&args.overbought)
                .map_err(|_| anyhow!("invalid overbought threshold '{}'", args.overbought))?;
            let oversold = Decimal::from_str(&args.oversold)
                .map_err(|_| anyhow!("invalid oversold threshold '{}'", args.oversold))?;
            if oversold >= overbought {
                bail!("rsi oversold threshold ({oversold}) must be less than overbought ({overbought})");
            }
            Ok((Box::new(RsiStrategy::new(args.period, overbought, oversold)), None))
        }
        other => {
            if let Some(path) = scripted::resolve_named_script_path(other)? {
                let (strategy, effective) =
                    scripted::load_scripted_strategy(&path, &args.instrument, &overrides)?;
                return Ok((strategy, Some(effective)));
            }
            bail!(
                "unknown strategy '{other}' (available: ma-crossover, rsi, \
                 or a .rhai script path/name under ~/.wickd/strategies/)"
            )
        }
    }
}

/// Build the engine config from CLI overrides, starting from the core defaults.
/// `pub(crate)` so the walk-forward path reuses the exact same config wiring.
pub(crate) fn build_config(args: &BacktestArgs) -> Result<BacktestConfig> {
    let mut config = BacktestConfig::default();
    config.instrument = args.instrument.clone();
    if let Some(balance) = &args.balance {
        let balance =
            Decimal::from_str(balance).map_err(|_| anyhow!("invalid balance '{balance}'"))?;
        if balance <= Decimal::ZERO {
            bail!("balance must be greater than 0");
        }
        config.initial_balance = balance;
    }
    if let Some(position_size) = &args.position_size {
        let position_size = Decimal::from_str(position_size)
            .map_err(|_| anyhow!("invalid position size '{position_size}'"))?;
        if position_size <= Decimal::ZERO {
            bail!("position size must be greater than 0");
        }
        config.position_size = position_size;
    }
    // Execution-cost model: pip size follows the instrument (0.01 for
    // JPY-quoted pairs), half-spread defaults to the per-instrument table
    // unless overridden.
    config.pip_value = costs::pip_value_for(&args.instrument);
    config.spread_pips = match &args.spread_pips {
        Some(s) => {
            let v = Decimal::from_str(s).map_err(|_| anyhow!("invalid --spread-pips '{s}'"))?;
            if v < Decimal::ZERO {
                bail!("--spread-pips must be >= 0");
            }
            v
        }
        None => costs::default_half_spread_pips(&args.instrument),
    };
    Ok(config)
}

/// Approximate calendar seconds per candle, used only to over-estimate how
/// far before `--from` to start the warmup fetch (weekend gaps are covered
/// by the 1.6× factor at the call site; the exact cut is done on the fetched
/// candles themselves).
fn approx_candle_secs(gran: Granularity) -> i64 {
    match gran {
        Granularity::S5 => 5,
        Granularity::S10 => 10,
        Granularity::S15 => 15,
        Granularity::S30 => 30,
        Granularity::M1 => 60,
        Granularity::M2 => 120,
        Granularity::M4 => 240,
        Granularity::M5 => 300,
        Granularity::M10 => 600,
        Granularity::M15 => 900,
        Granularity::M30 => 1800,
        Granularity::H1 => 3600,
        Granularity::H2 => 7200,
        Granularity::H3 => 10800,
        Granularity::H4 => 14400,
        Granularity::H6 => 21600,
        Granularity::H8 => 28800,
        Granularity::H12 => 43200,
        Granularity::D => 86400,
        Granularity::W => 604800,
        Granularity::M => 2_592_000,
    }
}

async fn run_backtest(args: BacktestArgs) -> Result<Value> {
    let gran = Granularity::from_str(&args.granularity)
        .map_err(|e| anyhow!("invalid granularity: {e}"))?;

    if args.to.is_some() && args.from.is_none() {
        bail!("--to requires --from");
    }

    // Validate + construct everything before any network call. In walk-forward
    // mode the per-window search builds its own strategies, so we skip the
    // single-strategy construction here (but still fail fast on a bad name via
    // the search-space builder inside `walk_forward::run`).
    let mut strategy = if args.walk_forward {
        if !args.set.is_empty() {
            bail!("'--set' parameter overrides conflict with --walk-forward (the per-window search optimizes parameters)");
        }
        if args.warmup > 0 {
            bail!("--warmup conflicts with --walk-forward (walk-forward manages its own windowing)");
        }
        None
    } else {
        Some(build_strategy(&args)?)
    };
    if args.warmup > 0 && (args.from.is_none() || args.to.is_none()) {
        bail!("--warmup requires both --from and --to");
    }
    let mut config = build_config(&args)?;

    let (_env, client) = client::resolve(&args.env, vault_store::DEFAULT_ACCOUNT)?;

    // A full date range paginates past OANDA's 5000-calendar-bar cap (which
    // counts weekends — H1 tops out around ~7 months per request): chunked
    // requests, stitched and deduped, bounded client-side by `to` (#292).
    // Otherwise fetch the most recent N candles in one request.
    let candles = if let (Some(from), Some(to)) = (args.from.as_deref(), args.to.as_deref()) {
        if args.warmup > 0 {
            // Fetch a lead-in before `from` so indicators are warm at the
            // window start. Over-fetch on calendar time (×1.6 + 3 days covers
            // weekends/holidays), then cut to exactly `warmup` lead-in
            // candles on the actual data.
            let from_dt = chrono::DateTime::parse_from_rfc3339(from)
                .map_err(|e| anyhow!("invalid --from '{from}': {e}"))?
                .with_timezone(&chrono::Utc);
            let lead_secs = (approx_candle_secs(gran) * args.warmup as i64 * 16) / 10
                + 3 * 86_400;
            let fetch_from = (from_dt - chrono::Duration::seconds(lead_secs)).to_rfc3339();
            let all = endpoints::get_candles_paginated(
                &client,
                &args.instrument,
                gran,
                &fetch_from,
                to,
            )
            .await?;
            // First candle at/after the requested window start.
            let window_start = all.partition_point(|c| c.time < from_dt);
            let lead_start = window_start.saturating_sub(args.warmup);
            config.warmup_bars = window_start - lead_start;
            if config.warmup_bars < args.warmup {
                eprintln!(
                    "warning: only {} of {} requested warmup candles available before --from",
                    config.warmup_bars, args.warmup
                );
            }
            Ok(all[lead_start..].to_vec())
        } else {
            endpoints::get_candles_paginated(&client, &args.instrument, gran, from, to).await
        }
    } else {
        // from-only or neither (--to without --from is rejected above, so
        // `to` is always None here; forwarded for self-evident parity).
        endpoints::get_candles(
            &client,
            &args.instrument,
            gran,
            Some(args.count.min(5000)),
            args.from.as_deref(),
            args.to.as_deref(),
        )
        .await
    }
    .with_context(|| "OANDA candle fetch failed")?;

    if candles.is_empty() {
        bail!("no candle data returned for the requested instrument/range");
    }

    // Walk-forward mode reports per-window IS/OOS metrics with per-window
    // parameter re-optimization; the single-shot path below is the default.
    if args.walk_forward {
        return walk_forward::run(&args, config, &candles);
    }

    let (strategy, effective_params) = strategy
        .as_mut()
        .expect("single-strategy path always constructs a strategy");
    // Echo the cost/warmup model so every run is self-describing.
    let spread_pips = config.spread_pips;
    let pip_value = config.pip_value;
    let warmup_bars = config.warmup_bars;
    let result = run_engine(strategy.as_mut(), config, &candles);

    let mut out = json!({
        "strategy": args.strategy,
        "instrument": args.instrument,
        "granularity": args.granularity,
        "candles": candles.len(),
        "spreadPips": spread_pips,
        "pipValue": pip_value,
        "warmupBars": warmup_bars,
        "metrics": result.metrics,
        "finalBalance": result.final_balance,
        "equityCurve": result.equity_curve,
        "trades": result.trades,
    });
    // Scripted runs are self-describing: echo the effective parameter values
    // (defaults merged with any --set overrides).
    if let Some(params) = effective_params.take() {
        if let Some(obj) = out.as_object_mut() {
            obj.insert("params".to_string(), params);
        }
    }
    Ok(out)
}

/// `BacktestEngine::run<S: Strategy>` carries an implicit `Sized` bound on `S`,
/// so it can't accept a `&mut dyn Strategy` directly. This zero-cost newtype is
/// the concrete (`Sized`) strategy the engine monomorphizes over; it forwards
/// every trait call to the boxed strategy `build_strategy` returns.
struct StrategyRef<'a>(&'a mut dyn Strategy);

impl Strategy for StrategyRef<'_> {
    fn prepare(&mut self, candles: &[Candle]) {
        self.0.prepare(candles)
    }
    fn on_candle(&mut self, candle: &Candle) -> Signal {
        self.0.on_candle(candle)
    }
    fn on_candle_extended(&mut self, candle: &Candle) -> ExtendedSignal {
        self.0.on_candle_extended(candle)
    }
    fn current_stop_loss(&self) -> Option<Decimal> {
        self.0.current_stop_loss()
    }
    fn current_take_profit(&self) -> Option<Decimal> {
        self.0.current_take_profit()
    }
    fn notify_position_closed(&mut self) {
        self.0.notify_position_closed()
    }
    fn notify_entry_rejected(&mut self) {
        self.0.notify_entry_rejected()
    }
    fn name(&self) -> &str {
        self.0.name()
    }
    fn reset(&mut self) {
        self.0.reset()
    }
}

/// Run the core backtest engine. Pulled out of `run_backtest` so the
/// subcommand → core wiring is unit-testable without a network call.
/// `pub(crate)` so the walk-forward path drives the same engine per window.
pub(crate) fn run_engine(
    strategy: &mut dyn Strategy,
    config: BacktestConfig,
    candles: &[Candle],
) -> wickd_core::backtest::BacktestResult {
    let engine = BacktestEngine::new(config);
    let mut wrapped = StrategyRef(strategy);
    engine.run(&mut wrapped, candles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wickd_core::models::Ohlc;
    use chrono::{TimeZone, Utc};

    fn backtest_args(strategy: &str) -> BacktestArgs {
        BacktestArgs {
            strategy: strategy.to_string(),
            instrument: "EUR_USD".to_string(),
            granularity: "H1".to_string(),
            count: 500,
            from: None,
            to: None,
            env: "practice".to_string(),
            balance: None,
            position_size: None,
            spread_pips: None,
            warmup: 0,
            walk_forward: false,
            is_size: 250,
            oos_size: 50,
            wf_step: None,
            anchored: false,
            max_combos: 512,
            fast: 10,
            slow: 30,
            period: 14,
            overbought: "70".to_string(),
            oversold: "30".to_string(),
            set: vec![],
        }
    }

    fn candle(close: &str, i: i64) -> Candle {
        let price = Decimal::from_str(close).unwrap();
        Candle {
            time: Utc.timestamp_opt(1_700_000_000 + i * 3600, 0).unwrap(),
            mid: Ohlc {
                open: price,
                high: price,
                low: price,
                close: price,
            },
            volume: 1,
            complete: true,
        }
    }

    /// Synthesize a swinging price series so the strategy actually fires.
    fn sample_candles() -> Vec<Candle> {
        let prices = [
            "1.00", "1.01", "1.03", "1.06", "1.10", "1.13", "1.10", "1.06", "1.02", "0.99",
            "0.97", "0.98", "1.01", "1.05", "1.09", "1.12", "1.08", "1.03", "0.99", "0.96",
        ];
        prices
            .iter()
            .enumerate()
            .map(|(i, p)| candle(p, i as i64))
            .collect()
    }

    #[test]
    fn build_rejects_unknown_strategy() {
        assert!(build_strategy(&backtest_args("nope")).is_err());
    }

    #[test]
    fn build_rejects_fast_ge_slow_without_panicking() {
        let mut args = backtest_args("ma-crossover");
        args.fast = 30;
        args.slow = 10;
        assert!(build_strategy(&args).is_err());
    }

    #[test]
    fn build_rejects_inverted_rsi_thresholds() {
        let mut args = backtest_args("rsi");
        args.overbought = "30".to_string();
        args.oversold = "70".to_string();
        assert!(build_strategy(&args).is_err());
    }

    /// A `.rhai` file under the OS temp dir, deleted when it drops.
    struct TempScript(std::path::PathBuf);

    impl TempScript {
        fn new(contents: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let mut p = std::env::temp_dir();
            p.push(format!("wickd-backtest-test-{pid}-{nanos}-{n}.rhai"));
            std::fs::write(&p, contents).expect("write temp script");
            Self(p)
        }
    }

    impl Drop for TempScript {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn build_strategy_rejects_a_malformed_rhai_script_without_panicking() {
        let script = TempScript::new(
            r#"
fn on_candle( {
    "buy"
}
"#,
        );
        let args = backtest_args(script.0.to_str().unwrap());
        let err = build_strategy(&args).err().unwrap();
        assert!(format!("{err:#}").contains("invalid strategy script"));
    }

    #[test]
    fn backtest_runs_a_scripted_strategy_through_the_engine_with_the_same_result_shape() {
        // AC 2: a .rhai script produces the identical metric struct/output
        // shape as built-in strategies — same BacktestResult, just a
        // different Strategy impl feeding it.
        let script = TempScript::new(
            r#"
let counter = 0;

fn on_candle() {
    counter += 1;
    if counter % 4 == 0 {
        return #{ signal: "buy" };
    }
    if counter % 4 == 2 {
        return #{ signal: "close" };
    }
    #{ signal: "hold" }
}
"#,
        );
        let args = backtest_args(script.0.to_str().unwrap());
        let (mut strategy, _params) = build_strategy(&args).unwrap();
        let config = build_config(&args).unwrap();
        let candles = sample_candles();

        let result = run_engine(strategy.as_mut(), config, &candles);

        assert!(!result.equity_curve.is_empty());
        assert_eq!(
            result.metrics.total_trades,
            result.metrics.winning_trades + result.metrics.losing_trades
        );
        assert_eq!(result.trades.len(), result.metrics.total_trades as usize);
    }

    #[test]
    fn config_rejects_non_positive_balance() {
        let mut args = backtest_args("ma-crossover");
        args.balance = Some("0".to_string());
        assert!(build_config(&args).is_err());
    }

    #[test]
    fn config_overrides_apply() {
        let mut args = backtest_args("ma-crossover");
        args.balance = Some("25000".to_string());
        args.position_size = Some("500".to_string());
        let config = build_config(&args).unwrap();
        assert_eq!(config.initial_balance, Decimal::from_str("25000").unwrap());
        assert_eq!(config.position_size, Decimal::from_str("500").unwrap());
    }

    #[test]
    fn backtest_wires_candles_into_core() {
        // Subcommand → core wiring: a valid strategy + config drive the engine
        // over candles and produce a well-formed result.
        let (mut strategy, _params) = build_strategy(&{
            let mut a = backtest_args("ma-crossover");
            a.fast = 2;
            a.slow = 4;
            a
        })
        .unwrap();
        let config = build_config(&backtest_args("ma-crossover")).unwrap();
        let candles = sample_candles();

        let result = run_engine(strategy.as_mut(), config, &candles);

        // Equity curve starts with the initial balance and grows one entry per
        // (at least the seed) — never empty.
        assert!(!result.equity_curve.is_empty());
        // The trade tally is internally consistent...
        assert_eq!(
            result.metrics.total_trades,
            result.metrics.winning_trades + result.metrics.losing_trades
        );
        // ...and the recorded trades match the metric count (subcommand fed the
        // candles all the way through the engine).
        assert_eq!(result.trades.len(), result.metrics.total_trades as usize);
    }

    #[test]
    fn backtest_runs_with_default_config() {
        // Default config (no overrides) must drive the engine without panicking.
        let (mut strategy, _params) = build_strategy(&backtest_args("rsi")).unwrap();
        let result = run_engine(
            strategy.as_mut(),
            BacktestConfig::default(),
            &sample_candles(),
        );
        // Default initial balance is 10000; with no closed trades the equity seed holds.
        assert_eq!(result.equity_curve.first().copied(), Some(Decimal::from(10000)));
    }
}
