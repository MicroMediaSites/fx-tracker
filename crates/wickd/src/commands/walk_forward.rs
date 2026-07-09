//! `wickd backtest --walk-forward` — rolling in-sample/out-of-sample analysis
//! with per-window parameter re-optimization.
//!
//! A plain control-vs-test split with fixed parameters only proves a strategy
//! is *stable*, not that its *fitting process* generalizes. Real walk-forward
//! analysis splits the fetched candle range into sequential in-sample (IS) /
//! out-of-sample (OOS) window pairs, rolls forward across the range, and — for
//! each window — *re-optimizes* the strategy's parameters on the IS data, then
//! measures those freshly-fit parameters on the untouched OOS data. Reporting
//! IS-vs-OOS per window (not just an aggregate) is what makes overfitting and
//! parameter instability visible.
//!
//! Scope (per AGT-608 AC5): this ships *only* the minimal per-window search
//! needed for walk-forward. A standalone, interactive parameter-sweep tool for
//! arbitrary-range exploration stays deferred as `wickd-core`'s
//! `optimizer.rs` concern — deliberately not wired here.
//!
//! Why a CLI-native implementation rather than `wickd_core::backtest::
//! walk_forward::run_walk_forward`? That engine is bound to the desktop app's
//! rules-engine world: it takes a `StrategyDefinition` JSON + `RulesBasedStrategy`
//! and slices windows by *calendar months*. The `wickd` CLI runs native
//! `Box<dyn Strategy>` built-ins (`ma-crossover`, `rsi`) — which have no
//! rules-engine JSON form at all — over candles fetched by *count*. So we reuse
//! the shared `BacktestEngine` (via `backtest::run_engine`) but supply our own
//! count-based windowing and a minimal, bounded parameter search that works
//! uniformly across built-ins and `.rhai` scripts.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{json, Value};

use wickd_core::backtest::{
    BacktestConfig, BacktestMetrics, MovingAverageCrossover, RsiStrategy, ScriptedStrategy,
    Strategy, validate_script,
};
use wickd_core::models::Candle;
use shared::{ParameterDefinition, ParameterType};

use super::backtest::{run_engine, BacktestArgs};
use super::scripted;

// ============================================================================
// Parameter search space
// ============================================================================

/// One optimizable axis: a discrete, bounded set of candidate values for a
/// single parameter. Bounds come from a `.rhai` script's `@parameters`
/// metadata (min/max/step) or, for built-ins, from a fixed sensible grid.
/// Values are pre-rounded per the declared parameter type at construction, so
/// the type isn't retained on the axis itself.
struct ParamAxis {
    id: String,
    values: Vec<f64>,
}

/// Builds a concrete strategy for a point in the search space. Invalid points
/// (e.g. `ma-crossover` with `fast >= slow`) return `Err` and are skipped by
/// the sweep rather than aborting it.
type StrategyFactory = Box<dyn Fn(&HashMap<String, f64>) -> Result<Box<dyn Strategy>>>;

/// A bounded parameter search space plus the factory that instantiates a
/// strategy for any point within it.
struct SearchSpace {
    axes: Vec<ParamAxis>,
    factory: StrategyFactory,
}

/// The best point found during an in-sample sweep, with its IS performance.
struct Optimized {
    params: HashMap<String, f64>,
    metrics: BacktestMetrics,
}

/// Round a candidate value according to its declared type (integer params must
/// land on whole numbers before they reach `usize`-typed built-in constructors
/// or integer-consuming scripts).
fn round_for_type(v: f64, ty: ParameterType) -> f64 {
    match ty {
        ParameterType::Integer => v.round(),
        _ => v,
    }
}

