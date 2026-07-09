//! Persistent per-instrument spread statistics (`~/.wickd/spreads.db`).
//!
//! CandleSight graded the live spread against historical stats (green =
//! historically low, yellow = average, red = high, purple = no history yet)
//! by batching samples to its queries-service. wickd is local-first, so the
//! same algorithm runs here against a SQLite store, mirroring the audit-db
//! pattern.
//!
//! ## The algorithm (ported from queries-service `processSpreadSamples`)
//!
//! One row per instrument: `sample_count, min_spread, max_spread, ema_spread`.
//! Each sample updates:
//! - `ema = ALPHA * sample + (1 - ALPHA) * ema` — α 0.001 ≈ 1-day half-life
//!   at 5-second samples.
//! - `min = min(min, sample)` then decayed toward the EMA by `DECAY`, and
//!   `max = max(max, sample)` symmetric — so a one-off spike doesn't pin the
//!   "red" end of the scale forever.
//! - clamp to `min ≤ ema ≤ max`.
//!
//! ## Writers
//!
//! Every price consumer contributes: `wickd stream` (the hub owner, running
//! all day) via [`SpreadSamplingSink`], and `wickd view ticket`'s quote loop
//! via [`SpreadSampler`]. Sampling is throttled per instrument
//! ([`SAMPLE_INTERVAL`]); concurrent processes are safe via SQLite's busy
//! timeout — worst case a sample is skipped, which is fine for display-only
//! statistics. These stats are NEVER inputs to order construction.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use rust_decimal::Decimal;

use wickd_core::oanda::streaming::{PriceUpdate, StreamError, StreamHealthStatus};
use wickd_core::strategy::{
    MatchStatusUpdateEvent, PatternMatchEvent, StrategyErrorEvent, StrategyStatusEvent,
    WatcherTickEvent,
};
use wickd_core::EventSink;

/// Minimum time between persisted samples per instrument (the CandleSight
/// collector's cadence).
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

/// EMA smoothing factor: 0.001 ≈ 1-day half-life at 5-second samples.
fn alpha() -> Decimal {
    Decimal::new(1, 3)
}

/// Decay pulling the min/max extremes toward the EMA each sample.
fn decay() -> Decimal {
    Decimal::new(1, 4)
}

/// One instrument's persisted spread statistics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stats {
    pub sample_count: i64,
    pub min: Decimal,
    pub max: Decimal,
    pub ema: Decimal,
}

/// Fold one spread sample into the running stats. Pure — mirrors the
/// queries-service update exactly (single-sample batches, so the batch
/// average IS the sample).
pub fn fold_sample(current: Option<Stats>, sample: Decimal) -> Stats {
    match current {
        None => Stats { sample_count: 1, min: sample, max: sample, ema: sample },
        Some(s) => {
            let ema = alpha() * sample + (Decimal::ONE - alpha()) * s.ema;
            let mut min = s.min.min(sample);
            min += decay() * (ema - min);
            let mut max = s.max.max(sample);
            max -= decay() * (max - ema);
            Stats {
                sample_count: s.sample_count + 1,
                min: min.min(ema),
                max: max.max(ema),
                ema,
            }
        }
    }
}

/// Path to the spread-stats DB (`~/.wickd/spreads.db`).
pub fn spreads_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not resolve home directory")?;
    Ok(home.join(".wickd").join("spreads.db"))
}

/// Open (creating on first use) the default store.
pub fn open() -> Result<Connection> {
    open_at(spreads_path()?)
}

/// Open (creating on first use) a store at an explicit path. Tests pass a
/// temp path so they never touch the real `~/.wickd/spreads.db`.
pub fn open_at(path: impl AsRef<Path>) -> Result<Connection> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create spreads dir {}", parent.display()))?;
    }
    let conn = Connection::open(path)
        .with_context(|| format!("could not open spreads db {}", path.display()))?;
    // Local trading data must not be world-readable (AGT-668).
    crate::fs_perms::restrict_file(path)?;
    // Multiple wickd processes (stream + ticket) sample concurrently; wait
    // briefly for a writer rather than erroring.
    conn.busy_timeout(Duration::from_millis(2000))
        .context("could not set spreads db busy timeout")?;
    init_schema(&conn)?;
    Ok(conn)
}

/// Create the stats table if it does not exist. Idempotent. Decimals are
/// stored as TEXT (house rule: no float representations of price-derived
/// values at rest).
fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS spread_stats(
            instrument   TEXT PRIMARY KEY,
            sample_count INTEGER NOT NULL,
            min_spread   TEXT NOT NULL,
            max_spread   TEXT NOT NULL,
            ema_spread   TEXT NOT NULL,
            last_updated TEXT NOT NULL,
            created_at   TEXT NOT NULL
        )",
        [],
    )
    .context("could not initialize spread_stats schema")?;
    Ok(())
}

