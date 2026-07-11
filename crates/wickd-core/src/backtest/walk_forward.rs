//! Walk-Forward Analysis
//!
//! Implements rolling window optimization to validate strategy robustness.
//! For each window: optimize parameters on training data, test on out-of-sample data.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

use crate::models::Candle;
use super::engine::{BacktestConfig, BacktestEngine, SimulatedTrade};
use super::optimizer::{
    OptimizationConfig, OptimizationMetrics, OptimizationObjective,
    run_optimization,
};
use super::rules_engine::{ParameterDefinition, SRZone, StrategyDefinition};
use super::rules_strategy::RulesBasedStrategy;
use super::scripted_strategy::ScriptedStrategy;
use super::strategy::Strategy;
use super::pivots::PivotConfig;
use super::mtf::MtfCandleStore;

// ============================================================================
// Walk-Forward Types
// ============================================================================

/// Configuration for walk-forward analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardConfig {
    /// Training window duration in months (e.g., 6)
    pub train_months: u32,
    /// Test window duration in months (e.g., 1)
    pub test_months: u32,
    /// Step size for window advancement in months (typically == test_months for rolling)
    pub step_months: u32,
    /// Optimization objective for training periods
    pub objective: OptimizationObjective,
    /// Minimum trades required per window for valid result
    pub min_trades_per_window: Option<usize>,
    /// Whether to use anchored mode (expanding training window from fixed start)
    pub anchored: bool,
}

impl Default for WalkForwardConfig {
    fn default() -> Self {
        Self {
            train_months: 6,
            test_months: 1,
            step_months: 1,
            objective: OptimizationObjective::SharpeRatio,
            min_trades_per_window: Some(5),
            anchored: false,
        }
    }
}

/// A single train/test window period
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardWindow {
    /// Window number (1-indexed)
    pub window_num: usize,
    /// Training period start date (RFC3339)
    pub train_start: String,
    /// Training period end date (RFC3339)
    pub train_end: String,
    /// Test period start date (RFC3339)
    pub test_start: String,
    /// Test period end date (RFC3339)
    pub test_end: String,
}

/// Result of a single walk-forward period
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardPeriod {
    /// Window timing information
    pub window: WalkForwardWindow,
    /// Best parameters found during in-sample optimization
    pub optimized_params: HashMap<String, f64>,
    /// In-sample (training) performance metrics
    pub in_sample_metrics: OptimizationMetrics,
    /// In-sample Sharpe ratio (for efficiency calculation)
    pub in_sample_sharpe: f64,
    /// Out-of-sample (test) performance metrics
    pub out_of_sample_metrics: OptimizationMetrics,
    /// Out-of-sample Sharpe ratio (for efficiency calculation)
    pub out_of_sample_sharpe: f64,
    /// Number of trades in OOS period
    pub oos_trade_count: u32,
    /// Whether this period was profitable OOS
    pub oos_profitable: bool,
    /// Individual trades from the OOS backtest (for drill-down analysis)
    pub oos_trades: Vec<SimulatedTrade>,
}

/// Parameter stability information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterStabilityInfo {
    /// Parameter ID
    pub param_id: String,
    /// Parameter display name
    pub param_name: String,
    /// Most frequently selected value across windows
    pub mode_value: f64,
    /// How many windows selected this value
    pub mode_count: usize,
    /// Total number of windows
    pub total_windows: usize,
    /// Stability percentage (mode_count / total_windows * 100)
    pub stability_pct: f64,
}

