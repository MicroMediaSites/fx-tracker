//! Local-store row types.
//!
//! Plain structs mapped from `rusqlite` rows (the local store is
//! `~/.wickd/app.db`; see `store.rs`). Each has a `from_row` used with
//! `query_map`. Column order in `from_row` matches the SELECT in main.rs.

use rusqlite::Row;

/// Strategy listing row (minimal fields)
pub struct StrategyRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub is_active: bool,
}

impl StrategyRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            is_active: row.get(3)?,
        })
    }
}

/// Full strategy row with all fields the MCP tools serve.
/// Many columns are nullable for older strategies created before all fields existed.
pub struct FullStrategyRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub schema_version: Option<i64>,
    pub parameters: Option<String>,
    pub variables: Option<String>,
    pub indicators: Option<String>,
    pub entry_rules: Option<String>,
    pub exit_rules: Option<String>,
    pub risk_settings: Option<String>,
    pub is_active: bool,
    pub version: i64,
}

impl FullStrategyRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            schema_version: row.get(3)?,
            parameters: row.get(4)?,
            variables: row.get(5)?,
            indicators: row.get(6)?,
            entry_rules: row.get(7)?,
            exit_rules: row.get(8)?,
            risk_settings: row.get(9)?,
            is_active: row.get(10)?,
            version: row.get(11)?,
        })
    }
}

/// Full trade row with all fields
pub struct FullTradeRow {
    pub id: String,
    pub instrument: String,
    pub units: String,
    pub open_price: String,
    pub close_price: Option<String>,
    pub open_time: i64,
    pub close_time: Option<i64>,
    pub realized_pl: Option<String>,
    pub state: String,
    pub account_id: Option<String>,
}

impl FullTradeRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            instrument: row.get(1)?,
            units: row.get(2)?,
            open_price: row.get(3)?,
            close_price: row.get(4)?,
            open_time: row.get(5)?,
            close_time: row.get(6)?,
            realized_pl: row.get(7)?,
            state: row.get(8)?,
            account_id: row.get(9)?,
        })
    }
}

/// Note row
pub struct NoteRow {
    pub id: String,
    pub trade_id: Option<String>,
    pub strategy_id: Option<String>,
    pub title: String,
    pub content: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl NoteRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            trade_id: row.get(1)?,
            strategy_id: row.get(2)?,
            title: row.get(3)?,
            content: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    }
}

/// S/R Zone row
pub struct ZoneRow {
    pub id: String,
    pub instrument: String,
    pub upper_price: String,
    pub lower_price: String,
    pub label: Option<String>,
    pub color: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ZoneRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
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
}

/// Backtest job row (from backtest_job table)
pub struct BacktestJobRow {
    pub id: String,
    pub job_type: String,
    pub params: String,
    pub result: Option<String>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

impl BacktestJobRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            job_type: row.get(1)?,
            params: row.get(2)?,
            result: row.get(3)?,
            created_at: row.get(4)?,
            completed_at: row.get(5)?,
        })
    }
}