/// Read one instrument's stats.
pub fn get(conn: &Connection, instrument: &str) -> Result<Option<Stats>> {
    conn.query_row(
        "SELECT sample_count, min_spread, max_spread, ema_spread
         FROM spread_stats WHERE instrument = ?1",
        [instrument],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    )
    .optional()
    .context("could not read spread stats")?
    .map(|(sample_count, min, max, ema)| {
        Ok(Stats {
            sample_count,
            min: Decimal::from_str(&min).context("corrupt min_spread")?,
            max: Decimal::from_str(&max).context("corrupt max_spread")?,
            ema: Decimal::from_str(&ema).context("corrupt ema_spread")?,
        })
    })
    .transpose()
}

/// Fold one sample into the store (read → fold → upsert). Returns the new
/// stats. Runs in a transaction so concurrent writers can't interleave the
/// read-modify-write.
pub fn record_sample(conn: &mut Connection, instrument: &str, sample: Decimal) -> Result<Stats> {
    let tx = conn.transaction().context("could not begin spreads transaction")?;
    let current = {
        let got = tx
            .query_row(
                "SELECT sample_count, min_spread, max_spread, ema_spread
                 FROM spread_stats WHERE instrument = ?1",
                [instrument],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .context("could not read spread stats")?;
        got.map(|(sample_count, min, max, ema)| -> Result<Stats> {
            Ok(Stats {
                sample_count,
                min: Decimal::from_str(&min).context("corrupt min_spread")?,
                max: Decimal::from_str(&max).context("corrupt max_spread")?,
                ema: Decimal::from_str(&ema).context("corrupt ema_spread")?,
            })
        })
        .transpose()?
    };
    let next = fold_sample(current, sample);
    let now = chrono::Utc::now().to_rfc3339();
    tx.execute(
        "INSERT INTO spread_stats
            (instrument, sample_count, min_spread, max_spread, ema_spread, last_updated, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
         ON CONFLICT(instrument) DO UPDATE SET
            sample_count = excluded.sample_count,
            min_spread   = excluded.min_spread,
            max_spread   = excluded.max_spread,
            ema_spread   = excluded.ema_spread,
            last_updated = excluded.last_updated",
        rusqlite::params![
            instrument,
            next.sample_count,
            next.min.to_string(),
            next.max.to_string(),
            next.ema.to_string(),
            now,
        ],
    )
    .context("could not upsert spread stats")?;
    tx.commit().context("could not commit spread stats")?;
    Ok(next)
}

/// Throttled sampler a price consumer holds for its lifetime: feed it every
/// quote, it persists at most one sample per instrument per
/// [`SAMPLE_INTERVAL`] and returns the refreshed stats when it does. All
/// failures are swallowed — spread history is display-only and must never
/// break a price path.
pub struct SpreadSampler {
    conn: Connection,
    last: HashMap<String, Instant>,
}

impl SpreadSampler {
    /// Open a sampler on the default store; `None` if the DB can't open.
    pub fn open_default() -> Option<SpreadSampler> {
        open().ok().map(|conn| SpreadSampler { conn, last: HashMap::new() })
    }

    /// Open a sampler on an explicit store path (tests).
    #[cfg(test)]
    pub fn open_at_path(path: impl AsRef<Path>) -> Option<SpreadSampler> {
        open_at(path).ok().map(|conn| SpreadSampler { conn, last: HashMap::new() })
    }

    /// Read an instrument's current stats (e.g. for an initial paint).
    pub fn stats(&self, instrument: &str) -> Option<Stats> {
        get(&self.conn, instrument).ok().flatten()
    }

    /// Offer one quote's spread. Returns the refreshed stats when a sample
    /// was persisted, `None` when throttled or on any failure.
    pub fn on_quote(&mut self, instrument: &str, spread: &str) -> Option<Stats> {
        let now = Instant::now();
        if let Some(prev) = self.last.get(instrument) {
            if now.duration_since(*prev) < SAMPLE_INTERVAL {
                return None;
            }
        }
        let sample = Decimal::from_str(spread).ok()?;
        if sample < Decimal::ZERO {
            return None;
        }
        self.last.insert(instrument.to_string(), now);
        record_sample(&mut self.conn, instrument, sample).ok()
    }
}

/// [`EventSink`] wrapper that samples spreads off every price-update and
/// forwards all events unchanged to the inner sink. `wickd stream` wraps its
/// sink with this so the always-on hub process builds spread history all day.
pub struct SpreadSamplingSink {
    inner: Arc<dyn EventSink>,
    sampler: Mutex<SpreadSampler>,
}

impl SpreadSamplingSink {
    /// Wrap `inner`; returns `inner` unchanged if the spreads DB can't open
    /// (history is best-effort, the price path must never depend on it).
    pub fn wrap(inner: Arc<dyn EventSink>) -> Arc<dyn EventSink> {
        match SpreadSampler::open_default() {
            Some(s) => Arc::new(SpreadSamplingSink { inner, sampler: Mutex::new(s) }),
            None => inner,
        }
    }
}

impl EventSink for SpreadSamplingSink {
    fn price_update(&self, ev: &PriceUpdate) {
        if let Ok(mut s) = self.sampler.lock() {
            let _ = s.on_quote(&ev.instrument, &ev.spread);
        }
        self.inner.price_update(ev);
    }
    fn pattern_matched(&self, ev: &PatternMatchEvent) {
        self.inner.pattern_matched(ev);
    }
    fn strategy_status(&self, ev: &StrategyStatusEvent) {
        self.inner.strategy_status(ev);
    }
    fn strategy_error(&self, ev: &StrategyErrorEvent) {
        self.inner.strategy_error(ev);
    }
    fn match_status_update(&self, ev: &MatchStatusUpdateEvent) {
        self.inner.match_status_update(ev);
    }
    fn watcher_tick(&self, ev: &WatcherTickEvent) {
        self.inner.watcher_tick(ev);
    }
    fn stream_error(&self, ev: &StreamError) {
        self.inner.stream_error(ev);
    }
    fn stream_health(&self, ev: &StreamHealthStatus) {
        self.inner.stream_health(ev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn temp_db() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static C: AtomicU64 = AtomicU64::new(0);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "wickd-spreads-test-{}-{}.db",
            std::process::id(),
            C.fetch_add(1, Ordering::Relaxed)
        ));
        p
    }

    /// AGT-668: a freshly created spreads DB is owner-only (`0600`), never
    /// world-readable.
    #[cfg(unix)]
    #[test]
    fn new_db_is_created_owner_only_0600() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_db();
        let _conn = open_at(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "spreads db must be created 0600, got {mode:o}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn first_sample_initializes_all_fields() {
        let s = fold_sample(None, dec!(0.00016));
        assert_eq!(s.sample_count, 1);
        assert_eq!(s.min, dec!(0.00016));
        assert_eq!(s.max, dec!(0.00016));
        assert_eq!(s.ema, dec!(0.00016));
    }

    #[test]
    fn extremes_widen_then_decay_toward_ema() {
        let mut s = fold_sample(None, dec!(0.00016));
        // A wide spike raises the max…
        s = fold_sample(Some(s), dec!(0.00080));
        assert_eq!(s.sample_count, 2);
        assert!(s.max > dec!(0.00079), "max captured the spike: {:?}", s);
        let spiked_max = s.max;
        // …and normal samples afterwards decay it back toward the EMA.
        for _ in 0..200 {
            s = fold_sample(Some(s), dec!(0.00016));
        }
        assert!(s.max < spiked_max, "max decays: {} !< {}", s.max, spiked_max);
        // Invariant: min ≤ ema ≤ max.
        assert!(s.min <= s.ema && s.ema <= s.max, "ordered: {s:?}");
    }

    #[test]
    fn ema_tracks_slowly() {
        let mut s = fold_sample(None, dec!(0.00010));
        s = fold_sample(Some(s), dec!(0.00020));
        // α = 0.001: one sample barely moves the EMA.
        assert!(s.ema > dec!(0.00010) && s.ema < dec!(0.000101), "ema: {}", s.ema);
    }

    #[test]
    fn store_round_trip_and_upsert() {
        let path = temp_db();
        let mut conn = open_at(&path).unwrap();
        assert!(get(&conn, "EUR_USD").unwrap().is_none());

        let s1 = record_sample(&mut conn, "EUR_USD", dec!(0.00016)).unwrap();
        assert_eq!(s1.sample_count, 1);
        let s2 = record_sample(&mut conn, "EUR_USD", dec!(0.00020)).unwrap();
        assert_eq!(s2.sample_count, 2);
        assert_eq!(get(&conn, "EUR_USD").unwrap().unwrap(), s2);

        // Second instrument is independent.
        record_sample(&mut conn, "USD_JPY", dec!(0.012)).unwrap();
        assert_eq!(get(&conn, "EUR_USD").unwrap().unwrap().sample_count, 2);
        assert_eq!(get(&conn, "USD_JPY").unwrap().unwrap().sample_count, 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sampler_throttles_and_validates() {
        let path = temp_db();
        let mut sampler = SpreadSampler::open_at_path(&path).unwrap();

        // First quote persists…
        assert!(sampler.on_quote("EUR_USD", "0.00016").is_some());
        // …an immediate second one is throttled…
        assert!(sampler.on_quote("EUR_USD", "0.00017").is_none());
        // …but another instrument samples independently.
        assert!(sampler.on_quote("USD_JPY", "0.012").is_some());

        // Garbage and negative spreads never persist (and never panic).
        assert!(sampler.on_quote("GBP_USD", "not-a-number").is_none());
        assert!(sampler.on_quote("GBP_USD", "-0.0001").is_none());
        assert!(sampler.stats("GBP_USD").is_none());

        assert_eq!(sampler.stats("EUR_USD").unwrap().sample_count, 1);
        let _ = std::fs::remove_file(&path);
    }
}