/// Full walk-forward analysis result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardResult {
    /// Configuration used for this analysis
    pub config: WalkForwardConfig,
    /// All period results
    pub periods: Vec<WalkForwardPeriod>,
    /// Number of periods generated
    pub total_periods: usize,
    /// Number of valid periods (met min_trades in both IS and OOS)
    pub valid_periods: usize,
    /// Number of profitable OOS periods
    pub profitable_periods: usize,

    // Aggregated OOS Metrics
    /// Total OOS P&L across all test periods
    pub oos_total_pnl: String,
    /// Total OOS return percentage
    pub oos_total_return_pct: String,
    /// Average OOS Sharpe ratio
    pub oos_avg_sharpe: f64,
    /// OOS win rate (winning trades / total trades)
    pub oos_win_rate: String,
    /// OOS max drawdown across stitched equity curve
    pub oos_max_drawdown_pct: String,
    /// Total OOS trades
    pub oos_total_trades: u32,

    // Efficiency Metrics
    /// Walk-forward efficiency based on Sharpe ratio (OOS Sharpe / IS Sharpe)
    pub sharpe_efficiency: f64,
    /// Walk-forward efficiency based on returns (OOS Return / IS Return)
    pub return_efficiency: f64,
    /// Robustness score (0-100, based on consistency across periods)
    pub robustness_score: u32,

    // Parameter Stability
    /// Stability analysis for each optimized parameter
    pub parameter_stability: Vec<ParameterStabilityInfo>,

    // Stitched OOS Equity Curve (as strings for precision)
    pub oos_equity_curve: Vec<String>,
}

/// Progress phase for walk-forward
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WalkForwardPhase {
    Optimization,
    Testing,
}

/// Progress event for walk-forward
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardProgress {
    pub phase: WalkForwardPhase,
    pub window_num: usize,
    pub total_windows: usize,
    pub optimization_current: Option<usize>,
    pub optimization_total: Option<usize>,
    pub percent: u32,
    /// Training period start date (RFC3339)
    pub train_start: Option<String>,
    /// Training period end date (RFC3339)
    pub train_end: Option<String>,
    /// Test period start date (RFC3339)
    pub test_start: Option<String>,
    /// Test period end date (RFC3339)
    pub test_end: Option<String>,
}

// ============================================================================
// Window Generation
// ============================================================================

/// Generate walk-forward windows from a date range
pub fn generate_windows(
    data_start: DateTime<Utc>,
    data_end: DateTime<Utc>,
    config: &WalkForwardConfig,
) -> Vec<WalkForwardWindow> {
    let mut windows = Vec::new();
    let mut window_num = 1;

    // For anchored mode, training always starts at data_start
    // For rolling mode, training window slides forward

    let mut current_train_start = data_start;

    loop {
        // Calculate train end (train_months after train start)
        let train_end = add_months(current_train_start, config.train_months as i32);

        // Test period immediately follows training
        let test_start = train_end;
        let test_end = add_months(test_start, config.test_months as i32);

        // Stop if test end exceeds data end
        if test_end > data_end {
            break;
        }

        windows.push(WalkForwardWindow {
            window_num,
            train_start: current_train_start.to_rfc3339(),
            train_end: train_end.to_rfc3339(),
            test_start: test_start.to_rfc3339(),
            test_end: test_end.to_rfc3339(),
        });

        window_num += 1;

        // Rolling mode: slide train window forward by step_months
        // (Anchored mode uses generate_anchored_windows() instead)
        current_train_start = add_months(current_train_start, config.step_months as i32);
    }

    windows
}

/// Generate anchored walk-forward windows (expanding training window)
pub fn generate_anchored_windows(
    data_start: DateTime<Utc>,
    data_end: DateTime<Utc>,
    config: &WalkForwardConfig,
) -> Vec<WalkForwardWindow> {
    let mut windows = Vec::new();
    let mut window_num = 1;

    // Training always starts at data_start
    // First test period starts after initial train_months
    let mut test_start = add_months(data_start, config.train_months as i32);

    loop {
        let test_end = add_months(test_start, config.test_months as i32);

        // Stop if test end exceeds data end
        if test_end > data_end {
            break;
        }

        // Training window: from data_start to test_start (expanding)
        windows.push(WalkForwardWindow {
            window_num,
            train_start: data_start.to_rfc3339(),
            train_end: test_start.to_rfc3339(),
            test_start: test_start.to_rfc3339(),
            test_end: test_end.to_rfc3339(),
        });

        window_num += 1;

        // Advance test window by step_months
        test_start = add_months(test_start, config.step_months as i32);
    }

    windows
}

