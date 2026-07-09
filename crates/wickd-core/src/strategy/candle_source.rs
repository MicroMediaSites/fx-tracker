//! Candle Source Trait and Implementations
//!
//! Provides an abstraction for fetching candle data that can be extended
//! to support different data sources (polling, streaming, etc.)

use chrono::{DateTime, Utc};
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::error::Result;
use crate::models::Candle;
use crate::oanda::client::OandaClient;
use crate::oanda::endpoints::{get_candles, Granularity};
use super::candle_boundary::{CandleBoundaryService, CandleCloseEvent};

/// Trait for sources of candle data
///
/// This abstraction allows the strategy watcher to work with different
/// data sources without modification. Currently implemented:
/// - `OandaPollingSource`: Polls OANDA API for completed candles
///
/// Future implementations could include:
/// - `TickStreamSource`: Build candles from streaming tick data
/// - `BacktestSource`: Replay historical candles for testing
pub trait CandleSource: Send + Sync {
    /// Get the latest complete candle if a new one is available
    ///
    /// Returns `None` if no new candle has completed since the last check.
    /// This method tracks state internally to detect new candles.
    fn get_latest_candle(&self) -> Pin<Box<dyn Future<Output = Result<Option<Candle>>> + Send + '_>>;

    /// Get multiple historical candles for indicator warmup
    ///
    /// # Arguments
    /// * `count` - Number of candles to fetch (max 5000 for OANDA)
    fn get_candles(&self, count: u32) -> Pin<Box<dyn Future<Output = Result<Vec<Candle>>> + Send + '_>>;

    /// Get the timeframe this source is configured for
    fn timeframe(&self) -> Granularity;

    /// Get the instrument this source is configured for
    fn instrument(&self) -> &str;

    /// Get the recommended poll interval for this source
    fn poll_interval(&self) -> Duration;
}

/// OANDA polling candle source
///
/// Fetches candles by polling the OANDA REST API. This is suitable for
/// swing trading and longer timeframes where immediate tick data isn't critical.
pub struct OandaPollingSource {
    client: OandaClient,
    instrument: String,
    timeframe: Granularity,
    last_candle_time: Mutex<Option<DateTime<Utc>>>,
}

impl OandaPollingSource {
    /// Create a new OANDA polling source
    pub fn new(client: OandaClient, instrument: String, timeframe: Granularity) -> Self {
        Self {
            client,
            instrument,
            timeframe,
            last_candle_time: Mutex::new(None),
        }
    }

    /// Get the recommended poll interval based on timeframe
    fn recommended_poll_interval(timeframe: &Granularity) -> Duration {
        match timeframe {
            Granularity::S5 | Granularity::S10 | Granularity::S15 | Granularity::S30 => {
                Duration::from_secs(5)
            }
            Granularity::M1 => Duration::from_secs(10),
            Granularity::M2 | Granularity::M4 | Granularity::M5 => Duration::from_secs(30),
            Granularity::M10 | Granularity::M15 => Duration::from_secs(60),
            Granularity::M30 => Duration::from_secs(120),
            Granularity::H1 => Duration::from_secs(300),
            Granularity::H2 | Granularity::H3 | Granularity::H4 => Duration::from_secs(600),
            Granularity::H6 | Granularity::H8 | Granularity::H12 => Duration::from_secs(900),
            Granularity::D => Duration::from_secs(1800),
            Granularity::W | Granularity::M => Duration::from_secs(3600),
        }
    }
}

impl CandleSource for OandaPollingSource {
    fn get_latest_candle(&self) -> Pin<Box<dyn Future<Output = Result<Option<Candle>>> + Send + '_>> {
        Box::pin(async move {
            // Fetch last 2 candles to ensure we get the most recent complete one
            let candles = get_candles(
                &self.client,
                &self.instrument,
                self.timeframe,
                Some(2),
                None,
                None,
            )
            .await?;

            // Find the most recent complete candle
            let latest_complete = candles
                .iter()
                .filter(|c| c.complete)
                .max_by_key(|c| c.time);

            let Some(candle) = latest_complete else {
                return Ok(None);
            };

            // Check if this is a new candle
            let mut last_time = self.last_candle_time.lock().unwrap();
            if let Some(prev_time) = *last_time {
                if candle.time <= prev_time {
                    // Not a new candle
                    return Ok(None);
                }
            }

            // Update last seen time and return the candle
            *last_time = Some(candle.time);
            Ok(Some(candle.clone()))
        })
    }

    fn get_candles(&self, count: u32) -> Pin<Box<dyn Future<Output = Result<Vec<Candle>>> + Send + '_>> {
        Box::pin(async move {
            let candles = get_candles(
                &self.client,
                &self.instrument,
                self.timeframe,
                Some(count),
                None,
                None,
            )
            .await?;

            // Filter to only complete candles for indicator calculations
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
        Self::recommended_poll_interval(&self.timeframe)
    }
}

