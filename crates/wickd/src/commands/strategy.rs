//! `wickd strategy` — list built-in strategies, validate a script, and run one
//! over OANDA candles.
//!
//!   wickd strategy list
//!   wickd strategy validate ./my-strategy.rhai      # JSON: valid, score, errors[]
//!   wickd strategy validate my-strategy             # looks up ~/.wickd/strategies/my-strategy.rhai
//!   wickd strategy run ma-crossover EUR_USD --fast 10 --slow 30 --granularity H1 --count 500
//!   wickd strategy run rsi EUR_USD --period 14 --overbought 70 --oversold 30
//!   wickd strategy run ./my-strategy.rhai EUR_USD --granularity H1 --count 500
//!   wickd strategy run my-strategy EUR_USD          # looks up ~/.wickd/strategies/my-strategy.rhai
//!
//! `strategy validate` is the agent-authoring surface: it compiles a `.rhai`
//! script through `wickd-core`'s `validate_script` (the same check the
//! run/backtest paths apply before loading) and reports the result as a clean
//! JSON object — `valid`, a `score`, an `errors` array, `warnings`, and the
//! parsed `@indicators`/`@parameters` metadata — so a driver agent can iterate
//! on a script without parsing prose. See `STRATEGY_ABI.md` ("Agent authoring
//! loop") for the generate → validate → backtest → iterate workflow.
//!
//! The strategy engine lives in `wickd-core`; this is the CLI surface so a
//! local agent (or the `wickd watch` daemon) can list and run strategies and reason
//! over the JSON signals. JSON by default; structured, non-panicking errors.
//!
//! `strategy` also accepts a Rhai script — an explicit `.rhai` path, or a bare
//! name resolved under `~/.wickd/strategies/` — via `commands::scripted`, which
//! loads it through `wickd-core`'s `ScriptedStrategy`/`validate_script`.

use std::path::Path;
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use rust_decimal::Decimal;
use serde_json::{json, Value};

use wickd_core::backtest::{
    validate_script_typed, MovingAverageCrossover, RsiStrategy, ScriptValidationError, Signal,
    Strategy,
};
use wickd_core::models::Candle;
use wickd_core::oanda::endpoints::{self, Granularity};

use crate::commands::{client, scripted};
use crate::vault_store;
use crate::output::{exit, Out};

#[derive(Args, Debug)]
pub struct StrategyArgs {
    #[command(subcommand)]
    command: StrategyCommand,
}

#[derive(Subcommand, Debug)]
enum StrategyCommand {
    /// List the built-in strategies and every `.rhai` strategy in the store
    /// (`~/.wickd/strategies/`), with parsed metadata.
    List,
    /// Validate a `.rhai` strategy script → JSON { valid, score, errors[] }.
    Validate(ValidateArgs),
    /// Run a strategy over OANDA candles and emit its signals as JSON.
    Run(RunArgs),
    /// Add a `.rhai` script to the store (validates first; refuses to
    /// overwrite an existing name — use `update` for that).
    Add(AddArgs),
    /// Print a stored strategy: metadata + full source.
    Show(ShowArgs),
    /// Replace an existing stored strategy with a new script (validates
    /// first; the name must already exist — use `add` for new strategies).
    Update(UpdateArgs),
    /// Remove a strategy from the store.
    Remove(RemoveArgs),
    /// Convert rules-JSON strategy definitions (the retired visual-builder
    /// format) into `.rhai` scripts, validating each result.
    Convert(ConvertArgs),
}

