//! Backtest commands for strategy testing and optimization.
//!
//! Handles running backtests, optimization, and walk-forward analysis.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tracing::info;

use crate::AppState;
use crate::commands::trading::is_valid_instrument;
use candlesight_lib::backtest::StrategyDefinition;
use candlesight_lib::backtest::mtf::{self, MtfCandleStore};

// =============================================================================
// MTF Candle Fetching
// =============================================================================

/// Fetch HTF candles for multi-timeframe strategies.
/// Extracts required timeframes from the strategy, fetches candles for each,
/// and returns a populated MtfCandleStore.
async fn fetch_htf_candles(
    strategy_json: &str,
    primary_granularity: &str,
    instrument: &str,
    from_rfc3339: &str,
    to_rfc3339: &str,
    state: &AppState,
) -> Result<MtfCandleStore, String> {
    use candlesight_lib::oanda::endpoints::{self, Granularity};
    use std::str::FromStr;

    let definition: StrategyDefinition = serde_json::from_str(strategy_json)
        .map_err(|e| format!("Failed to parse strategy for MTF extraction: {}", e))?;
    let htf_timeframes = mtf::extract_htf_timeframes(&definition, primary_granularity);

    let mut store = MtfCandleStore::new();
    if htf_timeframes.is_empty() {
        return Ok(store);
    }

    info!(
        "[MTF] Strategy requires {} HTF timeframe(s): {:?}",
        htf_timeframes.len(),
        htf_timeframes
    );

    // Clone client once outside the loop to avoid holding the read lock during network calls
    let client = state.client.read().await.clone();

    for tf in &htf_timeframes {
        let htf_gran = Granularity::from_str(tf)
            .map_err(|e| format!("Invalid HTF granularity '{}': {}", tf, e))?;
        let htf_candles = endpoints::get_candles_paginated(
            &client, instrument, htf_gran, from_rfc3339, to_rfc3339,
        )
        .await
        .map_err(|e| format!("Failed to fetch {} candles: {}", tf, e))?;
        info!("[MTF] Fetched {} candles for timeframe {}", htf_candles.len(), tf);
        store.add_timeframe(tf.clone(), htf_candles);
    }

    Ok(store)
}

// =============================================================================
// Types
// =============================================================================

/// Backtest metrics for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestResultData {
    pub total_pnl: String,
    pub total_return_pct: String,
    pub annualized_return_pct: String,
    pub winning_trades: u32,
    pub losing_trades: u32,
    pub win_rate: String,
    pub avg_win: String,
    pub avg_loss: String,
    pub profit_factor: String,
    pub max_drawdown_pct: String,
    pub sharpe_ratio: String,
    pub total_trades: u32,
    pub final_balance: String,
}

/// Trade data for frontend analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeData {
    pub trade_num: u32,
    pub direction: String,
    pub entry_time: String,
    pub exit_time: String,
    pub entry_price: String,
    pub exit_price: String,
    pub units: String,
    pub pnl: String,
    pub pnl_pct: String,
    pub cumulative_pnl: String,
}

/// Equity curve point for charting
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EquityPoint {
    pub time: String,
    pub balance: String,
}

/// Full backtest result with trades and equity curve for analysis
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestFullResult {
    pub metrics: BacktestResultData,
    pub trades: Vec<TradeData>,
    pub equity_curve: Vec<EquityPoint>,
    pub data_range: DataRange,
}

/// Detailed trade info for debugging
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugTrade {
    pub trade_num: u32,
    pub direction: String,
    pub entry_time: String,
    pub exit_time: String,
    pub entry_price: String,
    pub exit_price: String,
    pub units: String,
    pub pnl: String,
}

/// Candle data for debugging
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugCandle {
    pub time: String,
    pub open: String,
    pub high: String,
    pub low: String,
    pub close: String,
}

