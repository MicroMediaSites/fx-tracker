//! Candle Boundary Detection
//!
//! Detects when candle periods close based on incoming tick timestamps.
//! Uses OANDA's candle alignment settings (dailyAlignment=2, alignmentTimezone=UTC)
//! to correctly identify candle boundaries.
//!
//! # OANDA Candle Alignment
//!
//! With dailyAlignment=2 and UTC timezone:
//! - H4 candles: 02:00, 06:00, 10:00, 14:00, 18:00, 22:00 UTC
//! - H1 candles: Every hour on the hour
//! - Daily candles: Close at 21:00 UTC (NY close)

use chrono::{DateTime, Datelike, Duration, Timelike, Utc, Weekday};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info, trace};

use crate::oanda::endpoints::Granularity;

/// Check if forex markets are currently closed.
///
/// Forex markets are closed from Friday 21:00 UTC (5pm ET) to Sunday 21:00 UTC (5pm ET).
/// This uses a simplified UTC-based check that works year-round.
///
/// Note: This is an approximation. During DST, the actual close time shifts by an hour,
/// but since OANDA's daily candle close aligns with 5pm NY time, using 21:00 UTC
/// provides good coverage for both cases.
///
/// # Arguments
/// * `time` - The time to check
///
/// # Returns
/// `true` if forex markets are closed, `false` if open
pub fn is_forex_market_closed(time: DateTime<Utc>) -> bool {
    let weekday = time.weekday();
    let hour = time.hour();

    match weekday {
        // Saturday: Always closed
        Weekday::Sat => true,
        // Sunday: Closed until 21:00 UTC (5pm ET when markets reopen)
        Weekday::Sun => hour < 21,
        // Friday: Closed after 21:00 UTC (5pm ET when markets close)
        Weekday::Fri => hour >= 21,
        // Monday-Thursday: Always open
        _ => false,
    }
}

/// Event emitted when a candle period closes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandleCloseEvent {
    /// The instrument (e.g., "EUR_USD")
    pub instrument: String,
    /// The timeframe of the closed candle
    pub timeframe: Granularity,
    /// Start time of the closed candle
    pub closed_candle_start: DateTime<Utc>,
    /// End time of the closed candle (= start of next candle)
    pub closed_candle_end: DateTime<Utc>,
}

/// Tracks candle boundaries and detects when candles close.
///
/// For each registered (instrument, timeframe) pair, tracks the current
/// candle period and detects when incoming ticks cross into a new period.
#[derive(Debug, Default)]
pub struct CandleBoundaryDetector {
    /// Maps (instrument, timeframe) -> current candle start time
    candle_starts: HashMap<(String, Granularity), DateTime<Utc>>,
}

impl CandleBoundaryDetector {
    /// Create a new boundary detector
    pub fn new() -> Self {
        Self {
            candle_starts: HashMap::new(),
        }
    }

    /// Register an instrument/timeframe pair to track
    ///
    /// Initializes tracking with the current candle period based on the current time.
    pub fn register(&mut self, instrument: String, timeframe: Granularity) {
        let current_start = Self::current_candle_start(Utc::now(), timeframe);
        self.candle_starts
            .insert((instrument, timeframe), current_start);
    }

    /// Unregister an instrument/timeframe pair
    pub fn unregister(&mut self, instrument: &str, timeframe: Granularity) {
        self.candle_starts.remove(&(instrument.to_string(), timeframe));
    }

    /// Check if an instrument/timeframe pair is registered
    pub fn is_registered(&self, instrument: &str, timeframe: Granularity) -> bool {
        self.candle_starts
            .contains_key(&(instrument.to_string(), timeframe))
    }

    /// Get all registered pairs
    pub fn registered_pairs(&self) -> Vec<(String, Granularity)> {
        self.candle_starts.keys().cloned().collect()
    }

