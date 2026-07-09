//! Tauri commands over the wickd local app store (`~/.wickd/app.db`).
//!
//! Thin invoke-wrappers around `candlesight_lib::local_store::LocalStore`
//! (all SQL lives there, schema in `local_store::migrations`). The frontend
//! counterpart is `src/lib/localStore.ts`.
//!
//! The store opens **lazily on first command** and the handle is cached in
//! managed state. A failure to open (unwritable home dir, corrupt file, ...)
//! is returned as a command error string — it must never crash app boot:
//! the local window renders the error instead (AGT-642 AC2).

use std::sync::Mutex;

use tauri::State;
use tracing::info;

use candlesight_lib::local_store::{
    default_db_path, LocalBacktest, LocalBacktestJob, LocalCredential, LocalLabel, LocalNote,
    LocalPromotionAudit, LocalSRZone, LocalStore, LocalStrategy, LocalStrategyLabel,
    LocalStrategyTrade, LocalStrategyWatcher, LocalTrade, LocalTradeLabel, LocalTradeScore,
};

/// Managed state holding the lazily-opened local store.
#[derive(Default)]
pub struct LocalStoreState(Mutex<Option<LocalStore>>);

/// Run `f` against the (lazily opened) store.
fn with_store<T>(
    state: &LocalStoreState,
    f: impl FnOnce(&LocalStore) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = state.0.lock().map_err(|_| "local store lock poisoned".to_string())?;
    if guard.is_none() {
        let store = LocalStore::open_default()?;
        info!(
            "[LocalStore] opened {}",
            default_db_path().unwrap_or_default().display()
        );
        *guard = Some(store);
    }
    let store = guard.as_ref().ok_or_else(|| "store not initialized".to_string())?;
    f(store)
}

/// Absolute path of the local store (for display/diagnostics in the UI).
#[tauri::command]
pub fn local_store_path() -> Result<String, String> {
    Ok(default_db_path()?.display().to_string())
}

/// List all strategies in the local store, most recently updated first.
#[tauri::command]
pub fn local_list_strategies(
    state: State<'_, LocalStoreState>,
) -> Result<Vec<LocalStrategy>, String> {
    with_store(&state, |s| s.list_strategies())
}

/// Fetch one strategy by id (`None` if absent).
#[tauri::command]
pub fn local_get_strategy(
    state: State<'_, LocalStoreState>,
    id: String,
) -> Result<Option<LocalStrategy>, String> {
    with_store(&state, |s| s.get_strategy(&id))
}

/// Insert or update (upsert on id) a strategy.
#[tauri::command]
pub fn local_save_strategy(
    state: State<'_, LocalStoreState>,
    strategy: LocalStrategy,
) -> Result<(), String> {
    with_store(&state, |s| s.save_strategy(&strategy))
}

/// Delete a strategy by id. Returns whether a row was removed.
#[tauri::command]
pub fn local_delete_strategy(
    state: State<'_, LocalStoreState>,
    id: String,
) -> Result<bool, String> {
    with_store(&state, |s| s.delete_strategy(&id))
}

// =============================================================================
// S/R zones (AGT-646)
// =============================================================================

/// List S/R zones, oldest first. Scoped to one instrument when given;
/// all zones otherwise (watcher trigger maps).
#[tauri::command]
pub fn local_list_sr_zones(
    state: State<'_, LocalStoreState>,
    instrument: Option<String>,
) -> Result<Vec<LocalSRZone>, String> {
    with_store(&state, |s| s.list_sr_zones(instrument.as_deref()))
}

/// Insert or update (upsert on id) an S/R zone.
#[tauri::command]
pub fn local_save_sr_zone(
    state: State<'_, LocalStoreState>,
    zone: LocalSRZone,
) -> Result<(), String> {
    with_store(&state, |s| s.save_sr_zone(&zone))
}

/// Delete an S/R zone by id. Returns whether a row was removed.
#[tauri::command]
pub fn local_delete_sr_zone(
    state: State<'_, LocalStoreState>,
    id: String,
) -> Result<bool, String> {
    with_store(&state, |s| s.delete_sr_zone(&id))
}

/// Delete every S/R zone for an instrument. Returns the number removed.
#[tauri::command]
pub fn local_clear_sr_zones(
    state: State<'_, LocalStoreState>,
    instrument: String,
) -> Result<usize, String> {
    with_store(&state, |s| s.clear_sr_zones(&instrument))
}

// =============================================================================
// Notes (AGT-646)
// =============================================================================

/// List notes, optionally filtered to one trade or one strategy, most
/// recent first. No filters = all notes (note-count badges).
#[tauri::command]
pub fn local_list_notes(
    state: State<'_, LocalStoreState>,
    trade_id: Option<String>,
    strategy_id: Option<String>,
) -> Result<Vec<LocalNote>, String> {
    with_store(&state, |s| {
        s.list_notes(trade_id.as_deref(), strategy_id.as_deref())
    })
}

