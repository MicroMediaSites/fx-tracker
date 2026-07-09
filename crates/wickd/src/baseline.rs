//! Per-account performance baselines (AGT-631).
//!
//! A forward paper evaluation (wickd-lab PROTOCOL.md) adjudicates each named
//! practice account against the balance it started with — its **baseline**. So
//! `wickd trade report` needs a durable record of "account X started at $10,000
//! on date D", independent of OANDA (which does not store an arbitrary
//! start-of-evaluation marker). This module is that record.
//!
//! Baselines live in a local SQLite store at `~/.wickd/baselines.db`, mirroring
//! the home-dir + SQLite pattern of [`crate::audit`].
//!
//! ## Append-only, latest-wins (AC1)
//!
//! Recording a new baseline **supersedes** the old one but **keeps the prior in
//! history**: like [`crate::audit`], this module exposes only insert
//! ([`record`]) and select ([`latest`] / [`history`]) — there is deliberately
//! **no UPDATE or DELETE**. "Supersede" is expressed by insert order:
//! [`latest`] returns the highest-`id` row for an account; every prior baseline
//! stays readable via [`history`]. A re-baseline is therefore a new row, never a
//! mutation of the old one.
//!
//! ## Schema
//!
//! ```sql
//! CREATE TABLE IF NOT EXISTS baselines(
//!   id            INTEGER PRIMARY KEY AUTOINCREMENT,
//!   account       TEXT NOT NULL,   -- named account, e.g. h004 / default
//!   environment   TEXT,            -- 'practice' | 'live'
//!   balance       TEXT NOT NULL,   -- starting balance (OANDA-precision string)
//!   currency      TEXT,            -- account currency, e.g. USD
//!   baseline_date TEXT NOT NULL,   -- RFC3339 instant the balance is as-of
//!   recorded_at   TEXT NOT NULL    -- RFC3339 instant the row was written
//! );
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};

/// One recorded baseline row. Plain data: `trade baseline set` fills this in and
/// hands it to [`record`]; the report reads it back via [`latest`].
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Baseline {
    /// Row id (monotonic; higher = more recent). 0 before insert.
    pub id: i64,
    /// Named account the baseline belongs to (e.g. `h004`, `default`).
    pub account: String,
    /// Environment the account lives in (`practice` | `live`).
    pub environment: Option<String>,
    /// Starting balance, kept as the exact OANDA-precision string.
    pub balance: String,
    /// Account currency (e.g. `USD`), when known.
    pub currency: Option<String>,
    /// RFC3339 instant the balance is as-of (defaults to the record time).
    pub baseline_date: String,
    /// RFC3339 instant this row was written.
    pub recorded_at: String,
}

impl Baseline {
    /// Build a to-be-recorded baseline stamped `recorded_at = now`. `id` is 0
    /// until [`record`] assigns the real row id.
    pub fn new(
        account: &str,
        environment: Option<String>,
        balance: &str,
        currency: Option<String>,
        baseline_date: String,
    ) -> Self {
        Self {
            id: 0,
            account: account.to_string(),
            environment,
            balance: balance.to_string(),
            currency,
            baseline_date,
            recorded_at: Utc::now().to_rfc3339(),
        }
    }

    /// JSON view (used directly in the `baseline set|show` command output).
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "account": self.account,
            "environment": self.environment,
            "balance": self.balance,
            "currency": self.currency,
            "baseline_date": self.baseline_date,
            "recorded_at": self.recorded_at,
        })
    }
}

/// Path to the baseline DB (`~/.wickd/baselines.db`).
pub fn baseline_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not resolve home directory")?;
    Ok(home.join(".wickd").join("baselines.db"))
}

/// Open (creating on first use) the default baseline store.
pub fn open() -> Result<Connection> {
    open_at(baseline_path()?)
}

