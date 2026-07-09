//! wickd local app store — the local-first SQLite foundation (AGT-642).
//!
//! A single SQLite database at **`~/.wickd/app.db`** holds the desktop app's
//! local data. It lives in the same `~/.wickd` data home the wickd CLI uses
//! for its stores (`audit.db`, `baselines.db`, ...), and follows the same
//! conventions: bundled rusqlite (no system-sqlite dependency), a default
//! path resolved from the home directory, and an explicit-path constructor so
//! tests never touch the real store.
//!
//! This module is the **data-access layer**: all SQL for the app's local
//! datasets lives here (schema in [`migrations`]). Tauri command wrappers
//! live in `commands::local_store`; the frontend wrapper is
//! `src/lib/localStore.ts`. Follow-up domain migrations (AGT-645/646/647)
//! extend this module with new datasets rather than adding SQL elsewhere.
//!
//! The walking-skeleton dataset is **strategies** ([`LocalStrategy`]),
//! shaped to match the Zero `strategy` table (minus `user_id` — single-user)
//! so the later data migration is a straight copy.

pub mod import;
pub mod migrations;

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};

/// File name of the app store inside the wickd data home.
const APP_DB_FILE: &str = "app.db";

/// The wickd data home: `~/.wickd` (shared with the wickd CLI).
pub fn wickd_data_home() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "could not resolve home directory".to_string())?;
    Ok(home.join(".wickd"))
}

/// Default path of the app store: `~/.wickd/app.db`.
pub fn default_db_path() -> Result<PathBuf, String> {
    Ok(wickd_data_home()?.join(APP_DB_FILE))
}

/// One strategy row in the local store.
///
/// Field names/serialization are snake_case, matching the Zero `strategy` row
/// shape the frontend already consumes (`shared/schema.ts`), so components can
/// switch data sources without remapping. JSON-encoded columns stay `String`s
/// here; the frontend owns their parsing, exactly as it did with Zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalStrategy {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub schema_version: Option<i64>,
    pub parameters: Option<String>,
    pub variables: Option<String>,
    #[serde(default = "default_json_array")]
    pub indicators: String,
    #[serde(default = "default_json_array")]
    pub entry_rules: String,
    pub entry_logic: Option<String>,
    #[serde(default = "default_json_array")]
    pub exit_rules: String,
    #[serde(default = "default_json_object")]
    pub risk_settings: String,
    pub planning_conversation: Option<String>,
    pub auto_note_indicators: Option<String>,
    pub pivot_config: Option<String>,
    pub strategy_type: Option<String>,
    pub script_content: Option<String>,
    #[serde(default = "default_version")]
    pub version: i64,
    #[serde(default = "default_true")]
    pub is_active: bool,
    #[serde(default)]
    pub is_promoted: bool,
    #[serde(default)]
    pub is_locked: bool,
    #[serde(default)]
    pub is_archived: bool,
    /// Epoch milliseconds (same unit as the Zero schema).
    pub created_at: i64,
    /// Epoch milliseconds (same unit as the Zero schema).
    pub updated_at: i64,
    /// Provenance tag (AGT-648): `""` = native wickd data; `"candlesight"` =
    /// imported from the CandleSight archive. Serde-defaults to `""` so
    /// pre-v5 callers keep working unchanged.
    #[serde(default)]
    pub source: String,
}

fn default_json_array() -> String {
    "[]".to_string()
}
fn default_json_object() -> String {
    "{}".to_string()
}
fn default_version() -> i64 {
    1
}
fn default_true() -> bool {
    true
}

/// One S/R zone row in the local store (AGT-646).
///
/// Mirrors the Zero `sr_zone` row shape minus `user_id`. Prices stay TEXT
/// (Decimal-safe); the frontend parses for rendering only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalSRZone {
    pub id: String,
    pub instrument: String,
    pub upper_price: String,
    pub lower_price: String,
    pub label: Option<String>,
    pub color: Option<String>,
    /// Epoch milliseconds (same unit as the Zero schema).
    pub created_at: i64,
    /// Epoch milliseconds (same unit as the Zero schema).
    pub updated_at: i64,
}

/// One note row in the local store (AGT-646).
///
/// Mirrors the Zero `note` row shape minus `user_id`. A note attaches to a
/// trade (`trade_id`) or a strategy (`strategy_id`); both optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalNote {
    pub id: String,
    pub trade_id: Option<String>,
    pub strategy_id: Option<String>,
    #[serde(default)]
    pub title: String,
    pub content: String,
    /// Epoch milliseconds (same unit as the Zero schema).
    pub created_at: i64,
    /// Epoch milliseconds (same unit as the Zero schema).
    pub updated_at: i64,
}

/// One trade row in the local store (AGT-647).
///
/// Mirrors the Zero `trade` row minus `user_id`; `id` is the raw OANDA trade
/// id. Prices/units/P&L are decimal strings — never floats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalTrade {
    pub id: String,
    pub account_id: Option<String>,
    pub instrument: String,
    pub units: String,
    pub open_price: String,
    pub close_price: Option<String>,
    /// Epoch milliseconds.
    pub open_time: i64,
    /// Epoch milliseconds.
    pub close_time: Option<i64>,
    pub realized_pl: Option<String>,
    /// `'OPEN' | 'CLOSED' | 'CLOSE_WHEN_TRADEABLE'`.
    pub state: String,
    /// Epoch milliseconds.
    pub synced_at: i64,
    /// Epoch milliseconds.
    pub created_at: i64,
    /// Epoch milliseconds.
    pub updated_at: i64,
}

/// One AI trade-score row (persisted AI analysis artifact, AGT-647).
///
/// Mirrors the Zero `trade_score` row minus `user_id`. One score per trade
/// (`trade_id` is unique) — closed trades don't change, so a score is final.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalTradeScore {
    pub id: String,
    pub trade_id: String,
    pub score_entry: i64,
    pub score_exit: i64,
    pub score_risk_management: i64,
    pub score_overall: i64,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub entry_assessment: String,
    #[serde(default)]
    pub exit_assessment: String,
    /// JSON: IndicatorFinding[]
    #[serde(default = "default_json_array")]
    pub indicator_analysis: String,
    /// JSON: string[]
    #[serde(default = "default_json_array")]
    pub conflicting_indicators: String,
    /// JSON: string[]
    #[serde(default = "default_json_array")]
    pub learning_points: String,
    /// Epoch milliseconds.
    pub created_at: i64,
}

/// One saved backtest run in the local store (AGT-645).
///
/// Mirrors the Zero `backtest` row minus `user_id`. `results` is a JSON string
/// carrying the full run payload (metrics + trades + equity curve + config),
/// so the backtest UI renders everything offline from this one row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalBacktest {
    pub id: String,
    pub strategy_id: String,
    pub instrument: String,
    /// Epoch milliseconds (same unit as the Zero schema).
    pub start_date: i64,
    /// Epoch milliseconds (same unit as the Zero schema).
    pub end_date: i64,
    /// JSON: full backtest run payload.
    pub results: String,
    /// Epoch milliseconds.
    pub created_at: i64,
}

/// One long-running backtest/walk-forward job (AGT-645).
///
/// Mirrors the Zero `backtest_job` row minus `user_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalBacktestJob {
    pub id: String,
    pub strategy_id: String,
    /// 'walk_forward' | 'simple_backtest' | 'optimization'
    pub job_type: String,
    /// 'pending' | 'running' | 'completed' | 'failed' | 'cancelled'
    pub status: String,
    /// JSON: job parameters (instrument, dates, ...).
    pub params: String,
    /// 0-100 completion percentage.
    #[serde(default)]
    pub progress: i64,
    /// JSON: detailed progress (phase, window, ...).
    pub progress_detail: Option<String>,
    /// JSON: full result when completed.
    pub result: Option<String>,
    pub error_message: Option<String>,
    /// Epoch milliseconds.
    pub created_at: i64,
    /// Epoch milliseconds.
    pub updated_at: i64,
    /// Epoch milliseconds; set when the job finished (success or failure).
    pub completed_at: Option<i64>,
}

/// One promotion/demotion audit row (AGT-645).
///
/// Mirrors the Zero `promotion_audit` row minus `user_id`. Append-only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalPromotionAudit {
    pub id: String,
    pub strategy_id: String,
    pub strategy_name: String,
    /// 'promote' | 'demote'
    pub action: String,
    /// Epoch milliseconds.
    pub created_at: i64,
}

/// One user-defined label (AGT-650).
///
/// Mirrors the Zero `label` row minus `user_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalLabel {
    pub id: String,
    pub name: String,
    /// Optional hex color.
    pub color: Option<String>,
    /// Epoch milliseconds.
    pub created_at: i64,
}

/// Trade↔label junction row (AGT-650). Mirrors Zero `trade_label` minus `user_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalTradeLabel {
    pub id: String,
    pub trade_id: String,
    pub label_id: String,
    /// Epoch milliseconds.
    pub created_at: i64,
}

/// Strategy↔label junction row (AGT-650). Mirrors Zero `strategy_label` minus `user_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalStrategyLabel {
    pub id: String,
    pub strategy_id: String,
    pub label_id: String,
    /// Epoch milliseconds.
    pub created_at: i64,
}

/// One strategy↔OANDA-trade attribution row (AGT-650).
///
/// Mirrors the Zero `strategy_trade` row minus `user_id`. Written when a
/// pattern-match execution places a trade; read by the strategy stats UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalStrategyTrade {
    pub id: String,
    pub strategy_id: String,
    pub strategy_config_id: Option<String>,
    /// Raw OANDA trade id.
    pub trade_id: String,
    pub instrument: String,
    /// e.g. `H1`, `H4`.
    pub timeframe: String,
    /// 'long' | 'short'
    pub direction: String,
    /// Price at match/execution — TEXT, Decimal-safe.
    pub entry_price: String,
    /// Epoch milliseconds; when the pattern match was detected.
    pub match_time: i64,
    /// Epoch milliseconds; when the trade was executed.
    pub executed_at: i64,
    /// JSON: which entry rules fired.
    pub rules_triggered: Option<String>,
    /// Epoch milliseconds.
    pub created_at: i64,
}