/// Detailed backtest result for debugging/validation
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestDebugData {
    pub metrics: BacktestResultData,
    pub trades: Vec<DebugTrade>,
    pub equity_curve: Vec<EquityPoint>,
    pub data_range: DataRange,
    pub sample_candles: SampleCandles,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataRange {
    pub start_time: String,
    pub end_time: String,
    pub total_candles: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleCandles {
    pub first_10: Vec<DebugCandle>,
    pub last_10: Vec<DebugCandle>,
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Maximum number of equity curve points to send to the frontend.
/// Larger datasets are downsampled using LTTB (Largest-Triangle-Three-Buckets)
/// to preserve visual shape while reducing rendering overhead in lightweight-charts.
const MAX_EQUITY_CURVE_POINTS: usize = 500;

/// Downsample equity curve data using LTTB algorithm.
/// Always preserves the first and last points. For datasets smaller than
/// max_points, returns the data unchanged.
///
/// Balance values are parsed via `rust_decimal::Decimal` to maintain financial
/// precision, then converted to f64 only for the geometric triangle-area
/// comparison (which is purely for point selection, not financial calculation).
fn downsample_equity_curve(data: Vec<EquityPoint>, max_points: usize) -> Vec<EquityPoint> {
    use rust_decimal::Decimal;
    use rust_decimal::prelude::ToPrimitive;
    use std::str::FromStr;

    if data.len() <= max_points || max_points < 3 {
        return data;
    }

    /// Parse balance string via Decimal for precision, then convert to f64
    /// for geometric LTTB triangle-area math. Logs a warning and falls back
    /// to 0.0 if the balance string is unparseable (should never happen since
    /// balances are serialised from Decimal::to_string).
    fn balance_as_f64(balance: &str) -> f64 {
        Decimal::from_str(balance)
            .map(|d| d.to_f64().unwrap_or(0.0))
            .unwrap_or_else(|_| {
                tracing::warn!("LTTB: unparseable balance string: {:?}", balance);
                0.0
            })
    }

    let mut result = Vec::with_capacity(max_points);

    // Always include the first point
    result.push(data[0].clone());

    // Number of buckets between first and last point
    let bucket_count = max_points - 2;
    let bucket_size = (data.len() - 2) as f64 / bucket_count as f64;

    for i in 0..bucket_count {
        let bucket_start = 1 + (i as f64 * bucket_size) as usize;
        let bucket_end = 1 + ((i + 1) as f64 * bucket_size) as usize;
        let bucket_end = bucket_end.min(data.len() - 1);

        // When bucket collapses to a single point (bucket_start >= bucket_end),
        // still add that point so the output has the expected number of entries
        // and subsequent prev_idx calculations remain correct.
        if bucket_start >= bucket_end {
            result.push(data[bucket_start.min(data.len() - 2)].clone());
            continue;
        }

        // For LTTB, pick the point that forms the largest triangle with the
        // previous selected point and the average of the next bucket.
        // f64 is used here intentionally — triangle area is a geometric
        // comparison for point selection, not a financial calculation.
        let prev_balance: f64 = result.last()
            .map(|p: &EquityPoint| balance_as_f64(&p.balance))
            .unwrap_or(0.0);
        let prev_idx = result.len() as f64 - 1.0;

        // Calculate average of next bucket (or last point if final bucket).
        // Note: for the final bucket (i + 1 == bucket_count), the next "bucket"
        // includes the last data point which is also unconditionally pushed at
        // the end. This is acceptable — LTTB treats the last point as a fixed
        // anchor, and including it in the average for the final bucket's triangle
        // calculation does not affect the anchor itself.
        let next_bucket_start = bucket_end;
        let next_bucket_end = if i + 1 < bucket_count {
            (1 + ((i + 2) as f64 * bucket_size) as usize).min(data.len() - 1)
        } else {
            data.len()
        };

        let mut next_avg = 0.0;
        let next_count = next_bucket_end - next_bucket_start;
        if next_count > 0 {
            for j in next_bucket_start..next_bucket_end {
                next_avg += balance_as_f64(&data[j].balance);
            }
            next_avg /= next_count as f64;
        }
        let next_idx = (next_bucket_start + next_bucket_end) as f64 / 2.0;

        // Find point in current bucket with largest triangle area
        let mut max_area = -1.0_f64;
        let mut max_idx = bucket_start;

        for j in bucket_start..bucket_end {
            let balance = balance_as_f64(&data[j].balance);
            let area = ((prev_idx - next_idx) * (balance - prev_balance)
                - (prev_idx - j as f64) * (next_avg - prev_balance))
                .abs();
            if area > max_area {
                max_area = area;
                max_idx = j;
            }
        }

        result.push(data[max_idx].clone());
    }

    // Always include the last point
    result.push(data[data.len() - 1].clone());

    result
}

/// Build BacktestDebugData from a BacktestResult and candle data.
/// Shared by both rules-based and scripted strategy debug paths.
fn build_debug_data(
    result: candlesight_lib::backtest::BacktestResult,
    candles: &[candlesight_lib::models::Candle],
) -> BacktestDebugData {
    let debug_trades: Vec<DebugTrade> = result
        .trades
        .iter()
        .enumerate()
        .map(|(i, t)| DebugTrade {
            trade_num: (i + 1) as u32,
            direction: if t.is_long {
                "LONG".to_string()
            } else {
                "SHORT".to_string()
            },
            entry_time: t.entry_time.clone(),
            exit_time: t.exit_time.clone().unwrap_or_default(),
            entry_price: t.entry_price.to_string(),
            exit_price: t.exit_price.map(|p| p.to_string()).unwrap_or_default(),
            units: t.units.to_string(),
            pnl: t.pnl.round_dp(2).to_string(),
        })
        .collect();

    let to_debug_candle = |c: &candlesight_lib::models::Candle| DebugCandle {
        time: c.time.to_rfc3339(),
        open: c.mid.open.to_string(),
        high: c.mid.high.to_string(),
        low: c.mid.low.to_string(),
        close: c.mid.close.to_string(),
    };

    let first_10: Vec<DebugCandle> = candles.iter().take(10).map(to_debug_candle).collect();
    let last_10: Vec<DebugCandle> = candles
        .iter()
        .rev()
        .take(10)
        .rev()
        .map(to_debug_candle)
        .collect();

    let data_range = DataRange {
        start_time: candles
            .first()
            .map(|c| c.time.to_rfc3339())
            .unwrap_or_default(),
        end_time: candles
            .last()
            .map(|c| c.time.to_rfc3339())
            .unwrap_or_default(),
        total_candles: candles.len() as u32,
    };

    let metrics = BacktestResultData {
        total_pnl: result.metrics.total_pnl.to_string(),
        total_return_pct: result.metrics.total_return_pct.round_dp(2).to_string(),
        annualized_return_pct: result.metrics.annualized_return_pct.round_dp(2).to_string(),
        winning_trades: result.metrics.winning_trades,
        losing_trades: result.metrics.losing_trades,
        win_rate: result.metrics.win_rate.round_dp(1).to_string(),
        avg_win: result.metrics.avg_win.round_dp(2).to_string(),
        avg_loss: result.metrics.avg_loss.round_dp(2).to_string(),
        profit_factor: result.metrics.profit_factor.round_dp(2).to_string(),
        max_drawdown_pct: result.metrics.max_drawdown_pct.round_dp(2).to_string(),
        sharpe_ratio: result.metrics.sharpe_ratio.round_dp(2).to_string(),
        total_trades: result.metrics.total_trades,
        final_balance: result.final_balance.round_dp(2).to_string(),
    };

    let full_equity_curve: Vec<EquityPoint> = result
        .equity_curve
        .iter()
        .skip(1)
        .enumerate()
        .filter_map(|(i, &balance)| {
            candles.get(i).map(|c| EquityPoint {
                time: c.time.to_rfc3339(),
                balance: balance.round_dp(2).to_string(),
            })
        })
        .collect();
    let equity_curve = downsample_equity_curve(full_equity_curve, MAX_EQUITY_CURVE_POINTS);

    BacktestDebugData {
        metrics,
        trades: debug_trades,
        equity_curve,
        data_range,
        sample_candles: SampleCandles { first_10, last_10 },
    }
}

fn run_backtest_with_strategy<S: candlesight_lib::backtest::Strategy>(
    strategy: &mut S,
    candles: &[candlesight_lib::models::Candle],
    initial_balance: Option<f64>,
    risk_percent: Option<f64>,
    instrument: &str,
) -> BacktestFullResult {
    use candlesight_lib::backtest::{BacktestConfig, BacktestEngine};
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;
    use std::str::FromStr;

    let init_balance = initial_balance
        .map(|v| Decimal::from_str(&v.to_string()).unwrap_or(dec!(10000)))
        .unwrap_or(dec!(10000));

    // Use risk-based position sizing for proper compounding
    let risk_pct = risk_percent
        .map(|v| Decimal::from_str(&v.to_string()).unwrap_or(dec!(1)))
        .unwrap_or(dec!(1)); // Default 1% risk per trade

    let config = BacktestConfig {
        warmup_bars: 0,
        initial_balance: init_balance,
        position_size: dec!(1000), // Fallback, not used when risk_percent is set
        use_percentage: false,
        risk_percent: Some(risk_pct),
        estimated_stop_pips: dec!(20), // Reasonable default for forex
        spread_pips: dec!(1),
        pip_value: dec!(0.0001),
        instrument: instrument.to_string(),
    };

    let engine = BacktestEngine::new(config);
    let result = engine.run(strategy, candles);

    // Build trades with cumulative P&L and percentage
    let mut cumulative_pnl = Decimal::ZERO;
    let trades: Vec<TradeData> = result
        .trades
        .iter()
        .enumerate()
        .map(|(i, trade)| {
            cumulative_pnl += trade.pnl;
            let pnl_pct = if init_balance > Decimal::ZERO {
                (trade.pnl / init_balance * dec!(100)).round_dp(2)
            } else {
                Decimal::ZERO
            };

            TradeData {
                trade_num: (i + 1) as u32,
                direction: if trade.is_long {
                    "LONG".to_string()
                } else {
                    "SHORT".to_string()
                },
                entry_time: trade.entry_time.clone(),
                exit_time: trade.exit_time.clone().unwrap_or_default(),
                entry_price: trade.entry_price.to_string(),
                exit_price: trade.exit_price.map(|p| p.to_string()).unwrap_or_default(),
                units: trade.units.to_string(),
                pnl: trade.pnl.round_dp(2).to_string(),
                pnl_pct: pnl_pct.to_string(),
                cumulative_pnl: cumulative_pnl.round_dp(2).to_string(),
            }
        })
        .collect();

    // Build equity curve with timestamps from candles
    let full_equity_curve: Vec<EquityPoint> = result
        .equity_curve
        .iter()
        .skip(1) // Skip initial balance (no candle timestamp for it)
        .enumerate()
        .filter_map(|(i, &balance)| {
            candles.get(i).map(|c| EquityPoint {
                time: c.time.to_rfc3339(),
                balance: balance.round_dp(2).to_string(),
            })
        })
        .collect();

    // Downsample equity curve to reduce frontend rendering overhead (BUG-067)
    let equity_curve = downsample_equity_curve(full_equity_curve, MAX_EQUITY_CURVE_POINTS);

    let data_range = DataRange {
        start_time: candles
            .first()
            .map(|c| c.time.to_rfc3339())
            .unwrap_or_default(),
        end_time: candles
            .last()
            .map(|c| c.time.to_rfc3339())
            .unwrap_or_default(),
        total_candles: candles.len() as u32,
    };

    let metrics = BacktestResultData {
        total_pnl: result.metrics.total_pnl.to_string(),
        total_return_pct: result.metrics.total_return_pct.round_dp(2).to_string(),
        annualized_return_pct: result.metrics.annualized_return_pct.round_dp(2).to_string(),
        winning_trades: result.metrics.winning_trades,
        losing_trades: result.metrics.losing_trades,
        win_rate: result.metrics.win_rate.round_dp(1).to_string(),
        avg_win: result.metrics.avg_win.round_dp(2).to_string(),
        avg_loss: result.metrics.avg_loss.round_dp(2).to_string(),
        profit_factor: result.metrics.profit_factor.round_dp(2).to_string(),
        max_drawdown_pct: result.metrics.max_drawdown_pct.round_dp(2).to_string(),
        sharpe_ratio: result.metrics.sharpe_ratio.round_dp(2).to_string(),
        total_trades: result.metrics.total_trades,
        final_balance: result.final_balance.round_dp(2).to_string(),
    };

    BacktestFullResult {
        metrics,
        trades,
        equity_curve,
        data_range,
    }
}

// =============================================================================
// Commands
// =============================================================================

/// Run a backtest with a built-in strategy
///
/// # Arguments
/// * `instrument` - The currency pair (e.g., "EUR_USD")
/// * `granularity` - Time period: M1, M5, M15, H1, H4, D
/// * `strategy` - Strategy name: "ma_crossover" or "rsi"
/// * `strategy_params` - JSON params for strategy
/// * `count` - Number of candles (default 500)
/// * `initial_balance` - Starting balance (default 10000)
/// * `position_size` - Position size in units (default 1000)
#[tauri::command]
pub async fn run_backtest(
    instrument: String,
    granularity: String,
    strategy: String,
    strategy_params: Option<String>,
    count: Option<u32>,
    initial_balance: Option<f64>,
    position_size: Option<f64>,
    state: State<'_, AppState>,
) -> Result<BacktestFullResult, String> {
    use candlesight_lib::backtest::{MovingAverageCrossover, RsiStrategy};
    use candlesight_lib::oanda::endpoints::{self, Granularity};
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;
    use std::str::FromStr;

    // Parse granularity
    let gran = Granularity::from_str(&granularity).map_err(|e| e.to_string())?;

    // Fetch historical candles
    let client = state.client.read().await;
    let candles = endpoints::get_candles(
        &*client,
        &instrument,
        gran,
        Some(count.unwrap_or(500)),
        None,
        None,
    )
    .await
    .map_err(|e| e.to_string())?;

    if candles.is_empty() {
        return Err("No candle data available".to_string());
    }

    // Parse strategy params
    let params: serde_json::Value = strategy_params
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or(serde_json::json!({}));

    // BUG-067: Run CPU-intensive backtest computation on a blocking thread
    // to avoid starving the tokio async runtime.
    let strategy_name = strategy.clone();
    let result = tokio::task::spawn_blocking(move || {
        match strategy_name.as_str() {
            "ma_crossover" => {
                let fast = params
                    .get("fast")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10) as usize;
                let slow = params
                    .get("slow")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(20) as usize;
                let mut strat = MovingAverageCrossover::new(fast, slow);
                Ok(run_backtest_with_strategy(&mut strat, &candles, initial_balance, position_size, &instrument))
            }
            "rsi" => {
                let period = params
                    .get("period")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(14) as usize;
                let overbought = params
                    .get("overbought")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(70.0);
                let oversold = params
                    .get("oversold")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(30.0);
                let mut strat = RsiStrategy::new(
                    period,
                    Decimal::from_str(&overbought.to_string()).unwrap_or(dec!(70)),
                    Decimal::from_str(&oversold.to_string()).unwrap_or(dec!(30)),
                );
                Ok(run_backtest_with_strategy(&mut strat, &candles, initial_balance, position_size, &instrument))
            }
            _ => Err(format!("Unknown strategy: {}", strategy_name)),
        }
    })
    .await
    .map_err(|e| format!("Backtest task failed: {}", e))??;

    Ok(result)
}

/// Run a backtest with a custom rules-based or scripted strategy
#[tauri::command]
pub async fn run_custom_backtest(
    instrument: String,
    granularity: String,
    strategy_json: String,
    count: Option<u32>,
    date_from: Option<String>,
    date_to: Option<String>,
    initial_balance: Option<f64>,
    risk_percent: Option<f64>,
    sr_zones_json: Option<String>,
    pivot_config_json: Option<String>,
    state: State<'_, AppState>,
) -> Result<BacktestFullResult, String> {
    use candlesight_lib::backtest::{RulesBasedStrategy, ScriptedStrategy};
    use candlesight_lib::oanda::endpoints::{self, Granularity};
    use std::str::FromStr;

    // Validate instrument format
    if !is_valid_instrument(&instrument) {
        return Err(format!("Invalid instrument format: {}", instrument));
    }

    // Validate date formats if provided (YYYY-MM-DD)
    if let Some(ref date) = date_from {
        if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
            return Err(format!(
                "Invalid date_from format '{}' (expected YYYY-MM-DD)",
                date
            ));
        }
    }
    if let Some(ref date) = date_to {
        if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
            return Err(format!(
                "Invalid date_to format '{}' (expected YYYY-MM-DD)",
                date
            ));
        }
    }

    // Parse strategy_type early to determine routing
    let definition: StrategyDefinition = serde_json::from_str(&strategy_json)
        .map_err(|e| format!("Failed to parse strategy definition: {}", e))?;
    let is_scripted = definition.strategy_type == "scripted";

    // Parse granularity
    let gran = Granularity::from_str(&granularity).map_err(|e| e.to_string())?;

    // Fetch historical candles - use date range if provided, otherwise use count
    let candles = if let (Some(from), Some(to)) = (&date_from, &date_to) {
        let from_rfc3339 = format!("{}T00:00:00Z", from);
        let to_rfc3339 = {
            let today_utc = chrono::Utc::now().format("%Y-%m-%d").to_string();
            if to >= &today_utc {
                (chrono::Utc::now() - chrono::Duration::minutes(1))
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            } else {
                format!("{}T23:59:59Z", to)
            }
        };

        let client = state.client.read().await;
        endpoints::get_candles_paginated(&*client, &instrument, gran, &from_rfc3339, &to_rfc3339)
            .await
            .map_err(|e| e.to_string())?
    } else if let Some(from) = &date_from {
        let from_rfc3339 = format!("{}T00:00:00Z", from);
        let client = state.client.read().await;
        endpoints::get_candles(
            &*client,
            &instrument,
            gran,
            Some(5000),
            Some(&from_rfc3339),
            None,
        )
        .await
        .map_err(|e| e.to_string())?
    } else {
        let client = state.client.read().await;
        endpoints::get_candles(
            &*client,
            &instrument,
            gran,
            Some(count.unwrap_or(500).min(5000)),
            None,
            None,
        )
        .await
        .map_err(|e| e.to_string())?
    };

    if candles.is_empty() {
        return Err("No candle data available".to_string());
    }

    if is_scripted {
        // Scripted strategy path
        // Note: S/R zones, pivot config, and multi-timeframe candles are not yet
        // supported for scripted strategies. These parameters are intentionally
        // not passed through — the Rhai SDK doesn't expose them yet.
        let script = definition.script_content.as_ref()
            .ok_or("Scripted strategy missing script_content")?;
        // AGT-651: construct through the shared host constructor so the app
        // wires the exact same pip value + event calendar + surprise feed as
        // the wickd CLI (dialect report D1/D2: the app used to skip the
        // calendar setters, silently pinning hours_since_event() at -1 and
        // surprise_z() at -9999 for every app-run script).
        let mut strategy =
            ScriptedStrategy::for_host(script, &definition.name, HashMap::new(), &instrument)
                .map_err(|e| format!("Failed to create scripted strategy: {}", e))?;

        // BUG-067: Run CPU-intensive backtest computation on a blocking thread
        let result = tokio::task::spawn_blocking(move || {
            run_backtest_with_strategy(
                &mut strategy,
                &candles,
                initial_balance,
                risk_percent,
                &instrument,
            )
        })
        .await
        .map_err(|e| format!("Backtest task failed: {}", e))?;

        Ok(result)
    } else {
        // Existing rules-based strategy path
        let mut strategy = RulesBasedStrategy::from_json(&strategy_json)?;

        // Set pip value for the instrument (important for JPY pairs, gold, silver, indices)
        strategy.set_pip_value_for_instrument(&instrument);

        // Reclassify indicators whose explicit timeframe matches the chart timeframe
        // so they route to the primary engine instead of an empty HTF engine
        strategy.set_primary_granularity(&granularity);

        // Set S/R zones if provided
        if let Some(ref zones_json) = sr_zones_json {
            strategy.set_sr_zones_from_json(zones_json)?;
        }

        // Set pivot config if provided
        if let Some(ref config_json) = pivot_config_json {
            strategy.set_pivot_config_from_json(config_json)?;
        }

        // Fetch HTF candles for multi-timeframe strategies
        {
            let from_ts = candles.first().ok_or("No candles fetched")?.time.to_rfc3339();
            let to_ts = candles.last().ok_or("No candles fetched")?.time.to_rfc3339();
            let mtf_store = fetch_htf_candles(
                &strategy_json, &granularity, &instrument, &from_ts, &to_ts, &state,
            ).await?;
            if !mtf_store.is_empty() {
                strategy.set_mtf_candle_store(mtf_store);
            }
        }

        // BUG-067: Run CPU-intensive backtest computation on a blocking thread
        // to avoid starving the tokio async runtime. The backtest engine iterates
        // thousands of candles with Decimal arithmetic which can block for seconds.
        let result = tokio::task::spawn_blocking(move || {
            run_backtest_with_strategy(
                &mut strategy,
                &candles,
                initial_balance,
                risk_percent,
                &instrument,
            )
        })
        .await
        .map_err(|e| format!("Backtest task failed: {}", e))?;

        Ok(result)
    }
}