/// Insert or update (upsert on id) a note.
#[tauri::command]
pub fn local_save_note(state: State<'_, LocalStoreState>, note: LocalNote) -> Result<(), String> {
    with_store(&state, |s| s.save_note(&note))
}

/// Delete a note by id. Returns whether a row was removed.
#[tauri::command]
pub fn local_delete_note(state: State<'_, LocalStoreState>, id: String) -> Result<bool, String> {
    with_store(&state, |s| s.delete_note(&id))
}

// =============================================================================
// Chart config (AGT-646)
// =============================================================================

/// The persisted chart indicator config JSON for an instrument (or null).
#[tauri::command]
pub fn local_get_chart_config(
    state: State<'_, LocalStoreState>,
    instrument: String,
) -> Result<Option<String>, String> {
    with_store(&state, |s| s.get_chart_config(&instrument))
}

/// Persist the chart indicator config JSON for an instrument.
#[tauri::command]
pub fn local_set_chart_config(
    state: State<'_, LocalStoreState>,
    instrument: String,
    indicators: String,
) -> Result<(), String> {
    with_store(&state, |s| s.set_chart_config(&instrument, &indicators))
}

/// List all trades in the local store, most recently opened first (AGT-647).
#[tauri::command]
pub fn local_list_trades(state: State<'_, LocalStoreState>) -> Result<Vec<LocalTrade>, String> {
    with_store(&state, |s| s.list_trades())
}

/// Closed trades for one instrument (chart trade-marker overlay read path).
#[tauri::command]
pub fn local_list_closed_trades_by_instrument(
    state: State<'_, LocalStoreState>,
    instrument: String,
) -> Result<Vec<LocalTrade>, String> {
    with_store(&state, |s| s.list_closed_trades_by_instrument(&instrument))
}

/// List all stored AI trade scores.
#[tauri::command]
pub fn local_list_trade_scores(
    state: State<'_, LocalStoreState>,
) -> Result<Vec<LocalTradeScore>, String> {
    with_store(&state, |s| s.list_trade_scores())
}

/// The stored AI score for one trade (`None` if never scored).
#[tauri::command]
pub fn local_get_trade_score_by_trade(
    state: State<'_, LocalStoreState>,
    trade_id: String,
) -> Result<Option<LocalTradeScore>, String> {
    with_store(&state, |s| s.get_trade_score_by_trade(&trade_id))
}

/// Persist the AI score for a trade (upsert on `trade_id` — one per trade).
#[tauri::command]
pub fn local_save_trade_score(
    state: State<'_, LocalStoreState>,
    score: LocalTradeScore,
) -> Result<(), String> {
    with_store(&state, |s| s.save_trade_score(&score))
}

// =============================================================================
// Backtests domain (AGT-645)
// =============================================================================

/// List saved backtest runs for a strategy, oldest first (run order).
#[tauri::command]
pub fn local_list_backtests(
    state: State<'_, LocalStoreState>,
    strategy_id: String,
) -> Result<Vec<LocalBacktest>, String> {
    with_store(&state, |s| s.list_backtests_for_strategy(&strategy_id))
}

/// Insert or update (upsert on id) a backtest run.
#[tauri::command]
pub fn local_save_backtest(
    state: State<'_, LocalStoreState>,
    backtest: LocalBacktest,
) -> Result<(), String> {
    with_store(&state, |s| s.save_backtest(&backtest))
}

/// Delete every saved backtest run for a strategy. Returns how many were removed.
#[tauri::command]
pub fn local_delete_backtests_for_strategy(
    state: State<'_, LocalStoreState>,
    strategy_id: String,
) -> Result<usize, String> {
    with_store(&state, |s| s.delete_backtests_for_strategy(&strategy_id))
}

/// List backtest/walk-forward jobs for a strategy, most recently updated first.
#[tauri::command]
pub fn local_list_backtest_jobs(
    state: State<'_, LocalStoreState>,
    strategy_id: String,
) -> Result<Vec<LocalBacktestJob>, String> {
    with_store(&state, |s| s.list_backtest_jobs_for_strategy(&strategy_id))
}

/// Fetch one backtest job by id (`None` if absent).
#[tauri::command]
pub fn local_get_backtest_job(
    state: State<'_, LocalStoreState>,
    id: String,
) -> Result<Option<LocalBacktestJob>, String> {
    with_store(&state, |s| s.get_backtest_job(&id))
}

/// Insert or update (upsert on id) a backtest job.
#[tauri::command]
pub fn local_save_backtest_job(
    state: State<'_, LocalStoreState>,
    job: LocalBacktestJob,
) -> Result<(), String> {
    with_store(&state, |s| s.save_backtest_job(&job))
}

/// Append one promotion/demotion audit row (append-only).
#[tauri::command]
pub fn local_record_promotion(
    state: State<'_, LocalStoreState>,
    audit: LocalPromotionAudit,
) -> Result<(), String> {
    with_store(&state, |s| s.insert_promotion_audit(&audit))
}

// =============================================================================
// Labels domain (AGT-650)
// =============================================================================