/// Enumerate a single axis's candidate values from a `@parameters`-style
/// definition. A param with a full `min`/`max`/`step` range (and `max > min`,
/// `step > 0`) sweeps that inclusive range; anything else collapses to a single
/// point at its default (nothing to optimize — a degenerate, fixed-param axis).
fn axis_from_param(p: &ParameterDefinition) -> ParamAxis {
    let values = match (p.min, p.max, p.step) {
        (Some(min), Some(max), Some(step)) if step > 0.0 && max > min => {
            let mut vals = Vec::new();
            let mut x = min;
            // Cap iterations so a tiny step over a wide range can't run away;
            // the whole-space cap in `enumerate_points` is the real bound.
            let mut guard = 0;
            while x <= max + 1e-9 && guard < 10_000 {
                let rounded = round_for_type(x, p.param_type);
                if vals.last() != Some(&rounded) {
                    vals.push(rounded);
                }
                x += step;
                guard += 1;
            }
            if vals.is_empty() {
                vals.push(round_for_type(p.default, p.param_type));
            }
            vals
        }
        _ => vec![round_for_type(p.default, p.param_type)],
    };
    ParamAxis {
        id: p.id.clone(),
        values,
    }
}

/// Expand the axes into the full grid of parameter assignments (the cartesian
/// product), refusing to proceed if the space is larger than `max_combos` so a
/// careless `@parameters` range can't spin up an unbounded search.
fn enumerate_points(
    axes: &[ParamAxis],
    max_combos: usize,
) -> Result<Vec<HashMap<String, f64>>> {
    let mut total: usize = 1;
    for a in axes {
        total = total.saturating_mul(a.values.len().max(1));
    }
    if total > max_combos {
        bail!(
            "walk-forward parameter space too large: {total} combinations exceed \
             --max-combos {max_combos}; narrow the @parameters ranges or raise --max-combos"
        );
    }

    let mut points: Vec<HashMap<String, f64>> = vec![HashMap::new()];
    for a in axes {
        if a.values.is_empty() {
            continue;
        }
        let mut next = Vec::with_capacity(points.len() * a.values.len());
        for base in &points {
            for v in &a.values {
                let mut p = base.clone();
                p.insert(a.id.clone(), *v);
                next.push(p);
            }
        }
        points = next;
    }
    Ok(points)
}

// ============================================================================
// Search-space construction (mirrors backtest::build_strategy's dispatch)
// ============================================================================

/// Resolve the strategy argument to a bounded search space + strategy factory,
/// mirroring `backtest::build_strategy`'s precedence: an explicit `.rhai` path
/// first, then built-in names, then a bare name under `~/.wickd/strategies/`.
fn build_search_space(args: &BacktestArgs) -> Result<SearchSpace> {
    if let Some(path) = scripted::resolve_explicit_script_path(&args.strategy) {
        return scripted_search_space(&path, &args.instrument);
    }

    match args.strategy.to_lowercase().as_str() {
        "ma-crossover" | "ma" => Ok(ma_search_space()),
        "rsi" => Ok(rsi_search_space()),
        other => {
            if let Some(path) = scripted::resolve_named_script_path(other)? {
                return scripted_search_space(&path, &args.instrument);
            }
            bail!(
                "unknown strategy '{other}' (available: ma-crossover, rsi, \
                 or a .rhai script path/name under ~/.wickd/strategies/)"
            )
        }
    }
}

/// Bounded search grid for the built-in MA crossover: fast/slow period pairs.
/// Points where `fast >= slow` are produced but rejected by the factory (and so
/// skipped by the sweep), which keeps the grid declaration simple.
fn ma_search_space() -> SearchSpace {
    let axes = vec![
        ParamAxis {
            id: "fast".to_string(),
            values: vec![5.0, 10.0, 15.0, 20.0],
        },
        ParamAxis {
            id: "slow".to_string(),
            values: vec![20.0, 30.0, 40.0, 50.0],
        },
    ];
    let factory: StrategyFactory = Box::new(|params| {
        let fast = params.get("fast").copied().unwrap_or(10.0).round() as usize;
        let slow = params.get("slow").copied().unwrap_or(30.0).round() as usize;
        if fast == 0 || slow == 0 {
            bail!("ma-crossover periods must be greater than 0");
        }
        if fast >= slow {
            bail!("ma-crossover fast period ({fast}) must be less than slow period ({slow})");
        }
        Ok(Box::new(MovingAverageCrossover::new(fast, slow)) as Box<dyn Strategy>)
    });
    SearchSpace { axes, factory }
}