/// One persisted watcher configuration (AGT-650).
///
/// Mirrors the Zero `strategy_watcher` row minus `user_id`. `id` is the
/// config id (`strategy_id-instrument-timeframe`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalStrategyWatcher {
    pub id: String,
    pub strategy_id: String,
    /// Cached strategy name for display.
    pub strategy_name: Option<String>,
    pub instrument: String,
    pub timeframe: String,
    /// 'signal_only' | 'confirm_execute' | 'auto_execute'
    pub mode: String,
    /// 'all' | 'entries' | 'exits' | 'longs' | 'shorts'
    pub signal_filter: String,
    /// Whether to auto-start on app load.
    pub is_active: bool,
    /// Epoch milliseconds.
    pub created_at: i64,
    /// Epoch milliseconds.
    pub updated_at: i64,
}

/// The device's encrypted OANDA credential blobs (AGT-650).
///
/// Replaces the cloud `user_credentials` table. Blobs are ciphertext —
/// encryption/decryption stays in the Rust crypto vault; this row is just
/// where the ciphertext lives. Single-user store: one row in practice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalCredential {
    pub id: String,
    pub device_id: String,
    pub practice_blob: Option<String>,
    pub practice_account_id: Option<String>,
    pub live_blob: Option<String>,
    pub live_account_id: Option<String>,
    /// Epoch milliseconds.
    pub created_at: i64,
    /// Epoch milliseconds.
    pub updated_at: i64,
}

/// Handle over the local app store. Owns the connection; all SQL lives here.
pub struct LocalStore {
    conn: Connection,
}

impl LocalStore {
    /// Open (creating if needed) the default store at `~/.wickd/app.db` and
    /// apply pending migrations. Creates `~/.wickd` if it does not exist.
    pub fn open_default() -> Result<Self, String> {
        let path = default_db_path()?;
        Self::open_at(&path)
    }

