//! Local store for OANDA order/position-book snapshots (`~/.wickd/books.db`).
//!
//! `wickd books --store` (and the launchd collector built on it) appends the
//! client-sentiment snapshots OANDA publishes on 20-minute boundaries. The
//! store mirrors the home-dir + SQLite pattern of [`crate::baseline`] /
//! [`crate::audit`]: **insert and select only — no UPDATE or DELETE**. The
//! research value of this data is the uninterrupted time series, so nothing
//! may rewrite history.
//!
//! ## Idempotent sampling
//!
//! OANDA re-serves the same snapshot until the next 20-minute boundary, and a
//! collector sampling every N minutes will re-fetch snapshots it already has.
//! Rows are therefore keyed `UNIQUE(instrument, book_type, snapshot_time)`
//! and written with `INSERT OR IGNORE`: re-recording an existing snapshot is
//! a no-op, so any sampling cadence (or a historical backfill overlapping the
//! live collector) converges on one row per snapshot.
//!
//! ## What is stored
//!
//! Per snapshot: the identifying triple, the snapshot-time price and bucket
//! width, the **full raw bucket array as gzip-compressed JSON** (lossless —
//! future studies decide what matters), and two derived convenience columns —
//! the summed long/short percentages across all buckets — so positioning-
//! ratio time series can be queried without touching the blobs.
//!
//! ```sql
//! CREATE TABLE IF NOT EXISTS book_snapshots(
//!   id            INTEGER PRIMARY KEY AUTOINCREMENT,
//!   instrument    TEXT NOT NULL,   -- e.g. EUR_USD
//!   book_type     TEXT NOT NULL,   -- 'order' | 'position'
//!   snapshot_time TEXT NOT NULL,   -- RFC3339, 20-minute boundary (OANDA's key)
//!   price         TEXT NOT NULL,   -- instrument price at snapshot time
//!   bucket_width  TEXT NOT NULL,   -- price width of each bucket
//!   long_pct      TEXT NOT NULL,   -- sum of longCountPercent over all buckets
//!   short_pct     TEXT NOT NULL,   -- sum of shortCountPercent over all buckets
//!   buckets_gz    BLOB NOT NULL,   -- gzip(JSON array of raw buckets)
//!   fetched_at    TEXT NOT NULL,   -- RFC3339 instant the row was written
//!   UNIQUE(instrument, book_type, snapshot_time)
//! );
//! ```

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use rusqlite::Connection;
use rust_decimal::Decimal;

use wickd_core::oanda::types::{BookBucket, OandaBook};

/// Which of the two instrument books a snapshot came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookType {
    Order,
    Position,
}

impl BookType {
    pub fn as_str(self) -> &'static str {
        match self {
            BookType::Order => "order",
            BookType::Position => "position",
        }
    }
}

/// Summary of one stored (or skipped) snapshot, for the command's JSON output.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StoredSnapshot {
    pub instrument: String,
    pub book_type: String,
    pub snapshot_time: String,
    pub price: String,
    pub long_pct: String,
    pub short_pct: String,
    pub buckets: usize,
    /// `false` when the snapshot was already present (INSERT OR IGNORE no-op).
    pub stored: bool,
}

/// Path to the books DB (`~/.wickd/books.db`).
pub fn books_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not resolve home directory")?;
    Ok(home.join(".wickd").join("books.db"))
}

/// Open (creating on first use) the default books store.
pub fn open() -> Result<Connection> {
    open_at(books_path()?)
}

/// Open (creating on first use) a books store at an explicit path. Tests pass
/// a temp path so they never touch the real `~/.wickd/books.db`.
pub fn open_at(path: impl AsRef<Path>) -> Result<Connection> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create books dir {}", parent.display()))?;
    }
    let conn = Connection::open(path)
        .with_context(|| format!("could not open books db {}", path.display()))?;
    // books.db has concurrent writers by design (the launchd collector + the
    // wickd-lab historical backfill both INSERT OR IGNORE into the same
    // store). Without a busy handler rusqlite fails a colliding write with
    // SQLITE_BUSY instantly — which silently killed every collector tick
    // that landed inside the backfill's write window. Wait out short locks
    // instead.
    conn.busy_timeout(std::time::Duration::from_secs(60))
        .context("could not set books db busy timeout")?;
    // Local trading data must not be world-readable (AGT-668).
    crate::fs_perms::restrict_file(path)?;
    init_schema(&conn)?;
    Ok(conn)
}