#[derive(Args, Debug)]
pub struct ValidateArgs {
    /// Strategy to validate: a path to a `.rhai` script, a bare name resolved
    /// under `~/.wickd/strategies/`, or a built-in name (`ma-crossover`, `rsi`).
    /// No network access — this only compiles and inspects the script.
    pub strategy: String,
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Strategy to run: a built-in name (`ma-crossover`, `rsi`), a path to a
    /// `.rhai` script, or a bare name resolved under `~/.wickd/strategies/`.
    pub strategy: String,
    /// Instrument, e.g. EUR_USD.
    pub instrument: String,
    /// Candle granularity (M1, M5, M15, H1, H4, D, ...).
    #[arg(long, default_value = "H1")]
    pub granularity: String,
    /// Recent candle count (max 5000).
    #[arg(long, default_value_t = 500)]
    pub count: u32,
    /// OANDA environment whose stored credentials are used.
    #[arg(long, default_value = "practice")]
    pub env: String,
    /// Include `hold` signals in the output (default: only buy/sell).
    #[arg(long)]
    pub include_holds: bool,

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

#[derive(Args, Debug)]
pub struct AddArgs {
    /// Path to the `.rhai` script to add (use `-` to read from stdin).
    pub file: String,
    /// Store name (defaults to the file's stem). Slug characters only:
    /// ASCII letters, digits, `-`, `_`, `.`.
    #[arg(long)]
    pub name: Option<String>,
    /// Replace the strategy if the name already exists.
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct ShowArgs {
    /// Stored strategy name (as shown by `strategy list`).
    pub name: String,
}

#[derive(Args, Debug)]
pub struct UpdateArgs {
    /// Stored strategy name to replace.
    pub name: String,
    /// Path to the new `.rhai` script (use `-` to read from stdin).
    pub file: String,
}

#[derive(Args, Debug)]
pub struct RemoveArgs {
    /// Stored strategy name to remove.
    pub name: String,
}

#[derive(Args, Debug)]
pub struct ConvertArgs {
    /// Path to a JSON file holding one rules-JSON StrategyDefinition or an
    /// array of them (e.g. extracted from the CandleSight archive).
    pub file: String,
    /// Directory to write the converted `.rhai` files into (created if
    /// missing). Deliberately explicit — nothing is written into the live
    /// store unless you point it there.
    #[arg(long = "out-dir", value_name = "DIR")]
    pub out_dir: String,
    /// Overwrite existing `.rhai` files in the output directory.
    #[arg(long)]
    pub force: bool,
}

pub async fn run(args: StrategyArgs, out: Out) -> ! {
    match args.command {
        StrategyCommand::List => match list_strategies() {
            Ok(value) => {
                out.ok(&value);
                std::process::exit(exit::OK);
            }
            Err(e) => out.fail(exit::VALIDATION, "strategy_list_failed", format!("{e:#}")),
        },
        StrategyCommand::Validate(validate_args) => match validate_strategy(&validate_args) {
            // A script that fails to compile is a *valid* validation result —
            // `{ valid: false, score: 0, errors: [...] }` at exit 0 — so the
            // driving agent reads structured errors instead of an error
            // envelope. Only usage errors (unknown name, unreadable file) take
            // the `exit::VALIDATION` error-envelope path below.
            Ok(value) => {
                out.ok(&value);
                std::process::exit(exit::OK);
            }
            Err(e) => out.fail(exit::VALIDATION, "strategy_validate_failed", format!("{e:#}")),
        },
        StrategyCommand::Run(run_args) => match run_strategy(run_args).await {
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
                {
                    exit::VALIDATION
                } else {
                    exit::OANDA
                };
                out.fail(code, "strategy_failed", msg);
            }
        },
        StrategyCommand::Add(add_args) => match add_strategy(&add_args) {
            Ok(value) => {
                out.ok(&value);
                std::process::exit(exit::OK);
            }
            Err(e) => out.fail(exit::VALIDATION, "strategy_add_failed", format!("{e:#}")),
        },
        StrategyCommand::Show(show_args) => match show_strategy(&show_args) {
            Ok(value) => {
                out.ok(&value);
                std::process::exit(exit::OK);
            }
            Err(e) => out.fail(exit::VALIDATION, "strategy_show_failed", format!("{e:#}")),
        },
        StrategyCommand::Update(update_args) => match update_strategy(&update_args) {
            Ok(value) => {
                out.ok(&value);
                std::process::exit(exit::OK);
            }
            Err(e) => out.fail(exit::VALIDATION, "strategy_update_failed", format!("{e:#}")),
        },
        StrategyCommand::Remove(remove_args) => match remove_strategy(&remove_args) {
            Ok(value) => {
                out.ok(&value);
                std::process::exit(exit::OK);
            }
            Err(e) => out.fail(exit::VALIDATION, "strategy_remove_failed", format!("{e:#}")),
        },
        StrategyCommand::Convert(convert_args) => match convert_strategies(&convert_args) {
            Ok(value) => {
                out.ok(&value);
                std::process::exit(exit::OK);
            }
            Err(e) => out.fail(exit::VALIDATION, "strategy_convert_failed", format!("{e:#}")),
        },
    }
}

// ============================================================================
// Store lifecycle verbs (AGT-651): add / show / update / remove / convert
// ============================================================================

fn open_store() -> Result<wickd_core::strategy_store::StrategyStore> {
    wickd_core::strategy_store::StrategyStore::open_default().map_err(anyhow::Error::msg)
}

/// Read a script argument: a file path, or `-` for stdin.
fn read_script_arg(file: &str) -> Result<String> {
    if file == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading script from stdin")?;
        Ok(buf)
    } else {
        std::fs::read_to_string(file)
            .with_context(|| format!("failed to read strategy script '{file}'"))
    }
}