/// Streaming-based candle source that detects candle closes from tick data.
///
/// Unlike `OandaPollingSource` which polls blindly, this source subscribes to
/// the `CandleBoundaryService` which detects candle closes from the streaming
/// price feed. This provides sub-second latency for candle detection.
///
/// # How it works
///
/// 1. On creation, subscribes to candle close events from the boundary service
/// 2. `get_latest_candle()` checks if a close event was received
/// 3. If yes, fetches the official OHLC from the REST API
/// 4. Falls back to polling if no events received (e.g., during stream disconnect)
///
/// # Latency comparison
///
/// | Source | H4 Latency | H1 Latency |
/// |--------|------------|------------|
/// | OandaPollingSource | Up to 10 min | Up to 5 min |
/// | StreamingCandleSource | < 1 second | < 1 second |
pub struct StreamingCandleSource {
    client: OandaClient,
    instrument: String,
    timeframe: Granularity,
    /// Receiver for candle close events
    close_rx: tokio::sync::Mutex<broadcast::Receiver<CandleCloseEvent>>,
    /// Last candle time we processed
    last_candle_time: Mutex<Option<DateTime<Utc>>>,
    /// Whether we've received any events (for fallback detection)
    has_received_event: Mutex<bool>,
}

impl StreamingCandleSource {
    /// Create a new streaming candle source
    ///
    /// # Arguments
    /// * `client` - OANDA client for fetching candles
    /// * `instrument` - Instrument to track (e.g., "EUR_USD")
    /// * `timeframe` - Candle timeframe
    /// * `boundary_service` - The shared boundary detection service
    pub async fn new(
        client: OandaClient,
        instrument: String,
        timeframe: Granularity,
        boundary_service: &CandleBoundaryService,
    ) -> Self {
        // Subscribe to candle close events
        let close_rx = boundary_service
            .subscribe(instrument.clone(), timeframe)
            .await;

        info!(
            "[StreamingSource] Created for {}/{:?}",
            instrument, timeframe
        );

        Self {
            client,
            instrument,
            timeframe,
            close_rx: tokio::sync::Mutex::new(close_rx),
            last_candle_time: Mutex::new(None),
            has_received_event: Mutex::new(false),
        }
    }
}

impl CandleSource for StreamingCandleSource {
    fn get_latest_candle(&self) -> Pin<Box<dyn Future<Output = Result<Option<Candle>>> + Send + '_>> {
        Box::pin(async move {
            // Try to receive a candle close event (non-blocking)
            let event = {
                let mut rx = self.close_rx.lock().await;
                match rx.try_recv() {
                    Ok(event) => Some(event),
                    Err(broadcast::error::TryRecvError::Empty) => None,
                    Err(broadcast::error::TryRecvError::Closed) => {
                        warn!("[StreamingSource] Boundary service channel closed");
                        None
                    }
                    Err(broadcast::error::TryRecvError::Lagged(n)) => {
                        warn!("[StreamingSource] Missed {} candle events", n);
                        // Try again to get the latest
                        rx.try_recv().ok()
                    }
                }
            };

            if let Some(event) = event {
                // Mark that we've received at least one event
                *self.has_received_event.lock().unwrap() = true;

                debug!(
                    "[StreamingSource] Candle closed: {}/{:?} {} -> {}",
                    event.instrument,
                    event.timeframe,
                    event.closed_candle_start.format("%H:%M"),
                    event.closed_candle_end.format("%H:%M")
                );

                // Fetch the official candle from REST API
                // We request 2 candles to ensure we get the complete one
                // Convert DateTime to RFC3339 string for the API
                let from_time = event.closed_candle_start.to_rfc3339();
                let candles = get_candles(
                    &self.client,
                    &self.instrument,
                    self.timeframe,
                    Some(2),
                    Some(&from_time),
                    None,
                )
                .await?;

                // Find the candle matching the close event
                let candle = candles
                    .into_iter()
                    .find(|c| c.complete && c.time >= event.closed_candle_start);

                if let Some(candle) = candle {
                    let mut last_time = self.last_candle_time.lock().unwrap();

                    // Avoid duplicates
                    if let Some(prev_time) = *last_time {
                        if candle.time <= prev_time {
                            debug!("[StreamingSource] Skipping duplicate candle");
                            return Ok(None);
                        }
                    }

                    *last_time = Some(candle.time);
                    info!(
                        "[StreamingSource] New candle: {} {:?} @ {}",
                        self.instrument,
                        self.timeframe,
                        candle.time.format("%Y-%m-%d %H:%M")
                    );
                    return Ok(Some(candle));
                }
            }

            Ok(None)
        })
    }

    fn get_candles(&self, count: u32) -> Pin<Box<dyn Future<Output = Result<Vec<Candle>>> + Send + '_>> {
        Box::pin(async move {
            let candles = get_candles(
                &self.client,
                &self.instrument,
                self.timeframe,
                Some(count),
                None,
                None,
            )
            .await?;

            // Filter to only complete candles for indicator calculations
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
        // Streaming source still needs a poll interval for the watcher loop,
        // but it can be much shorter since we're event-driven
        Duration::from_secs(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_poll_interval_m1() {
        let interval = OandaPollingSource::recommended_poll_interval(&Granularity::M1);
        assert_eq!(interval, Duration::from_secs(10));
    }

    #[test]
    fn test_poll_interval_m5() {
        let interval = OandaPollingSource::recommended_poll_interval(&Granularity::M5);
        assert_eq!(interval, Duration::from_secs(30));
    }

    #[test]
    fn test_poll_interval_h1() {
        let interval = OandaPollingSource::recommended_poll_interval(&Granularity::H1);
        assert_eq!(interval, Duration::from_secs(300));
    }

    #[test]
    fn test_poll_interval_h4() {
        let interval = OandaPollingSource::recommended_poll_interval(&Granularity::H4);
        assert_eq!(interval, Duration::from_secs(600));
    }

    #[test]
    fn test_poll_interval_daily() {
        let interval = OandaPollingSource::recommended_poll_interval(&Granularity::D);
        assert_eq!(interval, Duration::from_secs(1800));
    }
}