/// Create the append-only table if it does not exist. Idempotent.
fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS book_snapshots(
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            instrument    TEXT NOT NULL,
            book_type     TEXT NOT NULL,
            snapshot_time TEXT NOT NULL,
            price         TEXT NOT NULL,
            bucket_width  TEXT NOT NULL,
            long_pct      TEXT NOT NULL,
            short_pct     TEXT NOT NULL,
            buckets_gz    BLOB NOT NULL,
            fetched_at    TEXT NOT NULL,
            UNIQUE(instrument, book_type, snapshot_time)
        )",
        [],
    )
    .context("could not initialize book_snapshots schema")?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_books_series
         ON book_snapshots(instrument, book_type, snapshot_time)",
        [],
    )
    .context("could not create book_snapshots series index")?;
    Ok(())
}

/// Sum the long/short percentages across all buckets with exact [`Decimal`]
/// arithmetic. For the position book this pair is the classic client
/// positioning ratio (long% + short% ≈ 100); for the order book it splits
/// pending orders long vs short.
fn aggregate_pcts(buckets: &[BookBucket]) -> Result<(Decimal, Decimal)> {
    let mut long = Decimal::ZERO;
    let mut short = Decimal::ZERO;
    for b in buckets {
        long += Decimal::from_str(&b.long_count_percent)
            .map_err(|e| anyhow!("bad longCountPercent {:?}: {e}", b.long_count_percent))?;
        short += Decimal::from_str(&b.short_count_percent)
            .map_err(|e| anyhow!("bad shortCountPercent {:?}: {e}", b.short_count_percent))?;
    }
    Ok((long, short))
}

fn gzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(bytes).context("could not gzip bucket JSON")?;
    enc.finish().context("could not finish gzip stream")
}

// Research consumers (wickd-lab python) read books.db directly with
// sqlite3+zlib; `gunzip`/`buckets` below are the in-repo reference reader,
// exercised by the round-trip tests to pin the storage format.
#[cfg_attr(not(test), allow(dead_code))]
fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut dec = GzDecoder::new(bytes);
    let mut out = Vec::new();
    dec.read_to_end(&mut out).context("could not gunzip bucket blob")?;
    Ok(out)
}

/// Summarize a fetched book without touching the store: derived aggregates +
/// bucket count, `stored: false`. [`record`] builds on this; the command uses
/// it directly for non-store runs.
pub fn summarize(book: &OandaBook, book_type: BookType) -> Result<StoredSnapshot> {
    let (long, short) = aggregate_pcts(&book.buckets)?;
    Ok(StoredSnapshot {
        instrument: book.instrument.clone(),
        book_type: book_type.as_str().to_string(),
        snapshot_time: book.time.clone(),
        price: book.price.clone(),
        long_pct: long.to_string(),
        short_pct: short.to_string(),
        buckets: book.buckets.len(),
        stored: false,
    })
}

/// Append one book snapshot. The ONLY write path — `INSERT OR IGNORE`, so a
/// snapshot already in the store (same instrument/book/time) is skipped and
/// reported with `stored: false`.
pub fn record(conn: &Connection, book: &OandaBook, book_type: BookType) -> Result<StoredSnapshot> {
    let mut summary = summarize(book, book_type)?;
    let buckets_json =
        serde_json::to_vec(&book.buckets).context("could not serialize buckets")?;
    let buckets_gz = gzip(&buckets_json)?;

    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO book_snapshots
                (instrument, book_type, snapshot_time, price, bucket_width,
                 long_pct, short_pct, buckets_gz, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                book.instrument,
                book_type.as_str(),
                book.time,
                book.price,
                book.bucket_width,
                summary.long_pct,
                summary.short_pct,
                buckets_gz,
                Utc::now().to_rfc3339(),
            ],
        )
        .context("could not write book snapshot row")?;

    summary.stored = inserted == 1;
    Ok(summary)
}

