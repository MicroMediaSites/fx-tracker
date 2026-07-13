//! Tick-aggregating [`CandleSource`] — builds candles from a live tick feed.
//!
//! This is the `TickStreamSource` the [`CandleSource`] trait doc names: it
//! consumes streaming ticks (fed in over a channel) and folds them into
//! completed OHLC candles bucketed by the timeframe's boundaries. It is the
//! integration surface for `wickd watch`'s socket-hub attach (AGT-618 AC2):
//! `wickd stream` fans one OANDA price subscription out to a Unix socket, and
//! `wickd watch` parses those NDJSON ticks and pumps them into one of these per
//! instrument — so N watchers share the single always-on upstream subscription
//! instead of each opening its own.
//!
//! ## Alignment
//!
//! Candle boundaries reuse [`CandleBoundaryDetector::current_candle_start`], the
//! same alignment machinery every other part of the system uses
//! (`dailyAlignment=2`, `alignmentTimezone=UTC`; e.g. H4 closes at
//! 02/06/10/14/18/22 UTC). We deliberately do NOT re-derive alignment here.
//!
//! ## Warmup
//!
//! The live tick feed carries no history, so indicator warmup
//! ([`CandleSource::get_candles`]) still does a one-off OANDA REST fetch. That
//! is fine — and is exactly the shared-subscription win: the expensive,
//! always-on streaming subscription is shared via the hub, while each watcher
//! only makes a cheap one-off REST warmup call of its own.
//!
//! ## Stale-feed REST fallback
//!
//! A hub can wedge: the socket stays open but no ticks arrive (observed
//! 2026-07-09, when two `--auto` watchers sat blind on a silent hub for three
//! days). The hub has no control channel, so silence is indistinguishable from
//! a closed market by the feed alone. The fallback disambiguates against the
//! source of truth instead: once the feed has been silent for [`STALE_AFTER`],
//! `get_latest_candle` starts polling OANDA REST (throttled to
//! [`REST_CHECK_INTERVAL`]) for completed candles newer than the last one
//! emitted. A closed market returns nothing new — quiet weekends stay quiet —
//! while a wedged hub gets its candle closes backfilled in order, up to
//! [`REST_RECOVERY_COUNT`] per check. Tick-built and REST-fetched candles are
//! deduped against the last emitted candle time, so the two paths never hand
//! the watcher the same period twice.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc::Receiver;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{info, warn};

use super::candle_boundary::CandleBoundaryDetector;
use super::candle_source::CandleSource;
use crate::error::Result;
use crate::models::{Candle, Ohlc};
use crate::oanda::client::OandaClient;
use crate::oanda::endpoints::{get_candles, Granularity};

/// How long the tick feed may stay silent before the REST fallback engages.
/// During an open market every instrument ticks many times a minute, so two
/// minutes of silence means the feed is dead (or the market is closed — the
/// REST check tells the two apart by simply finding nothing new).
const STALE_AFTER: Duration = Duration::from_secs(120);

/// Minimum spacing between REST fallback checks while the feed is silent.
/// Keeps a closed-market weekend to one cheap request a minute per instrument.
const REST_CHECK_INTERVAL: Duration = Duration::from_secs(60);

/// Completed candles fetched per fallback check — bounds how far back a long
/// feed outage is backfilled (10 bars = 40h on H4, 80h on H8).
const REST_RECOVERY_COUNT: u32 = 10;

/// A single price observation fed into a [`TickStreamSource`].
///
/// `price` is the mid price for the tick (`(bid + ask) / 2`), already a
/// [`Decimal`] — prices are never f64 in this codebase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tick {
    /// Mid price for this tick.
    pub price: Decimal,
    /// Event time of the tick (UTC).
    pub time: DateTime<Utc>,
}

/// The still-forming candle for the current period.
#[derive(Debug, Clone)]
struct InProgress {
    start: DateTime<Utc>,
    open: Decimal,
    high: Decimal,
    low: Decimal,
    close: Decimal,
    volume: i32,
}

impl InProgress {
    fn open(start: DateTime<Utc>, price: Decimal) -> Self {
        Self {
            start,
            open: price,
            high: price,
            low: price,
            close: price,
            volume: 1,
        }
    }

    fn update(&mut self, price: Decimal) {
        if price > self.high {
            self.high = price;
        }
        if price < self.low {
            self.low = price;
        }
        self.close = price;
        self.volume += 1;
    }

    fn into_candle(self) -> Candle {
        Candle {
            time: self.start,
            mid: Ohlc {
                open: self.open,
                high: self.high,
                low: self.low,
                close: self.close,
            },
            volume: self.volume,
            complete: true,
        }
    }
}