/// List all labels, alphabetical.
#[tauri::command]
pub fn local_list_labels(state: State<'_, LocalStoreState>) -> Result<Vec<LocalLabel>, String> {
    with_store(&state, |s| s.list_labels())
}

/// Insert or update (upsert on id) a label.
#[tauri::command]
pub fn local_save_label(
    state: State<'_, LocalStoreState>,
    label: LocalLabel,
) -> Result<(), String> {
    with_store(&state, |s| s.save_label(&label))
}

/// List trade↔label junctions (scoped to one trade when given).
#[tauri::command]
pub fn local_list_trade_labels(
    state: State<'_, LocalStoreState>,
    trade_id: Option<String>,
) -> Result<Vec<LocalTradeLabel>, String> {
    with_store(&state, |s| s.list_trade_labels(trade_id.as_deref()))
}

/// Attach a label to a trade.
#[tauri::command]
pub fn local_add_trade_label(
    state: State<'_, LocalStoreState>,
    trade_label: LocalTradeLabel,
) -> Result<(), String> {
    with_store(&state, |s| s.insert_trade_label(&trade_label))
}

/// Detach a label from a trade (by junction id). Returns whether a row was removed.
#[tauri::command]
pub fn local_delete_trade_label(
    state: State<'_, LocalStoreState>,
    id: String,
) -> Result<bool, String> {
    with_store(&state, |s| s.delete_trade_label(&id))
}

/// List strategy↔label junctions (scoped to one strategy when given).
#[tauri::command]
pub fn local_list_strategy_labels(
    state: State<'_, LocalStoreState>,
    strategy_id: Option<String>,
) -> Result<Vec<LocalStrategyLabel>, String> {
    with_store(&state, |s| s.list_strategy_labels(strategy_id.as_deref()))
}

/// Attach a label to a strategy.
#[tauri::command]
pub fn local_add_strategy_label(
    state: State<'_, LocalStoreState>,
    strategy_label: LocalStrategyLabel,
) -> Result<(), String> {
    with_store(&state, |s| s.insert_strategy_label(&strategy_label))
}

/// Detach a label from a strategy (by junction id). Returns whether a row was removed.
#[tauri::command]
pub fn local_delete_strategy_label(
    state: State<'_, LocalStoreState>,
    id: String,
) -> Result<bool, String> {
    with_store(&state, |s| s.delete_strategy_label(&id))
}

// =============================================================================
// Strategy-trade attribution (AGT-650)
// =============================================================================

/// List strategy↔trade attribution rows (scoped to one strategy when given),
/// most recently executed first.
#[tauri::command]
pub fn local_list_strategy_trades(
    state: State<'_, LocalStoreState>,
    strategy_id: Option<String>,
) -> Result<Vec<LocalStrategyTrade>, String> {
    with_store(&state, |s| s.list_strategy_trades(strategy_id.as_deref()))
}

/// Record one strategy↔trade attribution row (upsert on id).
#[tauri::command]
pub fn local_save_strategy_trade(
    state: State<'_, LocalStoreState>,
    strategy_trade: LocalStrategyTrade,
) -> Result<(), String> {
    with_store(&state, |s| s.insert_strategy_trade(&strategy_trade))
}

// =============================================================================
// Strategy-watcher configs (AGT-650)
// =============================================================================

/// List persisted watcher configs, most recently updated first.
#[tauri::command]
pub fn local_list_strategy_watchers(
    state: State<'_, LocalStoreState>,
) -> Result<Vec<LocalStrategyWatcher>, String> {
    with_store(&state, |s| s.list_strategy_watchers())
}

/// Insert or update (upsert on id) a watcher config.
#[tauri::command]
pub fn local_save_strategy_watcher(
    state: State<'_, LocalStoreState>,
    watcher: LocalStrategyWatcher,
) -> Result<(), String> {
    with_store(&state, |s| s.save_strategy_watcher(&watcher))
}

/// Delete a watcher config by id. Returns whether a row was removed.
#[tauri::command]
pub fn local_delete_strategy_watcher(
    state: State<'_, LocalStoreState>,
    id: String,
) -> Result<bool, String> {
    with_store(&state, |s| s.delete_strategy_watcher(&id))
}

// =============================================================================
// Device credentials (AGT-650)
// =============================================================================

/// The device's stored (encrypted) credential row, or null before onboarding.
#[tauri::command]
pub fn local_get_credential(
    state: State<'_, LocalStoreState>,
) -> Result<Option<LocalCredential>, String> {
    with_store(&state, |s| s.get_credential())
}

/// Insert or update (upsert on id) the device credential row. Blobs are
/// ciphertext produced by the crypto vault — never plaintext.
#[tauri::command]
pub fn local_save_credential(
    state: State<'_, LocalStoreState>,
    credential: LocalCredential,
) -> Result<(), String> {
    with_store(&state, |s| s.save_credential(&credential))
}

/// Delete all stored credential rows (the "reset credentials" flow).
#[tauri::command]
pub fn local_delete_credentials(state: State<'_, LocalStoreState>) -> Result<usize, String> {
    with_store(&state, |s| s.delete_credentials())
}