/// Add months to a datetime (handles month boundaries)
fn add_months(dt: DateTime<Utc>, months: i32) -> DateTime<Utc> {
    let naive = dt.naive_utc();
    let mut year = naive.year();
    let mut month = naive.month() as i32 + months;

    // Handle year rollover
    while month > 12 {
        month -= 12;
        year += 1;
    }
    while month < 1 {
        month += 12;
        year -= 1;
    }

    // Clamp day to valid range for the new month
    let max_day = days_in_month(year, month as u32);
    let day = naive.day().min(max_day);

    Utc.with_ymd_and_hms(year, month as u32, day, naive.hour(), naive.minute(), naive.second())
        .single()
        .unwrap_or(dt)
}

/// Get the number of days in a month
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

// ============================================================================
// Walk-Forward Analysis
// ============================================================================

/// Run walk-forward analysis on a strategy
pub fn run_walk_forward(
    strategy_json: &str,
    parameters: &[ParameterDefinition],
    candles: &[Candle],
    initial_balance: Decimal,
    sr_zones: Option<&[SRZone]>,
    pivot_config: Option<&PivotConfig>,
    config: &WalkForwardConfig,
    instrument: &str,
    progress_callback: Option<&dyn Fn(WalkForwardProgress)>,
    cancel_token: Option<&Arc<AtomicBool>>,
    htf_candle_store: Option<&MtfCandleStore>,
    granularity: &str,
) -> Result<WalkForwardResult, String> {
    // Get pip value for this instrument (used for both strategy and backtest config)
    let pip_value = shared::get_pip_value(instrument);

    // Get date range from candles (Candle.time is DateTime<Utc>)
    if candles.is_empty() {
        return Err("No candle data provided".to_string());
    }

    let data_start = candles[0].time;
    let data_end = candles[candles.len() - 1].time;

    // Generate windows based on mode
    let windows = if config.anchored {
        generate_anchored_windows(data_start, data_end, config)
    } else {
        generate_windows(data_start, data_end, config)
    };

    if windows.is_empty() {
        return Err("Insufficient data for walk-forward analysis. Need at least one complete train/test window.".to_string());
    }

    let total_windows = windows.len();

    // Detect strategy type for OOS test routing
    let definition: StrategyDefinition = serde_json::from_str(strategy_json)
        .map_err(|e| format!("Failed to parse strategy: {}", e))?;
    let is_scripted = definition.strategy_type == "scripted";

    // Count optimizable parameters
    let opt_param_count = parameters.iter().filter(|p| p.min.is_some() && p.max.is_some() && p.step.is_some()).count();

    // Detect baseline mode: all params have min == max (no actual optimization needed).
    // In this case, skip the optimizer entirely and just run OOS tests with defaults.
    let is_baseline = parameters.iter().all(|p| {
        match (p.min, p.max) {
            (Some(min), Some(max)) => (min - max).abs() < f64::EPSILON,
            _ => true, // No range defined = not optimizable = effectively baseline
        }
    });

    tracing::info!(
        "Starting walk-forward: {} windows, {} candles, {} optimizable params, baseline={}",
        total_windows, candles.len(), opt_param_count, is_baseline
    );
    let mut periods: Vec<WalkForwardPeriod> = Vec::new();
    let min_trades = config.min_trades_per_window.unwrap_or(5);

    // Process each window
    for (idx, window) in windows.iter().enumerate() {
        // Check for cancellation
        if let Some(token) = cancel_token {
            if token.load(Ordering::SeqCst) {
                return Err("Walk-forward analysis cancelled".to_string());
            }
        }

        // Report progress - optimization phase
        if let Some(cb) = progress_callback {
            cb(WalkForwardProgress {
                phase: WalkForwardPhase::Optimization,
                window_num: window.window_num,
                total_windows,
                optimization_current: Some(0),
                optimization_total: Some(100),
                percent: ((idx * 100) / total_windows) as u32,
                train_start: Some(window.train_start.clone()),
                train_end: Some(window.train_end.clone()),
                test_start: Some(window.test_start.clone()),
                test_end: Some(window.test_end.clone()),
            });
        }

        // Parse window times from RFC3339 strings
        let train_start = DateTime::parse_from_rfc3339(&window.train_start)
            .map_err(|e| format!("Failed to parse train_start: {}", e))?
            .with_timezone(&Utc);
        let train_end = DateTime::parse_from_rfc3339(&window.train_end)
            .map_err(|e| format!("Failed to parse train_end: {}", e))?
            .with_timezone(&Utc);
        let test_start = DateTime::parse_from_rfc3339(&window.test_start)
            .map_err(|e| format!("Failed to parse test_start: {}", e))?
            .with_timezone(&Utc);
        let test_end = DateTime::parse_from_rfc3339(&window.test_end)
            .map_err(|e| format!("Failed to parse test_end: {}", e))?
            .with_timezone(&Utc);

        // Filter candles for training period
        let train_candles: Vec<&Candle> = candles
            .iter()
            .filter(|c| c.time >= train_start && c.time < train_end)
            .collect();

        // Filter candles for test period
        let test_candles: Vec<&Candle> = candles
            .iter()
            .filter(|c| c.time >= test_start && c.time < test_end)
            .collect();

        if train_candles.is_empty() || test_candles.is_empty() {
            tracing::warn!("Window {} has empty train or test data, skipping", window.window_num);
            continue;
        }

        // Convert to owned candles
        let test_candles_owned: Vec<Candle> = test_candles.into_iter().cloned().collect();

        let (best_params, in_sample_metrics, in_sample_sharpe) = if is_baseline {
            // Baseline mode: skip optimization, use the fixed values directly.
            // When min==max, that IS the value the user wants (could differ from default).
            let fixed_params: HashMap<String, f64> = parameters
                .iter()
                .map(|p| (p.id.clone(), p.min.unwrap_or(p.default)))
                .collect();

            tracing::info!(
                "Window {}/{}: Baseline mode — skipping optimization, using fixed values for OOS",
                window.window_num, total_windows
            );

            let empty_metrics = OptimizationMetrics {
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
            };

            (fixed_params, empty_metrics, 0.0_f64)
        } else {
            // Full optimization mode
            let train_candles_owned: Vec<Candle> = train_candles.into_iter().cloned().collect();

            let opt_config = OptimizationConfig {
                objective: config.objective,
                param_ids: None,
                max_combinations: Some(10000),
                min_trades: Some(min_trades),
            };

            tracing::info!(
                "Window {}/{}: Optimizing on {} candles ({} to {})",
                window.window_num, total_windows,
                train_candles_owned.len(),
                window.train_start.split('T').next().unwrap_or(&window.train_start),
                window.train_end.split('T').next().unwrap_or(&window.train_end)
            );

            let train_htf = htf_candle_store.map(|store| store.filter_by_time_range(&train_start, &train_end));

            let opt_start = std::time::Instant::now();
            let opt_result = run_optimization(
                strategy_json,
                parameters,
                &train_candles_owned,
                initial_balance,
                sr_zones,
                pivot_config,
                &opt_config,
                instrument,
                None,
                train_htf.as_ref(),
                granularity,
            )?;

            tracing::info!(
                "Window {}/{}: Optimization completed in {:.1}s ({} combinations tested)",
                window.window_num, total_windows,
                opt_start.elapsed().as_secs_f64(),
                opt_result.total_combinations
            );

            let best = match opt_result.best_params {
                Some(params) => params,
                None => {
                    // No valid params found (e.g., no combinations met min trades).
                    // Fall back to min value (respects user's fixed range) or default.
                    tracing::warn!("Window {} optimization found no valid params, using fixed/default values", window.window_num);
                    parameters
                        .iter()
                        .map(|p| (p.id.clone(), p.min.unwrap_or(p.default)))
                        .collect()
                }
            };

            let metrics = opt_result.runs
                .first()
                .map(|r| r.metrics.clone())
                .unwrap_or_else(|| OptimizationMetrics {
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
                });

            let sharpe: f64 = metrics.sharpe_ratio.parse().unwrap_or(0.0);
            (best, metrics, sharpe)
        };

        // Report progress - testing phase
        if let Some(cb) = progress_callback {
            cb(WalkForwardProgress {
                phase: WalkForwardPhase::Testing,
                window_num: window.window_num,
                total_windows,
                optimization_current: None,
                optimization_total: None,
                percent: ((idx * 100) / total_windows + 50 / total_windows) as u32,
                train_start: Some(window.train_start.clone()),
                train_end: Some(window.train_end.clone()),
                test_start: Some(window.test_start.clone()),
                test_end: Some(window.test_end.clone()),
            });
        }

        // Run backtest on test data with optimized params
        let backtest_config = BacktestConfig {
            warmup_bars: 0,
            initial_balance,
            position_size: dec!(1000),
            use_percentage: false,
            risk_percent: Some(dec!(1)),
            estimated_stop_pips: dec!(20),
            spread_pips: dec!(1),
            pip_value,
            instrument: instrument.to_string(),
        };

        let bt_engine = BacktestEngine::new(backtest_config);

        let oos_result = if is_scripted {
            // Scripted strategy OOS path
            let script = definition.script_content.as_ref()
                .ok_or("Scripted strategy missing script_content")?;
            let mut test_strategy = ScriptedStrategy::from_script_with_params(script, &definition.name, best_params.clone())
                .map_err(|e| format!("Failed to create scripted test strategy: {}", e))?;
            test_strategy.set_pip_value_for_instrument(instrument);
            bt_engine.run(&mut test_strategy, &test_candles_owned)
        } else {
            // Rules-based strategy OOS path
            let mut test_strategy = RulesBasedStrategy::from_json_with_params(strategy_json, best_params.clone())
                .map_err(|e| format!("Failed to create test strategy: {}", e))?;

            test_strategy.set_pip_value_for_instrument(instrument);
            test_strategy.set_primary_granularity(granularity);

            if let Some(zones) = sr_zones {
                test_strategy.set_sr_zones(zones.to_vec());
            }
            if let Some(pivot) = pivot_config {
                test_strategy.set_pivot_config(pivot.clone());
            }

            if let Some(store) = htf_candle_store {
                let test_htf = store.filter_by_time_range(&test_start, &test_end);
                test_strategy.set_mtf_candle_store(test_htf);
            }

            bt_engine.run(&mut test_strategy, &test_candles_owned)
        };

        let out_of_sample_metrics = OptimizationMetrics::from(&oos_result);
        let out_of_sample_sharpe: f64 = out_of_sample_metrics.sharpe_ratio.parse().unwrap_or(0.0);
        let oos_trade_count = oos_result.metrics.total_trades;
        let oos_profitable = oos_result.metrics.total_pnl > dec!(0);

        tracing::info!(
            "Window {}/{}: OOS test {} trades, Sharpe {:.2}, P&L ${}",
            window.window_num, total_windows,
            oos_trade_count,
            out_of_sample_sharpe,
            oos_result.metrics.total_pnl
        );

        periods.push(WalkForwardPeriod {
            window: window.clone(),
            optimized_params: best_params,
            in_sample_metrics,
            in_sample_sharpe,
            out_of_sample_metrics,
            out_of_sample_sharpe,
            oos_trade_count,
            oos_profitable,
            oos_trades: oos_result.trades,
        });
    }

    // Calculate aggregated results
    let (
        oos_total_pnl,
        oos_total_return_pct,
        oos_avg_sharpe,
        oos_win_rate,
        oos_max_drawdown_pct,
        oos_total_trades,
        sharpe_efficiency,
        return_efficiency,
        robustness_score,
        oos_equity_curve,
    ) = calculate_aggregated_metrics(&periods, initial_balance);

    // Calculate parameter stability
    let parameter_stability = calculate_parameter_stability(&periods, parameters);

    // Count valid and profitable periods
    let valid_periods = periods.iter().filter(|p| p.oos_trade_count >= min_trades as u32).count();
    let profitable_periods = periods.iter().filter(|p| p.oos_profitable).count();

    tracing::info!(
        "Walk-forward complete: {} windows, {} trades, OOS P&L ${}, Sharpe {:.2}, Efficiency {:.0}%",
        total_windows,
        oos_total_trades,
        oos_total_pnl,
        oos_avg_sharpe,
        sharpe_efficiency
    );

    Ok(WalkForwardResult {
        config: config.clone(),
        periods,
        total_periods: total_windows,
        valid_periods,
        profitable_periods,
        oos_total_pnl,
        oos_total_return_pct,
        oos_avg_sharpe,
        oos_win_rate,
        oos_max_drawdown_pct,
        oos_total_trades,
        sharpe_efficiency,
        return_efficiency,
        robustness_score,
        parameter_stability,
        oos_equity_curve,
    })
}