    /// Open a store at an explicit path (tests use a temp path so they never
    /// touch the real `~/.wickd/app.db`).
    pub fn open_at(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
        }
        let mut conn = Connection::open(path)
            .map_err(|e| format!("failed to open local store {}: {e}", path.display()))?;
        // WAL keeps concurrent reads cheap; matches a long-lived desktop process.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| format!("failed to set WAL mode: {e}"))?;
        migrations::apply(&mut conn).map_err(|e| format!("failed to migrate local store: {e}"))?;
        Ok(Self { conn })
    }

    // =========================================================================
    // Strategies DAL
    // =========================================================================

    /// All strategies, most recently updated first. Includes archived and
    /// inactive rows — filtering is a presentation concern.
    pub fn list_strategies(&self) -> Result<Vec<LocalStrategy>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {STRATEGY_COLUMNS} FROM strategy ORDER BY updated_at DESC"
            ))
            .map_err(err_str)?;
        let rows = stmt
            .query_map([], strategy_from_row)
            .map_err(err_str)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err_str)?;
        Ok(rows)
    }

    /// A single strategy by id, or `None` if absent.
    pub fn get_strategy(&self, id: &str) -> Result<Option<LocalStrategy>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {STRATEGY_COLUMNS} FROM strategy WHERE id = ?1"
            ))
            .map_err(err_str)?;
        let mut rows = stmt.query_map(params![id], strategy_from_row).map_err(err_str)?;
        match rows.next() {
            Some(row) => Ok(Some(row.map_err(err_str)?)),
            None => Ok(None),
        }
    }

    /// Insert or replace a strategy (upsert keyed on `id`).
    pub fn save_strategy(&self, s: &LocalStrategy) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO strategy (
                    id, name, description, schema_version, parameters, variables,
                    indicators, entry_rules, entry_logic, exit_rules, risk_settings,
                    planning_conversation, auto_note_indicators, pivot_config,
                    strategy_type, script_content, version,
                    is_active, is_promoted, is_locked, is_archived,
                    created_at, updated_at, source
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24
                 )
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    description = excluded.description,
                    schema_version = excluded.schema_version,
                    parameters = excluded.parameters,
                    variables = excluded.variables,
                    indicators = excluded.indicators,
                    entry_rules = excluded.entry_rules,
                    entry_logic = excluded.entry_logic,
                    exit_rules = excluded.exit_rules,
                    risk_settings = excluded.risk_settings,
                    planning_conversation = excluded.planning_conversation,
                    auto_note_indicators = excluded.auto_note_indicators,
                    pivot_config = excluded.pivot_config,
                    strategy_type = excluded.strategy_type,
                    script_content = excluded.script_content,
                    version = excluded.version,
                    is_active = excluded.is_active,
                    is_promoted = excluded.is_promoted,
                    is_locked = excluded.is_locked,
                    is_archived = excluded.is_archived,
                    created_at = excluded.created_at,
                    updated_at = excluded.updated_at,
                    source = excluded.source",
                params![
                    s.id,
                    s.name,
                    s.description,
                    s.schema_version,
                    s.parameters,
                    s.variables,
                    s.indicators,
                    s.entry_rules,
                    s.entry_logic,
                    s.exit_rules,
                    s.risk_settings,
                    s.planning_conversation,
                    s.auto_note_indicators,
                    s.pivot_config,
                    s.strategy_type,
                    s.script_content,
                    s.version,
                    s.is_active,
                    s.is_promoted,
                    s.is_locked,
                    s.is_archived,
                    s.created_at,
                    s.updated_at,
                    s.source,
                ],
            )
            .map_err(err_str)?;
        Ok(())
    }

    /// Hard-delete a strategy. Returns whether a row was removed.
    ///
    /// (The Zero schema modeled soft-delete via `is_active`; the local skeleton
    /// exposes a hard delete for the walking-skeleton UI, and callers that want
    /// tombstoning can flip `is_active` through [`Self::save_strategy`].)
    pub fn delete_strategy(&self, id: &str) -> Result<bool, String> {
        let n = self
            .conn
            .execute("DELETE FROM strategy WHERE id = ?1", params![id])
            .map_err(err_str)?;
        Ok(n > 0)
    }

    // =========================================================================
    // S/R zones DAL (AGT-646)
    // =========================================================================

    /// S/R zones, oldest first (stable draw order). Pass an instrument to
    /// scope to one chart; `None` returns all zones (watcher trigger maps).
    pub fn list_sr_zones(&self, instrument: Option<&str>) -> Result<Vec<LocalSRZone>, String> {
        let (filter, param) = match instrument {
            Some(i) => ("WHERE instrument = ?1", Some(i)),
            None => ("", None),
        };
        let sql =
            format!("SELECT {SR_ZONE_COLUMNS} FROM sr_zone {filter} ORDER BY created_at ASC");
        let mut stmt = self.conn.prepare(&sql).map_err(err_str)?;
        let rows = match param {
            Some(p) => stmt
                .query_map(params![p], sr_zone_from_row)
                .map_err(err_str)?
                .collect::<Result<Vec<_>, _>>(),
            None => stmt
                .query_map([], sr_zone_from_row)
                .map_err(err_str)?
                .collect::<Result<Vec<_>, _>>(),
        }
        .map_err(err_str)?;
        Ok(rows)
    }

    /// Insert or replace an S/R zone (upsert keyed on `id`).
    pub fn save_sr_zone(&self, z: &LocalSRZone) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO sr_zone (
                    id, instrument, upper_price, lower_price, label, color,
                    created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET
                    instrument = excluded.instrument,
                    upper_price = excluded.upper_price,
                    lower_price = excluded.lower_price,
                    label = excluded.label,
                    color = excluded.color,
                    created_at = excluded.created_at,
                    updated_at = excluded.updated_at",
                params![
                    z.id,
                    z.instrument,
                    z.upper_price,
                    z.lower_price,
                    z.label,
                    z.color,
                    z.created_at,
                    z.updated_at,
                ],
            )
            .map_err(err_str)?;
        Ok(())
    }

    /// Hard-delete one S/R zone. Returns whether a row was removed.
    pub fn delete_sr_zone(&self, id: &str) -> Result<bool, String> {
        let n = self
            .conn
            .execute("DELETE FROM sr_zone WHERE id = ?1", params![id])
            .map_err(err_str)?;
        Ok(n > 0)
    }

    /// Delete every S/R zone for an instrument ("clear all"). Returns the
    /// number of rows removed.
    pub fn clear_sr_zones(&self, instrument: &str) -> Result<usize, String> {
        self.conn
            .execute("DELETE FROM sr_zone WHERE instrument = ?1", params![instrument])
            .map_err(err_str)
    }

    // =========================================================================
    // Notes DAL (AGT-646)
    // =========================================================================

    /// Notes, optionally filtered to one trade or one strategy, most recent
    /// first. With no filter, returns all notes (used for note-count badges).
    pub fn list_notes(
        &self,
        trade_id: Option<&str>,
        strategy_id: Option<&str>,
    ) -> Result<Vec<LocalNote>, String> {
        let (filter, param): (&str, Option<&str>) = match (trade_id, strategy_id) {
            (Some(t), _) => ("WHERE trade_id = ?1", Some(t)),
            (None, Some(s)) => ("WHERE strategy_id = ?1", Some(s)),
            (None, None) => ("", None),
        };
        let sql = format!("SELECT {NOTE_COLUMNS} FROM note {filter} ORDER BY created_at DESC");
        let mut stmt = self.conn.prepare(&sql).map_err(err_str)?;
        let rows = match param {
            Some(p) => stmt
                .query_map(params![p], note_from_row)
                .map_err(err_str)?
                .collect::<Result<Vec<_>, _>>(),
            None => stmt
                .query_map([], note_from_row)
                .map_err(err_str)?
                .collect::<Result<Vec<_>, _>>(),
        }
        .map_err(err_str)?;
        Ok(rows)
    }

    /// Insert or replace a note (upsert keyed on `id`).
    pub fn save_note(&self, n: &LocalNote) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO note (
                    id, trade_id, strategy_id, title, content, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                    trade_id = excluded.trade_id,
                    strategy_id = excluded.strategy_id,
                    title = excluded.title,
                    content = excluded.content,
                    created_at = excluded.created_at,
                    updated_at = excluded.updated_at",
                params![
                    n.id,
                    n.trade_id,
                    n.strategy_id,
                    n.title,
                    n.content,
                    n.created_at,
                    n.updated_at,
                ],
            )
            .map_err(err_str)?;
        Ok(())
    }

    /// Hard-delete a note. Returns whether a row was removed.
    pub fn delete_note(&self, id: &str) -> Result<bool, String> {
        let n = self
            .conn
            .execute("DELETE FROM note WHERE id = ?1", params![id])
            .map_err(err_str)?;
        Ok(n > 0)
    }

    // =========================================================================
    // Chart config DAL (AGT-646)
    // =========================================================================

    /// The persisted chart indicator config (JSON `ChartIndicatorConfig[]`)
    /// for an instrument, or `None` if never saved.
    pub fn get_chart_config(&self, instrument: &str) -> Result<Option<String>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT indicators FROM chart_config WHERE instrument = ?1")
            .map_err(err_str)?;
        let mut rows = stmt
            .query_map(params![instrument], |row| row.get::<_, String>(0))
            .map_err(err_str)?;
        match rows.next() {
            Some(row) => Ok(Some(row.map_err(err_str)?)),
            None => Ok(None),
        }
    }

    /// Upsert the chart indicator config (JSON string) for an instrument.
    pub fn set_chart_config(&self, instrument: &str, indicators: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO chart_config (instrument, indicators, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(instrument) DO UPDATE SET
                    indicators = excluded.indicators,
                    updated_at = excluded.updated_at",
                params![instrument, indicators, now_ms()],
            )
            .map_err(err_str)?;
        Ok(())
    }
    // =========================================================================
    // Trades DAL (AGT-647)
    // =========================================================================

    /// All trades, most recently opened first. Includes open and closed rows —
    /// filtering is a presentation concern.
    pub fn list_trades(&self) -> Result<Vec<LocalTrade>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {TRADE_COLUMNS} FROM trade ORDER BY open_time DESC"
            ))
            .map_err(err_str)?;
        let rows = stmt
            .query_map([], trade_from_row)
            .map_err(err_str)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err_str)?;
        Ok(rows)
    }

    /// Closed trades for one instrument, most recently opened first (the
    /// chart-overlay read path).
    pub fn list_closed_trades_by_instrument(
        &self,
        instrument: &str,
    ) -> Result<Vec<LocalTrade>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {TRADE_COLUMNS} FROM trade
                 WHERE instrument = ?1 AND state = 'CLOSED'
                 ORDER BY open_time DESC"
            ))
            .map_err(err_str)?;
        let rows = stmt
            .query_map(params![instrument], trade_from_row)
            .map_err(err_str)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err_str)?;
        Ok(rows)
    }

    /// Bulk insert-or-replace trades (upsert keyed on `id`), in one
    /// transaction. This is the OANDA-sync write path: re-syncing an open
    /// trade that has since closed flips its row in place.
    pub fn upsert_trades(&mut self, trades: &[LocalTrade]) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(err_str)?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO trade (
                        id, account_id, instrument, units, open_price, close_price,
                        open_time, close_time, realized_pl, state,
                        synced_at, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                     ON CONFLICT(id) DO UPDATE SET
                        account_id = excluded.account_id,
                        instrument = excluded.instrument,
                        units = excluded.units,
                        open_price = excluded.open_price,
                        close_price = excluded.close_price,
                        open_time = excluded.open_time,
                        close_time = excluded.close_time,
                        realized_pl = excluded.realized_pl,
                        state = excluded.state,
                        synced_at = excluded.synced_at,
                        updated_at = excluded.updated_at",
                )
                .map_err(err_str)?;
            for t in trades {
                stmt.execute(params![
                    t.id,
                    t.account_id,
                    t.instrument,
                    t.units,
                    t.open_price,
                    t.close_price,
                    t.open_time,
                    t.close_time,
                    t.realized_pl,
                    t.state,
                    t.synced_at,
                    t.created_at,
                    t.updated_at,
                ])
                .map_err(err_str)?;
            }
        }
        tx.commit().map_err(err_str)
    }

    // =========================================================================
    // Trade scores DAL (AGT-647)
    // =========================================================================

    /// All AI trade scores (used to badge scored trades in the list).
    pub fn list_trade_scores(&self) -> Result<Vec<LocalTradeScore>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {TRADE_SCORE_COLUMNS} FROM trade_score ORDER BY created_at DESC"
            ))
            .map_err(err_str)?;
        let rows = stmt
            .query_map([], trade_score_from_row)
            .map_err(err_str)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err_str)?;
        Ok(rows)
    }

    /// The stored AI score for one trade, or `None` if it was never scored.
    pub fn get_trade_score_by_trade(
        &self,
        trade_id: &str,
    ) -> Result<Option<LocalTradeScore>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {TRADE_SCORE_COLUMNS} FROM trade_score WHERE trade_id = ?1"
            ))
            .map_err(err_str)?;
        let mut rows = stmt
            .query_map(params![trade_id], trade_score_from_row)
            .map_err(err_str)?;
        match rows.next() {
            Some(row) => Ok(Some(row.map_err(err_str)?)),
            None => Ok(None),
        }
    }

    /// Insert or replace the AI score for a trade (upsert keyed on `trade_id`
    /// — one score per trade).
    pub fn save_trade_score(&self, s: &LocalTradeScore) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO trade_score (
                    id, trade_id, score_entry, score_exit, score_risk_management,
                    score_overall, summary, entry_assessment, exit_assessment,
                    indicator_analysis, conflicting_indicators, learning_points,
                    created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(trade_id) DO UPDATE SET
                    score_entry = excluded.score_entry,
                    score_exit = excluded.score_exit,
                    score_risk_management = excluded.score_risk_management,
                    score_overall = excluded.score_overall,
                    summary = excluded.summary,
                    entry_assessment = excluded.entry_assessment,
                    exit_assessment = excluded.exit_assessment,
                    indicator_analysis = excluded.indicator_analysis,
                    conflicting_indicators = excluded.conflicting_indicators,
                    learning_points = excluded.learning_points,
                    created_at = excluded.created_at",
                params![
                    s.id,
                    s.trade_id,
                    s.score_entry,
                    s.score_exit,
                    s.score_risk_management,
                    s.score_overall,
                    s.summary,
                    s.entry_assessment,
                    s.exit_assessment,
                    s.indicator_analysis,
                    s.conflicting_indicators,
                    s.learning_points,
                    s.created_at,
                ],
            )
            .map_err(err_str)?;
        Ok(())
    }

    // =========================================================================
    // Backtests DAL (AGT-645)
    // =========================================================================

    /// All saved backtest runs for one strategy, oldest first (run order).
    pub fn list_backtests_for_strategy(
        &self,
        strategy_id: &str,
    ) -> Result<Vec<LocalBacktest>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {BACKTEST_COLUMNS} FROM backtest \
                 WHERE strategy_id = ?1 ORDER BY created_at ASC"
            ))
            .map_err(err_str)?;
        let rows = stmt
            .query_map(params![strategy_id], backtest_from_row)
            .map_err(err_str)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err_str)?;
        Ok(rows)
    }

    /// Insert or replace a backtest run (upsert keyed on `id`).
    pub fn save_backtest(&self, b: &LocalBacktest) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO backtest (
                    id, strategy_id, instrument, start_date, end_date, results, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                    strategy_id = excluded.strategy_id,
                    instrument = excluded.instrument,
                    start_date = excluded.start_date,
                    end_date = excluded.end_date,
                    results = excluded.results,
                    created_at = excluded.created_at",
                params![
                    b.id,
                    b.strategy_id,
                    b.instrument,
                    b.start_date,
                    b.end_date,
                    b.results,
                    b.created_at,
                ],
            )
            .map_err(err_str)?;
        Ok(())
    }

    /// Delete every saved run for one strategy. Returns how many were removed.
    pub fn delete_backtests_for_strategy(&self, strategy_id: &str) -> Result<usize, String> {
        self.conn
            .execute(
                "DELETE FROM backtest WHERE strategy_id = ?1",
                params![strategy_id],
            )
            .map_err(err_str)
    }

    // =========================================================================
    // Backtest jobs DAL (AGT-645)
    // =========================================================================

    /// All jobs for one strategy, most recently updated first.
    pub fn list_backtest_jobs_for_strategy(
        &self,
        strategy_id: &str,
    ) -> Result<Vec<LocalBacktestJob>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {BACKTEST_JOB_COLUMNS} FROM backtest_job \
                 WHERE strategy_id = ?1 ORDER BY updated_at DESC"
            ))
            .map_err(err_str)?;
        let rows = stmt
            .query_map(params![strategy_id], backtest_job_from_row)
            .map_err(err_str)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err_str)?;
        Ok(rows)
    }

    /// One job by id, or `None` if absent.
    pub fn get_backtest_job(&self, id: &str) -> Result<Option<LocalBacktestJob>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {BACKTEST_JOB_COLUMNS} FROM backtest_job WHERE id = ?1"
            ))
            .map_err(err_str)?;
        let mut rows = stmt
            .query_map(params![id], backtest_job_from_row)
            .map_err(err_str)?;
        match rows.next() {
            Some(row) => Ok(Some(row.map_err(err_str)?)),
            None => Ok(None),
        }
    }

    /// Insert or replace a backtest job (upsert keyed on `id`).
    pub fn save_backtest_job(&self, j: &LocalBacktestJob) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO backtest_job (
                    id, strategy_id, job_type, status, params, progress,
                    progress_detail, result, error_message,
                    created_at, updated_at, completed_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(id) DO UPDATE SET
                    strategy_id = excluded.strategy_id,
                    job_type = excluded.job_type,
                    status = excluded.status,
                    params = excluded.params,
                    progress = excluded.progress,
                    progress_detail = excluded.progress_detail,
                    result = excluded.result,
                    error_message = excluded.error_message,
                    created_at = excluded.created_at,
                    updated_at = excluded.updated_at,
                    completed_at = excluded.completed_at",
                params![
                    j.id,
                    j.strategy_id,
                    j.job_type,
                    j.status,
                    j.params,
                    j.progress,
                    j.progress_detail,
                    j.result,
                    j.error_message,
                    j.created_at,
                    j.updated_at,
                    j.completed_at,
                ],
            )
            .map_err(err_str)?;
        Ok(())
    }

    // =========================================================================
    // Promotion audit DAL (AGT-645)
    // =========================================================================

    /// Append one promotion/demotion audit row (insert-only by design).
    pub fn insert_promotion_audit(&self, a: &LocalPromotionAudit) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO promotion_audit (
                    id, strategy_id, strategy_name, action, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![a.id, a.strategy_id, a.strategy_name, a.action, a.created_at],
            )
            .map_err(err_str)?;
        Ok(())
    }

    /// All audit rows for one strategy, oldest first (only used by tests and
    /// diagnostics today; the UI surface is write-only).
    pub fn list_promotion_audits_for_strategy(
        &self,
        strategy_id: &str,
    ) -> Result<Vec<LocalPromotionAudit>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, strategy_id, strategy_name, action, created_at \
                 FROM promotion_audit WHERE strategy_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(err_str)?;
        let rows = stmt
            .query_map(params![strategy_id], |row| {
                Ok(LocalPromotionAudit {
                    id: row.get(0)?,
                    strategy_id: row.get(1)?,
                    strategy_name: row.get(2)?,
                    action: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .map_err(err_str)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err_str)?;
        Ok(rows)
    }

    // =========================================================================
    // Labels DAL (AGT-650)
    // =========================================================================

    /// All labels, alphabetical.
    pub fn list_labels(&self) -> Result<Vec<LocalLabel>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {LABEL_COLUMNS} FROM label ORDER BY name ASC"
            ))
            .map_err(err_str)?;
        let rows = stmt
            .query_map([], label_from_row)
            .map_err(err_str)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err_str)?;
        Ok(rows)
    }

    /// Insert or replace a label (upsert keyed on `id`).
    pub fn save_label(&self, l: &LocalLabel) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO label (id, name, color, created_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    color = excluded.color",
                params![l.id, l.name, l.color, l.created_at],
            )
            .map_err(err_str)?;
        Ok(())
    }

    /// Trade↔label junctions; filtered to one trade when `trade_id` is given.
    pub fn list_trade_labels(
        &self,
        trade_id: Option<&str>,
    ) -> Result<Vec<LocalTradeLabel>, String> {
        let filter = if trade_id.is_some() {
            "WHERE trade_id = ?1"
        } else {
            ""
        };
        let sql = format!(
            "SELECT {TRADE_LABEL_COLUMNS} FROM trade_label {filter} ORDER BY created_at ASC"
        );
        let mut stmt = self.conn.prepare(&sql).map_err(err_str)?;
        let rows = match trade_id {
            Some(t) => stmt
                .query_map(params![t], trade_label_from_row)
                .map_err(err_str)?
                .collect::<Result<Vec<_>, _>>(),
            None => stmt
                .query_map([], trade_label_from_row)
                .map_err(err_str)?
                .collect::<Result<Vec<_>, _>>(),
        }
        .map_err(err_str)?;
        Ok(rows)
    }

    /// Attach a label to a trade.
    pub fn insert_trade_label(&self, tl: &LocalTradeLabel) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO trade_label (id, trade_id, label_id, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![tl.id, tl.trade_id, tl.label_id, tl.created_at],
            )
            .map_err(err_str)?;
        Ok(())
    }

    /// Detach a label from a trade (by junction id). Returns whether a row was deleted.
    pub fn delete_trade_label(&self, id: &str) -> Result<bool, String> {
        let n = self
            .conn
            .execute("DELETE FROM trade_label WHERE id = ?1", params![id])
            .map_err(err_str)?;
        Ok(n > 0)
    }

    /// Strategy↔label junctions; filtered to one strategy when `strategy_id` is given.
    pub fn list_strategy_labels(
        &self,
        strategy_id: Option<&str>,
    ) -> Result<Vec<LocalStrategyLabel>, String> {
        let filter = if strategy_id.is_some() {
            "WHERE strategy_id = ?1"
        } else {
            ""
        };
        let sql = format!(
            "SELECT {STRATEGY_LABEL_COLUMNS} FROM strategy_label {filter} ORDER BY created_at ASC"
        );
        let mut stmt = self.conn.prepare(&sql).map_err(err_str)?;
        let rows = match strategy_id {
            Some(s) => stmt
                .query_map(params![s], strategy_label_from_row)
                .map_err(err_str)?
                .collect::<Result<Vec<_>, _>>(),
            None => stmt
                .query_map([], strategy_label_from_row)
                .map_err(err_str)?
                .collect::<Result<Vec<_>, _>>(),
        }
        .map_err(err_str)?;
        Ok(rows)
    }

    /// Attach a label to a strategy.
    pub fn insert_strategy_label(&self, sl: &LocalStrategyLabel) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO strategy_label (id, strategy_id, label_id, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![sl.id, sl.strategy_id, sl.label_id, sl.created_at],
            )
            .map_err(err_str)?;
        Ok(())
    }

    /// Detach a label from a strategy (by junction id). Returns whether a row was deleted.
    pub fn delete_strategy_label(&self, id: &str) -> Result<bool, String> {
        let n = self
            .conn
            .execute("DELETE FROM strategy_label WHERE id = ?1", params![id])
            .map_err(err_str)?;
        Ok(n > 0)
    }

    // =========================================================================
    // Strategy-trade attribution DAL (AGT-650)
    // =========================================================================

    /// Attribution rows; filtered to one strategy when `strategy_id` is given.
    /// Most recently executed first.
    pub fn list_strategy_trades(
        &self,
        strategy_id: Option<&str>,
    ) -> Result<Vec<LocalStrategyTrade>, String> {
        let filter = if strategy_id.is_some() {
            "WHERE strategy_id = ?1"
        } else {
            ""
        };
        let sql = format!(
            "SELECT {STRATEGY_TRADE_COLUMNS} FROM strategy_trade {filter} \
             ORDER BY executed_at DESC"
        );
        let mut stmt = self.conn.prepare(&sql).map_err(err_str)?;
        let rows = match strategy_id {
            Some(s) => stmt
                .query_map(params![s], strategy_trade_from_row)
                .map_err(err_str)?
                .collect::<Result<Vec<_>, _>>(),
            None => stmt
                .query_map([], strategy_trade_from_row)
                .map_err(err_str)?
                .collect::<Result<Vec<_>, _>>(),
        }
        .map_err(err_str)?;
        Ok(rows)
    }

    /// Record one strategy↔trade attribution (upsert keyed on `id`).
    pub fn insert_strategy_trade(&self, st: &LocalStrategyTrade) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO strategy_trade (
                    id, strategy_id, strategy_config_id, trade_id, instrument,
                    timeframe, direction, entry_price, match_time, executed_at,
                    rules_triggered, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(id) DO UPDATE SET
                    strategy_id = excluded.strategy_id,
                    strategy_config_id = excluded.strategy_config_id,
                    trade_id = excluded.trade_id,
                    instrument = excluded.instrument,
                    timeframe = excluded.timeframe,
                    direction = excluded.direction,
                    entry_price = excluded.entry_price,
                    match_time = excluded.match_time,
                    executed_at = excluded.executed_at,
                    rules_triggered = excluded.rules_triggered",
                params![
                    st.id,
                    st.strategy_id,
                    st.strategy_config_id,
                    st.trade_id,
                    st.instrument,
                    st.timeframe,
                    st.direction,
                    st.entry_price,
                    st.match_time,
                    st.executed_at,
                    st.rules_triggered,
                    st.created_at,
                ],
            )
            .map_err(err_str)?;
        Ok(())
    }

    // =========================================================================
    // Strategy-watcher config DAL (AGT-650)
    // =========================================================================

    /// All persisted watcher configs, most recently updated first.
    pub fn list_strategy_watchers(&self) -> Result<Vec<LocalStrategyWatcher>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {STRATEGY_WATCHER_COLUMNS} FROM strategy_watcher \
                 ORDER BY updated_at DESC"
            ))
            .map_err(err_str)?;
        let rows = stmt
            .query_map([], strategy_watcher_from_row)
            .map_err(err_str)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err_str)?;
        Ok(rows)
    }

    /// Insert or replace a watcher config (upsert keyed on `id`).
    pub fn save_strategy_watcher(&self, w: &LocalStrategyWatcher) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO strategy_watcher (
                    id, strategy_id, strategy_name, instrument, timeframe,
                    mode, signal_filter, is_active, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(id) DO UPDATE SET
                    strategy_id = excluded.strategy_id,
                    strategy_name = excluded.strategy_name,
                    instrument = excluded.instrument,
                    timeframe = excluded.timeframe,
                    mode = excluded.mode,
                    signal_filter = excluded.signal_filter,
                    is_active = excluded.is_active,
                    updated_at = excluded.updated_at",
                params![
                    w.id,
                    w.strategy_id,
                    w.strategy_name,
                    w.instrument,
                    w.timeframe,
                    w.mode,
                    w.signal_filter,
                    w.is_active,
                    w.created_at,
                    w.updated_at,
                ],
            )
            .map_err(err_str)?;
        Ok(())
    }

    /// Delete a watcher config. Returns whether a row was deleted.
    pub fn delete_strategy_watcher(&self, id: &str) -> Result<bool, String> {
        let n = self
            .conn
            .execute("DELETE FROM strategy_watcher WHERE id = ?1", params![id])
            .map_err(err_str)?;
        Ok(n > 0)
    }

    // =========================================================================
    // Credential DAL (AGT-650)
    // =========================================================================

    /// The device credential row, or `None` before onboarding. Single-user
    /// store: at most one row is expected; the most recently updated wins.
    pub fn get_credential(&self) -> Result<Option<LocalCredential>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {CREDENTIAL_COLUMNS} FROM credential \
                 ORDER BY updated_at DESC LIMIT 1"
            ))
            .map_err(err_str)?;
        let mut rows = stmt.query_map([], credential_from_row).map_err(err_str)?;
        match rows.next() {
            Some(row) => Ok(Some(row.map_err(err_str)?)),
            None => Ok(None),
        }
    }

    /// Insert or replace the credential row (upsert keyed on `id`).
    pub fn save_credential(&self, c: &LocalCredential) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO credential (
                    id, device_id, practice_blob, practice_account_id,
                    live_blob, live_account_id, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET
                    device_id = excluded.device_id,
                    practice_blob = excluded.practice_blob,
                    practice_account_id = excluded.practice_account_id,
                    live_blob = excluded.live_blob,
                    live_account_id = excluded.live_account_id,
                    updated_at = excluded.updated_at",
                params![
                    c.id,
                    c.device_id,
                    c.practice_blob,
                    c.practice_account_id,
                    c.live_blob,
                    c.live_account_id,
                    c.created_at,
                    c.updated_at,
                ],
            )
            .map_err(err_str)?;
        Ok(())
    }

    /// Delete all credential rows (the "reset credentials" flow). Returns the
    /// number of rows removed.
    pub fn delete_credentials(&self) -> Result<usize, String> {
        self.conn
            .execute("DELETE FROM credential", [])
            .map_err(err_str)
    }

}

