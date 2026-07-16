//! Read-only access to the CLI's spread-history store (`~/.wickd/spreads.db`).
//!
//! The wickd CLI owns this database: `wickd stream` (the always-on hub) and
//! `wickd view ticket` sample every quote's spread into per-instrument
//! `min/max/ema` statistics (see `crates/wickd/src/spread_stats.rs` for the
//! algorithm and schema — that crate is bin-only, so the desktop app reads
//! the store directly rather than linking the module). The frontend uses the
//! stats to grade the live spread bar: green = historically low, yellow =
//! average, red = high, purple = no history yet.
//!
//! The app is strictly a READER. It opens the file read-only (never creating
//! it) so it cannot interfere with the CLI writers; a missing database or any
//! read failure degrades to "no stats", which the UI renders as the purple
//! fallback. Decimals stay TEXT end-to-end (house rule: no float
//! representations of price-derived values at rest) — the frontend parses
//! them only for display math.

use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use wickd_core::paths::wickd_data_home;

/// One instrument's persisted spread statistics, as stored by the CLI.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SpreadStatsRow {
    pub instrument: String,
    pub sample_count: i64,
    /// Historical minimum spread (decayed toward the EMA), decimal string.
    pub min_spread: String,
    /// Historical maximum spread (decayed toward the EMA), decimal string.
    pub max_spread: String,
    /// Slow EMA of sampled spreads, decimal string.
    pub ema_spread: String,
}

/// Read every instrument's stats. A missing database (the CLI has never
/// sampled on this machine) is not an error — it returns an empty list.
pub fn list_all() -> Result<Vec<SpreadStatsRow>, String> {
    let path = wickd_data_home()?.join("spreads.db");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("could not open spreads db {}: {e}", path.display()))?;
    // The CLI writers hold short write transactions; wait briefly instead of
    // surfacing a transient SQLITE_BUSY to the UI.
    conn.busy_timeout(Duration::from_millis(2000))
        .map_err(|e| format!("could not set spreads db busy timeout: {e}"))?;
    list_all_from(&conn)
}

/// Read every row from an already-open connection (split out for tests).
fn list_all_from(conn: &Connection) -> Result<Vec<SpreadStatsRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT instrument, sample_count, min_spread, max_spread, ema_spread
             FROM spread_stats ORDER BY instrument",
        )
        .map_err(|e| format!("could not read spread stats: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SpreadStatsRow {
                instrument: row.get(0)?,
                sample_count: row.get(1)?,
                min_spread: row.get(2)?,
                max_spread: row.get(3)?,
                ema_spread: row.get(4)?,
            })
        })
        .map_err(|e| format!("could not read spread stats: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("could not read spread stats: {e}"))?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an in-memory DB with the CLI's `spread_stats` schema (mirrors
    /// `crates/wickd/src/spread_stats.rs::init_schema`).
    fn cli_schema_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE spread_stats(
                instrument   TEXT PRIMARY KEY,
                sample_count INTEGER NOT NULL,
                min_spread   TEXT NOT NULL,
                max_spread   TEXT NOT NULL,
                ema_spread   TEXT NOT NULL,
                last_updated TEXT NOT NULL,
                created_at   TEXT NOT NULL
            )",
        )
        .unwrap();
        conn
    }

    #[test]
    fn reads_rows_in_instrument_order() {
        let conn = cli_schema_db();
        for (inst, n, min, max, ema) in [
            ("USD_JPY", 379, "0.012", "0.0209", "0.0188"),
            ("EUR_USD", 17871, "0.00014", "0.00026", "0.000158"),
        ] {
            conn.execute(
                "INSERT INTO spread_stats VALUES (?1, ?2, ?3, ?4, ?5, '2026-01-01', '2026-01-01')",
                rusqlite::params![inst, n, min, max, ema],
            )
            .unwrap();
        }
        let rows = list_all_from(&conn).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].instrument, "EUR_USD");
        assert_eq!(rows[0].sample_count, 17871);
        assert_eq!(rows[0].min_spread, "0.00014");
        assert_eq!(rows[1].instrument, "USD_JPY");
        assert_eq!(rows[1].max_spread, "0.0209");
    }

    #[test]
    fn empty_table_reads_empty() {
        let conn = cli_schema_db();
        assert!(list_all_from(&conn).unwrap().is_empty());
    }

    #[test]
    fn serializes_decimals_as_strings() {
        let row = SpreadStatsRow {
            instrument: "EUR_USD".into(),
            sample_count: 3,
            min_spread: "0.00011".into(),
            max_spread: "0.00035".into(),
            ema_spread: "0.00016".into(),
        };
        let json = serde_json::to_value(&row).unwrap();
        assert_eq!(json["min_spread"], "0.00011");
        assert_eq!(json["sample_count"], 3);
    }
}