/// Read back the raw bucket array of a stored snapshot (select-only; the
/// reference reader — see note on [`gunzip`]). `None` when the snapshot is
/// not in the store.
#[cfg_attr(not(test), allow(dead_code))]
pub fn buckets(
    conn: &Connection,
    instrument: &str,
    book_type: BookType,
    snapshot_time: &str,
) -> Result<Option<Vec<BookBucket>>> {
    let mut stmt = conn
        .prepare(
            "SELECT buckets_gz FROM book_snapshots
             WHERE instrument = ?1 AND book_type = ?2 AND snapshot_time = ?3",
        )
        .context("could not prepare bucket query")?;
    let blob: Option<Vec<u8>> = stmt
        .query_row(
            rusqlite::params![instrument, book_type.as_str(), snapshot_time],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .context("could not run bucket query")?;
    match blob {
        None => Ok(None),
        Some(gz) => {
            let json = gunzip(&gz)?;
            let buckets: Vec<BookBucket> =
                serde_json::from_slice(&json).context("could not parse stored buckets")?;
            Ok(Some(buckets))
        }
    }
}

/// Count of stored snapshots (select-only; command output + tests).
pub fn count(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM book_snapshots", [], |row| row.get(0))
        .context("could not count book snapshots")
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
        p.push(format!("wickd-books-test-{pid}-{nanos}-{n}.db"));
        p
    }

    fn sample_book(time: &str) -> OandaBook {
        OandaBook {
            instrument: "EUR_USD".into(),
            time: time.into(),
            price: "1.14150".into(),
            bucket_width: "0.00050".into(),
            buckets: vec![
                BookBucket {
                    price: "1.14100".into(),
                    long_count_percent: "0.6722".into(),
                    short_count_percent: "0.5418".into(),
                },
                BookBucket {
                    price: "1.14150".into(),
                    long_count_percent: "0.1630".into(),
                    short_count_percent: "0.1505".into(),
                },
            ],
        }
    }

    /// AGT-668 pattern: a freshly created books DB is owner-only (0600).
    #[cfg(unix)]
    #[test]
    fn fresh_db_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_db();
        let _conn = open_at(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_file(&path);
    }

    /// Round-trip: record → derived aggregates exact → buckets decompress
    /// back to the original array.
    #[test]
    fn record_and_read_back_roundtrip() {
        let path = temp_db();
        let conn = open_at(&path).unwrap();

        let book = sample_book("2026-07-11T18:00:00Z");
        let s = record(&conn, &book, BookType::Position).unwrap();
        assert!(s.stored);
        // 0.6722 + 0.1630 and 0.5418 + 0.1505, exact Decimal arithmetic.
        assert_eq!(s.long_pct, "0.8352");
        assert_eq!(s.short_pct, "0.6923");
        assert_eq!(s.buckets, 2);

        let read = buckets(&conn, "EUR_USD", BookType::Position, "2026-07-11T18:00:00Z")
            .unwrap()
            .expect("stored snapshot present");
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].price, "1.14100");
        assert_eq!(read[1].short_count_percent, "0.1505");

        let _ = std::fs::remove_file(&path);
    }

    /// Re-recording the same snapshot is an INSERT OR IGNORE no-op — one row,
    /// `stored: false` on the repeat. Same time under the OTHER book type is a
    /// distinct series and does insert.
    #[test]
    fn duplicate_snapshot_is_skipped_not_duplicated() {
        let path = temp_db();
        let conn = open_at(&path).unwrap();

        let book = sample_book("2026-07-11T18:00:00Z");
        assert!(record(&conn, &book, BookType::Order).unwrap().stored);
        assert!(!record(&conn, &book, BookType::Order).unwrap().stored);
        assert_eq!(count(&conn).unwrap(), 1);

        assert!(record(&conn, &book, BookType::Position).unwrap().stored);
        assert_eq!(count(&conn).unwrap(), 2);

        let _ = std::fs::remove_file(&path);
    }

    /// Missing snapshots read back as None, not an error.
    #[test]
    fn missing_snapshot_reads_none() {
        let path = temp_db();
        let conn = open_at(&path).unwrap();
        let got = buckets(&conn, "EUR_USD", BookType::Order, "2026-01-01T00:00:00Z").unwrap();
        assert!(got.is_none());
        let _ = std::fs::remove_file(&path);
    }
}
