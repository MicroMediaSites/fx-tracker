//! Append-only audit log of every execution decision.
//!
//! Every execution decision the CLI makes — paper *or* live, place *or* close —
//! is recorded as one immutable row in a local SQLite store at
//! `~/.wickd/audit.db`. The log captures the timestamp, the signal/inputs
//! (instrument, units, sl/tp), the mode (`paper`/`live`), the target
//! environment, the action, and the outcome (would-be/not-submitted for paper;
//! filled/rejected + detail for live).
//!
//! ## Append-only invariant (load-bearing)
//!
//! This module exposes **only** insert ([`record`]) and select ([`query`])
//! paths. There is deliberately **no UPDATE or DELETE** statement anywhere in
//! this codebase — rows, once written, are never mutated or removed in code.
//! The table is the immutable ledger of what the trader decided and did. Any
//! future read path must stay select-only; do not add an update/delete here.
//!
//! ## Schema
//!
//! ```sql
//! CREATE TABLE IF NOT EXISTS audit_log(
//!   id          INTEGER PRIMARY KEY AUTOINCREMENT,
//!   ts          TEXT NOT NULL,   -- RFC3339 timestamp
//!   instrument  TEXT,            -- e.g. EUR_USD
//!   units       INTEGER,         -- signed; negative = short
//!   sl          TEXT,            -- stop-loss price (OANDA precision)
//!   tp          TEXT,            -- take-profit price
//!   mode        TEXT NOT NULL,   -- 'paper' | 'live'
//!   environment TEXT,            -- 'practice' | 'live'
//!   action      TEXT NOT NULL,   -- 'place' | 'close' | 'adopt' (AGT-628 startup reconcile)
//!   outcome     TEXT NOT NULL,   -- 'not_submitted' | 'attempt' | 'filled' | 'partial' |
//!                                --   'rejected' | 'resting' | 'no_fill' | 'error' |
//!                                --   'adopted' (AGT-628: an existing open position adopted at startup)
//!   detail      TEXT,            -- free-form: fill id, cancel reason, realized pl
//!   strategy    TEXT             -- strategy attributed to the decision (AGT-630)
//! );
//! ```
//!
//! The `strategy` column was added by AGT-630. [`init_schema`] migrates an
//! existing store in place with an idempotent `ALTER TABLE … ADD COLUMN` —
//! a schema addition, not a row mutation, so the append-only invariant holds:
//! pre-existing rows are untouched and simply read back with a NULL strategy.
//!
//! Query recent rows directly with `wickd audit --limit N`, or over the raw
//! store with `sqlite3 ~/.wickd/audit.db 'SELECT * FROM audit_log ORDER BY id DESC'`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use serde_json::{json, Value};

/// One execution-decision row. Plain data: trade.rs fills this in and hands it
/// to [`record`] / [`record_decision`]; all persistence logic lives here.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub ts: String,
    pub instrument: Option<String>,
    pub units: Option<i64>,
    pub sl: Option<String>,
    pub tp: Option<String>,
    pub mode: String,
    pub environment: Option<String>,
    pub action: String,
    pub outcome: String,
    pub detail: Option<String>,
    /// Strategy the decision is attributed to (AGT-630, AC2): the pending
    /// signal's strategy on the `approve` path, `--strategy` on a manual
    /// `trade place`, `None` when no strategy is known.
    pub strategy: Option<String>,
}

impl AuditEntry {
    /// A new entry stamped with the current time. Builder methods fill the rest.
    pub fn now(action: &str, mode: &str, outcome: &str) -> Self {
        Self {
            ts: Utc::now().to_rfc3339(),
            instrument: None,
            units: None,
            sl: None,
            tp: None,
            mode: mode.to_string(),
            environment: None,
            action: action.to_string(),
            outcome: outcome.to_string(),
            detail: None,
            strategy: None,
        }
    }

    pub fn env(mut self, environment: &str) -> Self {
        self.environment = Some(environment.to_string());
        self
    }

    pub fn instrument(mut self, instrument: &str) -> Self {
        self.instrument = Some(instrument.to_string());
        self
    }

    pub fn units(mut self, units: i64) -> Self {
        self.units = Some(units);
        self
    }