    /// Process a tick and return any candle close events
    ///
    /// Checks all registered timeframes for the given instrument to see
    /// if the tick timestamp has crossed into a new candle period.
    pub fn on_tick(&mut self, instrument: &str, tick_time: DateTime<Utc>) -> Vec<CandleCloseEvent> {
        let mut events = Vec::new();

        // Collect keys that match this instrument
        let matching_keys: Vec<(String, Granularity)> = self
            .candle_starts
            .keys()
            .filter(|(inst, _)| inst == instrument)
            .cloned()
            .collect();

        for key in matching_keys {
            let timeframe = key.1;

            if let Some(current_start) = self.candle_starts.get_mut(&key) {
                let next_start = Self::next_candle_start(*current_start, timeframe);

                if tick_time >= next_start {
                    // Candle closed!
                    events.push(CandleCloseEvent {
                        instrument: instrument.to_string(),
                        timeframe,
                        closed_candle_start: *current_start,
                        closed_candle_end: next_start,
                    });

                    // Update to the current candle period for this tick
                    // (handles case where multiple candles may have passed)
                    *current_start = Self::current_candle_start(tick_time, timeframe);
                }
            }
        }

        events
    }

    /// Calculate the start time of the current candle period for a given time.
    ///
    /// Uses OANDA's candle alignment settings:
    /// - dailyAlignment = 3 (hour offset for daily candle close)
    /// - alignmentTimezone = UTC
    pub fn current_candle_start(now: DateTime<Utc>, timeframe: Granularity) -> DateTime<Utc> {
        let base = now
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap();

        match timeframe {
            // Sub-minute timeframes
            Granularity::S5 => {
                let secs = now.second();
                let aligned_secs = (secs / 5) * 5;
                base.with_second(aligned_secs).unwrap()
            }
            Granularity::S10 => {
                let secs = now.second();
                let aligned_secs = (secs / 10) * 10;
                base.with_second(aligned_secs).unwrap()
            }
            Granularity::S15 => {
                let secs = now.second();
                let aligned_secs = (secs / 15) * 15;
                base.with_second(aligned_secs).unwrap()
            }
            Granularity::S30 => {
                let secs = now.second();
                let aligned_secs = (secs / 30) * 30;
                base.with_second(aligned_secs).unwrap()
            }

            // Minute timeframes
            Granularity::M1 => base.with_minute(now.minute()).unwrap(),
            Granularity::M2 => {
                let mins = now.minute();
                let aligned_mins = (mins / 2) * 2;
                base.with_minute(aligned_mins).unwrap()
            }
            Granularity::M4 => {
                let mins = now.minute();
                let aligned_mins = (mins / 4) * 4;
                base.with_minute(aligned_mins).unwrap()
            }
            Granularity::M5 => {
                let mins = now.minute();
                let aligned_mins = (mins / 5) * 5;
                base.with_minute(aligned_mins).unwrap()
            }
            Granularity::M10 => {
                let mins = now.minute();
                let aligned_mins = (mins / 10) * 10;
                base.with_minute(aligned_mins).unwrap()
            }
            Granularity::M15 => {
                let mins = now.minute();
                let aligned_mins = (mins / 15) * 15;
                base.with_minute(aligned_mins).unwrap()
            }
            Granularity::M30 => {
                let mins = now.minute();
                let aligned_mins = (mins / 30) * 30;
                base.with_minute(aligned_mins).unwrap()
            }

            // Hourly timeframes
            Granularity::H1 => base.with_minute(0).unwrap(),
            Granularity::H2 => {
                let hour = now.hour();
                let aligned_hour = (hour / 2) * 2;
                base.with_minute(0).unwrap().with_hour(aligned_hour).unwrap()
            }
            Granularity::H3 => {
                // H3 aligned to 02:00 UTC base (dailyAlignment=2)
                // Candles at: 02, 05, 08, 11, 14, 17, 20, 23
                let hour = now.hour();
                let shifted = if hour >= 2 { hour - 2 } else { hour + 22 };
                let aligned_shifted = (shifted / 3) * 3;
                let aligned_hour = (aligned_shifted + 2) % 24;
                let result = base.with_minute(0).unwrap().with_hour(aligned_hour).unwrap();
                if hour < 2 && aligned_hour >= 22 {
                    result - Duration::days(1)
                } else {
                    result
                }
            }
            Granularity::H4 => {
                // H4 aligned to 02:00 UTC base (dailyAlignment=2)
                // Candles at: 02, 06, 10, 14, 18, 22
                let hour = now.hour();
                let aligned_hour = Self::h4_candle_start_hour(hour);
                let result = base.with_minute(0).unwrap().with_hour(aligned_hour).unwrap();

                // Handle wraparound to previous day for hours 0-1
                if hour < 2 {
                    result - Duration::days(1)
                } else {
                    result
                }
            }
            Granularity::H6 => {
                // H6 aligned to dailyAlignment=2
                // Candles at: 02, 08, 14, 20
                let hour = now.hour();
                let aligned_hour = if hour < 2 {
                    20 // Previous day's 20:00
                } else if hour < 8 {
                    2
                } else if hour < 14 {
                    8
                } else if hour < 20 {
                    14
                } else {
                    20
                };
                let result = base.with_minute(0).unwrap().with_hour(aligned_hour).unwrap();

                if hour < 2 {
                    result - Duration::days(1)
                } else {
                    result
                }
            }
            Granularity::H8 => {
                // H8 aligned to dailyAlignment=2
                // Candles at: 02, 10, 18
                let hour = now.hour();
                let aligned_hour = if hour < 2 {
                    18 // Previous day's 18:00
                } else if hour < 10 {
                    2
                } else if hour < 18 {
                    10
                } else {
                    18
                };
                let result = base.with_minute(0).unwrap().with_hour(aligned_hour).unwrap();

                if hour < 2 {
                    result - Duration::days(1)
                } else {
                    result
                }
            }
            Granularity::H12 => {
                // H12 aligned to dailyAlignment=2
                // Candles at: 02, 14
                let hour = now.hour();
                let aligned_hour = if hour < 2 {
                    14 // Previous day's 14:00
                } else if hour < 14 {
                    2
                } else {
                    14
                };
                let result = base.with_minute(0).unwrap().with_hour(aligned_hour).unwrap();

                if hour < 2 {
                    result - Duration::days(1)
                } else {
                    result
                }
            }

            // Daily - closes at 21:00 UTC (NY close, 5pm ET)
            Granularity::D => {
                let hour = now.hour();
                let result = base
                    .with_minute(0)
                    .unwrap()
                    .with_hour(21)
                    .unwrap();

                // If before 21:00, the current daily candle started at previous day's 21:00
                if hour < 21 {
                    result - Duration::days(1)
                } else {
                    result
                }
            }

            // Weekly - starts Sunday 21:00 UTC
            Granularity::W => {
                use chrono::Datelike;
                let weekday = now.weekday().num_days_from_sunday();
                let hour = now.hour();

                // Calculate days to subtract to get to Sunday
                let days_since_sunday = if weekday == 0 && hour < 21 {
                    7 // Previous Sunday
                } else {
                    weekday as i64
                };

                let sunday = now.date_naive() - chrono::Duration::days(days_since_sunday);
                sunday
                    .and_hms_opt(21, 0, 0)
                    .unwrap()
                    .and_utc()
            }

            // Monthly - starts first of month at 21:00 UTC
            Granularity::M => {
                use chrono::Datelike;
                let day = now.day();
                let hour = now.hour();

                if day == 1 && hour < 21 {
                    // Still in previous month's candle
                    let prev_month = if now.month() == 1 {
                        now.with_year(now.year() - 1)
                            .unwrap()
                            .with_month(12)
                            .unwrap()
                    } else {
                        now.with_month(now.month() - 1).unwrap()
                    };
                    prev_month
                        .with_day(1)
                        .unwrap()
                        .with_hour(21)
                        .unwrap()
                        .with_minute(0)
                        .unwrap()
                        .with_second(0)
                        .unwrap()
                        .with_nanosecond(0)
                        .unwrap()
                } else {
                    now.with_day(1)
                        .unwrap()
                        .with_hour(21)
                        .unwrap()
                        .with_minute(0)
                        .unwrap()
                        .with_second(0)
                        .unwrap()
                        .with_nanosecond(0)
                        .unwrap()
                }
            }
        }
    }