/// Aggregates ticks into timeframe-aligned candles.
///
/// Feed ticks with [`on_tick`](Self::on_tick); it returns the just-closed candle
/// on the first tick that crosses into a new candle period, and `None`
/// otherwise. Boundaries come from [`CandleBoundaryDetector::current_candle_start`]
/// so alignment matches the rest of the system exactly.
#[derive(Debug)]
pub struct TickCandleAggregator {
    timeframe: Granularity,
    current: Option<InProgress>,
}

impl TickCandleAggregator {
    /// Create an aggregator for `timeframe`.
    pub fn new(timeframe: Granularity) -> Self {
        Self {
            timeframe,
            current: None,
        }
    }

    /// Fold one tick in. Returns `Some(candle)` when this tick opens a new
    /// candle period — i.e. the previous period just closed and is complete —
    /// otherwise `None`.
    pub fn on_tick(&mut self, tick: &Tick) -> Option<Candle> {
        let bucket_start =
            CandleBoundaryDetector::current_candle_start(tick.time, self.timeframe);

        match self.current.take() {
            None => {
                // First tick ever: open the current candle.
                self.current = Some(InProgress::open(bucket_start, tick.price));
                None
            }
            Some(mut cur) => {
                if bucket_start == cur.start {
                    // Same period: extend the in-progress candle.
                    cur.update(tick.price);
                    self.current = Some(cur);
                    None
                } else if bucket_start > cur.start {
                    // Crossed into a new period: the old candle is complete.
                    let completed = cur.into_candle();
                    self.current = Some(InProgress::open(bucket_start, tick.price));
                    Some(completed)
                } else {
                    // Out-of-order tick older than the current period — ignore
                    // it rather than corrupt the candle we're building.
                    self.current = Some(cur);
                    None
                }
            }
        }
    }
}

/// Drains ticks off a channel and turns them into completed candles.
///
/// Split out from [`TickStreamSource`] so the aggregation/draining logic is
/// unit-testable with a plain channel (no OANDA client required).
struct TickCandleBuffer {
    rx: Receiver<Tick>,
    aggregator: TickCandleAggregator,
    /// Completed candles waiting to be handed to the watcher, oldest first.
    ready: VecDeque<Candle>,
}

impl TickCandleBuffer {
    fn new(rx: Receiver<Tick>, timeframe: Granularity) -> Self {
        Self {
            rx,
            aggregator: TickCandleAggregator::new(timeframe),
            ready: VecDeque::new(),
        }
    }

    /// Drain every currently-available tick into the aggregator, queueing any
    /// candles that closed. Returns how many ticks were drained — the caller
    /// uses this as the feed-liveness signal for the REST fallback.
    /// Non-blocking: a disconnected channel (hub gone) just stops the drain.
    fn drain(&mut self) -> usize {
        // `try_recv` returns `Err` for both an empty and a disconnected (hub
        // gone) channel — either way we stop draining and hand out what we have.
        let mut drained = 0;
        while let Ok(tick) = self.rx.try_recv() {
            drained += 1;
            if let Some(candle) = self.aggregator.on_tick(&tick) {
                self.ready.push_back(candle);
            }
        }
        drained
    }

    /// Pop the oldest completed candle strictly newer than `last_emitted`,
    /// discarding any older ones — those periods were already handed to the
    /// watcher (e.g. by the REST fallback while the feed was silent).
    fn pop_fresh(&mut self, last_emitted: Option<DateTime<Utc>>) -> Option<Candle> {
        while let Some(candle) = self.ready.pop_front() {
            if last_emitted.is_none_or(|t| candle.time > t) {
                return Some(candle);
            }
        }
        None
    }

    /// Queue a candle recovered out-of-band (REST fallback) behind whatever is
    /// already waiting, preserving oldest-first delivery.
    fn queue(&mut self, candle: Candle) {
        self.ready.push_back(candle);
    }
}

/// A [`CandleSource`] backed by a live tick feed (e.g. `wickd stream`'s socket
/// hub). Ticks arrive over `rx`; historical warmup falls back to OANDA REST,
/// and so do candle closes whenever the feed goes silent (see the module docs).
pub struct TickStreamSource {
    client: OandaClient,
    instrument: String,
    timeframe: Granularity,
    buffer: Mutex<TickCandleBuffer>,
    /// When the drain last saw a tick (construction counts as "now", so a hub
    /// that never delivers trips the fallback [`STALE_AFTER`] after startup).
    last_tick_at: Mutex<Instant>,
    /// Time of the newest candle handed to the watcher via either path — the
    /// dedupe guard between tick-built and REST-recovered candles.
    last_emitted: Mutex<Option<DateTime<Utc>>>,
    /// When the REST fallback last ran (throttle).
    last_rest_check: Mutex<Option<Instant>>,
    /// True while in fallback mode, so the transition is logged once, not
    /// once per poll.
    feed_stale: Mutex<bool>,
}