    pub fn sl(mut self, sl: Option<String>) -> Self {
        self.sl = sl;
        self
    }

    pub fn tp(mut self, tp: Option<String>) -> Self {
        self.tp = tp;
        self
    }

    pub fn detail(mut self, detail: Option<String>) -> Self {
        self.detail = detail;
        self
    }

    pub fn strategy(mut self, strategy: Option<String>) -> Self {
        self.strategy = strategy;
        self
    }
}

/// Path to the audit DB (`~/.wickd/audit.db`).
pub fn audit_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not resolve home directory")?;
    Ok(home.join(".wickd").join("audit.db"))
}

/// Open (creating on first use) the default audit store at `~/.wickd/audit.db`.
pub fn open() -> Result<Connection> {
    open_at(audit_path()?)
}

/// Open (creating on first use) an audit store at an explicit path. Tests pass
/// a temp path here so they never touch the real `~/.wickd/audit.db`.
pub fn open_at(path: impl AsRef<Path>) -> Result<Connection> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create audit dir {}", parent.display()))?;
    }
    let conn = Connection::open(path)
        .with_context(|| format!("could not open audit db {}", path.display()))?;
    // Local trading data must not be world-readable (AGT-668).
    crate::fs_perms::restrict_file(path)?;
    init_schema(&conn)?;
    Ok(conn)
}

/// Create the append-only table if it does not exist, then bring an existing
/// store up to the current schema. Idempotent.
fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS audit_log(
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            ts          TEXT NOT NULL,
            instrument  TEXT,
            units       INTEGER,
            sl          TEXT,
            tp          TEXT,
            mode        TEXT NOT NULL,
            environment TEXT,
            action      TEXT NOT NULL,
            outcome     TEXT NOT NULL,
            detail      TEXT,
            strategy    TEXT
        )",
        [],
    )
    .context("could not initialize audit_log schema")?;

    // AGT-630 (AC2) migration: a store created before the strategy column
    // existed gains it in place. ADD COLUMN is a pure schema addition — no row
    // is updated or deleted, so the append-only invariant is preserved and
    // every pre-existing row reads back with a NULL strategy.
    if !has_column(conn, "audit_log", "strategy")? {
        conn.execute("ALTER TABLE audit_log ADD COLUMN strategy TEXT", [])
            .context("could not add strategy column to audit_log")?;
    }
    Ok(())
}

/// Whether `table` already has a column named `column` (via `PRAGMA
/// table_info`). Backs the idempotent column migration in [`init_schema`].
fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .context("could not inspect audit_log schema")?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .context("could not read audit_log schema")?;
    for name in names {
        if name.context("could not read audit_log column name")? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Append one row. The ONLY write path in this module — insert only, no update
/// or delete. Returns the new row id.
pub fn record(conn: &Connection, entry: &AuditEntry) -> Result<i64> {
    conn.execute(
        "INSERT INTO audit_log
            (ts, instrument, units, sl, tp, mode, environment, action, outcome, detail, strategy)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            entry.ts,
            entry.instrument,
            entry.units,
            entry.sl,
            entry.tp,
            entry.mode,
            entry.environment,
            entry.action,
            entry.outcome,
            entry.detail,
            entry.strategy,
        ],
    )
    .context("could not write audit row")?;
    Ok(conn.last_insert_rowid())
}

/// Fire-and-forget write to the store at `path`. See [`record_decision`] (the
/// default-path wrapper production callers use). Split out — mirroring the
/// `_at` convention in [`crate::pending`] — so tests can exercise the exact
/// write path against a throwaway store instead of `~/.wickd/audit.db`.
pub fn record_decision_at(path: impl AsRef<Path>, entry: AuditEntry) {
    match open_at(path).and_then(|conn| record(&conn, &entry)) {
        Ok(_) => {}
        Err(e) => eprintln!("warning: audit log write failed: {e:#}"),
    }
}

/// Fire-and-forget write to the default store. A failed audit write must never
/// crash a trade — surface it as a warning on stderr and move on. Use this for
/// paper decisions and for recording the *outcome* of a live order once it has
/// already been logged as an attempt (see [`record_required`]).
pub fn record_decision(entry: AuditEntry) {
    match audit_path() {
        Ok(path) => record_decision_at(path, entry),
        Err(e) => eprintln!("warning: audit log write failed: {e:#}"),
    }
}

