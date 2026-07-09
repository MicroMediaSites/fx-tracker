//! Pivot point calculation module
//!
//! Calculates standard pivot points from previous period's high, low, and close.
//! Supports daily and weekly pivot periods.

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

pub use shared::{PivotLevel, PivotPeriod};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PivotLevels {
    pub pp: Decimal,
    pub r1: Decimal,
    pub r2: Decimal,
    pub r3: Decimal,
    pub s1: Decimal,
    pub s2: Decimal,
    pub s3: Decimal,
}

impl PivotLevels {
    pub fn get_level(&self, level: PivotLevel) -> Decimal {
        match level {
            PivotLevel::Pp => self.pp,
            PivotLevel::R1 => self.r1,
            PivotLevel::R2 => self.r2,
            PivotLevel::R3 => self.r3,
            PivotLevel::S1 => self.s1,
            PivotLevel::S2 => self.s2,
            PivotLevel::S3 => self.s3,
        }
    }
}

/// Configuration for pivot point calculation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PivotConfig {
    pub enabled: bool,
    pub period: PivotPeriod,
}

impl Default for PivotConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            period: PivotPeriod::Daily,
        }
    }
}

/// Tracks period HLC for pivot calculation
#[derive(Debug, Clone, Default)]
pub struct PivotPeriodTracker {
    /// High of current period
    pub high: Option<Decimal>,
    /// Low of current period
    pub low: Option<Decimal>,
    /// Close of current period (updated on each candle)
    pub close: Option<Decimal>,
    /// Date of current period (for daily: the date, for weekly: Monday of the week)
    pub period_start: Option<NaiveDate>,
    /// Previous period's HLC (used to calculate pivots)
    pub prev_high: Option<Decimal>,
    pub prev_low: Option<Decimal>,
    pub prev_close: Option<Decimal>,
}

impl PivotPeriodTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update tracker with a new candle
    /// Returns true if we crossed into a new period (pivots should be recalculated)
    pub fn update(
        &mut self,
        time: DateTime<Utc>,
        high: Decimal,
        low: Decimal,
        close: Decimal,
        period: PivotPeriod,
    ) -> bool {
        let candle_period = get_period_start(time, period);

        // Check if this is a new period
        let is_new_period = match self.period_start {
            Some(current) => candle_period > current,
            None => true, // First candle
        };

        if is_new_period && self.period_start.is_some() {
            // Save current period as previous before resetting
            self.prev_high = self.high;
            self.prev_low = self.low;
            self.prev_close = self.close;

            // Reset for new period
            self.high = Some(high);
            self.low = Some(low);
            self.close = Some(close);
            self.period_start = Some(candle_period);

            true
        } else if is_new_period {
            // First candle ever - initialize
            self.high = Some(high);
            self.low = Some(low);
            self.close = Some(close);
            self.period_start = Some(candle_period);

            false // No pivots yet (no previous period)
        } else {
            // Same period - update HLC
            self.high = Some(self.high.unwrap_or(high).max(high));
            self.low = Some(self.low.unwrap_or(low).min(low));
            self.close = Some(close);

            false
        }
    }

    /// Check if we have enough data to calculate pivots
    pub fn can_calculate(&self) -> bool {
        self.prev_high.is_some() && self.prev_low.is_some() && self.prev_close.is_some()
    }

    /// Calculate pivots from previous period's HLC
    pub fn calculate_pivots(&self) -> Option<PivotLevels> {
        if !self.can_calculate() {
            return None;
        }

        let high = self.prev_high.unwrap();
        let low = self.prev_low.unwrap();
        let close = self.prev_close.unwrap();

        Some(calculate_standard_pivots(high, low, close))
    }

    /// Reset the tracker
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Get the start date of the period containing the given timestamp
fn get_period_start(time: DateTime<Utc>, period: PivotPeriod) -> NaiveDate {
    let date = time.date_naive();

    match period {
        PivotPeriod::Daily => date,
        PivotPeriod::Weekly => {
            // Get the Monday of the week
            let weekday = date.weekday();
            let days_since_monday = weekday.num_days_from_monday();
            date - chrono::Duration::days(days_since_monday as i64)
        }
    }
}