/// Run a backtest with debug output for validation
#[tauri::command]
pub async fn run_backtest_debug(
    instrument: String,
    granularity: String,
    strategy_json: String,
    count: Option<u32>,
    date_from: Option<String>,
    date_to: Option<String>,
    initial_balance: Option<f64>,
    risk_percent: Option<f64>,
    sr_zones_json: Option<String>,
    pivot_config_json: Option<String>,
    state: State<'_, AppState>,
) -> Result<BacktestDebugData, String> {
    use candlesight_lib::backtest::{BacktestConfig, BacktestEngine, RulesBasedStrategy, ScriptedStrategy};
    use candlesight_lib::oanda::endpoints::{self, Granularity};
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;
    use std::str::FromStr;

    // Validate instrument format
    if !is_valid_instrument(&instrument) {
        return Err(format!("Invalid instrument format: {}", instrument));
    }

    // Validate date formats if provided
    if let Some(ref date) = date_from {
        if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
            return Err(format!(
                "Invalid date_from format '{}' (expected YYYY-MM-DD)",
                date
            ));
        }
    }
    if let Some(ref date) = date_to {
        if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
            return Err(format!(
                "Invalid date_to format '{}' (expected YYYY-MM-DD)",
                date
            ));
        }
    }

    // Parse strategy_type early to determine routing
    let definition: StrategyDefinition = serde_json::from_str(&strategy_json)
        .map_err(|e| format!("Failed to parse strategy definition: {}", e))?;
    let is_scripted = definition.strategy_type == "scripted";

    // Parse granularity
    let gran = Granularity::from_str(&granularity).map_err(|e| e.to_string())?;

    // Fetch historical candles
    let candles = if date_from.is_some() || date_to.is_some() {
        let from_rfc3339 = date_from.as_ref().map(|d| format!("{}T00:00:00Z", d));
        let to_rfc3339 = date_to.as_ref().map(|d| {
            let today_utc = chrono::Utc::now().format("%Y-%m-%d").to_string();
            if d >= &today_utc {
                (chrono::Utc::now() - chrono::Duration::minutes(1))
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            } else {
                format!("{}T23:59:59Z", d)
            }
        });

        let client = state.client.read().await;
        endpoints::get_candles(
            &*client,
            &instrument,
            gran,
            Some(5000),
            from_rfc3339.as_deref(),
            to_rfc3339.as_deref(),
        )
        .await
        .map_err(|e| e.to_string())?
    } else {
        let client = state.client.read().await;
        endpoints::get_candles(
            &*client,
            &instrument,
            gran,
            Some(count.unwrap_or(500).min(5000)),
            None,
            None,
        )
        .await
        .map_err(|e| e.to_string())?
    };

    if candles.is_empty() {
        return Err("No candle data available".to_string());
    }

    if is_scripted {
        // Scripted strategy path
        let script = definition.script_content.as_ref()
            .ok_or("Scripted strategy missing script_content")?
            .clone();
        let strategy_name = definition.name.clone();
        let instrument_clone = instrument.clone();

        // BUG-067: Run CPU-intensive backtest computation on a blocking thread
        let result = tokio::task::spawn_blocking(move || {
            // AGT-651: shared host constructor (see run_custom_backtest).
            let mut strategy = ScriptedStrategy::for_host(
                &script,
                &strategy_name,
                HashMap::new(),
                &instrument_clone,
            )
            .map_err(|e| format!("Failed to create scripted strategy: {}", e))?;

            let risk_pct = risk_percent
                .map(|v| Decimal::from_str(&v.to_string()).unwrap_or(dec!(1)))
                .unwrap_or(dec!(1));

            let pip_value = shared::get_pip_value(&instrument_clone);
            let config = BacktestConfig {
                warmup_bars: 0,
                initial_balance: initial_balance
                    .map(|v| Decimal::from_str(&v.to_string()).unwrap_or(dec!(10000)))
                    .unwrap_or(dec!(10000)),
                position_size: dec!(1000),
                use_percentage: false,
                risk_percent: Some(risk_pct),
                estimated_stop_pips: dec!(20),
                spread_pips: dec!(1),
                pip_value,
                instrument: instrument_clone.clone(),
            };

            let engine = BacktestEngine::new(config);
            let result = engine.run(&mut strategy, &candles);

            Ok::<BacktestDebugData, String>(build_debug_data(result, &candles))
        })
        .await
        .map_err(|e| format!("Debug backtest task failed: {}", e))??;

        return Ok(result);
    }

    // Existing rules-based strategy path
    let mut strategy = RulesBasedStrategy::from_json(&strategy_json)?;
    strategy.set_pip_value_for_instrument(&instrument);
    strategy.set_primary_granularity(&granularity);

    // Set S/R zones if provided
    if let Some(ref zones_json) = sr_zones_json {
        strategy.set_sr_zones_from_json(zones_json)?;
    }
    if let Some(ref config_json) = pivot_config_json {
        strategy.set_pivot_config_from_json(config_json)?;
    }

    // Fetch HTF candles for multi-timeframe strategies
    {
        let from_ts = candles.first().ok_or("No candles fetched")?.time.to_rfc3339();
        let to_ts = candles.last().ok_or("No candles fetched")?.time.to_rfc3339();
        let mtf_store = fetch_htf_candles(
            &strategy_json, &granularity, &instrument, &from_ts, &to_ts, &state,
        ).await?;
        if !mtf_store.is_empty() {
            strategy.set_mtf_candle_store(mtf_store);
        }
    }

    // BUG-067: Run CPU-intensive backtest computation on a blocking thread
    let instrument_clone = instrument.clone();
    let result = tokio::task::spawn_blocking(move || {
        // Use risk-based position sizing for compounding
        let risk_pct = risk_percent
            .map(|v| Decimal::from_str(&v.to_string()).unwrap_or(dec!(1)))
            .unwrap_or(dec!(1));

        // Configure and run backtest
        let pip_value = shared::get_pip_value(&instrument_clone);
        let config = BacktestConfig {
            warmup_bars: 0,
            initial_balance: initial_balance
                .map(|v| Decimal::from_str(&v.to_string()).unwrap_or(dec!(10000)))
                .unwrap_or(dec!(10000)),
            position_size: dec!(1000),
            use_percentage: false,
            risk_percent: Some(risk_pct),
            estimated_stop_pips: dec!(20),
            spread_pips: dec!(1),
            pip_value,
            instrument: instrument_clone.clone(),
        };

        let engine = BacktestEngine::new(config);
        let result = engine.run(&mut strategy, &candles);

        // Build debug trades
        let debug_trades: Vec<DebugTrade> = result
            .trades
            .iter()
            .enumerate()
            .map(|(i, t)| DebugTrade {
                trade_num: (i + 1) as u32,
                direction: if t.is_long {
                    "LONG".to_string()
                } else {
                    "SHORT".to_string()
                },
                entry_time: t.entry_time.clone(),
                exit_time: t.exit_time.clone().unwrap_or_default(),
                entry_price: t.entry_price.to_string(),
                exit_price: t.exit_price.map(|p| p.to_string()).unwrap_or_default(),
                units: t.units.to_string(),
                pnl: t.pnl.round_dp(2).to_string(),
            })
            .collect();

        // Build sample candles
        let to_debug_candle = |c: &candlesight_lib::models::Candle| DebugCandle {
            time: c.time.to_rfc3339(),
            open: c.mid.open.to_string(),
            high: c.mid.high.to_string(),
            low: c.mid.low.to_string(),
            close: c.mid.close.to_string(),
        };

        let first_10: Vec<DebugCandle> = candles.iter().take(10).map(to_debug_candle).collect();
        let last_10: Vec<DebugCandle> = candles
            .iter()
            .rev()
            .take(10)
            .rev()
            .map(to_debug_candle)
            .collect();

        let data_range = DataRange {
            start_time: candles
                .first()
                .map(|c| c.time.to_rfc3339())
                .unwrap_or_default(),
            end_time: candles
                .last()
                .map(|c| c.time.to_rfc3339())
                .unwrap_or_default(),
            total_candles: candles.len() as u32,
        };

        let metrics = BacktestResultData {
            total_pnl: result.metrics.total_pnl.to_string(),
            total_return_pct: result.metrics.total_return_pct.round_dp(2).to_string(),
            annualized_return_pct: result.metrics.annualized_return_pct.round_dp(2).to_string(),
            winning_trades: result.metrics.winning_trades,
            losing_trades: result.metrics.losing_trades,
            win_rate: result.metrics.win_rate.round_dp(1).to_string(),
            avg_win: result.metrics.avg_win.round_dp(2).to_string(),
            avg_loss: result.metrics.avg_loss.round_dp(2).to_string(),
            profit_factor: result.metrics.profit_factor.round_dp(2).to_string(),
            max_drawdown_pct: result.metrics.max_drawdown_pct.round_dp(2).to_string(),
            sharpe_ratio: result.metrics.sharpe_ratio.round_dp(2).to_string(),
            total_trades: result.metrics.total_trades,
            final_balance: result.final_balance.round_dp(2).to_string(),
        };

        // Build and downsample equity curve (BUG-067: prevent unbounded rendering)
        let full_equity_curve: Vec<EquityPoint> = result
            .equity_curve
            .iter()
            .skip(1) // Skip initial balance (no candle timestamp for it)
            .enumerate()
            .filter_map(|(i, &balance)| {
                candles.get(i).map(|c| EquityPoint {
                    time: c.time.to_rfc3339(),
                    balance: balance.round_dp(2).to_string(),
                })
            })
            .collect();
        let equity_curve = downsample_equity_curve(full_equity_curve, MAX_EQUITY_CURVE_POINTS);

        BacktestDebugData {
            metrics,
            trades: debug_trades,
            equity_curve,
            data_range,
            sample_candles: SampleCandles { first_10, last_10 },
        }
    })
    .await
    .map_err(|e| format!("Debug backtest task failed: {}", e))?;

    Ok(result)
}