    /// Get the hour when the H4 candle starts for a given hour
    ///
    /// With dailyAlignment=2, H4 candles start at: 02, 06, 10, 14, 18, 22
    fn h4_candle_start_hour(hour: u32) -> u32 {
        match hour {
            0..=1 => 22,   // Previous day's 22:00 candle
            2..=5 => 2,
            6..=9 => 6,
            10..=13 => 10,
            14..=17 => 14,
            18..=21 => 18,
            _ => 22,
        }
    }

    /// Calculate the start time of the next candle period
    pub fn next_candle_start(current_start: DateTime<Utc>, timeframe: Granularity) -> DateTime<Utc> {
        current_start + Self::candle_duration(timeframe)
    }

    /// Get the duration of a candle for a given timeframe
    pub fn candle_duration(timeframe: Granularity) -> Duration {
        match timeframe {
            Granularity::S5 => Duration::seconds(5),
            Granularity::S10 => Duration::seconds(10),
            Granularity::S15 => Duration::seconds(15),
            Granularity::S30 => Duration::seconds(30),
            Granularity::M1 => Duration::minutes(1),
            Granularity::M2 => Duration::minutes(2),
            Granularity::M4 => Duration::minutes(4),
            Granularity::M5 => Duration::minutes(5),
            Granularity::M10 => Duration::minutes(10),
            Granularity::M15 => Duration::minutes(15),
            Granularity::M30 => Duration::minutes(30),
            Granularity::H1 => Duration::hours(1),
            Granularity::H2 => Duration::hours(2),
            Granularity::H3 => Duration::hours(3),
            Granularity::H4 => Duration::hours(4),
            Granularity::H6 => Duration::hours(6),
            Granularity::H8 => Duration::hours(8),
            Granularity::H12 => Duration::hours(12),
            Granularity::D => Duration::days(1),
            Granularity::W => Duration::weeks(1),
            // Monthly is approximate - actual calculation handles month boundaries
            Granularity::M => Duration::days(30),
        }
    }