fn stored_entry_json(entry: &wickd_core::strategy_store::StoredStrategy) -> Value {
    serde_json::to_value(entry).unwrap_or(Value::Null)
}

fn add_strategy(args: &AddArgs) -> Result<Value> {
    let script = read_script_arg(&args.file)?;
    let name = match &args.name {
        Some(n) => n.clone(),
        None if args.file == "-" => bail!("--name is required when reading from stdin"),
        None => Path::new(&args.file)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(String::from)
            .ok_or_else(|| anyhow!("could not derive a name from '{}'; pass --name", args.file))?,
    };
    let store = open_store()?;
    let entry = store
        .add(&name, &script, args.force)
        .map_err(anyhow::Error::msg)?;
    Ok(json!({ "added": stored_entry_json(&entry) }))
}

fn show_strategy(args: &ShowArgs) -> Result<Value> {
    let store = open_store()?;
    let (entry, source) = store
        .read(&args.name)
        .map_err(anyhow::Error::msg)?
        .ok_or_else(|| {
            anyhow!(
                "no stored strategy '{}' (looked for {})",
                args.name,
                store.path_for(&args.name).display()
            )
        })?;
    let mut v = stored_entry_json(&entry);
    if let Value::Object(map) = &mut v {
        map.insert("source".into(), Value::String(source));
    }
    Ok(v)
}

fn update_strategy(args: &UpdateArgs) -> Result<Value> {
    let store = open_store()?;
    if store.read(&args.name).map_err(anyhow::Error::msg)?.is_none() {
        bail!(
            "no stored strategy '{}' to update (use `wickd strategy add` for new strategies)",
            args.name
        );
    }
    let script = read_script_arg(&args.file)?;
    let entry = store
        .add(&args.name, &script, true)
        .map_err(anyhow::Error::msg)?;
    Ok(json!({ "updated": stored_entry_json(&entry) }))
}

fn remove_strategy(args: &RemoveArgs) -> Result<Value> {
    let store = open_store()?;
    let removed = store.remove(&args.name).map_err(anyhow::Error::msg)?;
    if !removed {
        bail!("no stored strategy '{}' to remove", args.name);
    }
    Ok(json!({ "removed": args.name }))
}

fn convert_strategies(args: &ConvertArgs) -> Result<Value> {
    let raw = std::fs::read_to_string(&args.file)
        .with_context(|| format!("failed to read definitions file '{}'", args.file))?;
    crate::convert::convert_file(&raw, Path::new(&args.out_dir), args.force)
}

/// JSON describing the built-in strategies plus every `.rhai` strategy in
/// the unified store (`~/.wickd/strategies/`, AGT-651). The `strategies`
/// (built-ins) and `scripted` fields keep their historical shape; `store`
/// and `store_dir` are additive.
fn list_strategies() -> Result<Value> {
    let store = open_store()?;
    let entries = store.list().map_err(anyhow::Error::msg)?;
    let mut v = builtin_strategies_json();
    if let Value::Object(map) = &mut v {
        map.insert(
            "store".into(),
            Value::Array(entries.iter().map(stored_entry_json).collect()),
        );
        map.insert(
            "store_dir".into(),
            Value::String(store.root().display().to_string()),
        );
    }
    Ok(v)
}