/// Run parameter optimization using grid search
#[tauri::command]
pub async fn optimize_strategy(
    instrument: String,
    granularity: String,
    strategy_json: String,
    parameters_json: String,
    date_from: Option<String>,
    date_to: Option<String>,
    count: Option<u32>,
    initial_balance: Option<f64>,
    sr_zones_json: Option<String>,
    pivot_config_json: Option<String>,
    objective: Option<String>,
    param_ids: Option<Vec<String>>,
    max_combinations: Option<usize>,
    min_trades: Option<usize>,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<candlesight_lib::backtest::OptimizationResult, String> {
    use candlesight_lib::backtest::{
        optimizer,
        pivots::PivotConfig,
        rules_engine::{ParameterDefinition, SRZone},
        OptimizationConfig, OptimizationObjective,
    };
    use candlesight_lib::oanda::endpoints::{self, Granularity};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    // Validate instrument format
    if !is_valid_instrument(&instrument) {
        return Err(format!("Invalid instrument format: {}", instrument));
    }

    // Validate date formats if provided
    if let Some(ref date) = date_from {
        if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
            return Err(format!(
                "Invalid date_from format '{}' (expected YYYY-MM-DD)",
                date
            ));
        }
    }
    if let Some(ref date) = date_to {
        if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
            return Err(format!(
                "Invalid date_to format '{}' (expected YYYY-MM-DD)",
                date
            ));
        }
    }

    // Parse parameters
    info!("[optimize_strategy] Raw parameters_json: {}", parameters_json);
    let parameters: Vec<ParameterDefinition> = serde_json::from_str(&parameters_json)
        .map_err(|e| format!("Failed to parse parameters JSON: {}", e))?;

    // Parse granularity
    let gran = Granularity::from_str(&granularity).map_err(|e| e.to_string())?;

    // Fetch historical candles with pagination for large date ranges
    let candles = if let (Some(from), Some(to)) = (&date_from, &date_to) {
        let from_rfc3339 = format!("{}T00:00:00Z", from);
        let to_rfc3339 = {
            let today_utc = chrono::Utc::now().format("%Y-%m-%d").to_string();
            if to >= &today_utc {
                (chrono::Utc::now() - chrono::Duration::minutes(1))
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            } else {
                format!("{}T23:59:59Z", to)
            }
        };

        let client = state.client.read().await;
        endpoints::get_candles_paginated(&*client, &instrument, gran, &from_rfc3339, &to_rfc3339)
            .await
            .map_err(|e| e.to_string())?
    } else if let Some(from) = &date_from {
        let from_rfc3339 = format!("{}T00:00:00Z", from);
        let client = state.client.read().await;
        endpoints::get_candles(
            &*client,
            &instrument,
            gran,
            Some(5000),
            Some(&from_rfc3339),
            None,
        )
        .await
        .map_err(|e| e.to_string())?
    } else {
        let client = state.client.read().await;
        endpoints::get_candles(
            &*client,
            &instrument,
            gran,
            Some(count.unwrap_or(500).min(5000)),
            None,
            None,
        )
        .await
        .map_err(|e| e.to_string())?
    };

    if candles.is_empty() {
        return Err("No candle data available".to_string());
    }

    // Parse S/R zones if provided
    let sr_zones: Option<Vec<SRZone>> = if let Some(ref json) = sr_zones_json {
        Some(
            serde_json::from_str(json).map_err(|e| format!("Failed to parse S/R zones: {}", e))?,
        )
    } else {
        None
    };

    // Parse pivot config if provided
    let pivot_config: Option<PivotConfig> = if let Some(ref json) = pivot_config_json {
        Some(
            serde_json::from_str(json)
                .map_err(|e| format!("Failed to parse pivot config: {}", e))?,
        )
    } else {
        None
    };

    // Parse objective
    let opt_objective = match objective.as_deref() {
        Some("sharpe_ratio") => OptimizationObjective::SharpeRatio,
        Some("profit_factor") => OptimizationObjective::ProfitFactor,
        Some("total_return") => OptimizationObjective::TotalReturn,
        Some("win_rate") => OptimizationObjective::WinRate,
        Some("min_drawdown") => OptimizationObjective::MinDrawdown,
        Some("trade_count") => OptimizationObjective::TradeCount,
        _ => OptimizationObjective::SharpeRatio,
    };

    // Build optimization config
    let opt_config = OptimizationConfig {
        objective: opt_objective,
        param_ids,
        max_combinations: max_combinations.or(Some(10000)),
        min_trades: min_trades.or(Some(10)),
    };

    // Initial balance
    let balance = Decimal::try_from(initial_balance.unwrap_or(1000.0))
        .map_err(|e| format!("Invalid initial balance: {}", e))?;

    // Fetch HTF candles for multi-timeframe strategies
    let mtf_store = {
        let from_ts = candles.first().ok_or("No candles fetched")?.time.to_rfc3339();
        let to_ts = candles.last().ok_or("No candles fetched")?.time.to_rfc3339();
        let store = fetch_htf_candles(
            &strategy_json, &granularity, &instrument, &from_ts, &to_ts, &state,
        ).await?;
        if store.is_empty() { None } else { Some(store) }
    };

    // BUG-067: Run CPU-intensive optimization on a blocking thread to avoid
    // starving the tokio async runtime. Grid search uses rayon for parallelism
    // but the collecting thread still blocks.
    //
    // Safety: Tauri v2 `AppHandle` is `Send + Sync`, so `app.emit()` from
    // a spawn_blocking thread is safe. `Emitter::emit` posts to the webview
    // event loop synchronously — no async runtime required on the calling thread.
    // If a future Tauri version changes emit to async-only, this will become a
    // compile error (not a silent failure) since the closure is not async.
    let app = app_handle.clone();
    let result = tokio::task::spawn_blocking(move || {
        optimizer::run_optimization(
            &strategy_json,
            &parameters,
            &candles,
            balance,
            sr_zones.as_deref(),
            pivot_config.as_ref(),
            &opt_config,
            &instrument,
            Some(&|current, total| {
                let _ = app.emit(
                    "optimization-progress",
                    serde_json::json!({
                        "current": current,
                        "total": total,
                        "percent": (current as f64 / total as f64 * 100.0).round() as u32
                    }),
                );
            }),
            mtf_store.as_ref(),
            &granularity,
        )
    })
    .await
    .map_err(|e| format!("Optimization task failed: {}", e))??;

    Ok(result)
}