// ============================================================================
// Aggregation and Metrics
// ============================================================================

/// Calculate aggregated metrics from all periods
fn calculate_aggregated_metrics(
    periods: &[WalkForwardPeriod],
    initial_balance: Decimal,
) -> (String, String, f64, String, String, u32, f64, f64, u32, Vec<String>) {
    if periods.is_empty() {
        return (
            "0".to_string(),
            "0".to_string(),
            0.0,
            "0".to_string(),
            "0".to_string(),
            0,
            0.0,
            0.0,
            0,
            vec![initial_balance.to_string()],
        );
    }

    // Sum OOS P&L
    let total_pnl: f64 = periods
        .iter()
        .map(|p| p.out_of_sample_metrics.total_pnl.parse::<f64>().unwrap_or(0.0))
        .sum();

    // Calculate total return (compounded)
    let mut balance = initial_balance.to_string().parse::<f64>().unwrap_or(1000.0);
    for period in periods {
        let pnl = period.out_of_sample_metrics.total_pnl.parse::<f64>().unwrap_or(0.0);
        balance += pnl;
    }
    let total_return_pct = ((balance / initial_balance.to_string().parse::<f64>().unwrap_or(1000.0)) - 1.0) * 100.0;

    // Average OOS Sharpe
    let avg_sharpe: f64 = periods.iter().map(|p| p.out_of_sample_sharpe).sum::<f64>() / periods.len() as f64;

    // Aggregate win rate (total winning trades / total trades)
    let total_wins: u32 = periods.iter().map(|p| p.out_of_sample_metrics.winning_trades).sum();
    let total_losses: u32 = periods.iter().map(|p| p.out_of_sample_metrics.losing_trades).sum();
    let total_trades = total_wins + total_losses;
    let win_rate = if total_trades > 0 {
        (total_wins as f64 / total_trades as f64) * 100.0
    } else {
        0.0
    };

    // Max drawdown from stitched equity curve
    let equity_curve = stitch_equity_curves(periods, initial_balance);
    let max_dd = calculate_max_drawdown(&equity_curve);

    // Efficiency metrics
    let (sharpe_efficiency, return_efficiency) = calculate_efficiency_metrics(periods);

    // Robustness score (0-100)
    let robustness_score = calculate_robustness_score(periods);

    // Convert equity curve to strings
    let equity_strings: Vec<String> = equity_curve.iter().map(|d| d.round_dp(2).to_string()).collect();

    (
        format!("{:.2}", total_pnl),
        format!("{:.2}", total_return_pct),
        avg_sharpe,
        format!("{:.2}", win_rate),
        format!("{:.2}", max_dd),
        total_trades,
        sharpe_efficiency,
        return_efficiency,
        robustness_score,
        equity_strings,
    )
}