    /// Calculate time until the next candle close for a given timeframe
    pub fn time_until_close(now: DateTime<Utc>, timeframe: Granularity) -> Duration {
        let current_start = Self::current_candle_start(now, timeframe);
        let next_start = Self::next_candle_start(current_start, timeframe);
        next_start - now
    }
}

/// Shared service for candle boundary detection.
///
/// Provides thread-safe access to the boundary detector and broadcasts
/// candle close events to interested subscribers (strategy watchers).
///
/// # Architecture
///
/// ```text
/// ┌─────────────────────────────────────────────────────────────────┐
/// │                    PriceStreamManager                            │
/// │        (calls on_price_tick for each incoming tick)             │
/// └─────────────────────────────────────────────────────────────────┘
///                               │
///                               ▼
/// ┌─────────────────────────────────────────────────────────────────┐
/// │                  CandleBoundaryService                           │
/// │  - CandleBoundaryDetector (wrapped in RwLock)                   │
/// │  - Broadcast channels per (instrument, timeframe)               │
/// │                                                                  │
/// │  on_price_tick(instrument, time):                               │
/// │    1. Detector checks if any candle closed                      │
/// │    2. If closed, broadcast event to subscribers                 │
/// └─────────────────────────────────────────────────────────────────┘
///                               │
///               ┌───────────────┼───────────────┐
///               ▼               ▼               ▼
///        Watcher EUR/USD   Watcher GBP/USD   Watcher EUR/JPY
///        (H4 subscriber)   (H1 subscriber)   (D subscriber)
/// ```
pub struct CandleBoundaryService {
    detector: RwLock<CandleBoundaryDetector>,
    /// Broadcast senders for each (instrument, timeframe) pair
    /// Using broadcast channels so multiple watchers can subscribe to the same pair
    senders: RwLock<HashMap<(String, Granularity), broadcast::Sender<CandleCloseEvent>>>,
}