/// Bounded search grid for the built-in RSI strategy: period + threshold pairs.
/// Points where `oversold >= overbought` are rejected by the factory.
fn rsi_search_space() -> SearchSpace {
    let axes = vec![
        ParamAxis {
            id: "period".to_string(),
            values: vec![7.0, 14.0, 21.0],
        },
        ParamAxis {
            id: "overbought".to_string(),
            values: vec![65.0, 70.0, 75.0],
        },
        ParamAxis {
            id: "oversold".to_string(),
            values: vec![25.0, 30.0, 35.0],
        },
    ];
    let factory: StrategyFactory = Box::new(|params| {
        let period = params.get("period").copied().unwrap_or(14.0).round() as usize;
        let overbought = params.get("overbought").copied().unwrap_or(70.0);
        let oversold = params.get("oversold").copied().unwrap_or(30.0);
        if period == 0 {
            bail!("rsi period must be greater than 0");
        }
        if oversold >= overbought {
            bail!("rsi oversold threshold ({oversold}) must be less than overbought ({overbought})");
        }
        let overbought = Decimal::try_from(overbought)
            .map_err(|_| anyhow!("invalid overbought threshold '{overbought}'"))?;
        let oversold = Decimal::try_from(oversold)
            .map_err(|_| anyhow!("invalid oversold threshold '{oversold}'"))?;
        Ok(Box::new(RsiStrategy::new(period, overbought, oversold)) as Box<dyn Strategy>)
    });
    SearchSpace { axes, factory }
}

/// Build a search space for a `.rhai` script: axes come straight from the
/// script's `@parameters` metadata, and the factory recompiles the script with
/// each candidate parameter set. Validating up front means a malformed script
/// surfaces the same clear error the single-shot backtest path produces.
fn scripted_search_space(path: &Path, instrument: &str) -> Result<SearchSpace> {
    let script = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read strategy script '{}'", path.display()))?;
    let metadata = validate_script(&script)
        .map_err(|e| anyhow!("invalid strategy script '{}': {e}", path.display()))?;

    let axes: Vec<ParamAxis> = metadata.parameters.iter().map(axis_from_param).collect();

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("scripted")
        .to_string();
    let instrument = instrument.to_string();

    let factory: StrategyFactory = Box::new(move |params| {
        let mut strategy =
            ScriptedStrategy::from_script_with_params(&script, &name, params.clone())
                .map_err(|e| anyhow!("failed to load strategy script: {e}"))?;
        strategy.set_pip_value_for_instrument(&instrument);
        Ok(Box::new(strategy) as Box<dyn Strategy>)
    });

    Ok(SearchSpace { axes, factory })
}

// ============================================================================
// Windowing
// ============================================================================

/// A single walk-forward window: half-open candle index ranges for the
/// in-sample (training) and out-of-sample (test) slices.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Window {
    index: usize,
    is_start: usize,
    is_end: usize,
    oos_start: usize,
    oos_end: usize,
}

/// Generate sequential IS/OOS windows over `total` candles, rolling forward.
///
/// - Rolling: the IS window slides forward by `step` each iteration.
/// - Anchored: the IS window starts at 0 and *expands* by `step` each iteration.
///
/// OOS always immediately follows IS. Iteration stops once a window's OOS end
/// would run past the available candles. Returns empty if the sizes are zero or
/// the range can't fit even one complete window.
fn generate_windows(
    total: usize,
    is_size: usize,
    oos_size: usize,
    step: usize,
    anchored: bool,
) -> Vec<Window> {
    let mut windows = Vec::new();
    if is_size == 0 || oos_size == 0 || step == 0 {
        return windows;
    }

    let mut i = 0usize;
    loop {
        let (is_start, is_end) = if anchored {
            (0, is_size + i * step)
        } else {
            (i * step, i * step + is_size)
        };
        let oos_start = is_end;
        let oos_end = oos_start + oos_size;
        if oos_end > total {
            break;
        }
        windows.push(Window {
            index: i + 1,
            is_start,
            is_end,
            oos_start,
            oos_end,
        });
        i += 1;
    }
    windows
}

