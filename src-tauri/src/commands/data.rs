//! Data commands for candles, sync, and indicators.
//!
//! Handles OANDA candle fetching, trade sync, and indicator calculations.

use std::collections::HashMap;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tracing::info;

use crate::AppState;
use crate::commands::trading::is_valid_instrument;
use candlesight_lib::{models, oanda::endpoints};

// =============================================================================
// Types
// =============================================================================

/// Candle data for backtesting
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandleData {
    pub time: String,
    pub open: String,
    pub high: String,
    pub low: String,
    pub close: String,
    pub volume: i32,
    pub complete: bool,
}

impl From<models::Candle> for CandleData {
    fn from(c: models::Candle) -> Self {
        Self {
            time: c.time.to_rfc3339(),
            open: c.mid.open.to_string(),
            high: c.mid.high.to_string(),
            low: c.mid.low.to_string(),
            close: c.mid.close.to_string(),
            volume: c.volume,
            complete: c.complete,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AutochartistZone {
    pub pattern: String,
    pub probability: f64,
    pub support_upper: f64,
    pub support_lower: f64,
    pub resistance_upper: f64,
    pub resistance_lower: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PivotLevel {
    pub label: String,
    pub price: f64,
    pub level_type: String, // "pivot", "support", "resistance"
}

/// Sync response from trade sync operation
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub synced_count: usize,
    pub open_trades: usize,
    pub closed_trades: usize,
    pub deleted_count: usize,
}

/// Progress update during trade sync
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgress {
    pub stage: String,       // "fetching", "processing", "complete", "error"
    pub message: String,
    pub current: usize,
    pub total: Option<usize>,
}

/// Response when starting a sync (non-blocking)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStarted {
    pub message: String,
}

/// Indicator data point for charting
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorDataPoint {
    pub time: String,
    pub values: HashMap<String, String>, // output_name -> value
}

/// Full indicator series for a chart
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorSeries {
    pub id: String,
    #[serde(rename = "type")]
    pub indicator_type: String,
    pub outputs: Vec<String>, // Which outputs this indicator produces
    pub data: Vec<IndicatorDataPoint>,
}

// =============================================================================
// Commands
// =============================================================================

/// Fetch Autochartist S/R signals for an instrument
///
/// # Arguments
/// * `instrument` - Currency pair (e.g., "EUR_USD")
#[tauri::command]
pub async fn fetch_autochartist_signals(
    instrument: String,
    state: State<'_, AppState>,
) -> Result<Vec<AutochartistZone>, String> {
    let client = state.client.read().await;

    // Debug: log current client state (account masked for security)
    let account_id = client.account_id();
    let masked_account = if account_id.len() > 4 {
        format!("***{}", &account_id[account_id.len()-4..])
    } else {
        "****".to_string()
    };
    info!(
        "[fetch_autochartist_signals] Client state: env={:?}, base_url={}, account={}",
        client.environment(),
        client.base_url(),
        masked_account
    );

    let response = endpoints::get_autochartist_signals(&*client, &instrument)
        .await
        .map_err(|e| e.to_string())?;

    // Convert signals to zone-friendly format
    let zones: Vec<AutochartistZone> = response.signals.into_iter().map(|s| {
        AutochartistZone {
            pattern: s.meta.pattern,
            probability: s.meta.probability,
            support_upper: s.data.points.support.y0.max(s.data.points.support.y1),
            support_lower: s.data.points.support.y0.min(s.data.points.support.y1),
            resistance_upper: s.data.points.resistance.y0.max(s.data.points.resistance.y1),
            resistance_lower: s.data.points.resistance.y0.min(s.data.points.resistance.y1),
        }
    }).collect();

    Ok(zones)
}

/// Instrument info returned to the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentInfo {
    pub name: String,
    pub display_name: String,
    pub instrument_type: String,
}

/// Fetch all tradeable instruments for the current account
///
/// Returns the full list of instruments available for trading on OANDA.
#[tauri::command]
pub async fn fetch_instruments(
    state: State<'_, AppState>,
) -> Result<Vec<InstrumentInfo>, String> {
    let client = state.client.read().await;

    let instruments = endpoints::get_instruments(&*client)
        .await
        .map_err(|e| e.to_string())?;

    // Convert to frontend-friendly format and sort by name
    let mut result: Vec<InstrumentInfo> = instruments
        .into_iter()
        .map(|i| InstrumentInfo {
            name: i.name,
            display_name: i.display_name,
            instrument_type: i.instrument_type,
        })
        .collect();

    result.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(result)
}

/// Calculate pivot points from previous day's candle
///
/// Returns Classic pivot points: P, R1, R2, R3, S1, S2, S3
/// These can be imported as S/R zones on the chart
#[tauri::command]
pub async fn calculate_pivot_points(
    instrument: String,
    timeframe: Option<String>, // "daily" or "weekly", defaults to daily
    state: State<'_, AppState>,
) -> Result<Vec<PivotLevel>, String> {
    use candlesight_lib::oanda::endpoints::Granularity;
    use rust_decimal::prelude::ToPrimitive;

    let client = state.client.read().await;

    // Determine granularity based on timeframe
    let granularity = match timeframe.as_deref() {
        Some("weekly") => Granularity::W,
        _ => Granularity::D, // Default to daily
    };

    let timeframe_label = match granularity {
        Granularity::W => "Weekly",
        _ => "Daily",
    };

    info!(
        "[calculate_pivot_points] Fetching {} candles for {} pivots",
        instrument, timeframe_label
    );

    // Fetch last 2 candles to get the previous complete one
    let candles = endpoints::get_candles(&*client, &instrument, granularity, Some(2), None, None)
        .await
        .map_err(|e| format!("Failed to fetch candles: {}", e))?;

    if candles.is_empty() {
        return Err("No candle data available".to_string());
    }

    // Use the second-to-last candle (previous complete period)
    // If only 1 candle, use it (current incomplete)
    let candle = if candles.len() >= 2 {
        &candles[candles.len() - 2]
    } else {
        &candles[0]
    };

    let high = candle.mid.high.to_f64().ok_or("Invalid high price")?;
    let low = candle.mid.low.to_f64().ok_or("Invalid low price")?;
    let close = candle.mid.close.to_f64().ok_or("Invalid close price")?;

    // Classic Pivot Point formulas
    let pivot = (high + low + close) / 3.0;
    let r1 = (2.0 * pivot) - low;
    let s1 = (2.0 * pivot) - high;
    let r2 = pivot + (high - low);
    let s2 = pivot - (high - low);
    let r3 = high + 2.0 * (pivot - low);
    let s3 = low - 2.0 * (high - pivot);

    let levels = vec![
        PivotLevel { label: format!("{} R3", timeframe_label), price: r3, level_type: "resistance".to_string() },
        PivotLevel { label: format!("{} R2", timeframe_label), price: r2, level_type: "resistance".to_string() },
        PivotLevel { label: format!("{} R1", timeframe_label), price: r1, level_type: "resistance".to_string() },
        PivotLevel { label: format!("{} Pivot", timeframe_label), price: pivot, level_type: "pivot".to_string() },
        PivotLevel { label: format!("{} S1", timeframe_label), price: s1, level_type: "support".to_string() },
        PivotLevel { label: format!("{} S2", timeframe_label), price: s2, level_type: "support".to_string() },
        PivotLevel { label: format!("{} S3", timeframe_label), price: s3, level_type: "support".to_string() },
    ];

    info!(
        "[calculate_pivot_points] Calculated {} pivot levels for {}",
        levels.len(),
        instrument
    );

    Ok(levels)
}

/// Sync trades from OANDA to the local store (`~/.wickd/app.db`, AGT-647)
///
/// This is a NON-BLOCKING command - it spawns the sync in a background task
/// and returns immediately. Progress and completion are communicated via events:
/// - `sync-progress`: { stage, message, current, total }
/// - `sync-complete`: { syncedCount, openTrades, closedTrades }
///
/// The sync writes straight to the local SQLite store — no queries-service,
/// no auth token, no cloud round-trip. Trade rows are keyed by raw OANDA
/// trade id, so re-syncing upserts in place (an open trade that has since
/// closed flips to CLOSED).
///
/// # Arguments
/// * `count` - Maximum number of closed trades to fetch (default: 100)
/// * `data_source` - Data source: "demo" or "live" (default: "demo")
#[tauri::command]
pub async fn sync_trades(
    count: Option<u32>,
    data_source: Option<String>,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<SyncStarted, String> {
    use candlesight_lib::local_store::{LocalStore, LocalTrade};

    // Clone what we need for the background task
    let oanda_client = state.client.clone();
    let source = data_source.unwrap_or_else(|| "demo".to_string());
    let source_for_task = source.clone();
    let trade_count = count.unwrap_or(100);
    let app = app_handle.clone();

    // Emit initial progress
    let _ = app_handle.emit("sync-progress", SyncProgress {
        stage: "starting".to_string(),
        message: format!("Starting sync from {}...", source),
        current: 0,
        total: None,
    });

    // Spawn the actual sync work in a background task
    tokio::spawn(async move {
        let emit_progress = |stage: &str, message: &str, current: usize, total: Option<usize>| {
            let _ = app.emit("sync-progress", SyncProgress {
                stage: stage.to_string(),
                message: message.to_string(),
                current,
                total,
            });
        };

        let emit_error = |message: &str| {
            let _ = app.emit("sync-progress", SyncProgress {
                stage: "error".to_string(),
                message: message.to_string(),
                current: 0,
                total: None,
            });
        };

        emit_progress("fetching", &format!("Fetching trades from {}...", source_for_task), 0, None);

        // Fetch trades from OANDA
        let client_guard = oanda_client.read().await;

        // M65c: Get account_id from OANDA client for trade association
        let account_id = Some(client_guard.account_id().to_string());

        emit_progress("fetching", "Fetching open trades...", 0, None);
        let open_trades = match endpoints::get_trades(&*client_guard, None, None, Some("OPEN")).await {
            Ok(trades) => trades,
            Err(e) => {
                emit_error(&format!("Failed to fetch open trades: {}", e));
                return;
            }
        };

        emit_progress("fetching", &format!("Fetched {} open trades, fetching history...", open_trades.len()), open_trades.len(), None);
        let closed_trades = match endpoints::get_trade_history(&*client_guard, Some(trade_count), None).await {
            Ok(trades) => trades,
            Err(e) => {
                emit_error(&format!("Failed to fetch trade history: {}", e));
                return;
            }
        };

        // Drop the guard before the async call
        drop(client_guard);

        let open_count = open_trades.len();
        let closed_count = closed_trades.len();

        let mut all_trades = open_trades;
        all_trades.extend(closed_trades);

        let total_trades = all_trades.len();
        emit_progress("processing", &format!("Saving {} trades to the local store...", total_trades), 0, Some(total_trades));

        // Convert trades to local-store rows (mirrors the old TradePayload
        // mapping; id is the raw OANDA trade id, single-user so no user_id).
        // M65c: Include account_id for proper trade association.
        let now_ms = chrono::Utc::now().timestamp_millis();
        let trade_rows: Vec<LocalTrade> = all_trades
            .iter()
            .map(|t| {
                let state_str = match t.state {
                    candlesight_lib::models::TradeState::Open => "OPEN",
                    candlesight_lib::models::TradeState::Closed => "CLOSED",
                    candlesight_lib::models::TradeState::CloseWhenTradeable => "CLOSE_WHEN_TRADEABLE",
                };
                LocalTrade {
                    id: t.id.clone(),
                    account_id: account_id.clone(),
                    instrument: t.instrument.clone(),
                    units: t.units.to_string(),
                    open_price: t.open_price.to_string(),
                    close_price: t.close_price.map(|p| p.to_string()),
                    open_time: t.open_time.timestamp_millis(),
                    close_time: t.close_time.map(|t| t.timestamp_millis()),
                    realized_pl: Some(t.realized_pl.to_string()),
                    state: state_str.to_string(),
                    synced_at: now_ms,
                    created_at: now_ms,
                    updated_at: now_ms,
                }
            })
            .collect();

        // Upsert into the local store. A fresh handle is opened inside the
        // task (SQLite/WAL handles concurrent connections; the lazily-opened
        // command-state handle can't be moved into a 'static task).
        let synced_count = match LocalStore::open_default()
            .and_then(|mut store| store.upsert_trades(&trade_rows).map(|_| trade_rows.len()))
        {
            Ok(n) => n,
            Err(e) => {
                emit_error(&format!("Failed to save trades to the local store: {}", e));
                return;
            }
        };

        emit_progress("complete", &format!("Synced {} trades ({} open, {} closed)", synced_count, open_count, closed_count), synced_count, Some(total_trades));

        // Emit completion event with full details
        // Note: deleted_count is 0 — the local store only upserts; stale rows
        // beyond the fetch horizon are left in place (single-user history).
        let _ = app.emit("sync-complete", SyncResult {
            synced_count,
            open_trades: open_count,
            closed_trades: closed_count,
            deleted_count: 0,
        });

        info!(
            "Synced {} trades ({} open, {} closed) [source: {}]",
            synced_count, open_count, closed_count, source_for_task
        );
    });

    Ok(SyncStarted {
        message: format!("Sync started for {}", source),
    })
}

/// Check if trade sync is available
///
/// Always true since AGT-647: trades sync straight to the local store, so no
/// queries-service configuration is required.
#[tauri::command]
pub async fn is_sync_enabled() -> Result<bool, String> {
    Ok(true)
}

/// Fetch historical candles for backtesting
///
/// # Arguments
/// * `instrument` - The currency pair (e.g., "EUR_USD")
/// * `granularity` - Time period: S5, M1, M5, M15, M30, H1, H4, D, W, M
/// * `count` - Number of candles to fetch (max 5000)
/// * `from` - Start time in RFC3339 format (optional)
/// * `to` - End time in RFC3339 format (optional)
#[tauri::command]
pub async fn get_candles(
    instrument: String,
    granularity: String,
    count: Option<u32>,
    from: Option<String>,
    to: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<CandleData>, String> {
    use candlesight_lib::oanda::endpoints::Granularity;
    use std::str::FromStr;

    // Validate instrument format
    if !is_valid_instrument(&instrument) {
        return Err(format!("Invalid instrument format: {}", instrument));
    }

    // Validate timestamp formats if provided
    if let Some(ref from_str) = from {
        chrono::DateTime::parse_from_rfc3339(from_str)
            .map_err(|_| "Invalid 'from' timestamp format (RFC3339 required)")?;
    }
    if let Some(ref to_str) = to {
        chrono::DateTime::parse_from_rfc3339(to_str)
            .map_err(|_| "Invalid 'to' timestamp format (RFC3339 required)")?;
    }

    let gran = Granularity::from_str(&granularity).map_err(|e| e.to_string())?;

    let client = state.client.read().await;
    let candles = endpoints::get_candles(
        &*client,
        &instrument,
        gran,
        count,
        from.as_deref(),
        to.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(candles.into_iter().map(CandleData::from).collect())
}

/// Calculate indicator values for candles
///
/// Takes candle data and indicator configurations, returns calculated values
/// for each indicator at each candle timestamp.
///
/// Supports historical date ranges via `from` and `to` parameters (ISO 8601 format).
/// If not provided, fetches the most recent candles.
#[tauri::command]
pub async fn get_indicator_data(
    instrument: String,
    granularity: String,
    count: Option<u32>,
    from: Option<String>,  // ISO 8601 datetime for historical data
    to: Option<String>,    // ISO 8601 datetime for historical data
    indicators_json: String, // JSON array of IndicatorConfig
    state: State<'_, AppState>,
) -> Result<Vec<IndicatorSeries>, String> {
    use candlesight_lib::backtest::indicator_engine::{IndicatorConfig, IndicatorEngine};
    use candlesight_lib::oanda::endpoints::Granularity;
    use std::str::FromStr;

    // Parse indicator configs
    let indicator_configs: Vec<IndicatorConfig> =
        serde_json::from_str(&indicators_json).map_err(|e| format!("Invalid indicator config: {}", e))?;

    if indicator_configs.is_empty() {
        return Ok(vec![]);
    }

    // Fetch candles (with optional historical date range)
    let gran = Granularity::from_str(&granularity).map_err(|e| e.to_string())?;
    let client = state.client.read().await;
    let candles = endpoints::get_candles(&*client, &instrument, gran, count, from.as_deref(), to.as_deref())
        .await
        .map_err(|e| e.to_string())?;

    if candles.is_empty() {
        return Ok(vec![]);
    }

    // Create indicator engine
    let mut engine = IndicatorEngine::from_config(&indicator_configs, candles.len())
        .map_err(|e| format!("Failed to create indicators: {}", e))?;

    // Define outputs per indicator type
    // NOTE: Keep in sync with INDICATOR_OUTPUTS in src/types/strategy.ts
    let indicator_outputs: HashMap<&str, Vec<&str>> = [
        ("sma", vec!["value"]),
        ("ema", vec!["value"]),
        ("rsi", vec!["value"]),
        ("atr", vec!["value"]),
        ("adx", vec!["value", "plus_di", "minus_di"]),
        ("bollinger", vec!["upper", "middle", "lower"]),
        ("macd", vec!["macd", "signal", "histogram"]),
        ("stochastic", vec!["k", "d"]),
        ("ma_histogram", vec!["histogram", "fast_ma", "slow_ma"]),
        ("ma_bands", vec!["upper", "middle", "lower"]),
        ("dss", vec!["dss", "signal"]),
        ("adr", vec!["value", "ratio"]),
        ("daily", vec!["high", "low", "range", "open"]),
        ("swing", vec!["recent_high", "recent_high_bars", "recent_low", "recent_low_bars", "prev_high", "prev_high_bars", "prev_low", "prev_low_bars"]),
        ("ichimoku", vec!["tenkan", "kijun", "senkou_a", "senkou_b", "chikou"]),
        ("chandelier", vec!["exit_long", "exit_short"]),
        ("mfi", vec!["value"]),
        ("donchian", vec!["upper", "middle", "lower"]),
    ]
    .into_iter()
    .collect();

    // Initialize series for each indicator
    let mut series_map: HashMap<String, IndicatorSeries> = indicator_configs
        .iter()
        .map(|cfg| {
            let outputs = indicator_outputs
                .get(cfg.indicator_type.as_str())
                .map(|o| o.iter().map(|s| s.to_string()).collect())
                .unwrap_or_else(|| vec!["value".to_string()]);
            (
                cfg.id.clone(),
                IndicatorSeries {
                    id: cfg.id.clone(),
                    indicator_type: cfg.indicator_type.to_string(),
                    outputs,
                    data: Vec::with_capacity(candles.len()),
                },
            )
        })
        .collect();

    // Process each candle
    for candle in &candles {
        let outputs = engine.on_candle(candle);

        for (indicator_id, indicator_outputs) in &outputs {
            if let Some(series) = series_map.get_mut(indicator_id) {
                let values: HashMap<String, String> = indicator_outputs
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_string()))
                    .collect();

                series.data.push(IndicatorDataPoint {
                    time: candle.time.to_rfc3339(),
                    values,
                });
            }
        }
    }

    Ok(series_map.into_values().collect())
}