/// JSON describing the built-in strategies exposed by the CLI.
fn builtin_strategies_json() -> Value {
    json!({
        "strategies": [
            {
                "name": "ma-crossover",
                "description": "Moving-average crossover: buy when the fast MA crosses above the slow MA, sell on the reverse.",
                "params": [
                    {"name": "fast", "type": "usize", "default": 10},
                    {"name": "slow", "type": "usize", "default": 30, "note": "must be greater than fast"}
                ]
            },
            {
                "name": "rsi",
                "description": "RSI overbought/oversold: buy below the oversold threshold, sell above the overbought threshold.",
                "params": [
                    {"name": "period", "type": "usize", "default": 14},
                    {"name": "overbought", "type": "decimal", "default": "70"},
                    {"name": "oversold", "type": "decimal", "default": "30"}
                ]
            }
        ],
        "scripted": "`strategy run`/`backtest` also accept a Rhai .rhai script: an explicit file path, or a bare name resolved under ~/.wickd/strategies/<name>.rhai."
    })
}

// ============================================================================
// `strategy validate` — the agent-authoring validation surface
// ============================================================================

/// Resolve `args.strategy` (script path, bare name, or built-in) and return a
/// clean JSON validation report. Reuses the exact resolution precedence of
/// `build_strategy`/`commands::scripted` (explicit `.rhai` path → built-in
/// name → `~/.wickd/strategies/<name>.rhai`) so `validate` and `run` agree on
/// what a given argument refers to.
///
/// A script that *compiles cleanly* and a script that *fails to compile* both
/// return `Ok(Value)` — the report's `valid` field distinguishes them — so the
/// caller emits structured errors at exit 0. `Err` is reserved for usage
/// failures: an unknown strategy name, or a script file that can't be read.
fn validate_strategy(args: &ValidateArgs) -> Result<Value> {
    if let Some(path) = scripted::resolve_explicit_script_path(&args.strategy) {
        return validate_script_at(&args.strategy, &path);
    }

    match args.strategy.to_lowercase().as_str() {
        "ma-crossover" | "ma" => Ok(builtin_validation(&args.strategy, "ma-crossover")),
        "rsi" => Ok(builtin_validation(&args.strategy, "rsi")),
        other => {
            if let Some(path) = scripted::resolve_named_script_path(other)? {
                return validate_script_at(&args.strategy, &path);
            }
            bail!(
                "unknown strategy '{other}' (available: ma-crossover, rsi, \
                 or a .rhai script path/name under ~/.wickd/strategies/)"
            )
        }
    }
}

/// Read a `.rhai` file and turn `validate_script`'s result into the JSON report.
/// A read failure is a usage error (`Err`); a compile failure is a valid report
/// with `valid: false`.
fn validate_script_at(strategy_arg: &str, path: &Path) -> Result<Value> {
    let script = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read strategy script '{}'", path.display()))?;
    Ok(build_script_validation(strategy_arg, path, &script))
}

/// Assemble the JSON validation report for a script's source. Shared shape:
/// `{ strategy, kind, path, valid, score, errors[], warnings[], metadata? }`.
fn build_script_validation(strategy_arg: &str, path: &Path, script: &str) -> Value {
    match validate_script_typed(script) {
        Ok(metadata) => {
            // Warnings are non-fatal authoring smells an agent may want to fix.
            let mut warnings = Vec::new();
            if metadata.parameters.is_empty() {
                warnings.push(json!({
                    "code": "no_parameters",
                    "message": "script declares no @parameters; it cannot be tuned or \
                                walk-forward-optimized (backtest still runs with fixed logic)",
                }));
            }
            // Score: 100 for a clean compile, minus 10 per warning (floored at 0).
            // Deterministic so an agent can rank variants by it.
            let score = 100u32.saturating_sub(10 * warnings.len() as u32);
            json!({
                "strategy": strategy_arg,
                "kind": "script",
                "path": path.display().to_string(),
                "valid": true,
                "score": score,
                "errors": [],
                "warnings": warnings,
                "metadata": {
                    "indicators": metadata.indicators,
                    "parameters": metadata.parameters,
                },
            })
        }
        Err(e) => {
            // Match on the typed variant — no substring-matching a human
            // message — so the agent-facing `code` can't silently drift when
            // core's error wording changes. The `Display` message is preserved
            // verbatim alongside the code.
            let code = match e {
                ScriptValidationError::Compile(_) => "compile_error",
                ScriptValidationError::MissingOnCandle => "missing_on_candle",
                ScriptValidationError::Metadata(_) => "metadata_error",
            };
            json!({
                "strategy": strategy_arg,
                "kind": "script",
                "path": path.display().to_string(),
                "valid": false,
                "score": 0,
                "errors": [ { "code": code, "message": e.to_string() } ],
                "warnings": [],
            })
        }
    }
}

