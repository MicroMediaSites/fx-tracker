//! Strategy Parameter Optimizer
//!
//! Implements grid search optimization to find the best parameter combinations
//! for a given strategy over historical data.

use std::collections::HashMap;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::models::Candle;
use super::engine::{BacktestConfig, BacktestEngine, BacktestResult};
use super::rules_strategy::RulesBasedStrategy;
use super::scripted_strategy::ScriptedStrategy;
use super::rules_engine::{ParameterDefinition, SRZone, StrategyDefinition};
use super::pivots::PivotConfig;
use super::mtf::MtfCandleStore;

// ============================================================================
// Optimization Types
// ============================================================================

/// Optimization objective function
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationObjective {
    /// Maximize Sharpe ratio (risk-adjusted returns)
    SharpeRatio,
    /// Maximize profit factor (gross profit / gross loss)
    ProfitFactor,
    /// Maximize total return percentage
    TotalReturn,
    /// Maximize win rate percentage
    WinRate,
    /// Minimize maximum drawdown percentage
    MinDrawdown,
    /// Maximize number of trades (for statistical significance)
    TradeCount,
}

impl Default for OptimizationObjective {
    fn default() -> Self {
        Self::SharpeRatio
    }
}

/// Configuration for running an optimization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationConfig {
    /// Which objective to optimize for
    pub objective: OptimizationObjective,
    /// Parameter IDs to optimize (None = all parameters with min/max/step)
    pub param_ids: Option<Vec<String>>,
    /// Maximum number of combinations to test (for safety limits)
    pub max_combinations: Option<usize>,
    /// Minimum number of trades required for a result to be valid
    pub min_trades: Option<usize>,
}

impl Default for OptimizationConfig {
    fn default() -> Self {
        Self {
            objective: OptimizationObjective::SharpeRatio,
            param_ids: None,
            max_combinations: None, // No limit - warn instead of blocking
            min_trades: Some(10),
        }
    }
}

/// Simplified metrics for optimization results (serializable)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationMetrics {
    pub total_pnl: String,
    pub total_return_pct: String,
    pub winning_trades: u32,
    pub losing_trades: u32,
    pub win_rate: String,
    pub profit_factor: String,
    pub max_drawdown_pct: String,
    pub sharpe_ratio: String,
    pub total_trades: u32,
    pub final_balance: String,
}

impl From<&BacktestResult> for OptimizationMetrics {
    fn from(result: &BacktestResult) -> Self {
        let m = &result.metrics;
        Self {
            total_pnl: m.total_pnl.round_dp(2).to_string(),
            total_return_pct: m.total_return_pct.round_dp(2).to_string(),
            winning_trades: m.winning_trades,
            losing_trades: m.losing_trades,
            win_rate: m.win_rate.round_dp(2).to_string(),
            profit_factor: m.profit_factor.round_dp(2).to_string(),
            max_drawdown_pct: m.max_drawdown_pct.round_dp(2).to_string(),
            sharpe_ratio: m.sharpe_ratio.round_dp(2).to_string(),
            total_trades: m.total_trades,
            final_balance: result.final_balance.round_dp(2).to_string(),
        }
    }
}

/// Result of a single parameter combination backtest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationRun {
    /// The parameter values used for this run
    pub params: HashMap<String, f64>,
    /// The backtest metrics (simplified for serialization)
    pub metrics: OptimizationMetrics,
    /// The objective score for ranking
    pub score: f64,
}

/// Full optimization result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationResult {
    /// Total number of combinations tested
    pub total_combinations: usize,
    /// Number of valid results (met min_trades requirement)
    pub valid_results: usize,
    /// All runs sorted by objective score (best first)
    pub runs: Vec<OptimizationRun>,
    /// The best parameter set
    pub best_params: Option<HashMap<String, f64>>,
    /// The objective used for ranking
    pub objective: OptimizationObjective,
}

// ============================================================================
// Grid Generation
// ============================================================================

/// A parameter range for optimization
#[derive(Debug, Clone)]
pub struct ParameterRange {
    pub id: String,
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

impl ParameterRange {
    /// Generate all values in this range
    pub fn values(&self) -> Vec<f64> {
        let mut values = Vec::new();
        let mut current = self.min;
        while current <= self.max + f64::EPSILON {
            values.push(current);
            current += self.step;
        }
        values
    }