/// Calculate standard pivot points from HLC
///
/// Formulas:
/// - PP = (H + L + C) / 3
/// - R1 = (2 × PP) - L
/// - S1 = (2 × PP) - H
/// - R2 = PP + (H - L)
/// - S2 = PP - (H - L)
/// - R3 = H + 2(PP - L)
/// - S3 = L - 2(H - PP)
pub fn calculate_standard_pivots(high: Decimal, low: Decimal, close: Decimal) -> PivotLevels {
    let three = dec!(3);
    let two = dec!(2);

    let pp = (high + low + close) / three;
    let range = high - low;

    PivotLevels {
        pp,
        r1: (two * pp) - low,
        s1: (two * pp) - high,
        r2: pp + range,
        s2: pp - range,
        r3: high + two * (pp - low),
        s3: low - two * (high - pp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_standard_pivot_calculation() {
        // Example: H=1.1050, L=1.1000, C=1.1030
        let high = dec!(1.1050);
        let low = dec!(1.1000);
        let close = dec!(1.1030);

        let pivots = calculate_standard_pivots(high, low, close);

        // PP = (1.1050 + 1.1000 + 1.1030) / 3 = 1.10266...
        let expected_pp = (high + low + close) / dec!(3);
        assert_eq!(pivots.pp, expected_pp);

        // R1 = 2 * PP - L
        let expected_r1 = dec!(2) * expected_pp - low;
        assert_eq!(pivots.r1, expected_r1);

        // S1 = 2 * PP - H
        let expected_s1 = dec!(2) * expected_pp - high;
        assert_eq!(pivots.s1, expected_s1);

        // Verify R levels are above PP and S levels below
        assert!(pivots.r1 > pivots.pp);
        assert!(pivots.r2 > pivots.r1);
        assert!(pivots.r3 > pivots.r2);
        assert!(pivots.s1 < pivots.pp);
        assert!(pivots.s2 < pivots.s1);
        assert!(pivots.s3 < pivots.s2);
    }

    #[test]
    fn test_get_level() {
        let pivots = calculate_standard_pivots(dec!(1.1050), dec!(1.1000), dec!(1.1030));

        assert_eq!(pivots.get_level(PivotLevel::Pp), pivots.pp);
        assert_eq!(pivots.get_level(PivotLevel::R1), pivots.r1);
        assert_eq!(pivots.get_level(PivotLevel::S2), pivots.s2);
        // With enums, there's no "invalid" case - all values are type-checked at compile time
    }

    #[test]
    fn test_period_tracker_daily() {
        let mut tracker = PivotPeriodTracker::new();

        // Day 1, candle 1
        let time1 = Utc.with_ymd_and_hms(2024, 1, 15, 10, 0, 0).unwrap();
        let new_period = tracker.update(time1, dec!(1.1050), dec!(1.1000), dec!(1.1030), PivotPeriod::Daily);
        assert!(!new_period); // First candle, no previous period
        assert!(!tracker.can_calculate());

        // Day 1, candle 2 - same day, updates HLC
        let time2 = Utc.with_ymd_and_hms(2024, 1, 15, 14, 0, 0).unwrap();
        let new_period = tracker.update(time2, dec!(1.1060), dec!(1.0990), dec!(1.1040), PivotPeriod::Daily);
        assert!(!new_period);
        assert_eq!(tracker.high, Some(dec!(1.1060))); // Updated high
        assert_eq!(tracker.low, Some(dec!(1.0990)));  // Updated low

        // Day 2 - new period, pivots should be calculable
        let time3 = Utc.with_ymd_and_hms(2024, 1, 16, 10, 0, 0).unwrap();
        let new_period = tracker.update(time3, dec!(1.1070), dec!(1.1020), dec!(1.1050), PivotPeriod::Daily);
        assert!(new_period);
        assert!(tracker.can_calculate());

        let pivots = tracker.calculate_pivots().unwrap();
        // Should use day 1's final HLC
        let expected = calculate_standard_pivots(dec!(1.1060), dec!(1.0990), dec!(1.1040));
        assert_eq!(pivots.pp, expected.pp);
    }

    #[test]
    fn test_period_tracker_weekly() {
        let mut tracker = PivotPeriodTracker::new();

        // Week 1 (Monday Jan 15, 2024)
        let monday = Utc.with_ymd_and_hms(2024, 1, 15, 10, 0, 0).unwrap();
        tracker.update(monday, dec!(1.1050), dec!(1.1000), dec!(1.1030), PivotPeriod::Weekly);

        // Same week (Friday Jan 19)
        let friday = Utc.with_ymd_and_hms(2024, 1, 19, 10, 0, 0).unwrap();
        let new_period = tracker.update(friday, dec!(1.1100), dec!(1.0950), dec!(1.1080), PivotPeriod::Weekly);
        assert!(!new_period); // Same week

        // Next week (Monday Jan 22)
        let next_monday = Utc.with_ymd_and_hms(2024, 1, 22, 10, 0, 0).unwrap();
        let new_period = tracker.update(next_monday, dec!(1.1090), dec!(1.1070), dec!(1.1085), PivotPeriod::Weekly);
        assert!(new_period); // New week
        assert!(tracker.can_calculate());
    }
}