// ============================================================================
// Per-window optimize + out-of-sample evaluation
// ============================================================================

/// Sweep the search space over the in-sample candles and return the best point.
/// Objective: total P&L (net profit) on the IS slice — deterministic, and the
/// quantity a trader is ultimately fitting for. Invalid points (factory `Err`)
/// are skipped; ties keep the first point in enumeration order.
fn optimize_in_sample(
    space: &SearchSpace,
    config: &BacktestConfig,
    is_candles: &[Candle],
    max_combos: usize,
) -> Result<Optimized> {
    let points = enumerate_points(&space.axes, max_combos)?;

    let mut best: Option<(Decimal, Optimized)> = None;
    for point in points {
        let mut strategy = match (space.factory)(&point) {
            Ok(s) => s,
            Err(_) => continue, // invalid combination — skip, don't abort the sweep
        };
        let result = run_engine(strategy.as_mut(), config.clone(), is_candles);
        let score = result.metrics.total_pnl;
        let is_better = match &best {
            Some((best_score, _)) => score > *best_score,
            None => true,
        };
        if is_better {
            best = Some((
                score,
                Optimized {
                    params: point,
                    metrics: result.metrics,
                },
            ));
        }
    }

    best.map(|(_, o)| o).ok_or_else(|| {
        anyhow!("no valid parameter combination for the in-sample window")
    })
}

/// Evaluate an already-optimized parameter set on the out-of-sample candles.
fn evaluate_out_of_sample(
    space: &SearchSpace,
    config: &BacktestConfig,
    params: &HashMap<String, f64>,
    oos_candles: &[Candle],
) -> Result<BacktestMetrics> {
    let mut strategy = (space.factory)(params)?;
    let result = run_engine(strategy.as_mut(), config.clone(), oos_candles);
    Ok(result.metrics)
}

/// Run optimize→OOS for every window and collect per-window reports. Pulled out
/// of `run` so the core walk-forward loop is unit-testable with a synthetic
/// search space and candles, without any network fetch.
fn evaluate_windows(
    space: &SearchSpace,
    config: &BacktestConfig,
    candles: &[Candle],
    windows: &[Window],
    max_combos: usize,
) -> Result<Vec<WindowReport>> {
    let mut reports = Vec::with_capacity(windows.len());
    for w in windows {
        let is_candles = &candles[w.is_start..w.is_end];
        let oos_candles = &candles[w.oos_start..w.oos_end];

        let optimized = optimize_in_sample(space, config, is_candles, max_combos)?;
        let oos_metrics =
            evaluate_out_of_sample(space, config, &optimized.params, oos_candles)?;

        // Sort params for stable output ordering across runs.
        let optimized_params: BTreeMap<String, f64> =
            optimized.params.iter().map(|(k, v)| (k.clone(), *v)).collect();

        reports.push(WindowReport {
            window: w.index,
            is_range: [w.is_start, w.is_end],
            oos_range: [w.oos_start, w.oos_end],
            is_start: candles[w.is_start].time.to_rfc3339(),
            is_end: candles[w.is_end - 1].time.to_rfc3339(),
            oos_start: candles[w.oos_start].time.to_rfc3339(),
            oos_end: candles[w.oos_end - 1].time.to_rfc3339(),
            optimized_params,
            in_sample: optimized.metrics,
            out_of_sample: oos_metrics,
        });
    }
    Ok(reports)
}