    /// Count of values in this range
    pub fn count(&self) -> usize {
        if self.step <= 0.0 || self.max < self.min {
            return 1;
        }
        ((self.max - self.min) / self.step).floor() as usize + 1
    }
}

/// Extract optimizable parameter ranges from strategy parameters
pub fn extract_param_ranges(
    parameters: &[ParameterDefinition],
    filter_ids: Option<&[String]>,
) -> Vec<ParameterRange> {
    parameters
        .iter()
        .filter(|p| {
            // Must have min, max, and step defined
            p.min.is_some() && p.max.is_some() && p.step.is_some()
        })
        .filter(|p| {
            // If filter provided, only include matching IDs
            match filter_ids {
                Some(ids) => ids.contains(&p.id),
                None => true,
            }
        })
        .map(|p| ParameterRange {
            id: p.id.clone(),
            min: p.min.unwrap(),
            max: p.max.unwrap(),
            step: p.step.unwrap(),
        })
        .collect()
}

/// Generate all parameter combinations (cartesian product)
pub fn generate_combinations(ranges: &[ParameterRange]) -> Vec<HashMap<String, f64>> {
    if ranges.is_empty() {
        return vec![HashMap::new()];
    }

    let mut results = Vec::new();
    generate_combinations_recursive(ranges, 0, HashMap::new(), &mut results);
    results
}

fn generate_combinations_recursive(
    ranges: &[ParameterRange],
    index: usize,
    current: HashMap<String, f64>,
    results: &mut Vec<HashMap<String, f64>>,
) {
    if index >= ranges.len() {
        results.push(current);
        return;
    }

    let range = &ranges[index];
    for value in range.values() {
        let mut next = current.clone();
        next.insert(range.id.clone(), value);
        generate_combinations_recursive(ranges, index + 1, next, results);
    }
}

/// Calculate total number of combinations without generating them
pub fn count_combinations(ranges: &[ParameterRange]) -> usize {
    if ranges.is_empty() {
        return 1;
    }
    ranges.iter().map(|r| r.count()).product()
}

// ============================================================================
// Objective Scoring
// ============================================================================

/// Calculate objective score from backtest result
pub fn calculate_score(result: &BacktestResult, objective: OptimizationObjective) -> f64 {
    let m = &result.metrics;
    match objective {
        OptimizationObjective::SharpeRatio => {
            m.sharpe_ratio.to_string().parse::<f64>().unwrap_or(f64::NEG_INFINITY)
        }
        OptimizationObjective::ProfitFactor => {
            m.profit_factor.to_string().parse::<f64>().unwrap_or(0.0)
        }
        OptimizationObjective::TotalReturn => {
            m.total_return_pct.to_string().parse::<f64>().unwrap_or(f64::NEG_INFINITY)
        }
        OptimizationObjective::WinRate => {
            m.win_rate.to_string().parse::<f64>().unwrap_or(0.0)
        }
        OptimizationObjective::MinDrawdown => {
            // Negate so lower drawdown = higher score
            let dd = m.max_drawdown_pct.to_string().parse::<f64>().unwrap_or(100.0);
            -dd
        }
        OptimizationObjective::TradeCount => {
            m.total_trades as f64
        }
    }
}

// ============================================================================
// Optimizer
// ============================================================================

/// Helper to create a dummy optimization run for strategy creation failures
fn dummy_run(params: &HashMap<String, f64>, initial_balance: Decimal) -> OptimizationRun {
    OptimizationRun {
        params: params.clone(),
        metrics: OptimizationMetrics {
            total_pnl: "0".to_string(),
            total_return_pct: "0".to_string(),
            winning_trades: 0,
            losing_trades: 0,
            win_rate: "0".to_string(),
            profit_factor: "0".to_string(),
            max_drawdown_pct: "0".to_string(),
            sharpe_ratio: "0".to_string(),
            total_trades: 0,
            final_balance: initial_balance.to_string(),
        },
        score: f64::NEG_INFINITY,
    }
}

/// Collect optimization runs into a sorted result
fn collect_optimization_result(
    runs: Vec<OptimizationRun>,
    total_combinations: usize,
    objective: OptimizationObjective,
) -> OptimizationResult {
    let valid_count = runs.iter().filter(|r| r.score > f64::NEG_INFINITY).count();

    let mut runs = runs;
    runs.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    let best_params = runs
        .iter()
        .find(|r| r.score > f64::NEG_INFINITY)
        .map(|r| r.params.clone());

    OptimizationResult {
        total_combinations,
        valid_results: valid_count,
        runs,
        best_params,
        objective,
    }
}

/// Run grid search optimization over a strategy (parallelized with rayon)
pub fn run_optimization(
    strategy_json: &str,
    parameters: &[ParameterDefinition],
    candles: &[Candle],
    initial_balance: Decimal,
    sr_zones: Option<&[SRZone]>,
    pivot_config: Option<&PivotConfig>,
    config: &OptimizationConfig,
    instrument: &str,
    _progress_callback: Option<&dyn Fn(usize, usize)>, // Unused in parallel mode
    htf_candle_store: Option<&MtfCandleStore>,
    granularity: &str,
) -> Result<OptimizationResult, String> {
    // Get pip value for this instrument (used for both strategy and backtest config)
    let pip_value = shared::get_pip_value(instrument);
    // Extract parameter ranges
    let filter_ids = config.param_ids.as_ref().map(|v| v.as_slice());
    let ranges = extract_param_ranges(parameters, filter_ids);

    if ranges.is_empty() {
        return Err("No optimizable parameters found. Parameters must have min, max, and step defined.".to_string());
    }

    // Check combination count and warn if very large
    let total_combinations = count_combinations(&ranges);
    if total_combinations > 10000 {
        tracing::warn!(
            "Large optimization: {} parameter combinations. This may take a while.",
            total_combinations
        );
    }

    // Generate all combinations
    let combinations = generate_combinations(&ranges);
    let min_trades = config.min_trades.unwrap_or(0);
    let objective = config.objective;

    // Determine strategy type to route scripted vs rules-based
    let definition: StrategyDefinition = serde_json::from_str(strategy_json)
        .map_err(|e| format!("Failed to parse strategy: {}", e))?;
    let is_scripted = definition.strategy_type == "scripted";

    if is_scripted {
        // Scripted strategy optimization path
        // Pre-compile once: parse metadata + compile AST (expensive), then reuse per combination
        let script = definition.script_content.as_ref()
            .ok_or("Scripted strategy missing script_content")?;
        let name = definition.name.clone();

        tracing::info!(
            "Scripted optimization: {} combinations, {} candles, script len={}",
            combinations.len(), candles.len(), script.len()
        );

        let (metadata, ast) = ScriptedStrategy::precompile(script)?;

        let completed = std::sync::atomic::AtomicUsize::new(0);
        let opt_start = std::time::Instant::now();

        let runs: Vec<OptimizationRun> = combinations
            .par_iter()
            .map(|params| {
                let mut strategy = match ScriptedStrategy::from_precompiled(&metadata, &ast, &name, params.clone()) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("Failed to create scripted strategy for params {:?}: {}", params, e);
                        return dummy_run(params, initial_balance);
                    }
                };

                strategy.set_pip_value_for_instrument(instrument);

                let backtest_config = BacktestConfig {
                    initial_balance,
                    position_size: dec!(1000),
                    use_percentage: false,
                    risk_percent: Some(dec!(1)),
                    estimated_stop_pips: dec!(20),
                    spread_pips: dec!(1),
                    pip_value,
                    instrument: instrument.to_string(),
                };

                let engine = BacktestEngine::new(backtest_config);
                let result = engine.run(&mut strategy, candles);

                let done = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if done == 1 || done % 50 == 0 || done == combinations.len() {
                    tracing::info!(
                        "Scripted opt progress: {}/{} ({:.1}s elapsed, {} trades this run)",
                        done, combinations.len(), opt_start.elapsed().as_secs_f64(),
                        result.metrics.total_trades
                    );
                }

                let trade_count = result.metrics.total_trades;
                let meets_min_trades = trade_count >= min_trades as u32;

                let score = if meets_min_trades {
                    calculate_score(&result, objective)
                } else {
                    f64::NEG_INFINITY
                };

                OptimizationRun {
                    params: params.clone(),
                    metrics: OptimizationMetrics::from(&result),
                    score,
                }
            })
            .collect();