/// Guard that atomically acquires the wf_running flag and clears it on drop.
/// Baseline runs get a no-op guard that doesn't touch the flag.
#[derive(Debug)]
struct WfRunningGuard(Option<Arc<std::sync::atomic::AtomicBool>>);

impl WfRunningGuard {
    /// Acquire the running flag. Returns Err if a run is already in progress.
    /// Baseline runs skip acquisition entirely.
    fn acquire(flag: &Arc<std::sync::atomic::AtomicBool>, is_baseline: bool) -> Result<Self, &'static str> {
        use std::sync::atomic::Ordering::SeqCst;
        if !is_baseline {
            if flag.compare_exchange(false, true, SeqCst, SeqCst).is_err() {
                return Err("A walk-forward analysis is already running. Please wait for it to complete or cancel it first.");
            }
            Ok(Self(Some(flag.clone())))
        } else {
            Ok(Self(None))
        }
    }
}

impl Drop for WfRunningGuard {
    fn drop(&mut self) {
        if let Some(ref flag) = self.0 {
            flag.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// Run walk-forward analysis on a strategy
#[tauri::command]
pub async fn run_walk_forward(
    instrument: String,
    granularity: String,
    strategy_json: String,
    parameters_json: String,
    date_from: String,
    date_to: String,
    initial_balance: Option<f64>,
    sr_zones_json: Option<String>,
    pivot_config_json: Option<String>,
    train_months: u32,
    test_months: u32,
    step_months: Option<u32>,
    objective: Option<String>,
    min_trades_per_window: Option<usize>,
    anchored: Option<bool>,
    job_id: Option<String>,
    strategy_id: Option<String>,
    strategy_name: Option<String>,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<candlesight_lib::backtest::WalkForwardResult, String> {
    use candlesight_lib::backtest::{
        pivots::PivotConfig,
        rules_engine::{ParameterDefinition, SRZone},
        walk_forward::{self, WalkForwardConfig, WalkForwardProgress},
        OptimizationObjective,
    };
    use candlesight_lib::oanda::endpoints::{self, Granularity};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    // Baseline runs have no job_id — they're secondary runs triggered automatically
    // after the primary WF completes. Inferred server-side so the frontend can't
    // bypass the concurrency guard.
    let is_baseline = job_id.is_none();
    let _wf_guard = WfRunningGuard::acquire(&state.wf_running, is_baseline)
        .map_err(|e| e.to_string())?;

    // AGT-650: cloud (queries-service) job mirroring removed. Job rows are
    // persisted by the frontend in the local store (AGT-645 useBacktestJob),
    // driven by the job-heartbeat / job-completed events emitted below —
    // failures are logged here for diagnostics only.
    let fail_job = |job_id: &Option<String>, error: &str| {
        if let Some(jid) = job_id {
            tracing::warn!(job_id = %jid, error = %error, "[WalkForward] Job failed");
        }
    };

    // Validate instrument format
    if !is_valid_instrument(&instrument) {
        let err = format!("Invalid instrument format: {}", instrument);
        fail_job(&job_id, &err);
        return Err(err);
    }

    // Validate date formats
    if chrono::NaiveDate::parse_from_str(&date_from, "%Y-%m-%d").is_err() {
        let err = format!(
            "Invalid date_from format '{}' (expected YYYY-MM-DD)",
            date_from
        );
        fail_job(&job_id, &err);
        return Err(err);
    }
    if chrono::NaiveDate::parse_from_str(&date_to, "%Y-%m-%d").is_err() {
        let err = format!(
            "Invalid date_to format '{}' (expected YYYY-MM-DD)",
            date_to
        );
        fail_job(&job_id, &err);
        return Err(err);
    }

    // Parse parameters
    info!("[run_walk_forward] Raw parameters_json: {}", parameters_json);
    let parameters: Vec<ParameterDefinition> = match serde_json::from_str(&parameters_json) {
        Ok(p) => p,
        Err(e) => {
            let err = format!("Failed to parse parameters JSON: {}", e);
            fail_job(&job_id, &err);
            return Err(err);
        }
    };

    // Parse granularity
    let gran = match Granularity::from_str(&granularity) {
        Ok(g) => g,
        Err(e) => {
            let err = e.to_string();
            fail_job(&job_id, &err);
            return Err(err);
        }
    };

    // Fetch historical candles with pagination for large date ranges
    let from_rfc3339 = format!("{}T00:00:00Z", date_from);
    let to_rfc3339 = {
        let today_utc = chrono::Utc::now().format("%Y-%m-%d").to_string();
        if date_to >= today_utc {
            (chrono::Utc::now() - chrono::Duration::minutes(1))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        } else {
            format!("{}T23:59:59Z", date_to)
        }
    };

    let client = state.client.read().await;
    let candles =
        match endpoints::get_candles_paginated(&*client, &instrument, gran, &from_rfc3339, &to_rfc3339)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                let err = e.to_string();
                fail_job(&job_id, &err);
                return Err(err);
            }
        };

    if candles.is_empty() {
        let err = "No candle data available".to_string();
        fail_job(&job_id, &err);
        return Err(err);
    }

    // Parse S/R zones if provided
    let sr_zones: Option<Vec<SRZone>> = if let Some(ref json) = sr_zones_json {
        match serde_json::from_str(json) {
            Ok(z) => Some(z),
            Err(e) => {
                let err = format!("Failed to parse S/R zones: {}", e);
                fail_job(&job_id, &err);
                return Err(err);
            }
        }
    } else {
        None
    };

    // Parse pivot config if provided
    let pivot_config: Option<PivotConfig> = if let Some(ref json) = pivot_config_json {
        match serde_json::from_str(json) {
            Ok(p) => Some(p),
            Err(e) => {
                let err = format!("Failed to parse pivot config: {}", e);
                fail_job(&job_id, &err);
                return Err(err);
            }
        }
    } else {
        None
    };

    // Parse objective
    let opt_objective = match objective.as_deref() {
        Some("sharpe_ratio") => OptimizationObjective::SharpeRatio,
        Some("profit_factor") => OptimizationObjective::ProfitFactor,
        Some("total_return") => OptimizationObjective::TotalReturn,
        Some("win_rate") => OptimizationObjective::WinRate,
        Some("min_drawdown") => OptimizationObjective::MinDrawdown,
        Some("trade_count") => OptimizationObjective::TradeCount,
        _ => OptimizationObjective::SharpeRatio,
    };

    // Build walk-forward config
    let wf_config = WalkForwardConfig {
        train_months,
        test_months,
        step_months: step_months.unwrap_or(test_months),
        objective: opt_objective,
        min_trades_per_window: min_trades_per_window.or(Some(5)),
        anchored: anchored.unwrap_or(false),
    };

    // Initial balance
    let balance = match Decimal::try_from(initial_balance.unwrap_or(1000.0)) {
        Ok(b) => b,
        Err(e) => {
            let err = format!("Invalid initial balance: {}", e);
            fail_job(&job_id, &err);
            return Err(err);
        }
    };

    // Reset cancellation token before starting
    state
        .wf_cancel_token
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let cancel_token = state.wf_cancel_token.clone();

    let job_id_for_progress = job_id.clone();
    let strategy_id_for_progress = strategy_id.clone();

    // Fetch HTF candles for multi-timeframe strategies
    let mtf_store = {
        let from_ts = candles.first().ok_or("No candles fetched")?.time.to_rfc3339();
        let to_ts = candles.last().ok_or("No candles fetched")?.time.to_rfc3339();
        let store = fetch_htf_candles(
            &strategy_json, &granularity, &instrument, &from_ts, &to_ts, &state,
        ).await?;
        if store.is_empty() { None } else { Some(store) }
    };

    // BUG-067: Run CPU-intensive walk-forward computation on a blocking thread
    // to avoid starving the tokio async runtime. Walk-forward iterates over multiple
    // windows, each running full backtest + optimization with Decimal arithmetic.
    //
    // Safety: Tauri v2 `AppHandle` is `Send + Sync`, so `app.emit()` from
    // a spawn_blocking thread is safe. `tokio::spawn()` from a blocking thread
    // is also safe — it schedules work on the runtime without requiring an async context.
    let app = app_handle.clone();
    let strategy_id_for_events = strategy_id.clone();
    let cancel_token_for_blocking = cancel_token.clone();
    let result = tokio::task::spawn_blocking(move || {
        walk_forward::run_walk_forward(
            &strategy_json,
            &parameters,
            &candles,
            balance,
            sr_zones.as_deref(),
            pivot_config.as_ref(),
            &wf_config,
            &instrument,
            Some(&|progress: WalkForwardProgress| {
                let progress_json = serde_json::json!({
                    "phase": progress.phase,
                    "windowNum": progress.window_num,
                    "totalWindows": progress.total_windows,
                    "optimizationCurrent": progress.optimization_current,
                    "optimizationTotal": progress.optimization_total,
                    "percent": progress.percent,
                    "trainStart": progress.train_start,
                    "trainEnd": progress.train_end,
                    "testStart": progress.test_start,
                    "testEnd": progress.test_end,
                    "strategyId": strategy_id_for_events
                });

                // Always emit the progress event for real-time UI
                let _ = app.emit("walk-forward-progress", progress_json.clone());

                // Also emit job heartbeat if we have a job_id
                if let Some(ref jid) = job_id_for_progress {
                    let _ = app.emit(
                        "job-heartbeat",
                        serde_json::json!({
                            "jobId": jid,
                            "strategyId": strategy_id_for_progress,
                            "status": "running",
                            "progress": progress.percent,
                            "progressDetail": progress_json
                        }),
                    );
                }

            }),
            Some(&cancel_token_for_blocking),
            mtf_store.as_ref(),
            &granularity,
        )
    })
    .await
    .map_err(|e| format!("Walk-forward task failed: {}", e))?;

    // Handle result
    match result {
        Ok(wf_result) => {
            // Job completed successfully — emit the completion event (the
            // frontend persists the job row to the local store).
            if let Some(ref jid) = job_id {
                let _ = app_handle.emit(
                    "job-completed",
                    serde_json::json!({
                        "jobId": jid,
                        "strategyId": strategy_id,
                        "status": "completed",
                        "hasResult": true,
                        "result": &wf_result
                    }),
                );
            }

            // Send system notification (works even if window is closed)
            let pnl: f64 = wf_result.oos_total_pnl.parse().unwrap_or(0.0);
            candlesight_lib::notifications::send_job_completion_notification(
                app_handle.clone(),
                strategy_name.as_deref().unwrap_or("Strategy"),
                true,
                Some(pnl),
                Some(wf_result.sharpe_efficiency),
                None,
            );

            Ok(wf_result)
        }
        Err(e) => {
            // Check if it was cancelled
            if cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
                if let Some(ref jid) = job_id {
                    let _ = app_handle.emit(
                        "job-completed",
                        serde_json::json!({
                            "jobId": jid,
                            "strategyId": strategy_id,
                            "status": "cancelled",
                            "hasResult": false
                        }),
                    );
                }
                // No notification for cancellation - user initiated it
            } else {
                // Actual error
                fail_job(&job_id, &e);
                if let Some(ref jid) = job_id {
                    let _ = app_handle.emit(
                        "job-completed",
                        serde_json::json!({
                            "jobId": jid,
                            "strategyId": strategy_id,
                            "status": "failed",
                            "hasResult": false,
                            "error": e
                        }),
                    );
                }

                // Send failure notification
                candlesight_lib::notifications::send_job_completion_notification(
                    app_handle.clone(),
                    strategy_name.as_deref().unwrap_or("Strategy"),
                    false,
                    None,
                    None,
                    Some(&e),
                );
            }
            Err(e)
        }
    }
}

/// Cancel a running walk-forward analysis
#[tauri::command]
pub async fn cancel_walk_forward(state: State<'_, AppState>) -> Result<(), String> {
    state
        .wf_cancel_token
        .store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

/// Check if a walk-forward analysis is currently running (BUG-028)
///
/// Used by the frontend to check backend state after page reloads,
/// preventing duplicate job submissions when the frontend loses its
/// local running state.
#[tauri::command]
pub async fn is_walk_forward_running(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.wf_running.load(std::sync::atomic::Ordering::SeqCst))
}

/// Result for a single sweep value
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepValueResult {
    pub value: f64,
    pub oos_total_pnl: String,
    pub oos_total_return_pct: String,
    pub oos_avg_sharpe: f64,
    pub oos_total_trades: u32,
    pub oos_max_drawdown_pct: String,
    pub oos_win_rate: String,
}

/// Result for a complete parameter sweep
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterSweepResult {
    pub param_id: String,
    pub param_name: String,
    pub default_value: f64,
    pub results: Vec<SweepValueResult>,
}

/// Run a fixed-value parameter sweep: for each value, run a walk-forward with that param fixed.
#[tauri::command]
pub async fn run_parameter_sweep(
    instrument: String,
    granularity: String,
    strategy_json: String,
    parameters_json: String,
    date_from: String,
    date_to: String,
    initial_balance: Option<f64>,
    sr_zones_json: Option<String>,
    pivot_config_json: Option<String>,
    train_months: u32,
    test_months: u32,
    step_months: Option<u32>,
    objective: Option<String>,
    min_trades_per_window: Option<usize>,
    anchored: Option<bool>,
    sweep_param_id: String,
    sweep_values: Vec<f64>,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<ParameterSweepResult, String> {
    use candlesight_lib::backtest::{
        pivots::PivotConfig,
        rules_engine::{ParameterDefinition, SRZone},
        walk_forward::{self, WalkForwardConfig},
        OptimizationObjective,
    };
    use candlesight_lib::oanda::endpoints::{self, Granularity};
    use rust_decimal::Decimal;
    use std::str::FromStr;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    // BUG-028: Atomically check-and-set running flag to prevent duplicate jobs.
    if state
        .wf_running
        .compare_exchange(false, true, std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst)
        .is_err()
    {
        return Err("A walk-forward analysis is already running. Please wait for it to complete or cancel it first.".to_string());
    }

    // Guard to ensure wf_running is cleared on ALL exit paths
    struct WfRunningGuard(Arc<AtomicBool>);
    impl Drop for WfRunningGuard {
        fn drop(&mut self) {
            self.0.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let _wf_guard = WfRunningGuard(state.wf_running.clone());

    // Validate instrument format
    if !is_valid_instrument(&instrument) {
        return Err(format!("Invalid instrument format: {}", instrument));
    }

    // Validate date formats
    if chrono::NaiveDate::parse_from_str(&date_from, "%Y-%m-%d").is_err() {
        return Err(format!("Invalid date_from format '{}' (expected YYYY-MM-DD)", date_from));
    }
    if chrono::NaiveDate::parse_from_str(&date_to, "%Y-%m-%d").is_err() {
        return Err(format!("Invalid date_to format '{}' (expected YYYY-MM-DD)", date_to));
    }

    // Parse parameters
    let all_parameters: Vec<ParameterDefinition> = serde_json::from_str(&parameters_json)
        .map_err(|e| format!("Failed to parse parameters JSON: {}", e))?;

    // Find the sweep parameter
    let sweep_param = all_parameters.iter().find(|p| p.id == sweep_param_id)
        .ok_or_else(|| format!("Sweep parameter '{}' not found", sweep_param_id))?;
    let param_name = sweep_param.name.clone();
    let default_value = sweep_param.default;

    // Parse granularity
    let gran = Granularity::from_str(&granularity)
        .map_err(|e| e.to_string())?;

    // Fetch candles once (shared across all sweep values)
    let from_rfc3339 = format!("{}T00:00:00Z", date_from);
    let to_rfc3339 = {
        let today_utc = chrono::Utc::now().format("%Y-%m-%d").to_string();
        if date_to >= today_utc {
            (chrono::Utc::now() - chrono::Duration::minutes(1))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        } else {
            format!("{}T23:59:59Z", date_to)
        }
    };

    let client = state.client.read().await;
    let candles = endpoints::get_candles_paginated(&*client, &instrument, gran, &from_rfc3339, &to_rfc3339)
        .await
        .map_err(|e| e.to_string())?;

    if candles.is_empty() {
        return Err("No candle data available".to_string());
    }

    // Parse S/R zones if provided
    let sr_zones: Option<Vec<SRZone>> = if let Some(ref json) = sr_zones_json {
        Some(serde_json::from_str(json).map_err(|e| format!("Failed to parse S/R zones: {}", e))?)
    } else {
        None
    };

    // Parse pivot config if provided
    let pivot_config: Option<PivotConfig> = if let Some(ref json) = pivot_config_json {
        Some(serde_json::from_str(json).map_err(|e| format!("Failed to parse pivot config: {}", e))?)
    } else {
        None
    };

    // Parse objective
    let opt_objective = match objective.as_deref() {
        Some("sharpe_ratio") => OptimizationObjective::SharpeRatio,
        Some("profit_factor") => OptimizationObjective::ProfitFactor,
        Some("total_return") => OptimizationObjective::TotalReturn,
        Some("win_rate") => OptimizationObjective::WinRate,
        Some("min_drawdown") => OptimizationObjective::MinDrawdown,
        Some("trade_count") => OptimizationObjective::TradeCount,
        _ => OptimizationObjective::SharpeRatio,
    };

    let wf_config = WalkForwardConfig {
        train_months,
        test_months,
        step_months: step_months.unwrap_or(test_months),
        objective: opt_objective,
        min_trades_per_window: min_trades_per_window.or(Some(5)),
        anchored: anchored.unwrap_or(false),
    };

    let balance = Decimal::try_from(initial_balance.unwrap_or(1000.0))
        .map_err(|e| format!("Invalid initial balance: {}", e))?;

    // Reset cancellation token
    state.wf_cancel_token.store(false, std::sync::atomic::Ordering::SeqCst);
    let cancel_token = state.wf_cancel_token.clone();

    // Fetch HTF candles for multi-timeframe strategies
    let mtf_store = {
        let from_ts = candles.first().ok_or("No candles fetched")?.time.to_rfc3339();
        let to_ts = candles.last().ok_or("No candles fetched")?.time.to_rfc3339();
        let store = fetch_htf_candles(
            &strategy_json, &granularity, &instrument, &from_ts, &to_ts, &state,
        ).await?;
        if store.is_empty() { None } else { Some(store) }
    };

    // BUG-067: Run CPU-intensive parameter sweep on a blocking thread to avoid
    // starving the tokio async runtime. Each sweep value runs a full walk-forward
    // analysis with Decimal arithmetic across potentially thousands of candles.
    //
    // Safety: Tauri v2 `AppHandle` is `Send + Sync`, so `app_handle.emit()` from
    // a spawn_blocking thread is safe (see optimize_strategy for precedent).
    let app = app_handle.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut results = Vec::with_capacity(sweep_values.len());

        for (idx, &sweep_value) in sweep_values.iter().enumerate() {
            // Check cancellation
            if cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("cancelled".to_string());
            }

            // Build parameters: sweep param fixed at this value, all others at defaults
            let fixed_params: Vec<ParameterDefinition> = all_parameters.iter().map(|p| {
                if p.id == sweep_param_id {
                    let mut fixed = p.clone();
                    fixed.min = Some(sweep_value);
                    fixed.max = Some(sweep_value);
                    fixed.step = Some(1.0);
                    fixed
                } else {
                    let mut fixed = p.clone();
                    fixed.min = Some(p.default);
                    fixed.max = Some(p.default);
                    fixed.step = Some(1.0);
                    fixed
                }
            }).collect();

            // Emit progress
            let _ = app.emit("parameter-sweep-progress", serde_json::json!({
                "currentIndex": idx,
                "totalValues": sweep_values.len(),
                "currentValue": sweep_value,
                "paramId": sweep_param_id,
            }));

            // Run walk-forward for this fixed value (no progress callback, no job tracking)
            let wf_result = walk_forward::run_walk_forward(
                &strategy_json,
                &fixed_params,
                &candles,
                balance,
                sr_zones.as_deref(),
                pivot_config.as_ref(),
                &wf_config,
                &instrument,
                None, // No progress callback for individual sweep runs
                Some(&cancel_token),
                mtf_store.as_ref(),
                &granularity,
            ).map_err(|e| e.to_string())?;

            results.push(SweepValueResult {
                value: sweep_value,
                oos_total_pnl: wf_result.oos_total_pnl,
                oos_total_return_pct: wf_result.oos_total_return_pct,
                oos_avg_sharpe: wf_result.oos_avg_sharpe,
                oos_total_trades: wf_result.oos_total_trades,
                oos_max_drawdown_pct: wf_result.oos_max_drawdown_pct,
                oos_win_rate: wf_result.oos_win_rate,
            });
        }

        Ok(ParameterSweepResult {
            param_id: sweep_param_id,
            param_name,
            default_value,
            results,
        })
    })
    .await
    .map_err(|e| format!("Parameter sweep task failed: {}", e))?;

    result
}

/// Validate strategy JSON against StrategyDefinition schema
///
/// Called when users paste JSON in the Strategy Builder to catch errors early.
/// Returns Ok(()) if valid, or an error message explaining what's wrong.
#[tauri::command]
pub fn validate_strategy_json(strategy_json: String) -> Result<(), String> {
    // First, check if it's valid JSON at all
    let mut value: serde_json::Value = serde_json::from_str(&strategy_json)
        .map_err(|e| format!("Invalid JSON syntax: {}", e))?;

    // Check schema_version
    let schema_version = value.get("schema_version")
        .and_then(|v| v.as_i64())
        .ok_or("Missing or invalid schema_version. Add \"schema_version\": 2 to your JSON.")?;

    if schema_version != 2 {
        return Err(format!("Unsupported schema_version: {}. This editor requires schema_version: 2", schema_version));
    }

    // Add placeholder fields if missing (these are database/metadata fields, not required for import)
    // BUG-042: JSON export doesn't include id/user_id/version/is_active, so round-trip import fails without this fix
    if let Some(obj) = value.as_object_mut() {
        if !obj.contains_key("id") {
            obj.insert("id".to_string(), serde_json::Value::String("import-placeholder".to_string()));
        }
        if !obj.contains_key("user_id") {
            obj.insert("user_id".to_string(), serde_json::Value::String("import-placeholder".to_string()));
        }
        if !obj.contains_key("version") {
            obj.insert("version".to_string(), serde_json::Value::Number(1.into()));
        }
        if !obj.contains_key("is_active") {
            obj.insert("is_active".to_string(), serde_json::Value::Bool(true));
        }
        if !obj.contains_key("description") {
            obj.insert("description".to_string(), serde_json::Value::String("".to_string()));
        }
    }

    // Parse as StrategyDefinition to validate all fields
    let strategy: StrategyDefinition = serde_json::from_value(value)
        .map_err(|e| format!("Invalid strategy: {}", e))?;

    // Validate is_within triggers have distance field
    validate_is_within_triggers(&strategy)?;

    Ok(())
}

/// Validate that all is_within triggers have proper distance configuration
fn validate_is_within_triggers(strategy: &StrategyDefinition) -> Result<(), String> {
    use shared::{Trigger, ComparisonOperator};

    // Helper to check a single trigger
    fn check_trigger(trigger: &Trigger, rule_id: &str, rule_type: &str) -> Result<(), String> {
        if let Trigger::Compare(compare) = trigger {
            if compare.operator == ComparisonOperator::IsWithin && compare.distance.is_none() {
                return Err(format!(
                    "{} rule '{}' uses 'is_within' operator but is missing required 'distance' field. \
                    Use: \"distance\": {{ \"value\": 15, \"unit\": \"pips\" }}",
                    rule_type, rule_id
                ));
            }
        }
        Ok(())
    }

    // Check entry rules
    for rule in &strategy.entry_rules {
        for condition in &rule.conditions {
            check_trigger(&condition.primary.trigger, &rule.id, "Entry")?;
            for chained in &condition.chain {
                check_trigger(&chained.trigger.trigger, &rule.id, "Entry")?;
            }
        }
    }

    // Check exit rules
    for rule in &strategy.exit_rules {
        for condition in &rule.conditions {
            check_trigger(&condition.primary.trigger, &rule.id, "Exit")?;
            for chained in &condition.chain {
                check_trigger(&chained.trigger.trigger, &rule.id, "Exit")?;
            }
        }
    }

    Ok(())
}

// =============================================================================
// Strategy Conversion
// =============================================================================

/// Convert a trading strategy script (Pine Script, MQL4/5, or natural language)
/// to wickd's V2 strategy JSON format using AI.
///
/// Flow:
/// 1. Validate source_language
/// 2. Sanitize the script (strip comments, neutralize strings, enforce limits)
/// 3. Send to AI via the direct Anthropic client (AGT-650: queries-service
///    proxy removed; requires runtime `ANTHROPIC_API_KEY`)
/// 4. Validate the returned JSON against StrategyDefinition
/// 5. Return the valid strategy JSON
#[tauri::command]
pub async fn convert_strategy_script(
    script: String,
    source_language: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    use candlesight_lib::strategy_convert;

    // Validate source language
    strategy_convert::validate_source_language(&source_language)
        .map_err(|e| e.to_string())?;

    // Sanitize the script (SECURITY: must happen before any AI processing)
    let sanitized = strategy_convert::sanitize_script(&script)
        .map_err(|e| e.to_string())?;

    info!(
        source_language = %source_language,
        sanitized_len = sanitized.len(),
        "[ConvertStrategy] Script sanitized, sending to AI"
    );

    let claude = state.claude.as_ref().ok_or_else(|| {
        "Strategy conversion unavailable: ANTHROPIC_API_KEY is not configured".to_string()
    })?;

    // Call AI for conversion
    let strategy_json = strategy_convert::convert_script_to_strategy(
        &sanitized,
        &source_language,
        claude,
    )
    .await
    .map_err(|e| e.to_string())?;

    // Validate the AI output against our strategy schema
    // This ensures the returned JSON is a valid StrategyDefinition
    validate_strategy_json(strategy_json.clone())?;

    info!("[ConvertStrategy] Conversion successful, strategy validated");

    Ok(strategy_json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn wf_guard_acquires_and_releases_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        {
            let _guard = WfRunningGuard::acquire(&flag, false).unwrap();
            assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
        }
        // Flag cleared after guard dropped
        assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn wf_guard_rejects_concurrent_primary_runs() {
        let flag = Arc::new(AtomicBool::new(false));
        let _guard1 = WfRunningGuard::acquire(&flag, false).unwrap();

        let result = WfRunningGuard::acquire(&flag, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already running"));
    }

    #[test]
    fn wf_guard_baseline_skips_flag() {
        let flag = Arc::new(AtomicBool::new(false));

        // Primary run acquires the flag
        let _guard1 = WfRunningGuard::acquire(&flag, false).unwrap();
        assert!(flag.load(std::sync::atomic::Ordering::SeqCst));

        // Baseline run succeeds even though flag is set
        let guard2 = WfRunningGuard::acquire(&flag, true).unwrap();
        assert!(guard2.0.is_none()); // Baseline guard holds no flag reference

        // Dropping baseline guard does NOT clear the primary's flag
        drop(guard2);
        assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn wf_guard_flag_cleared_on_panic_path() {
        let flag = Arc::new(AtomicBool::new(false));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = WfRunningGuard::acquire(&flag, false).unwrap();
            panic!("simulated error");
        }));
        assert!(result.is_err());
        // Guard's Drop still ran during unwind
        assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn wf_guard_primary_runs_sequentially_after_release() {
        let flag = Arc::new(AtomicBool::new(false));

        // First run
        {
            let _guard = WfRunningGuard::acquire(&flag, false).unwrap();
        }
        // Second run succeeds after first is dropped
        {
            let _guard = WfRunningGuard::acquire(&flag, false).unwrap();
            assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
        }
        assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
    }
}