/// Open (creating on first use) a baseline store at an explicit path. Tests pass
/// a temp path so they never touch the real `~/.wickd/baselines.db`.
pub fn open_at(path: impl AsRef<Path>) -> Result<Connection> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create baseline dir {}", parent.display()))?;
    }
    let conn = Connection::open(path)
        .with_context(|| format!("could not open baseline db {}", path.display()))?;
    // Local trading data must not be world-readable (AGT-668).
    crate::fs_perms::restrict_file(path)?;
    init_schema(&conn)?;
    Ok(conn)
}

/// Create the append-only table if it does not exist. Idempotent.
fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS baselines(
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            account       TEXT NOT NULL,
            environment   TEXT,
            balance       TEXT NOT NULL,
            currency      TEXT,
            baseline_date TEXT NOT NULL,
            recorded_at   TEXT NOT NULL
        )",
        [],
    )
    .context("could not initialize baselines schema")?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_baselines_account ON baselines(account)",
        [],
    )
    .context("could not create baselines account index")?;
    Ok(())
}

/// Append one baseline row. The ONLY write path — insert only, no update or
/// delete (AC1). Returns the new row id.
pub fn record(conn: &Connection, b: &Baseline) -> Result<i64> {
    conn.execute(
        "INSERT INTO baselines
            (account, environment, balance, currency, baseline_date, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            b.account,
            b.environment,
            b.balance,
            b.currency,
            b.baseline_date,
            b.recorded_at,
        ],
    )
    .context("could not write baseline row")?;
    Ok(conn.last_insert_rowid())
}

/// Record against the store at `path`, returning the stored [`Baseline`] with
/// its assigned id. Split out (mirroring the `_at` convention in
/// [`crate::audit`]) so tests exercise the exact write path against a throwaway
/// store.
pub fn record_at(path: impl AsRef<Path>, mut b: Baseline) -> Result<Baseline> {
    let conn = open_at(path)?;
    b.id = record(&conn, &b)?;
    Ok(b)
}

fn row_to_baseline(row: &rusqlite::Row) -> rusqlite::Result<Baseline> {
    Ok(Baseline {
        id: row.get(0)?,
        account: row.get(1)?,
        environment: row.get(2)?,
        balance: row.get(3)?,
        currency: row.get(4)?,
        baseline_date: row.get(5)?,
        recorded_at: row.get(6)?,
    })
}

const SELECT_COLS: &str =
    "id, account, environment, balance, currency, baseline_date, recorded_at";

/// The current (most recently recorded) baseline for `account`, or `None` if the
/// account has never been baselined. "Most recent" = highest id, so a
/// re-baseline supersedes without mutating the prior row (AC1). Select-only.
pub fn latest(conn: &Connection, account: &str) -> Result<Option<Baseline>> {
    let sql = format!(
        "SELECT {SELECT_COLS} FROM baselines WHERE account = ?1 ORDER BY id DESC LIMIT 1"
    );
    let mut stmt = conn.prepare(&sql).context("could not prepare baseline query")?;
    let mut rows = stmt
        .query_map([account], row_to_baseline)
        .context("could not run baseline query")?;
    match rows.next() {
        Some(r) => Ok(Some(r.context("could not read baseline row")?)),
        None => Ok(None),
    }
}