        tracing::info!(
            "Scripted optimization complete: {} combinations in {:.1}s",
            combinations.len(), opt_start.elapsed().as_secs_f64()
        );

        return Ok(collect_optimization_result(runs, total_combinations, config.objective));
    }

    // Rules-based strategy optimization path (existing code, unchanged)
    // Run backtests in parallel using rayon
    // Each thread gets its own strategy instance - no shared mutable state
    // Progress is reported at the window level by walk_forward.rs, not per-combination
    let runs: Vec<OptimizationRun> = combinations
        .par_iter()
        .map(|params| {
            // Create strategy with parameter overrides (each thread gets its own)
            let mut strategy = match RulesBasedStrategy::from_json_with_params(strategy_json, params.clone()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to create strategy for params {:?}: {}", params, e);
                    return dummy_run(params, initial_balance);
                }
            };

            // Set pip value for the instrument (important for JPY pairs, gold, silver, indices)
            strategy.set_pip_value_for_instrument(instrument);

            // Reclassify indicators whose explicit timeframe matches the chart timeframe
            // so they route to the primary engine instead of an empty HTF engine
            strategy.set_primary_granularity(granularity);

            // Set S/R zones if provided
            if let Some(zones) = sr_zones {
                strategy.set_sr_zones(zones.to_vec());
            }

            // Set pivot config if provided
            if let Some(pivot) = pivot_config {
                strategy.set_pivot_config(pivot.clone());
            }

            // Set MTF candle store for multi-timeframe indicator support
            if let Some(store) = htf_candle_store {
                strategy.set_mtf_candle_store(store.clone());
            }

            // Create backtest config with risk-based position sizing
            let backtest_config = BacktestConfig {
                initial_balance,
                position_size: dec!(1000), // Fallback, not used when risk_percent is set
                use_percentage: false,
                risk_percent: Some(dec!(1)), // 1% risk per trade
                estimated_stop_pips: dec!(20),
                spread_pips: dec!(1),
                pip_value,
                instrument: instrument.to_string(),
            };

            // Run backtest
            let engine = BacktestEngine::new(backtest_config);
            let result = engine.run(&mut strategy, candles);

            // Check minimum trades requirement
            let trade_count = result.metrics.total_trades;
            let meets_min_trades = trade_count >= min_trades as u32;

            let score = if meets_min_trades {
                calculate_score(&result, objective)
            } else {
                f64::NEG_INFINITY // Invalid results rank last
            };

            OptimizationRun {
                params: params.clone(),
                metrics: OptimizationMetrics::from(&result),
                score,
            }
        })
        .collect();

    Ok(collect_optimization_result(runs, total_combinations, config.objective))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::rules_engine::ParameterType;

    #[test]
    fn test_parameter_range_values() {
        let range = ParameterRange {
            id: "test".to_string(),
            min: 10.0,
            max: 30.0,
            step: 10.0,
        };

        let values = range.values();
        assert_eq!(values, vec![10.0, 20.0, 30.0]);
    }

    #[test]
    fn test_parameter_range_count() {
        let range = ParameterRange {
            id: "test".to_string(),
            min: 10.0,
            max: 30.0,
            step: 10.0,
        };

        assert_eq!(range.count(), 3);
    }

    #[test]
    fn test_generate_combinations_single() {
        let ranges = vec![ParameterRange {
            id: "a".to_string(),
            min: 1.0,
            max: 2.0,
            step: 1.0,
        }];

        let combos = generate_combinations(&ranges);
        assert_eq!(combos.len(), 2);
        assert_eq!(combos[0].get("a"), Some(&1.0));
        assert_eq!(combos[1].get("a"), Some(&2.0));
    }

    #[test]
    fn test_generate_combinations_multiple() {
        let ranges = vec![
            ParameterRange {
                id: "a".to_string(),
                min: 1.0,
                max: 2.0,
                step: 1.0,
            },
            ParameterRange {
                id: "b".to_string(),
                min: 10.0,
                max: 20.0,
                step: 10.0,
            },
        ];

        let combos = generate_combinations(&ranges);
        // 2 values for a * 2 values for b = 4 combinations
        assert_eq!(combos.len(), 4);
    }

    #[test]
    fn test_count_combinations() {
        let ranges = vec![
            ParameterRange {
                id: "a".to_string(),
                min: 1.0,
                max: 3.0,
                step: 1.0, // 3 values
            },
            ParameterRange {
                id: "b".to_string(),
                min: 10.0,
                max: 40.0,
                step: 10.0, // 4 values
            },
        ];

        assert_eq!(count_combinations(&ranges), 12); // 3 * 4
    }

    #[test]
    fn test_extract_param_ranges() {
        let params = vec![
            ParameterDefinition {
                id: "rsi_period".to_string(),
                name: "RSI Period".to_string(),
                description: None,
                param_type: ParameterType::Number,
                default: 14.0,
                min: Some(7.0),
                max: Some(21.0),
                step: Some(7.0),
                options: None,
                group: None,
            },
            ParameterDefinition {
                id: "threshold".to_string(),
                name: "Threshold".to_string(),
                description: None,
                param_type: ParameterType::Number,
                default: 70.0,
                min: None, // Not optimizable
                max: None,
                step: None,
                options: None,
                group: None,
            },
        ];

        let ranges = extract_param_ranges(&params, None);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].id, "rsi_period");
    }

    #[test]
    fn test_extract_param_ranges_with_filter() {
        let params = vec![
            ParameterDefinition {
                id: "a".to_string(),
                name: "A".to_string(),
                description: None,
                param_type: ParameterType::Number,
                default: 1.0,
                min: Some(1.0),
                max: Some(3.0),
                step: Some(1.0),
                options: None,
                group: None,
            },
            ParameterDefinition {
                id: "b".to_string(),
                name: "B".to_string(),
                description: None,
                param_type: ParameterType::Number,
                default: 1.0,
                min: Some(1.0),
                max: Some(3.0),
                step: Some(1.0),
                options: None,
                group: None,
            },
        ];

        let filter = vec!["a".to_string()];
        let ranges = extract_param_ranges(&params, Some(&filter));
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].id, "a");
    }

    #[test]
    fn test_generate_combinations_empty() {
        let combos = generate_combinations(&[]);
        assert_eq!(combos.len(), 1);
        assert!(combos[0].is_empty());
    }
}