/// Built-in strategies are compiled Rust, so they're always valid — report a
/// score of 100 with no errors so the same JSON shape covers every strategy an
/// agent might name.
fn builtin_validation(strategy_arg: &str, canonical: &str) -> Value {
    json!({
        "strategy": strategy_arg,
        "kind": "builtin",
        "name": canonical,
        "valid": true,
        "score": 100,
        "errors": [],
        "warnings": [],
    })
}

/// Build a boxed strategy from the CLI name + params, validating without panicking.
/// (`MovingAverageCrossover::new` asserts `fast < slow`, so guard it here.)
///
/// `args.strategy` may be a built-in name, an explicit `.rhai` script path, or
/// a bare name resolved under `~/.wickd/strategies/`. An unambiguous script
/// reference (existing literal path, or a `.rhai` suffix) is checked first;
/// built-in names are matched next so they keep existing behavior unchanged;
/// only names that match neither fall back to the `~/.wickd/strategies/`
/// lookup, so a script can never silently shadow a built-in.
fn build_strategy(args: &RunArgs) -> Result<(Box<dyn Strategy>, Option<Value>)> {
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

async fn run_strategy(args: RunArgs) -> Result<Value> {
    let gran = Granularity::from_str(&args.granularity)
        .map_err(|e| anyhow!("invalid granularity: {e}"))?;

    // Validate + construct the strategy before any network call.
    let (mut strategy, effective_params) = build_strategy(&args)?;

    let (_env, client) = client::resolve(&args.env, vault_store::DEFAULT_ACCOUNT)?;
    let candles = endpoints::get_candles(
        &client,
        &args.instrument,
        gran,
        Some(args.count.min(5000)),
        None,
        None,
    )
    .await
    .with_context(|| "OANDA candle fetch failed")?;

    let eval = evaluate(strategy.as_mut(), &candles, args.include_holds);

    let mut out = json!({
        "strategy": args.strategy,
        "instrument": args.instrument,
        "granularity": args.granularity,
        "candles": candles.len(),
        "signals": eval.signals,
        "summary": { "buy": eval.buy, "sell": eval.sell, "close": eval.close, "hold": eval.hold },
    });
    // Scripted runs are self-describing: echo the effective parameter values
    // (defaults merged with any --set overrides).
    if let Some(params) = effective_params {
        if let Some(obj) = out.as_object_mut() {
            obj.insert("params".to_string(), params);
        }
    }
    Ok(out)
}

struct Evaluated {
    signals: Vec<Value>,
    buy: usize,
    sell: usize,
    close: usize,
    hold: usize,
}

/// Feed each candle to the strategy, collecting non-Hold signals (or all, with
/// `include_holds`) plus per-signal tallies. Single pass — the strategy is stateful.
fn evaluate(strategy: &mut dyn Strategy, candles: &[Candle], include_holds: bool) -> Evaluated {
    let mut out = Evaluated {
        signals: Vec::new(),
        buy: 0,
        sell: 0,
        close: 0,
        hold: 0,
    };
    for candle in candles {
        let signal = strategy.on_candle(candle);
        match signal {
            Signal::Buy => out.buy += 1,
            Signal::Sell => out.sell += 1,
            Signal::ClosePosition => out.close += 1,
            Signal::Hold => out.hold += 1,
        }
        if matches!(signal, Signal::Hold) && !include_holds {
            continue;
        }
        out.signals.push(json!({
            "time": candle.time.to_rfc3339(),
            "close": candle.mid.close.to_string(),
            "signal": signal_str(signal),
        }));
    }
    out
}

fn signal_str(signal: Signal) -> &'static str {
    match signal {
        Signal::Buy => "buy",
        Signal::Sell => "sell",
        Signal::ClosePosition => "close",
        Signal::Hold => "hold",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wickd_core::models::Ohlc;
    use chrono::{TimeZone, Utc};

    fn run_args(strategy: &str) -> RunArgs {
        RunArgs {
            strategy: strategy.to_string(),
            instrument: "EUR_USD".to_string(),
            granularity: "H1".to_string(),
            count: 500,
            env: "practice".to_string(),
            include_holds: false,
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

    #[test]
    fn list_exposes_the_builtins() {
        // builtin_strategies_json is the pure part of `strategy list`; the
        // store section is covered by the StrategyStore tests in wickd-core.
        let v = builtin_strategies_json();
        let names: Vec<&str> = v["strategies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"ma-crossover"));
        assert!(names.contains(&"rsi"));
    }

    #[test]
    fn build_rejects_unknown_strategy() {
        assert!(build_strategy(&run_args("nope")).is_err());
    }

    #[test]
    fn build_rejects_fast_ge_slow_without_panicking() {
        let mut args = run_args("ma-crossover");
        args.fast = 30;
        args.slow = 10;
        assert!(build_strategy(&args).is_err());
    }

    #[test]
    fn build_rejects_inverted_rsi_thresholds() {
        let mut args = run_args("rsi");
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
            p.push(format!("wickd-strategy-test-{pid}-{nanos}-{n}.rhai"));
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
    fn build_strategy_loads_a_valid_rhai_script_by_explicit_path() {
        let script = TempScript::new(
            r#"
fn on_candle() {
    "buy"
}
"#,
        );
        let args = run_args(script.0.to_str().unwrap());
        let (mut strategy, _params) = build_strategy(&args).unwrap();
        let candle = candle("1.1000", 0);
        assert_eq!(strategy.on_candle(&candle), Signal::Buy);
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
        let args = run_args(script.0.to_str().unwrap());
        let err = build_strategy(&args).err().unwrap();
        assert!(format!("{err:#}").contains("invalid strategy script"));
    }

    #[test]
    fn build_strategy_still_uses_the_builtin_when_name_matches() {
        // A bare "rsi" with no matching ~/.wickd/strategies/rsi.rhai on the
        // test box must still resolve to the built-in strategy.
        let (strategy, _params) = build_strategy(&run_args("rsi")).unwrap();
        assert_eq!(strategy.name(), "RSI Strategy");
    }

    // --- `strategy validate` JSON surface (AGT-609 AC1) --------------------

    #[test]
    fn validate_reports_a_clean_valid_result_with_metadata_for_a_good_script() {
        // A script with declared @parameters validates cleanly: valid, score 100,
        // no warnings, and the parsed parameter metadata is echoed so an agent
        // knows what's tunable. These are the exact fields the agent depends on.
        let script = TempScript::new(
            r#"
// @parameters: [
//   { "id": "period", "name": "Period", "type": "integer", "default": 10, "min": 4, "max": 20, "step": 2 }
// ]
fn on_candle() {
    "hold"
}
"#,
        );
        let args = ValidateArgs {
            strategy: script.0.to_str().unwrap().to_string(),
        };
        let v = validate_strategy(&args).unwrap();
        assert_eq!(v["valid"], serde_json::json!(true));
        assert_eq!(v["kind"], "script");
        assert_eq!(v["score"], serde_json::json!(100));
        assert_eq!(v["errors"].as_array().unwrap().len(), 0);
        assert_eq!(v["warnings"].as_array().unwrap().len(), 0);
        let params = v["metadata"]["parameters"].as_array().unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0]["id"], "period");
        assert_eq!(params[0]["type"], "integer");
    }

    #[test]
    fn validate_warns_and_docks_score_when_a_script_declares_no_parameters() {
        // Valid but un-tunable: still valid, but a `no_parameters` warning and a
        // score of 90 tell the agent the script can't be walk-forward-optimized.
        let script = TempScript::new(
            r#"
fn on_candle() {
    "buy"
}
"#,
        );
        let args = ValidateArgs {
            strategy: script.0.to_str().unwrap().to_string(),
        };
        let v = validate_strategy(&args).unwrap();
        assert_eq!(v["valid"], serde_json::json!(true));
        assert_eq!(v["score"], serde_json::json!(90));
        let warnings = v["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0]["code"], "no_parameters");
    }

    #[test]
    fn validate_reports_a_compile_error_as_valid_false_not_an_err() {
        // A malformed script is a *validation result*, not a usage error: the
        // function returns Ok with valid:false, score 0, and a coded error the
        // agent can act on. exit 0 keeps the structured payload on stdout.
        let script = TempScript::new(
            r#"
fn on_candle( {
    "buy"
}
"#,
        );
        let args = ValidateArgs {
            strategy: script.0.to_str().unwrap().to_string(),
        };
        let v = validate_strategy(&args).unwrap();
        assert_eq!(v["valid"], serde_json::json!(false));
        assert_eq!(v["score"], serde_json::json!(0));
        let errors = v["errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["code"], "compile_error");
        assert!(errors[0]["message"].as_str().unwrap().len() > 0);
    }

    #[test]
    fn validate_flags_a_missing_on_candle_with_a_stable_code() {
        let script = TempScript::new(
            r#"
fn not_on_candle() {
    "buy"
}
"#,
        );
        let args = ValidateArgs {
            strategy: script.0.to_str().unwrap().to_string(),
        };
        let v = validate_strategy(&args).unwrap();
        assert_eq!(v["valid"], serde_json::json!(false));
        assert_eq!(v["errors"][0]["code"], "missing_on_candle");
    }

    #[test]
    fn validate_treats_a_builtin_name_as_always_valid() {
        // A bare "rsi" with no matching ~/.wickd/strategies/rsi.rhai resolves to
        // the built-in, which is compiled Rust and therefore always valid.
        let v = validate_strategy(&ValidateArgs {
            strategy: "rsi".to_string(),
        })
        .unwrap();
        assert_eq!(v["valid"], serde_json::json!(true));
        assert_eq!(v["kind"], "builtin");
        assert_eq!(v["name"], "rsi");
        assert_eq!(v["score"], serde_json::json!(100));
    }

    #[test]
    fn validate_errors_on_an_unknown_strategy_name() {
        let err = validate_strategy(&ValidateArgs {
            strategy: "definitely-not-a-strategy-xyz".to_string(),
        })
        .err()
        .unwrap();
        assert!(format!("{err:#}").contains("unknown strategy"));
    }

    #[test]
    fn validate_errors_when_a_dot_rhai_path_does_not_exist() {
        // A `.rhai` suffix is resolved as an explicit path; a missing file is a
        // usage error (Err → error envelope), not a valid:false report.
        let err = validate_strategy(&ValidateArgs {
            strategy: "/nonexistent/dir/nope.rhai".to_string(),
        })
        .err()
        .unwrap();
        assert!(format!("{err:#}").contains("failed to read strategy script"));
    }

    #[test]
    fn evaluate_wires_candles_into_core_and_tallies() {
        // Subcommand → core wiring: a valid strategy consumes candles and every
        // candle is accounted for in the buy/sell/hold tally.
        let (mut strategy, _params) = build_strategy(&{
            let mut a = run_args("ma-crossover");
            a.fast = 2;
            a.slow = 4;
            a
        })
        .unwrap();

        let prices = ["1.00", "1.01", "1.03", "1.06", "1.10", "1.06", "1.02", "0.99", "0.97"];
        let candles: Vec<Candle> = prices
            .iter()
            .enumerate()
            .map(|(i, p)| candle(p, i as i64))
            .collect();

        let eval = evaluate(strategy.as_mut(), &candles, true);
        assert_eq!(eval.buy + eval.sell + eval.close + eval.hold, candles.len());
        // include_holds=true → every candle emits a signal row.
        assert_eq!(eval.signals.len(), candles.len());
    }
}