/// Calculate walk-forward efficiency metrics
fn calculate_efficiency_metrics(periods: &[WalkForwardPeriod]) -> (f64, f64) {
    if periods.is_empty() {
        return (0.0, 0.0);
    }

    // Sharpe efficiency: average(OOS Sharpe / IS Sharpe) for each period
    let mut sharpe_ratios: Vec<f64> = Vec::new();
    let mut return_ratios: Vec<f64> = Vec::new();

    for period in periods {
        // Sharpe efficiency for this period
        if period.in_sample_sharpe.abs() > 0.001 {
            let ratio = period.out_of_sample_sharpe / period.in_sample_sharpe;
            // Cap at 200% to avoid outliers
            sharpe_ratios.push(ratio.min(2.0).max(-2.0));
        }

        // Return efficiency for this period
        let is_return: f64 = period.in_sample_metrics.total_return_pct.parse().unwrap_or(0.0);
        let oos_return: f64 = period.out_of_sample_metrics.total_return_pct.parse().unwrap_or(0.0);
        if is_return.abs() > 0.001 {
            let ratio = oos_return / is_return;
            return_ratios.push(ratio.min(2.0).max(-2.0));
        }
    }

    let sharpe_efficiency = if !sharpe_ratios.is_empty() {
        (sharpe_ratios.iter().sum::<f64>() / sharpe_ratios.len() as f64) * 100.0
    } else {
        0.0
    };

    let return_efficiency = if !return_ratios.is_empty() {
        (return_ratios.iter().sum::<f64>() / return_ratios.len() as f64) * 100.0
    } else {
        0.0
    };

    (sharpe_efficiency, return_efficiency)
}