/// Write to the store at `path`, propagating any error. See [`record_required`]
/// (the default-path wrapper production callers use) for the fatal-write
/// invariant this backs. Split out — mirroring the `_at` convention in
/// [`crate::pending`] — so tests can exercise the exact write path against a
/// throwaway store instead of `~/.wickd/audit.db`.
pub fn record_required_at(path: impl AsRef<Path>, entry: &AuditEntry) -> Result<()> {
    let conn = open_at(path)?;
    record(&conn, entry)?;
    Ok(())
}

/// Write to the default store, propagating any error. Used on the **live** path
/// *before* an order is submitted to OANDA: a failed audit write here aborts the
/// trade, so no live order can ever reach the broker without a ledger row
/// already on disk. (Paper decisions and post-submission outcome rows stay
/// fire-and-forget via [`record_decision`] — only the pre-submission live
/// attempt is fatal.)
pub fn record_required(entry: &AuditEntry) -> Result<()> {
    record_required_at(audit_path()?, entry)
}

/// Read back the most recent rows (newest first) as JSON objects. Select-only.
pub fn query(conn: &Connection, limit: usize) -> Result<Vec<Value>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, ts, instrument, units, sl, tp, mode, environment, action, outcome, detail,
                    strategy
             FROM audit_log
             ORDER BY id DESC
             LIMIT ?1",
        )
        .context("could not prepare audit query")?;
    let rows = stmt
        .query_map([limit as i64], |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "ts": row.get::<_, String>(1)?,
                "instrument": row.get::<_, Option<String>>(2)?,
                "units": row.get::<_, Option<i64>>(3)?,
                "sl": row.get::<_, Option<String>>(4)?,
                "tp": row.get::<_, Option<String>>(5)?,
                "mode": row.get::<_, String>(6)?,
                "environment": row.get::<_, Option<String>>(7)?,
                "action": row.get::<_, String>(8)?,
                "outcome": row.get::<_, String>(9)?,
                "detail": row.get::<_, Option<String>>(10)?,
                "strategy": row.get::<_, Option<String>>(11)?,
            }))
        })
        .context("could not run audit query")?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.context("could not read audit row")?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let mut p = std::env::temp_dir();
        p.push(format!("wickd-audit-test-{pid}-{nanos}-{n}.db"));
        p
    }

    /// AGT-668: a freshly created audit DB is owner-only (`0600`), never
    /// world-readable.
    #[cfg(unix)]
    #[test]
    fn new_db_is_created_owner_only_0600() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_db();
        let _conn = open_at(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "audit db must be created 0600, got {mode:o}");
        let _ = std::fs::remove_file(&path);
    }

    /// AC4: a live-order attempt produces an audit row.
    #[test]
    fn live_order_attempt_produces_a_row() {
        let path = temp_db();
        let conn = open_at(&path).unwrap();

        let entry = AuditEntry::now("place", "live", "filled")
            .env("practice")
            .instrument("EUR_USD")
            .units(1000)
            .sl(Some("1.0850".into()))
            .tp(Some("1.0950".into()))
            .detail(Some("fill_id=123".into()));
        record(&conn, &entry).unwrap();

        let rows = query(&conn, 10).unwrap();
        assert_eq!(rows.len(), 1, "exactly one audit row expected");
        let r = &rows[0];
        assert_eq!(r["mode"], "live");
        assert_eq!(r["action"], "place");
        assert_eq!(r["outcome"], "filled");
        assert_eq!(r["instrument"], "EUR_USD");
        assert_eq!(r["units"], 1000);
        assert_eq!(r["environment"], "practice");

        let _ = std::fs::remove_file(&path);
    }

    /// AC1: paper decisions are recorded too.
    #[test]
    fn paper_decision_is_recorded() {
        let path = temp_db();
        let conn = open_at(&path).unwrap();

        record(
            &conn,
            &AuditEntry::now("place", "paper", "not_submitted")
                .env("practice")
                .instrument("GBP_USD")
                .units(-500),
        )
        .unwrap();

        let rows = query(&conn, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["mode"], "paper");
        assert_eq!(rows[0]["outcome"], "not_submitted");
        assert_eq!(rows[0]["units"], -500);

        let _ = std::fs::remove_file(&path);
    }

    /// AGT-630 (AC2): the strategy column round-trips — written when known,
    /// NULL when not.
    #[test]
    fn strategy_column_round_trips() {
        let path = temp_db();
        let conn = open_at(&path).unwrap();

        record(
            &conn,
            &AuditEntry::now("place", "paper", "not_submitted")
                .instrument("EUR_USD")
                .strategy(Some("ma-crossover".into())),
        )
        .unwrap();
        record(
            &conn,
            &AuditEntry::now("place", "paper", "not_submitted").instrument("GBP_USD"),
        )
        .unwrap();

        let rows = query(&conn, 10).unwrap();
        assert_eq!(rows.len(), 2);
        // Newest first: the unattributed row reads back with a null strategy.
        assert_eq!(rows[0]["strategy"], serde_json::Value::Null);
        assert_eq!(rows[1]["strategy"], "ma-crossover");

        let _ = std::fs::remove_file(&path);
    }

    /// AGT-630 (AC2): a store created BEFORE the strategy column existed is
    /// migrated in place on open — existing rows stay readable (strategy NULL)
    /// and new attributed rows land alongside them.
    #[test]
    fn legacy_store_without_strategy_column_is_migrated_and_stays_readable() {
        let path = temp_db();

        // Hand-build the pre-AGT-630 schema and seed one legacy row, exactly
        // as an existing ~/.wickd/audit.db would look.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute(
                "CREATE TABLE audit_log(
                    id          INTEGER PRIMARY KEY AUTOINCREMENT,
                    ts          TEXT NOT NULL,
                    instrument  TEXT,
                    units       INTEGER,
                    sl          TEXT,
                    tp          TEXT,
                    mode        TEXT NOT NULL,
                    environment TEXT,
                    action      TEXT NOT NULL,
                    outcome     TEXT NOT NULL,
                    detail      TEXT
                )",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO audit_log (ts, instrument, units, mode, action, outcome)
                 VALUES ('2026-07-01T00:00:00Z', 'EUR_USD', 1000, 'paper', 'place', 'not_submitted')",
                [],
            )
            .unwrap();
        }

        // Opening through the normal path migrates the schema…
        let conn = open_at(&path).unwrap();
        // …the legacy row is still there, reading back a null strategy…
        let rows = query(&conn, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["instrument"], "EUR_USD");
        assert_eq!(rows[0]["strategy"], serde_json::Value::Null);

        // …and a new attributed row appends cleanly next to it.
        record(
            &conn,
            &AuditEntry::now("place", "live", "filled")
                .instrument("EUR_USD")
                .strategy(Some("rsi-reversion".into())),
        )
        .unwrap();
        let rows = query(&conn, 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["strategy"], "rsi-reversion");
        assert_eq!(rows[1]["strategy"], serde_json::Value::Null);

        // Re-opening is idempotent (no duplicate-column error).
        drop(conn);
        let conn = open_at(&path).unwrap();
        assert_eq!(query(&conn, 10).unwrap().len(), 2);

        let _ = std::fs::remove_file(&path);
    }

    /// Append-only behavior: successive records accumulate, newest-first.
    #[test]
    fn records_are_append_only_and_ordered() {
        let path = temp_db();
        let conn = open_at(&path).unwrap();

        record(&conn, &AuditEntry::now("place", "paper", "not_submitted").instrument("A")).unwrap();
        record(&conn, &AuditEntry::now("close", "live", "filled").instrument("B")).unwrap();

        let rows = query(&conn, 10).unwrap();
        assert_eq!(rows.len(), 2);
        // Newest first.
        assert_eq!(rows[0]["instrument"], "B");
        assert_eq!(rows[1]["instrument"], "A");
        // Monotonic ids — nothing was overwritten.
        assert!(rows[0]["id"].as_i64().unwrap() > rows[1]["id"].as_i64().unwrap());

        let _ = std::fs::remove_file(&path);
    }
}