// ============================================================================
// Report shapes
// ============================================================================

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WindowReport {
    /// 1-indexed window number.
    window: usize,
    /// In-sample candle index range `[start, end)`.
    is_range: [usize; 2],
    /// Out-of-sample candle index range `[start, end)`.
    oos_range: [usize; 2],
    /// RFC3339 timestamp of the first in-sample candle.
    is_start: String,
    /// RFC3339 timestamp of the last in-sample candle.
    is_end: String,
    /// RFC3339 timestamp of the first out-of-sample candle.
    oos_start: String,
    /// RFC3339 timestamp of the last out-of-sample candle.
    oos_end: String,
    /// Parameters chosen by the in-sample optimization for this window.
    optimized_params: BTreeMap<String, f64>,
    /// In-sample metrics for the optimized parameters (the fitted result).
    in_sample: BacktestMetrics,
    /// Out-of-sample metrics for those same parameters (the honest result).
    out_of_sample: BacktestMetrics,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Aggregate {
    /// Number of completed windows.
    windows: usize,
    /// Windows whose OOS P&L was strictly positive.
    oos_profitable_windows: usize,
    /// Sum of OOS P&L across all windows.
    oos_net_profit: Decimal,
    /// Mean in-sample return % across windows (the fitted, optimistic figure).
    is_avg_return_pct: Decimal,
    /// Mean out-of-sample return % across windows. A large gap below
    /// `is_avg_return_pct` is the tell-tale sign of overfitting.
    oos_avg_return_pct: Decimal,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WalkForwardReport {
    /// `"rolling"` or `"anchored"`.
    mode: &'static str,
    is_size: usize,
    oos_size: usize,
    step: usize,
    max_combos: usize,
    windows: Vec<WindowReport>,
    aggregate: Aggregate,
}

/// Summarize per-window results into the overfitting-at-a-glance aggregate.
fn summarize(windows: &[WindowReport]) -> Aggregate {
    let n = windows.len();
    let oos_profitable_windows = windows
        .iter()
        .filter(|w| w.out_of_sample.total_pnl > Decimal::ZERO)
        .count();
    let oos_net_profit: Decimal = windows.iter().map(|w| w.out_of_sample.total_pnl).sum();

    let (is_avg_return_pct, oos_avg_return_pct) = if n == 0 {
        (Decimal::ZERO, Decimal::ZERO)
    } else {
        let is_sum: Decimal = windows.iter().map(|w| w.in_sample.total_return_pct).sum();
        let oos_sum: Decimal = windows.iter().map(|w| w.out_of_sample.total_return_pct).sum();
        let d = Decimal::from(n as u64);
        (is_sum / d, oos_sum / d)
    };

    Aggregate {
        windows: n,
        oos_profitable_windows,
        oos_net_profit,
        is_avg_return_pct,
        oos_avg_return_pct,
    }
}

// ============================================================================
// Entry point (called from `backtest::run_backtest` once candles are fetched)
// ============================================================================

/// Run walk-forward analysis over already-fetched candles and assemble the JSON
/// report. Kept network-free so it mirrors `backtest::run_engine`'s testability
/// rationale: the whole pipeline can be exercised offline. `config` is the same
/// engine config the single-shot backtest builds, so both paths size positions
/// and model spread identically.
pub(crate) fn run(
    args: &BacktestArgs,
    config: BacktestConfig,
    candles: &[Candle],
) -> Result<Value> {
    let is_size = args.is_size;
    let oos_size = args.oos_size;
    let step = args.wf_step.unwrap_or(oos_size);

    if is_size == 0 || oos_size == 0 {
        bail!("walk-forward window sizes must be greater than 0 (--is-size, --oos-size)");
    }
    if step == 0 {
        bail!("walk-forward --wf-step must be greater than 0");
    }
    if candles.len() < is_size + oos_size {
        bail!(
            "not enough candles for a walk-forward window: have {}, need at least {} \
             (--is-size {is_size} + --oos-size {oos_size})",
            candles.len(),
            is_size + oos_size
        );
    }

    let space = build_search_space(args)?;

    let windows = generate_windows(candles.len(), is_size, oos_size, step, args.anchored);
    if windows.is_empty() {
        bail!(
            "walk-forward produced no complete windows over {} candles \
             (--is-size {is_size}, --oos-size {oos_size}, --wf-step {step})",
            candles.len()
        );
    }

    let reports = evaluate_windows(&space, &config, candles, &windows, args.max_combos)?;
    let aggregate = summarize(&reports);

    let report = WalkForwardReport {
        mode: if args.anchored { "anchored" } else { "rolling" },
        is_size,
        oos_size,
        step,
        max_combos: args.max_combos,
        windows: reports,
        aggregate,
    };

    Ok(json!({
        "strategy": args.strategy,
        "instrument": args.instrument,
        "granularity": args.granularity,
        "candles": candles.len(),
        "mode": "walk-forward",
        "walkForward": serde_json::to_value(report)?,
    }))
}

// ============================================================================
// Tests — deterministic, hand-computed (AGT-604 fixture pattern)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use wickd_core::backtest::strategy::Signal;
    use wickd_core::models::Ohlc;
    use chrono::{TimeZone, Utc};
    use rust_decimal_macros::dec;

    // --- window splitting --------------------------------------------------

    #[test]
    fn rolling_windows_split_the_range_into_the_expected_index_pairs() {
        // 100 candles, IS=40, OOS=20, step=20 (rolling).
        // w1: IS[0,40)  OOS[40,60)
        // w2: IS[20,60) OOS[60,80)
        // w3: IS[40,80) OOS[80,100)
        // w4 would need OOS end 120 > 100 → stop.
        let windows = generate_windows(100, 40, 20, 20, false);
        assert_eq!(windows.len(), 3);
        assert_eq!(
            windows[0],
            Window { index: 1, is_start: 0, is_end: 40, oos_start: 40, oos_end: 60 }
        );
        assert_eq!(
            windows[1],
            Window { index: 2, is_start: 20, is_end: 60, oos_start: 60, oos_end: 80 }
        );
        assert_eq!(
            windows[2],
            Window { index: 3, is_start: 40, is_end: 80, oos_start: 80, oos_end: 100 }
        );
    }

    #[test]
    fn anchored_windows_hold_the_start_fixed_and_expand_in_sample() {
        // 100 candles, IS=40, OOS=20, step=20 (anchored).
        // w1: IS[0,40)  OOS[40,60)
        // w2: IS[0,60)  OOS[60,80)
        // w3: IS[0,80)  OOS[80,100)
        let windows = generate_windows(100, 40, 20, 20, true);
        assert_eq!(windows.len(), 3);
        for w in &windows {
            assert_eq!(w.is_start, 0, "anchored IS always starts at 0");
        }
        assert_eq!(windows[0].is_end, 40);
        assert_eq!(windows[1].is_end, 60);
        assert_eq!(windows[2].is_end, 80);
        assert_eq!(windows[2].oos_end, 100);
    }

    #[test]
    fn windows_are_empty_when_the_range_cannot_fit_one_pair() {
        assert!(generate_windows(50, 40, 20, 20, false).is_empty());
        assert!(generate_windows(100, 0, 20, 20, false).is_empty());
        assert!(generate_windows(100, 40, 0, 20, false).is_empty());
    }

    // --- axis enumeration --------------------------------------------------

    fn param(id: &str, ty: ParameterType, default: f64, min: Option<f64>, max: Option<f64>, step: Option<f64>) -> ParameterDefinition {
        ParameterDefinition {
            id: id.to_string(),
            name: id.to_string(),
            description: None,
            param_type: ty,
            default,
            min,
            max,
            step,
            options: None,
            group: None,
        }
    }

    #[test]
    fn axis_sweeps_integer_range_inclusively_on_whole_numbers() {
        let p = param("n", ParameterType::Integer, 10.0, Some(2.0), Some(6.0), Some(2.0));
        let axis = axis_from_param(&p);
        assert_eq!(axis.values, vec![2.0, 4.0, 6.0]);
    }

    #[test]
    fn axis_without_a_range_collapses_to_its_default() {
        let p = param("n", ParameterType::Number, 42.0, None, None, None);
        let axis = axis_from_param(&p);
        assert_eq!(axis.values, vec![42.0]);
    }

    #[test]
    fn enumerate_points_rejects_a_space_larger_than_the_cap() {
        let axes = vec![
            ParamAxis { id: "a".into(), values: vec![1.0, 2.0, 3.0] },
            ParamAxis { id: "b".into(), values: vec![1.0, 2.0, 3.0] },
        ];
        // 3 x 3 = 9 combinations; cap of 4 must refuse.
        assert!(enumerate_points(&axes, 4).is_err());
        // Cap of 9 (or more) is fine.
        assert_eq!(enumerate_points(&axes, 9).unwrap().len(), 9);
    }

    // --- re-optimization on a rigged in-sample series ----------------------

    /// A synthetic strategy that opens a long at candle index `entry` and closes
    /// it `HOLD` candles later. Its P&L therefore depends entirely on which
    /// stretch of the series it holds through — letting a test rig a series so a
    /// specific `entry` value is unambiguously optimal in-sample.
    struct WindowedLong {
        entry: usize,
        idx: usize,
    }

    impl WindowedLong {
        const HOLD: usize = 3;
    }

    impl Strategy for WindowedLong {
        fn on_candle(&mut self, _candle: &Candle) -> Signal {
            let here = self.idx;
            self.idx += 1;
            if here == self.entry {
                Signal::Buy
            } else if here == self.entry + Self::HOLD {
                Signal::ClosePosition
            } else {
                Signal::Hold
            }
        }
        fn name(&self) -> &str {
            "WindowedLong"
        }
        fn reset(&mut self) {
            self.idx = 0;
        }
    }

    /// Search space over the `entry` index, three candidate values.
    fn windowed_long_space(candidates: Vec<f64>) -> SearchSpace {
        let axes = vec![ParamAxis {
            id: "entry".to_string(),
            values: candidates,
        }];
        let factory: StrategyFactory = Box::new(|params| {
            let entry = params.get("entry").copied().unwrap_or(0.0).round() as usize;
            Ok(Box::new(WindowedLong { entry, idx: 0 }) as Box<dyn Strategy>)
        });
        SearchSpace { axes, factory }
    }

    fn candle(close: &str, i: i64) -> Candle {
        let price = Decimal::from_str(close).unwrap();
        Candle {
            time: Utc.timestamp_opt(1_700_000_000 + i * 3600, 0).unwrap(),
            mid: Ohlc { open: price, high: price, low: price, close: price },
            volume: 1,
            complete: true,
        }
    }

    use std::str::FromStr;

    fn zero_cost_config() -> BacktestConfig {
        BacktestConfig {
            initial_balance: dec!(10000),
            position_size: dec!(1000),
            use_percentage: false,
            risk_percent: None,
            estimated_stop_pips: dec!(20),
            spread_pips: dec!(0), // zero spread → P&L is pure price move, no fixed drag
            pip_value: dec!(0.0001),
        }
    }

    /// Prices: flat 1.00 everywhere except a sharp rally between index 4 and 7.
    /// A long that enters at index 4 (buy next open, hold 3 bars, close at 7)
    /// rides the whole rally; entries at 0 or 8 sit in the flat zones and make
    /// nothing. So the in-sample optimizer must select entry == 4.
    fn rigged_rally_series() -> Vec<Candle> {
        let prices = [
            "1.00", "1.00", "1.00", "1.00", "1.00", // 0..4 flat (entry 4 buys at next open, idx5)
            "1.05", "1.10", "1.15", // 5..7 rally
            "1.15", "1.15", "1.15", "1.15", "1.15", // 8..12 flat plateau
        ];
        prices.iter().enumerate().map(|(i, p)| candle(p, i as i64)).collect()
    }

    #[test]
    fn in_sample_optimization_picks_the_parameter_that_captures_the_rally() {
        let space = windowed_long_space(vec![0.0, 4.0, 8.0]);
        let config = zero_cost_config();
        let series = rigged_rally_series();

        let best = optimize_in_sample(&space, &config, &series, 64).unwrap();

        assert_eq!(
            best.params.get("entry").copied(),
            Some(4.0),
            "entry=4 rides the rally and must win in-sample"
        );
        // The winning fit is genuinely profitable in-sample.
        assert!(best.metrics.total_pnl > Decimal::ZERO);
    }

    #[test]
    fn optimize_reports_zero_pnl_when_no_candidate_trades_profitably() {
        // All-flat series: no entry makes money, best P&L is exactly zero.
        let space = windowed_long_space(vec![0.0, 3.0, 6.0]);
        let config = zero_cost_config();
        let flat: Vec<Candle> = (0..12).map(|i| candle("1.00", i)).collect();

        let best = optimize_in_sample(&space, &config, &flat, 64).unwrap();
        assert_eq!(best.metrics.total_pnl, Decimal::ZERO);
    }

    // --- end-to-end per-window IS vs OOS reporting -------------------------

    #[test]
    fn evaluate_windows_reports_both_in_sample_and_out_of_sample_per_window() {
        // 24 candles, IS=8, OOS=4, step=4, rolling → windows at:
        //   w1 IS[0,8)  OOS[8,12)
        //   w2 IS[4,12) OOS[12,16)
        //   w3 IS[8,16) OOS[16,20)
        //   w4 IS[12,20) OOS[20,24)
        let series: Vec<Candle> = (0..24)
            .map(|i| {
                // Gentle rising ramp so longs are generally profitable.
                let price = format!("{:.2}", 1.00 + (i as f64) * 0.01);
                candle(&price, i)
            })
            .collect();
        let windows = generate_windows(series.len(), 8, 4, 4, false);
        assert_eq!(windows.len(), 4);

        let space = windowed_long_space(vec![0.0, 2.0, 4.0]);
        let config = zero_cost_config();

        let reports = evaluate_windows(&space, &config, &series, &windows, 64).unwrap();
        assert_eq!(reports.len(), 4);

        for (w, r) in windows.iter().zip(&reports) {
            // Each window carries a distinct IS range and OOS range...
            assert_eq!(r.is_range, [w.is_start, w.is_end]);
            assert_eq!(r.oos_range, [w.oos_start, w.oos_end]);
            // ...an optimized parameter chosen from the grid...
            assert!(r.optimized_params.contains_key("entry"));
            // ...and BOTH in-sample and out-of-sample metric blocks are present,
            // which is the whole point of AC3 (overfitting must be visible).
            assert_eq!(
                r.in_sample.total_trades,
                r.in_sample.winning_trades + r.in_sample.losing_trades
            );
            assert_eq!(
                r.out_of_sample.total_trades,
                r.out_of_sample.winning_trades + r.out_of_sample.losing_trades
            );
        }

        // Aggregate summarizes across the windows.
        let agg = summarize(&reports);
        assert_eq!(agg.windows, 4);
    }

    #[test]
    fn scripted_search_space_reads_bounds_from_parameters_metadata() {
        // A .rhai script whose @parameters declares one integer axis with a
        // min/max/step must produce exactly that swept axis — proving AC4's
        // scripted path is bounded by the script metadata, not a fixed grid.
        let script = r#"
// @parameters: [
//   { "id": "period", "name": "Period", "type": "integer", "default": 10, "min": 4, "max": 8, "step": 2 }
// ]
fn on_candle() {
    "hold"
}
"#;
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "wickd-wf-test-{}-{}.rhai",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, script).unwrap();

        let space = scripted_search_space(&path, "EUR_USD").unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(space.axes.len(), 1);
        assert_eq!(space.axes[0].id, "period");
        assert_eq!(space.axes[0].values, vec![4.0, 6.0, 8.0]);
    }
}