/// Calculate robustness score (0-100) based on consistency
fn calculate_robustness_score(periods: &[WalkForwardPeriod]) -> u32 {
    if periods.is_empty() {
        return 0;
    }

    let total = periods.len() as f64;

    // Factor 1: Percentage of profitable OOS periods (0-40 points)
    let profitable_count = periods.iter().filter(|p| p.oos_profitable).count() as f64;
    let profitable_score = (profitable_count / total) * 40.0;

    // Factor 2: Percentage of positive OOS Sharpe ratios (0-30 points)
    let positive_sharpe_count = periods.iter().filter(|p| p.out_of_sample_sharpe > 0.0).count() as f64;
    let sharpe_score = (positive_sharpe_count / total) * 30.0;

    // Factor 3: Consistency of OOS returns (low std dev = higher score, 0-30 points)
    let returns: Vec<f64> = periods
        .iter()
        .map(|p| p.out_of_sample_metrics.total_return_pct.parse::<f64>().unwrap_or(0.0))
        .collect();

    let mean_return = returns.iter().sum::<f64>() / total;
    let variance: f64 = returns.iter().map(|r| (r - mean_return).powi(2)).sum::<f64>() / total;
    let std_dev = variance.sqrt();

    // Lower std dev = more consistent = higher score
    // Scale: std_dev of 5% = 30 points, std_dev of 50%+ = 0 points
    let consistency_score = ((50.0 - std_dev.min(50.0)) / 50.0) * 30.0;

    (profitable_score + sharpe_score + consistency_score).round() as u32
}