impl CandleBoundaryService {
    /// Create a new boundary service
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            detector: RwLock::new(CandleBoundaryDetector::new()),
            senders: RwLock::new(HashMap::new()),
        })
    }

    /// Subscribe to candle close events for an instrument/timeframe pair.
    ///
    /// Returns a receiver that will receive events when candles close.
    /// The detector automatically registers the pair if not already registered.
    pub async fn subscribe(
        &self,
        instrument: String,
        timeframe: Granularity,
    ) -> broadcast::Receiver<CandleCloseEvent> {
        let key = (instrument.clone(), timeframe);

        // Register with detector
        {
            let mut detector = self.detector.write().await;
            if !detector.is_registered(&instrument, timeframe) {
                detector.register(instrument.clone(), timeframe);
                info!(
                    "[CandleBoundary] Registered {}/{:?} for boundary detection",
                    instrument, timeframe
                );
            }
        }

        // Get or create broadcast channel
        let mut senders = self.senders.write().await;
        if let Some(sender) = senders.get(&key) {
            sender.subscribe()
        } else {
            // Create new channel with reasonable buffer
            let (tx, rx) = broadcast::channel(16);
            senders.insert(key, tx);
            rx
        }
    }

    /// Unsubscribe from candle close events.
    ///
    /// Note: This doesn't actually remove the sender since broadcast::Receiver
    /// is dropped when the subscriber drops it. The detector continues tracking
    /// even if no subscribers exist (low overhead).
    pub async fn unsubscribe(&self, instrument: &str, timeframe: Granularity) {
        // For now, we keep the detector registration even when all subscribers leave
        // This is fine since the overhead is minimal and avoids race conditions
        debug!(
            "[CandleBoundary] Unsubscribe requested for {}/{:?} (detector still active)",
            instrument, timeframe
        );
    }

    /// Called by PriceStreamManager when a price tick arrives.
    ///
    /// Checks if any candle boundaries were crossed and broadcasts events
    /// to all subscribers.
    ///
    /// Skips processing when forex markets are closed (Friday 21:00 UTC to Sunday 21:00 UTC)
    /// to reduce unnecessary computation during weekends.
    pub async fn on_price_tick(&self, instrument: &str, tick_time: DateTime<Utc>) {
        // Skip boundary detection when forex markets are closed
        if is_forex_market_closed(tick_time) {
            trace!(
                "[CandleBoundary] Skipping tick for {} - forex market closed",
                instrument
            );
            return;
        }

        // Check for candle closes
        let events = {
            let mut detector = self.detector.write().await;
            detector.on_tick(instrument, tick_time)
        };

        if events.is_empty() {
            return;
        }

        // Broadcast each close event
        let senders = self.senders.read().await;
        for event in events {
            let key = (event.instrument.clone(), event.timeframe);
            if let Some(sender) = senders.get(&key) {
                match sender.send(event.clone()) {
                    Ok(count) => {
                        info!(
                            "[CandleBoundary] Candle closed: {}/{:?} {} -> {} (notified {} subscribers)",
                            event.instrument,
                            event.timeframe,
                            event.closed_candle_start.format("%Y-%m-%d %H:%M"),
                            event.closed_candle_end.format("%H:%M"),
                            count
                        );
                    }
                    Err(_) => {
                        // No receivers - that's fine, just log at debug level
                        debug!(
                            "[CandleBoundary] Candle closed but no subscribers: {}/{:?}",
                            event.instrument, event.timeframe
                        );
                    }
                }
            }
        }
    }

    /// Get the time until the next candle close for a given timeframe
    pub fn time_until_close(timeframe: Granularity) -> Duration {
        CandleBoundaryDetector::time_until_close(Utc::now(), timeframe)
    }

    /// Get all registered (instrument, timeframe) pairs
    pub async fn registered_pairs(&self) -> Vec<(String, Granularity)> {
        self.detector.read().await.registered_pairs()
    }

    /// Check if a pair is registered
    pub async fn is_registered(&self, instrument: &str, timeframe: Granularity) -> bool {
        self.detector.read().await.is_registered(instrument, timeframe)
    }
}

