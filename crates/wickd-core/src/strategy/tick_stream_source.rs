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

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc::Receiver;
use std::sync::Mutex;
use std::time::Duration;

use super::candle_boundary::CandleBoundaryDetector;
use super::candle_source::CandleSource;
use crate::error::Result;
use crate::models::{Candle, Ohlc};
use crate::oanda::client::OandaClient;
use crate::oanda::endpoints::{get_candles, Granularity};

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
    /// candles that closed, then pop the oldest completed candle (if any).
    /// Non-blocking: a disconnected channel (hub gone) just stops the drain.
    fn poll(&mut self) -> Option<Candle> {
        // `try_recv` returns `Err` for both an empty and a disconnected (hub
        // gone) channel — either way we stop draining and hand out what we have.
        while let Ok(tick) = self.rx.try_recv() {
            if let Some(candle) = self.aggregator.on_tick(&tick) {
                self.ready.push_back(candle);
            }
        }
        self.ready.pop_front()
    }
}

/// A [`CandleSource`] backed by a live tick feed (e.g. `wickd stream`'s socket
/// hub). Ticks arrive over `rx`; historical warmup falls back to OANDA REST.
pub struct TickStreamSource {
    client: OandaClient,
    instrument: String,
    timeframe: Granularity,
    buffer: Mutex<TickCandleBuffer>,
}

impl TickStreamSource {
    /// Build a source for `instrument`/`timeframe` that aggregates the ticks
    /// delivered on `rx`. `client` is used only for the one-off REST warmup
    /// fetch (the live feed carries no history).
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
        }
    }
}

impl CandleSource for TickStreamSource {
    fn get_latest_candle(&self) -> Pin<Box<dyn Future<Output = Result<Option<Candle>>> + Send + '_>> {
        Box::pin(async move {
            // Pure, synchronous drain — no await while the lock is held. Don't
            // panic on the daemon path: a poisoned mutex (another thread
            // panicked mid-drain) surfaces as a structured error, not a crash.
            let candle = self
                .buffer
                .lock()
                .map_err(|_| crate::error::Error::Strategy("tick buffer mutex poisoned".to_string()))?
                .poll();
            Ok(candle)
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
    // order, one per drain — nothing is dropped.
    #[test]
    fn buffer_queues_each_closed_candle_in_order() {
        let (tx, rx) = std::sync::mpsc::channel::<Tick>();
        let mut buf = TickCandleBuffer::new(rx, Granularity::H1);

        // 10:00 candle, then 11:00 candle, then a tick opening the 12:00 candle.
        tx.send(tick(utc(2024, 1, 15, 10, 0, 0), dec!(1.10))).unwrap();
        tx.send(tick(utc(2024, 1, 15, 11, 0, 0), dec!(1.11))).unwrap();
        tx.send(tick(utc(2024, 1, 15, 12, 0, 0), dec!(1.12))).unwrap();

        let first = buf.poll().expect("first closed candle");
        assert_eq!(first.time, utc(2024, 1, 15, 10, 0, 0));
        assert_eq!(first.mid.close, dec!(1.10));

        let second = buf.poll().expect("second closed candle");
        assert_eq!(second.time, utc(2024, 1, 15, 11, 0, 0));

        // Only the still-forming 12:00 candle remains — nothing to hand out.
        assert!(buf.poll().is_none());
    }

    // A disconnected channel (the hub went away) is not an error — poll just
    // stops draining and returns whatever it already has.
    #[test]
    fn buffer_survives_disconnected_channel() {
        let (tx, rx) = std::sync::mpsc::channel::<Tick>();
        let mut buf = TickCandleBuffer::new(rx, Granularity::H1);
        tx.send(tick(utc(2024, 1, 15, 10, 0, 0), dec!(1.10))).unwrap();
        tx.send(tick(utc(2024, 1, 15, 11, 0, 0), dec!(1.11))).unwrap();
        drop(tx); // hub gone

        let candle = buf.poll().expect("queued candle is still delivered");
        assert_eq!(candle.time, utc(2024, 1, 15, 10, 0, 0));
        // Further polls just return None, no panic on the disconnected channel.
        assert!(buf.poll().is_none());
    }
}