impl TickStreamSource {
    /// Build a source for `instrument`/`timeframe` that aggregates the ticks
    /// delivered on `rx`. `client` serves the one-off REST warmup fetch (the
    /// live feed carries no history) and the stale-feed REST fallback.
    pub fn new(
        client: OandaClient,
        instrument: String,
        timeframe: Granularity,
        rx: Receiver<Tick>,
    ) -> Self {
        Self {
            client,
            instrument,
            timeframe,
            buffer: Mutex::new(TickCandleBuffer::new(rx, timeframe)),
            last_tick_at: Mutex::new(Instant::now()),
            last_emitted: Mutex::new(None),
            last_rest_check: Mutex::new(None),
            feed_stale: Mutex::new(false),
        }
    }

    /// Lock helper: surface a poisoned mutex as a structured error, not a
    /// panic — this runs on the daemon path.
    fn lock<'a, T>(m: &'a Mutex<T>, what: &str) -> Result<std::sync::MutexGuard<'a, T>> {
        m.lock()
            .map_err(|_| crate::error::Error::Strategy(format!("{what} mutex poisoned")))
    }
}

impl CandleSource for TickStreamSource {
    fn get_latest_candle(&self) -> Pin<Box<dyn Future<Output = Result<Option<Candle>>> + Send + '_>> {
        Box::pin(async move {
            // --- Tick path: pure, synchronous drain — no await under a lock. ---
            let last_emitted = *Self::lock(&self.last_emitted, "last_emitted")?;
            let (candle, silent_for) = {
                let mut buffer = Self::lock(&self.buffer, "tick buffer")?;
                let drained = buffer.drain();

                let mut last_tick_at = Self::lock(&self.last_tick_at, "last_tick_at")?;
                if drained > 0 {
                    *last_tick_at = Instant::now();
                    let mut stale = Self::lock(&self.feed_stale, "feed_stale")?;
                    if *stale {
                        *stale = false;
                        let msg = format!(
                            "wickd watch: {} {}: tick feed recovered — leaving REST fallback",
                            self.instrument, self.timeframe
                        );
                        eprintln!("{msg}");
                        info!("{msg}");
                    }
                }
                (buffer.pop_fresh(last_emitted), last_tick_at.elapsed())
            };

            if let Some(candle) = candle {
                *Self::lock(&self.last_emitted, "last_emitted")? = Some(candle.time);
                return Ok(Some(candle));
            }

            // --- Stale-feed REST fallback. ---
            if silent_for < STALE_AFTER {
                return Ok(None);
            }
            {
                let mut last_check = Self::lock(&self.last_rest_check, "last_rest_check")?;
                if let Some(at) = *last_check {
                    if at.elapsed() < REST_CHECK_INTERVAL {
                        return Ok(None);
                    }
                }
                *last_check = Some(Instant::now());
            }
            {
                let mut stale = Self::lock(&self.feed_stale, "feed_stale")?;
                if !*stale {
                    *stale = true;
                    let msg = format!(
                        "wickd watch: {} {}: no ticks from the feed for {}s — \
                         falling back to OANDA REST polling for candle closes \
                         (wedged hub or closed market)",
                        self.instrument,
                        self.timeframe,
                        silent_for.as_secs()
                    );
                    eprintln!("{msg}");
                    warn!("{msg}");
                }
            }

            let fetched = get_candles(
                &self.client,
                &self.instrument,
                self.timeframe,
                Some(REST_RECOVERY_COUNT),
                None,
                None,
            )
            .await?;

            let mut fresh: Vec<Candle> = fetched
                .into_iter()
                .filter(|c| c.complete && last_emitted.is_none_or(|t| c.time > t))
                .collect();
            fresh.sort_by_key(|c| c.time);

            let Some(first) = (!fresh.is_empty()).then(|| fresh.remove(0)) else {
                return Ok(None); // Nothing new: the market is just closed.
            };
            {
                // Later recovered candles queue behind `first`, emitted one per
                // poll in order; `pop_fresh` dedupes them if ticks resume and
                // rebuild the same periods.
                let mut buffer = Self::lock(&self.buffer, "tick buffer")?;
                for candle in fresh {
                    buffer.queue(candle);
                }
            }
            *Self::lock(&self.last_emitted, "last_emitted")? = Some(first.time);
            Ok(Some(first))
        })
    }

    fn get_candles(&self, count: u32) -> Pin<Box<dyn Future<Output = Result<Vec<Candle>>> + Send + '_>> {
        Box::pin(async move {
            // Warmup CANNOT come from the live socket — the hub fans out only
            // real-time ticks, no history. So each watcher still does its own
            // cheap, one-off REST fetch here. That is the whole shared-hub win:
            // the expensive, always-on streaming subscription is shared across
            // every watcher via the hub, while warmup stays per-watcher REST.
            let candles = get_candles(
                &self.client,
                &self.instrument,
                self.timeframe,
                Some(count),
                None,
                None,
            )
            .await?;

            Ok(candles.into_iter().filter(|c| c.complete).collect())
        })
    }

    fn timeframe(&self) -> Granularity {
        self.timeframe
    }

    fn instrument(&self) -> &str {
        &self.instrument
    }

    fn poll_interval(&self) -> Duration {
        // Event-driven: we're just checking the tick channel, so poll often.
        Duration::from_secs(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rust_decimal_macros::dec;

    fn utc(year: i32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, min, sec).unwrap()
    }

    fn tick(time: DateTime<Utc>, price: Decimal) -> Tick {
        Tick { price, time }
    }

    // AC4: a hand-fed tick sequence within one H1 period, then a tick that
    // crosses the boundary, produces a completed candle with the right OHLC and
    // the correct (aligned) boundary time.
    #[test]
    fn aggregates_ticks_into_h1_candle_on_boundary_cross() {
        let mut agg = TickCandleAggregator::new(Granularity::H1);

        // Four ticks inside the 10:00 candle — none close it yet.
        assert!(agg.on_tick(&tick(utc(2024, 1, 15, 10, 0, 5), dec!(1.1000))).is_none());
        assert!(agg.on_tick(&tick(utc(2024, 1, 15, 10, 15, 0), dec!(1.1050))).is_none());
        assert!(agg.on_tick(&tick(utc(2024, 1, 15, 10, 30, 0), dec!(1.0950))).is_none());
        assert!(agg.on_tick(&tick(utc(2024, 1, 15, 10, 45, 0), dec!(1.1020))).is_none());

        // First tick in the 11:00 period closes the 10:00 candle.
        let closed = agg
            .on_tick(&tick(utc(2024, 1, 15, 11, 0, 1), dec!(1.1030)))
            .expect("a candle should close when the boundary is crossed");

        assert_eq!(closed.time, utc(2024, 1, 15, 10, 0, 0), "aligned to the hour");
        assert_eq!(closed.mid.open, dec!(1.1000));
        assert_eq!(closed.mid.high, dec!(1.1050));
        assert_eq!(closed.mid.low, dec!(1.0950));
        assert_eq!(closed.mid.close, dec!(1.1020));
        assert_eq!(closed.volume, 4);
        assert!(closed.complete);
    }

    // No candle is emitted until a boundary is actually crossed.
    #[test]
    fn no_candle_until_boundary_crossed() {
        let mut agg = TickCandleAggregator::new(Granularity::H1);
        for m in [0u32, 10, 20, 30, 40, 50, 59] {
            assert!(
                agg.on_tick(&tick(utc(2024, 1, 15, 10, m, 0), dec!(1.10))).is_none(),
                "ticks within a single period must not close a candle"
            );
        }
    }

    // AC4 (alignment): H4 uses dailyAlignment=2, so the 02:00 candle spans
    // 02:00–06:00 UTC. A tick at 06:00 closes it at start time 02:00.
    #[test]
    fn h4_uses_dailyalignment_2_boundaries() {
        let mut agg = TickCandleAggregator::new(Granularity::H4);
        assert!(agg.on_tick(&tick(utc(2024, 1, 15, 3, 0, 0), dec!(1.10))).is_none());
        assert!(agg.on_tick(&tick(utc(2024, 1, 15, 5, 0, 0), dec!(1.12))).is_none());
        let closed = agg
            .on_tick(&tick(utc(2024, 1, 15, 6, 0, 1), dec!(1.11)))
            .expect("H4 candle should close at the 06:00 boundary");
        assert_eq!(closed.time, utc(2024, 1, 15, 2, 0, 0));
        assert_eq!(closed.mid.high, dec!(1.12));
    }

    // Multiple candles closing between polls are all queued and handed out in
    // order, one per drain — nothing is dropped. The drain also reports how
    // many ticks it saw (the feed-liveness signal).
    #[test]
    fn buffer_queues_each_closed_candle_in_order() {
        let (tx, rx) = std::sync::mpsc::channel::<Tick>();
        let mut buf = TickCandleBuffer::new(rx, Granularity::H1);

        // 10:00 candle, then 11:00 candle, then a tick opening the 12:00 candle.
        tx.send(tick(utc(2024, 1, 15, 10, 0, 0), dec!(1.10))).unwrap();
        tx.send(tick(utc(2024, 1, 15, 11, 0, 0), dec!(1.11))).unwrap();
        tx.send(tick(utc(2024, 1, 15, 12, 0, 0), dec!(1.12))).unwrap();

        assert_eq!(buf.drain(), 3, "all three ticks drained and counted");

        let first = buf.pop_fresh(None).expect("first closed candle");
        assert_eq!(first.time, utc(2024, 1, 15, 10, 0, 0));
        assert_eq!(first.mid.close, dec!(1.10));

        let second = buf.pop_fresh(None).expect("second closed candle");
        assert_eq!(second.time, utc(2024, 1, 15, 11, 0, 0));

        // Only the still-forming 12:00 candle remains — nothing to hand out.
        assert_eq!(buf.drain(), 0);
        assert!(buf.pop_fresh(None).is_none());
    }

    // A disconnected channel (the hub went away) is not an error — the drain
    // just stops and whatever already closed is still delivered.
    #[test]
    fn buffer_survives_disconnected_channel() {
        let (tx, rx) = std::sync::mpsc::channel::<Tick>();
        let mut buf = TickCandleBuffer::new(rx, Granularity::H1);
        tx.send(tick(utc(2024, 1, 15, 10, 0, 0), dec!(1.10))).unwrap();
        tx.send(tick(utc(2024, 1, 15, 11, 0, 0), dec!(1.11))).unwrap();
        drop(tx); // hub gone

        assert_eq!(buf.drain(), 2);
        let candle = buf.pop_fresh(None).expect("queued candle is still delivered");
        assert_eq!(candle.time, utc(2024, 1, 15, 10, 0, 0));
        // Further drains just see nothing, no panic on the disconnected channel.
        assert_eq!(buf.drain(), 0);
        assert!(buf.pop_fresh(None).is_none());
    }

    // Dedupe guard: candle periods at or before `last_emitted` were already
    // handed to the watcher (by the REST fallback) — pop_fresh discards them
    // and returns the first strictly-newer one.
    #[test]
    fn pop_fresh_skips_periods_already_emitted() {
        let (tx, rx) = std::sync::mpsc::channel::<Tick>();
        let mut buf = TickCandleBuffer::new(rx, Granularity::H1);

        // Ticks resume after a silent stretch and rebuild 10:00 and 11:00 —
        // but the REST fallback already emitted through 10:00.
        tx.send(tick(utc(2024, 1, 15, 10, 0, 0), dec!(1.10))).unwrap();
        tx.send(tick(utc(2024, 1, 15, 11, 0, 0), dec!(1.11))).unwrap();
        tx.send(tick(utc(2024, 1, 15, 12, 0, 0), dec!(1.12))).unwrap();
        buf.drain();

        let candle = buf
            .pop_fresh(Some(utc(2024, 1, 15, 10, 0, 0)))
            .expect("11:00 is newer than last_emitted and must survive");
        assert_eq!(candle.time, utc(2024, 1, 15, 11, 0, 0));
        assert!(buf.pop_fresh(Some(utc(2024, 1, 15, 10, 0, 0))).is_none());
    }

    // REST-recovered candles queue behind whatever the ticks already closed,
    // preserving oldest-first delivery, and are themselves subject to the
    // dedupe guard on the way out.
    #[test]
    fn queued_recovery_candles_deliver_in_order_and_dedupe() {
        let (_tx, rx) = std::sync::mpsc::channel::<Tick>();
        let mut buf = TickCandleBuffer::new(rx, Granularity::H1);

        let mk = |h: u32, price: Decimal| Candle {
            time: utc(2024, 1, 15, h, 0, 0),
            mid: Ohlc { open: price, high: price, low: price, close: price },
            volume: 1,
            complete: true,
        };
        buf.queue(mk(10, dec!(1.10)));
        buf.queue(mk(11, dec!(1.11)));

        assert_eq!(
            buf.pop_fresh(Some(utc(2024, 1, 15, 10, 0, 0))).expect("11:00").time,
            utc(2024, 1, 15, 11, 0, 0),
            "the 10:00 duplicate is discarded, 11:00 delivered"
        );
        assert!(buf.pop_fresh(Some(utc(2024, 1, 15, 11, 0, 0))).is_none());
    }
}