impl Default for CandleBoundaryService {
    fn default() -> Self {
        Self {
            detector: RwLock::new(CandleBoundaryDetector::new()),
            senders: RwLock::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(year: i32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, min, sec)
            .unwrap()
    }

    #[test]
    fn test_h1_candle_start() {
        // 10:35:42 should give 10:00:00
        let now = utc(2024, 1, 15, 10, 35, 42);
        let start = CandleBoundaryDetector::current_candle_start(now, Granularity::H1);
        assert_eq!(start, utc(2024, 1, 15, 10, 0, 0));
    }

    #[test]
    fn test_h4_candle_start_hour_3() {
        // 03:30 should give 02:00 (H4 boundaries: 02, 06, 10, 14, 18, 22)
        let now = utc(2024, 1, 15, 3, 30, 0);
        let start = CandleBoundaryDetector::current_candle_start(now, Granularity::H4);
        assert_eq!(start, utc(2024, 1, 15, 2, 0, 0));
    }

    #[test]
    fn test_h4_candle_start_hour_7() {
        // 09:15 should give 06:00
        let now = utc(2024, 1, 15, 9, 15, 0);
        let start = CandleBoundaryDetector::current_candle_start(now, Granularity::H4);
        assert_eq!(start, utc(2024, 1, 15, 6, 0, 0));
    }

    #[test]
    fn test_h4_candle_start_hour_11() {
        // 13:45 should give 10:00
        let now = utc(2024, 1, 15, 13, 45, 0);
        let start = CandleBoundaryDetector::current_candle_start(now, Granularity::H4);
        assert_eq!(start, utc(2024, 1, 15, 10, 0, 0));
    }

    #[test]
    fn test_h4_candle_start_hour_15() {
        // 17:00 should give 14:00
        let now = utc(2024, 1, 15, 17, 0, 0);
        let start = CandleBoundaryDetector::current_candle_start(now, Granularity::H4);
        assert_eq!(start, utc(2024, 1, 15, 14, 0, 0));
    }

    #[test]
    fn test_h4_candle_start_hour_19() {
        // 21:30 should give 18:00
        let now = utc(2024, 1, 15, 21, 30, 0);
        let start = CandleBoundaryDetector::current_candle_start(now, Granularity::H4);
        assert_eq!(start, utc(2024, 1, 15, 18, 0, 0));
    }

    #[test]
    fn test_h4_candle_start_hour_23() {
        // 23:30 should give 22:00 same day
        let now = utc(2024, 1, 15, 23, 30, 0);
        let start = CandleBoundaryDetector::current_candle_start(now, Granularity::H4);
        assert_eq!(start, utc(2024, 1, 15, 22, 0, 0));
    }

    #[test]
    fn test_h4_candle_start_before_03() {
        // 01:30 on Jan 15 should give 22:00 on Jan 14
        let now = utc(2024, 1, 15, 1, 30, 0);
        let start = CandleBoundaryDetector::current_candle_start(now, Granularity::H4);
        assert_eq!(start, utc(2024, 1, 14, 22, 0, 0));
    }

    #[test]
    fn test_m15_candle_start() {
        // 10:37:42 should give 10:30:00
        let now = utc(2024, 1, 15, 10, 37, 42);
        let start = CandleBoundaryDetector::current_candle_start(now, Granularity::M15);
        assert_eq!(start, utc(2024, 1, 15, 10, 30, 0));
    }

    #[test]
    fn test_m5_candle_start() {
        // 10:37:42 should give 10:35:00
        let now = utc(2024, 1, 15, 10, 37, 42);
        let start = CandleBoundaryDetector::current_candle_start(now, Granularity::M5);
        assert_eq!(start, utc(2024, 1, 15, 10, 35, 0));
    }

    #[test]
    fn test_daily_candle_start_after_21() {
        // 22:30 on Jan 15 should give 21:00 on Jan 15
        let now = utc(2024, 1, 15, 22, 30, 0);
        let start = CandleBoundaryDetector::current_candle_start(now, Granularity::D);
        assert_eq!(start, utc(2024, 1, 15, 21, 0, 0));
    }

    #[test]
    fn test_daily_candle_start_before_21() {
        // 10:30 on Jan 15 should give 21:00 on Jan 14
        let now = utc(2024, 1, 15, 10, 30, 0);
        let start = CandleBoundaryDetector::current_candle_start(now, Granularity::D);
        assert_eq!(start, utc(2024, 1, 14, 21, 0, 0));
    }

    #[test]
    fn test_candle_duration() {
        assert_eq!(
            CandleBoundaryDetector::candle_duration(Granularity::M1),
            Duration::minutes(1)
        );
        assert_eq!(
            CandleBoundaryDetector::candle_duration(Granularity::H1),
            Duration::hours(1)
        );
        assert_eq!(
            CandleBoundaryDetector::candle_duration(Granularity::H4),
            Duration::hours(4)
        );
        assert_eq!(
            CandleBoundaryDetector::candle_duration(Granularity::D),
            Duration::days(1)
        );
    }

    #[test]
    fn test_next_candle_start() {
        let start = utc(2024, 1, 15, 7, 0, 0);
        let next = CandleBoundaryDetector::next_candle_start(start, Granularity::H4);
        assert_eq!(next, utc(2024, 1, 15, 11, 0, 0));
    }

    #[test]
    fn test_on_tick_no_close() {
        let mut detector = CandleBoundaryDetector::new();
        detector.register("EUR_USD".to_string(), Granularity::H1);

        // Tick within current candle period - no close event
        let tick_time = utc(2024, 1, 15, 10, 30, 0);
        let events = detector.on_tick("EUR_USD", tick_time);
        assert!(events.is_empty());
    }

    #[test]
    fn test_on_tick_detects_close() {
        let mut detector = CandleBoundaryDetector::new();

        // Manually set the candle start to 10:00
        detector
            .candle_starts
            .insert(("EUR_USD".to_string(), Granularity::H1), utc(2024, 1, 15, 10, 0, 0));

        // Tick at 11:00:01 should detect the 10:00 candle closing
        let tick_time = utc(2024, 1, 15, 11, 0, 1);
        let events = detector.on_tick("EUR_USD", tick_time);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].instrument, "EUR_USD");
        assert_eq!(events[0].timeframe, Granularity::H1);
        assert_eq!(events[0].closed_candle_start, utc(2024, 1, 15, 10, 0, 0));
        assert_eq!(events[0].closed_candle_end, utc(2024, 1, 15, 11, 0, 0));
    }

    #[test]
    fn test_on_tick_updates_after_close() {
        let mut detector = CandleBoundaryDetector::new();

        // Set candle start to 10:00
        detector
            .candle_starts
            .insert(("EUR_USD".to_string(), Granularity::H1), utc(2024, 1, 15, 10, 0, 0));

        // First tick at 11:00:01 detects close
        let events = detector.on_tick("EUR_USD", utc(2024, 1, 15, 11, 0, 1));
        assert_eq!(events.len(), 1);

        // Second tick at 11:00:30 should not detect another close
        let events = detector.on_tick("EUR_USD", utc(2024, 1, 15, 11, 0, 30));
        assert!(events.is_empty());

        // Tick at 12:00:01 should detect the 11:00 candle closing
        let events = detector.on_tick("EUR_USD", utc(2024, 1, 15, 12, 0, 1));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].closed_candle_start, utc(2024, 1, 15, 11, 0, 0));
    }

    #[test]
    fn test_multiple_timeframes_same_instrument() {
        let mut detector = CandleBoundaryDetector::new();

        // Register both H1 and H4 for EUR_USD
        detector
            .candle_starts
            .insert(("EUR_USD".to_string(), Granularity::H1), utc(2024, 1, 15, 10, 0, 0));
        detector
            .candle_starts
            .insert(("EUR_USD".to_string(), Granularity::H4), utc(2024, 1, 15, 7, 0, 0));

        // Tick at 11:00:01 - closes H1 but not H4
        let events = detector.on_tick("EUR_USD", utc(2024, 1, 15, 11, 0, 1));
        assert_eq!(events.len(), 2); // Both H1 and H4 close at 11:00

        // H4 should have closed (7:00-11:00)
        let h4_event = events.iter().find(|e| e.timeframe == Granularity::H4);
        assert!(h4_event.is_some());
        assert_eq!(h4_event.unwrap().closed_candle_start, utc(2024, 1, 15, 7, 0, 0));
    }

    #[test]
    fn test_time_until_close() {
        let now = utc(2024, 1, 15, 10, 45, 0);
        let time_left = CandleBoundaryDetector::time_until_close(now, Granularity::H1);
        assert_eq!(time_left, Duration::minutes(15));
    }

    #[test]
    fn test_h6_candle_start() {
        // H6 with dailyAlignment=2: candles at 02, 08, 14, 20

        // 05:00 should give 02:00
        let now = utc(2024, 1, 15, 5, 0, 0);
        let start = CandleBoundaryDetector::current_candle_start(now, Granularity::H6);
        assert_eq!(start, utc(2024, 1, 15, 2, 0, 0));

        // 10:00 should give 08:00
        let now = utc(2024, 1, 15, 10, 0, 0);
        let start = CandleBoundaryDetector::current_candle_start(now, Granularity::H6);
        assert_eq!(start, utc(2024, 1, 15, 8, 0, 0));

        // 01:00 should give previous day's 20:00
        let now = utc(2024, 1, 15, 1, 0, 0);
        let start = CandleBoundaryDetector::current_candle_start(now, Granularity::H6);
        assert_eq!(start, utc(2024, 1, 14, 20, 0, 0));
    }

    #[test]
    fn test_register_unregister() {
        let mut detector = CandleBoundaryDetector::new();

        detector.register("EUR_USD".to_string(), Granularity::H1);
        assert!(detector.is_registered("EUR_USD", Granularity::H1));

        detector.unregister("EUR_USD", Granularity::H1);
        assert!(!detector.is_registered("EUR_USD", Granularity::H1));
    }

    // Market hours tests

    #[test]
    fn test_market_open_monday() {
        // Monday 10:00 UTC - market open
        let time = utc(2024, 1, 15, 10, 0, 0); // Monday
        assert!(!is_forex_market_closed(time));
    }

    #[test]
    fn test_market_open_wednesday() {
        // Wednesday 15:00 UTC - market open
        let time = utc(2024, 1, 17, 15, 0, 0); // Wednesday
        assert!(!is_forex_market_closed(time));
    }

    #[test]
    fn test_market_open_friday_before_close() {
        // Friday 20:00 UTC - market still open (closes at 21:00)
        let time = utc(2024, 1, 19, 20, 0, 0); // Friday
        assert!(!is_forex_market_closed(time));
    }

    #[test]
    fn test_market_closed_friday_after_close() {
        // Friday 21:00 UTC - market closed
        let time = utc(2024, 1, 19, 21, 0, 0); // Friday
        assert!(is_forex_market_closed(time));
    }

    #[test]
    fn test_market_closed_friday_late() {
        // Friday 23:00 UTC - market closed
        let time = utc(2024, 1, 19, 23, 0, 0); // Friday
        assert!(is_forex_market_closed(time));
    }

    #[test]
    fn test_market_closed_saturday() {
        // Saturday any time - market closed
        let morning = utc(2024, 1, 20, 8, 0, 0); // Saturday morning
        let evening = utc(2024, 1, 20, 20, 0, 0); // Saturday evening
        assert!(is_forex_market_closed(morning));
        assert!(is_forex_market_closed(evening));
    }

    #[test]
    fn test_market_closed_sunday_before_open() {
        // Sunday 20:00 UTC - market still closed (opens at 21:00)
        let time = utc(2024, 1, 21, 20, 0, 0); // Sunday
        assert!(is_forex_market_closed(time));
    }

    #[test]
    fn test_market_open_sunday_after_open() {
        // Sunday 21:00 UTC - market open
        let time = utc(2024, 1, 21, 21, 0, 0); // Sunday
        assert!(!is_forex_market_closed(time));
    }

    #[test]
    fn test_market_open_sunday_late() {
        // Sunday 23:00 UTC - market open
        let time = utc(2024, 1, 21, 23, 0, 0); // Sunday
        assert!(!is_forex_market_closed(time));
    }
}