/// Column list shared by the strategy SELECTs (order must match
/// [`strategy_from_row`]).
const STRATEGY_COLUMNS: &str = "id, name, description, schema_version, parameters, variables, \
     indicators, entry_rules, entry_logic, exit_rules, risk_settings, \
     planning_conversation, auto_note_indicators, pivot_config, \
     strategy_type, script_content, version, \
     is_active, is_promoted, is_locked, is_archived, created_at, updated_at, source";

fn strategy_from_row(row: &Row<'_>) -> rusqlite::Result<LocalStrategy> {
    Ok(LocalStrategy {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        schema_version: row.get(3)?,
        parameters: row.get(4)?,
        variables: row.get(5)?,
        indicators: row.get(6)?,
        entry_rules: row.get(7)?,
        entry_logic: row.get(8)?,
        exit_rules: row.get(9)?,
        risk_settings: row.get(10)?,
        planning_conversation: row.get(11)?,
        auto_note_indicators: row.get(12)?,
        pivot_config: row.get(13)?,
        strategy_type: row.get(14)?,
        script_content: row.get(15)?,
        version: row.get(16)?,
        is_active: row.get(17)?,
        is_promoted: row.get(18)?,
        is_locked: row.get(19)?,
        is_archived: row.get(20)?,
        created_at: row.get(21)?,
        updated_at: row.get(22)?,
        source: row.get(23)?,
    })
}