/// Full baseline history for `account`, newest first — every prior baseline is
/// retained (AC1). Select-only.
pub fn history(conn: &Connection, account: &str) -> Result<Vec<Baseline>> {
    let sql = format!(
        "SELECT {SELECT_COLS} FROM baselines WHERE account = ?1 ORDER BY id DESC"
    );
    let mut stmt = conn.prepare(&sql).context("could not prepare baseline query")?;
    let rows = stmt
        .query_map([account], row_to_baseline)
        .context("could not run baseline query")?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.context("could not read baseline row")?);
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
        p.push(format!("wickd-baseline-test-{pid}-{nanos}-{n}.db"));
        p
    }

    fn bl(account: &str, balance: &str, date: &str) -> Baseline {
        Baseline::new(
            account,
            Some("practice".into()),
            balance,
            Some("USD".into()),
            date.to_string(),
        )
    }

    /// AGT-668: a freshly created baseline DB is owner-only (`0600`), never
    /// world-readable.
    #[cfg(unix)]
    #[test]
    fn new_db_is_created_owner_only_0600() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_db();
        let _conn = open_at(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "baseline db must be created 0600, got {mode:o}");
        let _ = std::fs::remove_file(&path);
    }

    /// AC1: a recorded baseline round-trips and is the account's latest.
    #[test]
    fn record_then_latest_round_trips() {
        let path = temp_db();
        let conn = open_at(&path).unwrap();

        let id = record(&conn, &bl("h004", "10000.0000", "2026-07-05T00:00:00Z")).unwrap();
        assert!(id > 0);

        let got = latest(&conn, "h004").unwrap().unwrap();
        assert_eq!(got.account, "h004");
        assert_eq!(got.balance, "10000.0000");
        assert_eq!(got.currency.as_deref(), Some("USD"));
        assert_eq!(got.baseline_date, "2026-07-05T00:00:00Z");

        let _ = std::fs::remove_file(&path);
    }

    /// AC1: recording a new baseline supersedes the old one (latest), and the
    /// prior baseline is kept in history.
    #[test]
    fn new_baseline_supersedes_but_history_is_retained() {
        let path = temp_db();
        let conn = open_at(&path).unwrap();

        record(&conn, &bl("h004", "10000.0000", "2026-07-05T00:00:00Z")).unwrap();
        record(&conn, &bl("h004", "10250.5000", "2026-08-01T00:00:00Z")).unwrap();

        // Latest is the newest baseline…
        let got = latest(&conn, "h004").unwrap().unwrap();
        assert_eq!(got.balance, "10250.5000");
        assert_eq!(got.baseline_date, "2026-08-01T00:00:00Z");

        // …and BOTH baselines are retained, newest first.
        let hist = history(&conn, "h004").unwrap();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].balance, "10250.5000");
        assert_eq!(hist[1].balance, "10000.0000");
        // Nothing was overwritten — ids are monotonic.
        assert!(hist[0].id > hist[1].id);

        let _ = std::fs::remove_file(&path);
    }

    /// Baselines are isolated per account.
    #[test]
    fn baselines_are_per_account() {
        let path = temp_db();
        let conn = open_at(&path).unwrap();

        record(&conn, &bl("h004", "10000.0000", "2026-07-05T00:00:00Z")).unwrap();
        record(&conn, &bl("h015", "10000.0000", "2026-07-05T00:00:00Z")).unwrap();
        record(&conn, &bl("h004", "10500.0000", "2026-08-01T00:00:00Z")).unwrap();

        assert_eq!(latest(&conn, "h004").unwrap().unwrap().balance, "10500.0000");
        assert_eq!(latest(&conn, "h015").unwrap().unwrap().balance, "10000.0000");
        assert_eq!(history(&conn, "h004").unwrap().len(), 2);
        assert_eq!(history(&conn, "h015").unwrap().len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    /// An un-baselined account has no latest and an empty history.
    #[test]
    fn missing_account_has_no_baseline() {
        let path = temp_db();
        let conn = open_at(&path).unwrap();
        assert!(latest(&conn, "nope").unwrap().is_none());
        assert!(history(&conn, "nope").unwrap().is_empty());
        let _ = std::fs::remove_file(&path);
    }

    /// `record_at` assigns and returns the row id.
    #[test]
    fn record_at_returns_stored_row_with_id() {
        let path = temp_db();
        let stored = record_at(&path, bl("h004", "10000.0000", "2026-07-05T00:00:00Z")).unwrap();
        assert!(stored.id > 0);
        assert_eq!(stored.balance, "10000.0000");
        let _ = std::fs::remove_file(&path);
    }
}