/// Calculate parameter stability across windows
fn calculate_parameter_stability(
    periods: &[WalkForwardPeriod],
    param_defs: &[ParameterDefinition],
) -> Vec<ParameterStabilityInfo> {
    if periods.is_empty() {
        return vec![];
    }

    let mut stability_info = Vec::new();

    // Get parameter IDs from the first period's optimized params
    let param_ids: Vec<String> = if let Some(first) = periods.first() {
        first.optimized_params.keys().cloned().collect()
    } else {
        return vec![];
    };

    for param_id in param_ids {
        // Collect all values used for this parameter across windows
        let values: Vec<f64> = periods
            .iter()
            .filter_map(|p| p.optimized_params.get(&param_id).copied())
            .collect();

        if values.is_empty() {
            continue;
        }

        // Find mode (most common value)
        let mut value_counts: HashMap<String, usize> = HashMap::new();
        for v in &values {
            // Round to avoid floating point comparison issues
            let key = format!("{:.4}", v);
            *value_counts.entry(key).or_insert(0) += 1;
        }

        let (mode_key, mode_count) = value_counts
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(k, c)| (k.clone(), *c))
            .unwrap_or(("0".to_string(), 0));

        let mode_value: f64 = mode_key.parse().unwrap_or(0.0);
        let total_windows = periods.len();
        let stability_pct = (mode_count as f64 / total_windows as f64) * 100.0;

        // Get parameter name from definitions
        let param_name = param_defs
            .iter()
            .find(|p| p.id == param_id)
            .map(|p| p.name.clone())
            .unwrap_or(param_id.clone());

        stability_info.push(ParameterStabilityInfo {
            param_id,
            param_name,
            mode_value,
            mode_count,
            total_windows,
            stability_pct,
        });
    }

    stability_info
}