/// Column list shared by the sr_zone SELECTs (order must match
/// [`sr_zone_from_row`]).
const SR_ZONE_COLUMNS: &str =
    "id, instrument, upper_price, lower_price, label, color, created_at, updated_at";

fn sr_zone_from_row(row: &Row<'_>) -> rusqlite::Result<LocalSRZone> {
    Ok(LocalSRZone {
        id: row.get(0)?,
        instrument: row.get(1)?,
        upper_price: row.get(2)?,
        lower_price: row.get(3)?,
        label: row.get(4)?,
        color: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

/// Column list shared by the note SELECTs (order must match
/// [`note_from_row`]).
const NOTE_COLUMNS: &str = "id, trade_id, strategy_id, title, content, created_at, updated_at";

fn note_from_row(row: &Row<'_>) -> rusqlite::Result<LocalNote> {
    Ok(LocalNote {
        id: row.get(0)?,
        trade_id: row.get(1)?,
        strategy_id: row.get(2)?,
        title: row.get(3)?,
        content: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

/// Current time in epoch milliseconds (the store's timestamp unit).
/// Column list shared by the trade SELECTs (order must match
/// [`trade_from_row`]).
const TRADE_COLUMNS: &str = "id, account_id, instrument, units, open_price, close_price, \
     open_time, close_time, realized_pl, state, synced_at, created_at, updated_at";

fn trade_from_row(row: &Row<'_>) -> rusqlite::Result<LocalTrade> {
    Ok(LocalTrade {
        id: row.get(0)?,
        account_id: row.get(1)?,
        instrument: row.get(2)?,
        units: row.get(3)?,
        open_price: row.get(4)?,
        close_price: row.get(5)?,
        open_time: row.get(6)?,
        close_time: row.get(7)?,
        realized_pl: row.get(8)?,
        state: row.get(9)?,
        synced_at: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

/// Column list shared by the trade-score SELECTs (order must match
/// [`trade_score_from_row`]).
const TRADE_SCORE_COLUMNS: &str = "id, trade_id, score_entry, score_exit, \
     score_risk_management, score_overall, summary, entry_assessment, \
     exit_assessment, indicator_analysis, conflicting_indicators, \
     learning_points, created_at";

fn trade_score_from_row(row: &Row<'_>) -> rusqlite::Result<LocalTradeScore> {
    Ok(LocalTradeScore {
        id: row.get(0)?,
        trade_id: row.get(1)?,
        score_entry: row.get(2)?,
        score_exit: row.get(3)?,
        score_risk_management: row.get(4)?,
        score_overall: row.get(5)?,
        summary: row.get(6)?,
        entry_assessment: row.get(7)?,
        exit_assessment: row.get(8)?,
        indicator_analysis: row.get(9)?,
        conflicting_indicators: row.get(10)?,
        learning_points: row.get(11)?,
        created_at: row.get(12)?,
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Column list shared by the backtest SELECTs (order must match
/// [`backtest_from_row`]).
const BACKTEST_COLUMNS: &str =
    "id, strategy_id, instrument, start_date, end_date, results, created_at";

fn backtest_from_row(row: &Row<'_>) -> rusqlite::Result<LocalBacktest> {
    Ok(LocalBacktest {
        id: row.get(0)?,
        strategy_id: row.get(1)?,
        instrument: row.get(2)?,
        start_date: row.get(3)?,
        end_date: row.get(4)?,
        results: row.get(5)?,
        created_at: row.get(6)?,
    })
}

/// Column list shared by the backtest_job SELECTs (order must match
/// [`backtest_job_from_row`]).
const BACKTEST_JOB_COLUMNS: &str = "id, strategy_id, job_type, status, params, progress, \
     progress_detail, result, error_message, created_at, updated_at, completed_at";

fn backtest_job_from_row(row: &Row<'_>) -> rusqlite::Result<LocalBacktestJob> {
    Ok(LocalBacktestJob {
        id: row.get(0)?,
        strategy_id: row.get(1)?,
        job_type: row.get(2)?,
        status: row.get(3)?,
        params: row.get(4)?,
        progress: row.get(5)?,
        progress_detail: row.get(6)?,
        result: row.get(7)?,
        error_message: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        completed_at: row.get(11)?,
    })
}

/// Column list shared by the label SELECTs (order must match [`label_from_row`]).
const LABEL_COLUMNS: &str = "id, name, color, created_at";

fn label_from_row(row: &Row<'_>) -> rusqlite::Result<LocalLabel> {
    Ok(LocalLabel {
        id: row.get(0)?,
        name: row.get(1)?,
        color: row.get(2)?,
        created_at: row.get(3)?,
    })
}

/// Column list shared by the trade_label SELECTs (order must match
/// [`trade_label_from_row`]).
const TRADE_LABEL_COLUMNS: &str = "id, trade_id, label_id, created_at";

fn trade_label_from_row(row: &Row<'_>) -> rusqlite::Result<LocalTradeLabel> {
    Ok(LocalTradeLabel {
        id: row.get(0)?,
        trade_id: row.get(1)?,
        label_id: row.get(2)?,
        created_at: row.get(3)?,
    })
}

/// Column list shared by the strategy_label SELECTs (order must match
/// [`strategy_label_from_row`]).
const STRATEGY_LABEL_COLUMNS: &str = "id, strategy_id, label_id, created_at";

fn strategy_label_from_row(row: &Row<'_>) -> rusqlite::Result<LocalStrategyLabel> {
    Ok(LocalStrategyLabel {
        id: row.get(0)?,
        strategy_id: row.get(1)?,
        label_id: row.get(2)?,
        created_at: row.get(3)?,
    })
}

/// Column list shared by the strategy_trade SELECTs (order must match
/// [`strategy_trade_from_row`]).
const STRATEGY_TRADE_COLUMNS: &str = "id, strategy_id, strategy_config_id, trade_id, \
     instrument, timeframe, direction, entry_price, match_time, executed_at, \
     rules_triggered, created_at";

fn strategy_trade_from_row(row: &Row<'_>) -> rusqlite::Result<LocalStrategyTrade> {
    Ok(LocalStrategyTrade {
        id: row.get(0)?,
        strategy_id: row.get(1)?,
        strategy_config_id: row.get(2)?,
        trade_id: row.get(3)?,
        instrument: row.get(4)?,
        timeframe: row.get(5)?,
        direction: row.get(6)?,
        entry_price: row.get(7)?,
        match_time: row.get(8)?,
        executed_at: row.get(9)?,
        rules_triggered: row.get(10)?,
        created_at: row.get(11)?,
    })
}

/// Column list shared by the strategy_watcher SELECTs (order must match
/// [`strategy_watcher_from_row`]).
const STRATEGY_WATCHER_COLUMNS: &str = "id, strategy_id, strategy_name, instrument, \
     timeframe, mode, signal_filter, is_active, created_at, updated_at";

fn strategy_watcher_from_row(row: &Row<'_>) -> rusqlite::Result<LocalStrategyWatcher> {
    Ok(LocalStrategyWatcher {
        id: row.get(0)?,
        strategy_id: row.get(1)?,
        strategy_name: row.get(2)?,
        instrument: row.get(3)?,
        timeframe: row.get(4)?,
        mode: row.get(5)?,
        signal_filter: row.get(6)?,
        is_active: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

/// Column list shared by the credential SELECTs (order must match
/// [`credential_from_row`]).
const CREDENTIAL_COLUMNS: &str = "id, device_id, practice_blob, practice_account_id, \
     live_blob, live_account_id, created_at, updated_at";

fn credential_from_row(row: &Row<'_>) -> rusqlite::Result<LocalCredential> {
    Ok(LocalCredential {
        id: row.get(0)?,
        device_id: row.get(1)?,
        practice_blob: row.get(2)?,
        practice_account_id: row.get(3)?,
        live_blob: row.get(4)?,
        live_account_id: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Per-process unique suffix: parallel test threads can observe the same
    /// SystemTime on coarse clocks, which made two tests share one temp DB
    /// (and fail on the WAL lock). A counter cannot collide.
    fn unique_suffix() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        format!(
            "{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn temp_store() -> (LocalStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("wickd-local-store-test-{}", unique_suffix()));
        let path = dir.join("app.db");
        let store = LocalStore::open_at(&path).expect("open temp store");
        (store, dir)
    }

    fn sample(id: &str, name: &str, updated_at: i64) -> LocalStrategy {
        LocalStrategy {
            id: id.to_string(),
            name: name.to_string(),
            description: "test strategy".to_string(),
            schema_version: Some(2),
            parameters: None,
            variables: None,
            indicators: "[]".to_string(),
            entry_rules: "[]".to_string(),
            entry_logic: None,
            exit_rules: "[]".to_string(),
            risk_settings: "{}".to_string(),
            planning_conversation: None,
            auto_note_indicators: None,
            pivot_config: None,
            strategy_type: Some("rules".to_string()),
            script_content: None,
            version: 1,
            is_active: true,
            is_promoted: false,
            is_locked: false,
            is_archived: false,
            created_at: 1_000,
            updated_at,
            source: String::new(),
        }
    }

    #[test]
    fn open_at_creates_parent_dir_and_migrates() {
        let (store, dir) = temp_store();
        // Fresh store: schema exists, list is empty.
        assert_eq!(store.list_strategies().unwrap(), vec![]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn save_get_roundtrip_preserves_all_fields() {
        let (store, dir) = temp_store();
        let mut s = sample("s-1", "Ichimoku breakout", 2_000);
        s.parameters = Some("[{\"name\":\"period\"}]".to_string());
        s.script_content = Some("fn on_candle() {}".to_string());
        s.is_promoted = true;
        store.save_strategy(&s).unwrap();

        let got = store.get_strategy("s-1").unwrap().expect("row exists");
        assert_eq!(got, s);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn get_missing_returns_none() {
        let (store, dir) = temp_store();
        assert!(store.get_strategy("nope").unwrap().is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn save_is_upsert_and_list_orders_by_updated_at_desc() {
        let (store, dir) = temp_store();
        store.save_strategy(&sample("a", "Alpha", 1_000)).unwrap();
        store.save_strategy(&sample("b", "Beta", 2_000)).unwrap();

        // Update "a" in place — newest updated_at wins the ordering.
        let mut a2 = sample("a", "Alpha v2", 3_000);
        a2.version = 2;
        store.save_strategy(&a2).unwrap();

        let all = store.list_strategies().unwrap();
        assert_eq!(all.len(), 2, "upsert must not duplicate rows");
        assert_eq!(all[0].id, "a");
        assert_eq!(all[0].name, "Alpha v2");
        assert_eq!(all[0].version, 2);
        assert_eq!(all[1].id, "b");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn delete_removes_row_and_reports_missing() {
        let (store, dir) = temp_store();
        store.save_strategy(&sample("a", "Alpha", 1_000)).unwrap();
        assert!(store.delete_strategy("a").unwrap());
        assert!(!store.delete_strategy("a").unwrap());
        assert!(store.list_strategies().unwrap().is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    fn sample_zone(id: &str, instrument: &str, created_at: i64) -> LocalSRZone {
        LocalSRZone {
            id: id.to_string(),
            instrument: instrument.to_string(),
            upper_price: "1.08550".to_string(),
            lower_price: "1.08450".to_string(),
            label: Some("Daily pivot".to_string()),
            color: Some("rgba(234, 179, 8, 0.25)".to_string()),
            created_at,
            updated_at: created_at,
        }
    }

    fn sample_note(id: &str, trade_id: Option<&str>, strategy_id: Option<&str>, created_at: i64) -> LocalNote {
        LocalNote {
            id: id.to_string(),
            trade_id: trade_id.map(str::to_string),
            strategy_id: strategy_id.map(str::to_string),
            title: String::new(),
            content: "Clean breakout, took entry at retest.".to_string(),
            created_at,
            updated_at: created_at,
        }
    }

    #[test]
    fn sr_zone_crud_roundtrip_scoped_by_instrument() {
        let (store, dir) = temp_store();
        store.save_sr_zone(&sample_zone("z1", "EUR_USD", 1_000)).unwrap();
        store.save_sr_zone(&sample_zone("z2", "EUR_USD", 2_000)).unwrap();
        store.save_sr_zone(&sample_zone("z3", "GBP_USD", 3_000)).unwrap();

        let eur = store.list_sr_zones(Some("EUR_USD")).unwrap();
        assert_eq!(eur.len(), 2);
        assert_eq!(eur[0].id, "z1", "oldest first");
        assert_eq!(eur[0], sample_zone("z1", "EUR_USD", 1_000));

        // Upsert updates in place (resize/relabel path).
        let mut z1 = sample_zone("z1", "EUR_USD", 1_000);
        z1.upper_price = "1.09000".to_string();
        z1.label = None;
        store.save_sr_zone(&z1).unwrap();
        let eur = store.list_sr_zones(Some("EUR_USD")).unwrap();
        assert_eq!(eur.len(), 2, "upsert must not duplicate rows");
        assert_eq!(eur[0].upper_price, "1.09000");
        assert_eq!(eur[0].label, None);

        // Delete one, then clear the rest for the instrument.
        assert!(store.delete_sr_zone("z1").unwrap());
        assert!(!store.delete_sr_zone("z1").unwrap());
        assert_eq!(store.clear_sr_zones("EUR_USD").unwrap(), 1);
        assert!(store.list_sr_zones(Some("EUR_USD")).unwrap().is_empty());
        assert_eq!(store.list_sr_zones(Some("GBP_USD")).unwrap().len(), 1, "other instruments untouched");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn note_crud_roundtrip_with_filters() {
        let (store, dir) = temp_store();
        store.save_note(&sample_note("n1", Some("t1"), None, 1_000)).unwrap();
        store.save_note(&sample_note("n2", Some("t1"), None, 2_000)).unwrap();
        store.save_note(&sample_note("n3", None, Some("s1"), 3_000)).unwrap();

        let all = store.list_notes(None, None).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].id, "n3", "most recent first");

        let by_trade = store.list_notes(Some("t1"), None).unwrap();
        assert_eq!(by_trade.len(), 2);
        assert_eq!(by_trade[0].id, "n2");
        assert_eq!(by_trade[1], sample_note("n1", Some("t1"), None, 1_000));

        let by_strategy = store.list_notes(None, Some("s1")).unwrap();
        assert_eq!(by_strategy.len(), 1);
        assert_eq!(by_strategy[0].strategy_id.as_deref(), Some("s1"));

        assert!(store.delete_note("n1").unwrap());
        assert!(!store.delete_note("n1").unwrap());
        assert_eq!(store.list_notes(Some("t1"), None).unwrap().len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn chart_config_get_set_roundtrip() {
        let (store, dir) = temp_store();
        assert_eq!(store.get_chart_config("EUR_USD").unwrap(), None);

        let cfg = r#"[{"id":"sma-1","type":"sma","params":{"period":20}}]"#;
        store.set_chart_config("EUR_USD", cfg).unwrap();
        assert_eq!(store.get_chart_config("EUR_USD").unwrap().as_deref(), Some(cfg));

        // Upsert replaces.
        store.set_chart_config("EUR_USD", "[]").unwrap();
        assert_eq!(store.get_chart_config("EUR_USD").unwrap().as_deref(), Some("[]"));
        assert_eq!(store.get_chart_config("GBP_USD").unwrap(), None);
        std::fs::remove_dir_all(dir).ok();
    }

    fn sample_trade(id: &str, instrument: &str, state: &str, open_time: i64) -> LocalTrade {
        LocalTrade {
            id: id.to_string(),
            account_id: Some("101-001-1234567-001".to_string()),
            instrument: instrument.to_string(),
            units: "10000".to_string(),
            open_price: "1.08500".to_string(),
            close_price: if state == "CLOSED" { Some("1.08750".to_string()) } else { None },
            open_time,
            close_time: if state == "CLOSED" { Some(open_time + 3_600_000) } else { None },
            realized_pl: if state == "CLOSED" { Some("25.00".to_string()) } else { None },
            state: state.to_string(),
            synced_at: 5_000,
            created_at: 5_000,
            updated_at: 5_000,
        }
    }

    fn sample_score(trade_id: &str, overall: i64) -> LocalTradeScore {
        LocalTradeScore {
            id: format!("score-{trade_id}"),
            trade_id: trade_id.to_string(),
            score_entry: 7,
            score_exit: 6,
            score_risk_management: 8,
            score_overall: overall,
            summary: "Solid trade".to_string(),
            entry_assessment: "Good entry".to_string(),
            exit_assessment: "Early exit".to_string(),
            indicator_analysis: "[]".to_string(),
            conflicting_indicators: "[]".to_string(),
            learning_points: "[\"let winners run\"]".to_string(),
            created_at: 9_000,
        }
    }

    #[test]
    fn trade_upsert_roundtrip_and_open_time_ordering() {
        let (mut store, dir) = temp_store();
        let older = sample_trade("t-1", "EUR_USD", "OPEN", 1_000);
        let newer = sample_trade("t-2", "GBP_USD", "CLOSED", 2_000);
        store.upsert_trades(&[older.clone(), newer.clone()]).unwrap();

        let all = store.list_trades().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0], newer, "newest open_time first");
        assert_eq!(all[1], older);

        // Re-sync after the open trade closes: same id flips in place.
        let mut closed = sample_trade("t-1", "EUR_USD", "CLOSED", 1_000);
        closed.synced_at = 6_000;
        closed.updated_at = 6_000;
        store.upsert_trades(&[closed.clone()]).unwrap();

        let all = store.list_trades().unwrap();
        assert_eq!(all.len(), 2, "upsert must not duplicate rows");
        assert_eq!(all[1], closed);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn closed_trades_by_instrument_filters_state_and_instrument() {
        let (mut store, dir) = temp_store();
        store
            .upsert_trades(&[
                sample_trade("t-1", "EUR_USD", "CLOSED", 1_000),
                sample_trade("t-2", "EUR_USD", "OPEN", 2_000),
                sample_trade("t-3", "USD_JPY", "CLOSED", 3_000),
            ])
            .unwrap();

        let closed = store.list_closed_trades_by_instrument("EUR_USD").unwrap();
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].id, "t-1");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn trade_score_roundtrip_and_one_score_per_trade() {
        let (store, dir) = temp_store();
        assert!(store.get_trade_score_by_trade("t-1").unwrap().is_none());

        let score = sample_score("t-1", 7);
        store.save_trade_score(&score).unwrap();
        assert_eq!(store.get_trade_score_by_trade("t-1").unwrap(), Some(score));

        // A second save for the same trade replaces, never duplicates.
        let rescore = LocalTradeScore { id: "score-other".to_string(), ..sample_score("t-1", 9) };
        store.save_trade_score(&rescore).unwrap();
        let all = store.list_trade_scores().unwrap();
        assert_eq!(all.len(), 1, "trade_id is unique");
        assert_eq!(all[0].score_overall, 9);
        std::fs::remove_dir_all(dir).ok();
    }

    fn sample_backtest(id: &str, strategy_id: &str, created_at: i64) -> LocalBacktest {
        LocalBacktest {
            id: id.to_string(),
            strategy_id: strategy_id.to_string(),
            instrument: "EUR_USD".to_string(),
            start_date: 1_700_000_000_000,
            end_date: 1_707_000_000_000,
            results: "{\"metrics\":{\"totalPnl\":\"12.5\"},\"trades\":[],\"equity_curve\":[]}"
                .to_string(),
            created_at,
        }
    }

    fn sample_job(id: &str, strategy_id: &str, status: &str, updated_at: i64) -> LocalBacktestJob {
        LocalBacktestJob {
            id: id.to_string(),
            strategy_id: strategy_id.to_string(),
            job_type: "walk_forward".to_string(),
            status: status.to_string(),
            params: "{\"instrument\":\"EUR_USD\"}".to_string(),
            progress: 0,
            progress_detail: None,
            result: None,
            error_message: None,
            created_at: 1_000,
            updated_at,
            completed_at: None,
        }
    }

    #[test]
    fn backtest_save_list_delete_roundtrip() {
        let (store, dir) = temp_store();
        store.save_backtest(&sample_backtest("b1", "s1", 1_000)).unwrap();
        store.save_backtest(&sample_backtest("b2", "s1", 2_000)).unwrap();
        store.save_backtest(&sample_backtest("b3", "other", 3_000)).unwrap();

        // Scoped to the strategy, oldest first (run order).
        let runs = store.list_backtests_for_strategy("s1").unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].id, "b1");
        assert_eq!(runs[1].id, "b2");
        assert_eq!(runs[0], sample_backtest("b1", "s1", 1_000));

        // Upsert does not duplicate.
        let mut b1 = sample_backtest("b1", "s1", 1_000);
        b1.results = "{\"metrics\":{}}".to_string();
        store.save_backtest(&b1).unwrap();
        let runs = store.list_backtests_for_strategy("s1").unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].results, "{\"metrics\":{}}");

        // Delete is scoped to the strategy.
        assert_eq!(store.delete_backtests_for_strategy("s1").unwrap(), 2);
        assert!(store.list_backtests_for_strategy("s1").unwrap().is_empty());
        assert_eq!(store.list_backtests_for_strategy("other").unwrap().len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn backtest_job_save_get_list_roundtrip() {
        let (store, dir) = temp_store();
        store.save_backtest_job(&sample_job("j1", "s1", "running", 1_000)).unwrap();
        store.save_backtest_job(&sample_job("j2", "s1", "completed", 2_000)).unwrap();

        // Ordered by updated_at DESC.
        let jobs = store.list_backtest_jobs_for_strategy("s1").unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].id, "j2");

        // Upsert (status transition) round-trips optional fields.
        let mut j1 = sample_job("j1", "s1", "failed", 3_000);
        j1.error_message = Some("boom".to_string());
        j1.completed_at = Some(3_000);
        j1.progress = 40;
        store.save_backtest_job(&j1).unwrap();
        let got = store.get_backtest_job("j1").unwrap().expect("row exists");
        assert_eq!(got, j1);

        assert!(store.get_backtest_job("nope").unwrap().is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn promotion_audit_is_append_only_and_ordered() {
        let (store, dir) = temp_store();
        let promote = LocalPromotionAudit {
            id: "a1".to_string(),
            strategy_id: "s1".to_string(),
            strategy_name: "Alpha".to_string(),
            action: "promote".to_string(),
            created_at: 1_000,
        };
        let demote = LocalPromotionAudit {
            id: "a2".to_string(),
            strategy_id: "s1".to_string(),
            strategy_name: "Alpha".to_string(),
            action: "demote".to_string(),
            created_at: 2_000,
        };
        store.insert_promotion_audit(&promote).unwrap();
        store.insert_promotion_audit(&demote).unwrap();

        let rows = store.list_promotion_audits_for_strategy("s1").unwrap();
        assert_eq!(rows, vec![promote.clone(), demote]);

        // Append-only: re-inserting the same id is an error, not an upsert.
        assert!(store.insert_promotion_audit(&promote).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn v1_store_upgrades_to_latest_preserving_strategies() {
        // Simulate a foundation-only (AGT-642, user_version == 1) store: apply
        // just the first migration, write a strategy, then reopen through the
        // normal path and confirm the v4 backtest tables appear with data intact.
        let dir = std::env::temp_dir()
            .join(format!("wickd-local-store-upgrade-{}", unique_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(migrations::MIGRATIONS[0]).unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
            conn.execute(
                "INSERT INTO strategy (id, name, created_at, updated_at) \
                 VALUES ('s1', 'From v1', 1000, 1000)",
                [],
            )
            .unwrap();
        }

        let store = LocalStore::open_at(&path).expect("v1 -> latest upgrade");
        let version: i64 = store
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, migrations::MIGRATIONS.len() as i64);

        // Old data survives; new datasets are usable.
        assert_eq!(store.list_strategies().unwrap().len(), 1);
        store.save_backtest(&sample_backtest("b1", "s1", 1_000)).unwrap();
        store.save_backtest_job(&sample_job("j1", "s1", "pending", 1_000)).unwrap();
        assert_eq!(store.list_backtests_for_strategy("s1").unwrap().len(), 1);
        assert_eq!(store.list_backtest_jobs_for_strategy("s1").unwrap().len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn every_intermediate_version_upgrades_cleanly_to_latest() {
        // Simulate a store left at each historical schema version (v0 fresh,
        // v1 = AGT-642 foundation, v2 = AGT-646 charting, ...) and prove the
        // normal open path applies exactly the remaining tail every time.
        for start in 0..migrations::MIGRATIONS.len() {
            let dir = std::env::temp_dir().join(format!(
                "wickd-local-store-upgrade-v{start}-{}",
                unique_suffix()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("app.db");
            {
                let mut conn = Connection::open(&path).unwrap();
                let tx = conn.transaction().unwrap();
                for sql in &migrations::MIGRATIONS[..start] {
                    tx.execute_batch(sql).unwrap();
                }
                tx.pragma_update(None, "user_version", start as i64).unwrap();
                tx.commit().unwrap();
            }

            let mut store = LocalStore::open_at(&path).unwrap();
            let version: i64 = store
                .conn
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            assert_eq!(
                version,
                migrations::MIGRATIONS.len() as i64,
                "upgrade from v{start} must land on the latest version"
            );

            // Every dataset is usable after the upgrade, wherever it started.
            assert!(store.list_strategies().unwrap().is_empty());
            store.save_sr_zone(&sample_zone("z1", "EUR_USD", 1_000)).unwrap();
            store.save_note(&sample_note("n1", Some("t1"), None, 1_000)).unwrap();
            store.set_chart_config("EUR_USD", "[]").unwrap();
            assert_eq!(store.list_sr_zones(Some("EUR_USD")).unwrap().len(), 1);
            assert_eq!(store.list_notes(None, None).unwrap().len(), 1);
            store
                .upsert_trades(&[sample_trade("t-1", "EUR_USD", "CLOSED", 1_000)])
                .unwrap();
            store.save_trade_score(&sample_score("t-1", 7)).unwrap();
            assert_eq!(store.list_trades().unwrap().len(), 1);
            store.save_backtest(&sample_backtest("b1", "s1", 1_000)).unwrap();
            store.save_backtest_job(&sample_job("j1", "s1", "pending", 1_000)).unwrap();
            assert_eq!(store.list_backtests_for_strategy("s1").unwrap().len(), 1);
            assert_eq!(store.list_backtest_jobs_for_strategy("s1").unwrap().len(), 1);
            // v5 datasets (AGT-650).
            store.save_label(&sample_label("l1", "Breakout", 1_000)).unwrap();
            store
                .insert_trade_label(&LocalTradeLabel {
                    id: "tl1".into(),
                    trade_id: "t-1".into(),
                    label_id: "l1".into(),
                    created_at: 1_000,
                })
                .unwrap();
            store.save_strategy_watcher(&sample_watcher("w1", "s1", true, 1_000)).unwrap();
            store.insert_strategy_trade(&sample_strategy_trade("st1", "s1", "t-1", 1_000)).unwrap();
            store.save_credential(&sample_credential(1_000)).unwrap();
            assert_eq!(store.list_labels().unwrap().len(), 1);
            assert_eq!(store.list_trade_labels(Some("t-1")).unwrap().len(), 1);
            assert_eq!(store.list_strategy_watchers().unwrap().len(), 1);
            assert_eq!(store.list_strategy_trades(Some("s1")).unwrap().len(), 1);
            assert!(store.get_credential().unwrap().is_some());
            std::fs::remove_dir_all(dir).ok();
        }
    }

    #[test]
    fn reopen_persists_data_and_migration_is_idempotent() {
        let dir =
            std::env::temp_dir().join(format!("wickd-local-store-reopen-{}", unique_suffix()));
        let path = dir.join("app.db");
        {
            let store = LocalStore::open_at(&path).unwrap();
            store.save_strategy(&sample("a", "Alpha", 1_000)).unwrap();
        }
        // Second open runs migrations::apply again — must be a no-op.
        let store = LocalStore::open_at(&path).unwrap();
        assert_eq!(store.list_strategies().unwrap().len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    // =========================================================================
    // v5 datasets (AGT-650)
    // =========================================================================

    fn sample_label(id: &str, name: &str, created_at: i64) -> LocalLabel {
        LocalLabel {
            id: id.to_string(),
            name: name.to_string(),
            color: Some("#eab308".to_string()),
            created_at,
        }
    }

    fn sample_watcher(id: &str, strategy_id: &str, is_active: bool, ts: i64) -> LocalStrategyWatcher {
        LocalStrategyWatcher {
            id: id.to_string(),
            strategy_id: strategy_id.to_string(),
            strategy_name: Some("Ichimoku breakout".to_string()),
            instrument: "EUR_USD".to_string(),
            timeframe: "H1".to_string(),
            mode: "signal_only".to_string(),
            signal_filter: "all".to_string(),
            is_active,
            created_at: ts,
            updated_at: ts,
        }
    }

    fn sample_strategy_trade(id: &str, strategy_id: &str, trade_id: &str, ts: i64) -> LocalStrategyTrade {
        LocalStrategyTrade {
            id: id.to_string(),
            strategy_id: strategy_id.to_string(),
            strategy_config_id: Some(format!("{strategy_id}-EUR_USD-H1")),
            trade_id: trade_id.to_string(),
            instrument: "EUR_USD".to_string(),
            timeframe: "H1".to_string(),
            direction: "long".to_string(),
            entry_price: "1.08512".to_string(),
            match_time: ts,
            executed_at: ts + 500,
            rules_triggered: Some("[\"rule-1\"]".to_string()),
            created_at: ts,
        }
    }

    fn sample_credential(ts: i64) -> LocalCredential {
        LocalCredential {
            id: "device-1".to_string(),
            device_id: "device-1".to_string(),
            practice_blob: Some("ciphertext-practice".to_string()),
            practice_account_id: Some("101-001-1234567-001".to_string()),
            live_blob: None,
            live_account_id: None,
            created_at: ts,
            updated_at: ts,
        }
    }

    #[test]
    fn label_crud_and_junctions_roundtrip() {
        let (store, dir) = temp_store();
        store.save_label(&sample_label("l1", "Breakout", 1_000)).unwrap();
        store.save_label(&sample_label("l2", "Asia session", 2_000)).unwrap();

        // Alphabetical list; upsert edits in place.
        let labels = store.list_labels().unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].name, "Asia session");
        let mut renamed = sample_label("l1", "Breakout retest", 1_000);
        renamed.color = None;
        store.save_label(&renamed).unwrap();
        let labels = store.list_labels().unwrap();
        assert_eq!(labels.len(), 2, "label save must upsert");
        assert!(labels.iter().any(|l| l.name == "Breakout retest" && l.color.is_none()));

        // Trade junction: attach, scope by trade, detach.
        let tl = LocalTradeLabel {
            id: "tl1".into(),
            trade_id: "t-9".into(),
            label_id: "l1".into(),
            created_at: 1_000,
        };
        store.insert_trade_label(&tl).unwrap();
        assert_eq!(store.list_trade_labels(Some("t-9")).unwrap(), vec![tl.clone()]);
        assert!(store.list_trade_labels(Some("t-other")).unwrap().is_empty());
        assert_eq!(store.list_trade_labels(None).unwrap().len(), 1);
        assert!(store.delete_trade_label("tl1").unwrap());
        assert!(!store.delete_trade_label("tl1").unwrap());

        // Strategy junction mirrors the same shape.
        let sl = LocalStrategyLabel {
            id: "sl1".into(),
            strategy_id: "s-1".into(),
            label_id: "l2".into(),
            created_at: 1_000,
        };
        store.insert_strategy_label(&sl).unwrap();
        assert_eq!(store.list_strategy_labels(Some("s-1")).unwrap(), vec![sl]);
        assert!(store.delete_strategy_label("sl1").unwrap());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn strategy_trade_rows_scope_and_order_by_executed_at_desc() {
        let (store, dir) = temp_store();
        store.insert_strategy_trade(&sample_strategy_trade("st1", "s1", "t-1", 1_000)).unwrap();
        store.insert_strategy_trade(&sample_strategy_trade("st2", "s1", "t-2", 5_000)).unwrap();
        store.insert_strategy_trade(&sample_strategy_trade("st3", "s2", "t-3", 3_000)).unwrap();

        let s1 = store.list_strategy_trades(Some("s1")).unwrap();
        assert_eq!(s1.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["st2", "st1"]);
        assert_eq!(store.list_strategy_trades(None).unwrap().len(), 3);

        // Upsert keyed on id must not duplicate.
        store.insert_strategy_trade(&sample_strategy_trade("st1", "s1", "t-1", 9_000)).unwrap();
        assert_eq!(store.list_strategy_trades(Some("s1")).unwrap().len(), 2);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn strategy_watcher_upsert_toggle_and_delete() {
        let (store, dir) = temp_store();
        store.save_strategy_watcher(&sample_watcher("w1", "s1", true, 1_000)).unwrap();

        // Toggle is_active via full-row upsert (the frontend read-modify-write path).
        let mut w = sample_watcher("w1", "s1", false, 1_000);
        w.updated_at = 2_000;
        store.save_strategy_watcher(&w).unwrap();
        let all = store.list_strategy_watchers().unwrap();
        assert_eq!(all.len(), 1, "watcher save must upsert");
        assert!(!all[0].is_active);
        assert_eq!(all[0].updated_at, 2_000);

        assert!(store.delete_strategy_watcher("w1").unwrap());
        assert!(!store.delete_strategy_watcher("w1").unwrap());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn credential_roundtrip_update_and_reset() {
        let (store, dir) = temp_store();
        assert!(store.get_credential().unwrap().is_none(), "fresh store has no credential");

        let c = sample_credential(1_000);
        store.save_credential(&c).unwrap();
        assert_eq!(store.get_credential().unwrap(), Some(c.clone()));

        // Adding live credentials updates in place.
        let mut c2 = c.clone();
        c2.live_blob = Some("ciphertext-live".to_string());
        c2.live_account_id = Some("001-001-7654321-001".to_string());
        c2.updated_at = 2_000;
        store.save_credential(&c2).unwrap();
        let got = store.get_credential().unwrap().unwrap();
        assert_eq!(got.live_account_id.as_deref(), Some("001-001-7654321-001"));
        assert_eq!(got.practice_blob.as_deref(), Some("ciphertext-practice"));

        // Reset clears everything.
        assert_eq!(store.delete_credentials().unwrap(), 1);
        assert!(store.get_credential().unwrap().is_none());
        std::fs::remove_dir_all(dir).ok();
    }
}