/// Stitch OOS equity curves into a continuous curve
fn stitch_equity_curves(periods: &[WalkForwardPeriod], initial_balance: Decimal) -> Vec<Decimal> {
    let mut curve = vec![initial_balance];
    let mut current_balance = initial_balance;

    for period in periods {
        // Add the PnL from this period to carry forward
        let pnl: Decimal = period.out_of_sample_metrics.total_pnl
            .parse()
            .unwrap_or(dec!(0));
        current_balance += pnl;
        curve.push(current_balance);
    }

    curve
}

/// Calculate max drawdown from an equity curve
fn calculate_max_drawdown(curve: &[Decimal]) -> f64 {
    if curve.is_empty() {
        return 0.0;
    }

    let mut peak = curve[0];
    let mut max_dd = dec!(0);

    for &value in curve {
        if value > peak {
            peak = value;
        }
        // Guard against division by zero
        if peak > Decimal::ZERO {
            let dd = (peak - value) / peak * dec!(100);
            if dd > max_dd {
                max_dd = dd;
            }
        }
    }

    max_dd.to_string().parse().unwrap_or(0.0)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_months() {
        let dt = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();

        // Add 6 months
        let result = add_months(dt, 6);
        assert_eq!(result.month(), 7);
        assert_eq!(result.year(), 2024);

        // Add 12 months (year rollover)
        let result = add_months(dt, 12);
        assert_eq!(result.month(), 1);
        assert_eq!(result.year(), 2025);
    }

    #[test]
    fn test_days_in_month() {
        assert_eq!(days_in_month(2024, 1), 31);
        assert_eq!(days_in_month(2024, 2), 29); // Leap year
        assert_eq!(days_in_month(2023, 2), 28); // Non-leap year
        assert_eq!(days_in_month(2024, 4), 30);
    }

    #[test]
    fn test_generate_windows() {
        let start = Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();

        let config = WalkForwardConfig {
            train_months: 6,
            test_months: 1,
            step_months: 1,
            ..Default::default()
        };

        let windows = generate_windows(start, end, &config);

        // With 12 months of data, 6 month train, 1 month test, 1 month step
        // We should get: Jan-Jun train, Jul test; Feb-Jul train, Aug test; etc.
        // Until we can't fit another test period
        assert!(!windows.is_empty());
        assert_eq!(windows[0].window_num, 1);
    }

    #[test]
    fn test_generate_anchored_windows() {
        let start = Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();

        let config = WalkForwardConfig {
            train_months: 6,
            test_months: 1,
            step_months: 1,
            anchored: true,
            ..Default::default()
        };

        let windows = generate_anchored_windows(start, end, &config);

        // First window: train Jan-Jun, test Jul
        // Second window: train Jan-Jul, test Aug (training expands)
        assert!(!windows.is_empty());

        // All windows should have same train_start (anchored)
        for window in &windows {
            assert_eq!(window.train_start, start.to_rfc3339());
        }
    }

    #[test]
    fn test_calculate_robustness_score() {
        // Empty periods
        assert_eq!(calculate_robustness_score(&[]), 0);
    }

    #[test]
    fn test_calculate_max_drawdown() {
        let curve = vec![
            dec!(1000),
            dec!(1100),
            dec!(1050),
            dec!(1200),
            dec!(1000), // 16.67% drawdown from peak of 1200
        ];

        let dd = calculate_max_drawdown(&curve);
        assert!(dd > 16.0 && dd < 17.0);
    }
}
