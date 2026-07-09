//! Candlestick pattern detection functions.
//!
//! Pure functions that take a slice of recent candles and return whether
//! a specific candlestick pattern is detected. Each pattern returns
//! `Decimal::ONE` (detected) or `Decimal::ZERO` (not detected) when
//! used as a DataSource.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use shared::CandlestickPattern;

use crate::models::Candle;

// ============================================================================
// Helper Functions
// ============================================================================

fn body_size(candle: &Candle) -> Decimal {
    (candle.mid.close - candle.mid.open).abs()
}

fn upper_wick(candle: &Candle) -> Decimal {
    candle.mid.high - std::cmp::max(candle.mid.open, candle.mid.close)
}

fn lower_wick(candle: &Candle) -> Decimal {
    std::cmp::min(candle.mid.open, candle.mid.close) - candle.mid.low
}

fn range(candle: &Candle) -> Decimal {
    candle.mid.high - candle.mid.low
}

fn is_bullish(candle: &Candle) -> bool {
    candle.mid.close > candle.mid.open
}

fn is_bearish(candle: &Candle) -> bool {
    candle.mid.close < candle.mid.open
}

/// Body top (max of open, close)
fn body_top(candle: &Candle) -> Decimal {
    std::cmp::max(candle.mid.open, candle.mid.close)
}

/// Body bottom (min of open, close)
fn body_bottom(candle: &Candle) -> Decimal {
    std::cmp::min(candle.mid.open, candle.mid.close)
}

// ============================================================================
// Pattern Detection Functions
// ============================================================================

/// Bullish Engulfing: prev bearish, current bullish with body fully engulfing prev body.
/// Needs 2 candles.
pub fn detect_bullish_engulfing(candles: &[Candle]) -> bool {
    if candles.len() < 2 {
        return false;
    }
    let prev = &candles[candles.len() - 2];
    let curr = &candles[candles.len() - 1];

    is_bearish(prev)
        && is_bullish(curr)
        && body_bottom(curr) <= body_bottom(prev)
        && body_top(curr) >= body_top(prev)
}

/// Bearish Engulfing: prev bullish, current bearish with body fully engulfing prev body.
/// Needs 2 candles.
pub fn detect_bearish_engulfing(candles: &[Candle]) -> bool {
    if candles.len() < 2 {
        return false;
    }
    let prev = &candles[candles.len() - 2];
    let curr = &candles[candles.len() - 1];

    is_bullish(prev)
        && is_bearish(curr)
        && body_bottom(curr) <= body_bottom(prev)
        && body_top(curr) >= body_top(prev)
}

/// Hammer: small body (< 1/3 of range), long lower wick (> 2x body), small upper wick (< body).
/// Needs 1 candle.
pub fn detect_hammer(candles: &[Candle]) -> bool {
    if candles.is_empty() {
        return false;
    }
    let c = &candles[candles.len() - 1];
    let r = range(c);
    if r.is_zero() {
        return false;
    }
    let body = body_size(c);
    let lw = lower_wick(c);
    let uw = upper_wick(c);

    // body < 1/3 of range, lower wick > 2x body, upper wick < body
    body < r / dec!(3)
        && lw > body * dec!(2)
        && uw < body
}

/// Inverted Hammer: small body (< 1/3 of range), long upper wick (> 2x body), small lower wick (< body).
/// Needs 1 candle.
pub fn detect_inverted_hammer(candles: &[Candle]) -> bool {
    if candles.is_empty() {
        return false;
    }
    let c = &candles[candles.len() - 1];
    let r = range(c);
    if r.is_zero() {
        return false;
    }
    let body = body_size(c);
    let uw = upper_wick(c);
    let lw = lower_wick(c);

    // body < 1/3 of range, upper wick > 2x body, lower wick < body
    body < r / dec!(3)
        && uw > body * dec!(2)
        && lw < body
}

/// Doji: body < 10% of range (or range is zero).
/// Needs 1 candle.
pub fn detect_doji(candles: &[Candle]) -> bool {
    if candles.is_empty() {
        return false;
    }
    let c = &candles[candles.len() - 1];
    let r = range(c);
    if r.is_zero() {
        // Zero range means open == close == high == low - technically a doji
        return true;
    }
    let body = body_size(c);

    body < r / dec!(10)
}

/// Pin Bar: one wick > 2/3 of range, body < 1/3 of range.
/// Needs 1 candle.
pub fn detect_pin_bar(candles: &[Candle]) -> bool {
    if candles.is_empty() {
        return false;
    }
    let c = &candles[candles.len() - 1];
    let r = range(c);
    if r.is_zero() {
        return false;
    }
    let body = body_size(c);
    let uw = upper_wick(c);
    let lw = lower_wick(c);

    let two_thirds_range = r * dec!(2) / dec!(3);
    let one_third_range = r / dec!(3);

    body < one_third_range && (uw > two_thirds_range || lw > two_thirds_range)
}

/// Morning Star: 3-candle pattern.
/// 1st: bearish with significant body
/// 2nd: small body (doji-like, body < 1/3 of its range or body < 1/3 of first candle's body)
/// 3rd: bullish that closes above midpoint of first candle's body
/// Needs 3 candles.
pub fn detect_morning_star(candles: &[Candle]) -> bool {
    if candles.len() < 3 {
        return false;
    }
    let first = &candles[candles.len() - 3];
    let second = &candles[candles.len() - 2];
    let third = &candles[candles.len() - 1];

    let first_body = body_size(first);
    let second_body = body_size(second);
    let first_midpoint = (first.mid.open + first.mid.close) / dec!(2);

    // First must be bearish
    if !is_bearish(first) {
        return false;
    }
    // Third must be bullish
    if !is_bullish(third) {
        return false;
    }
    // Second must have small body (doji-like)
    let second_range = range(second);
    let is_small_body = if second_range.is_zero() {
        true // zero-range candle counts as small body
    } else {
        second_body < second_range / dec!(3) || second_body < first_body / dec!(3)
    };
    if !is_small_body {
        return false;
    }
    // Third closes above midpoint of first candle
    third.mid.close > first_midpoint
}

/// Evening Star: 3-candle pattern.
/// 1st: bullish with significant body
/// 2nd: small body (doji-like)
/// 3rd: bearish that closes below midpoint of first candle's body
/// Needs 3 candles.
pub fn detect_evening_star(candles: &[Candle]) -> bool {
    if candles.len() < 3 {
        return false;
    }
    let first = &candles[candles.len() - 3];
    let second = &candles[candles.len() - 2];
    let third = &candles[candles.len() - 1];

    let first_body = body_size(first);
    let second_body = body_size(second);
    let first_midpoint = (first.mid.open + first.mid.close) / dec!(2);

    // First must be bullish
    if !is_bullish(first) {
        return false;
    }
    // Third must be bearish
    if !is_bearish(third) {
        return false;
    }
    // Second must have small body (doji-like)
    let second_range = range(second);
    let is_small_body = if second_range.is_zero() {
        true
    } else {
        second_body < second_range / dec!(3) || second_body < first_body / dec!(3)
    };
    if !is_small_body {
        return false;
    }
    // Third closes below midpoint of first candle
    third.mid.close < first_midpoint
}

/// Bullish Harami: prev bearish, current bullish with body fully contained within prev body.
/// Needs 2 candles.
pub fn detect_bullish_harami(candles: &[Candle]) -> bool {
    if candles.len() < 2 {
        return false;
    }
    let prev = &candles[candles.len() - 2];
    let curr = &candles[candles.len() - 1];

    is_bearish(prev)
        && is_bullish(curr)
        && body_bottom(curr) >= body_bottom(prev)
        && body_top(curr) <= body_top(prev)
}

/// Bearish Harami: prev bullish, current bearish with body fully contained within prev body.
/// Needs 2 candles.
pub fn detect_bearish_harami(candles: &[Candle]) -> bool {
    if candles.len() < 2 {
        return false;
    }
    let prev = &candles[candles.len() - 2];
    let curr = &candles[candles.len() - 1];

    is_bullish(prev)
        && is_bearish(curr)
        && body_bottom(curr) >= body_bottom(prev)
        && body_top(curr) <= body_top(prev)
}

// ============================================================================
// Dispatch Function
// ============================================================================

/// Detect a candlestick pattern from a slice of candles.
/// Returns true if the pattern is detected, false otherwise.
pub fn detect_pattern(pattern: &CandlestickPattern, candles: &[Candle]) -> bool {
    match pattern {
        CandlestickPattern::BullishEngulfing => detect_bullish_engulfing(candles),
        CandlestickPattern::BearishEngulfing => detect_bearish_engulfing(candles),
        CandlestickPattern::Hammer => detect_hammer(candles),
        CandlestickPattern::InvertedHammer => detect_inverted_hammer(candles),
        CandlestickPattern::Doji => detect_doji(candles),
        CandlestickPattern::PinBar => detect_pin_bar(candles),
        CandlestickPattern::MorningStar => detect_morning_star(candles),
        CandlestickPattern::EveningStar => detect_evening_star(candles),
        CandlestickPattern::BullishHarami => detect_bullish_harami(candles),
        CandlestickPattern::BearishHarami => detect_bearish_harami(candles),
    }
}

/// Returns the number of candles needed for a given pattern.
pub fn candles_needed(pattern: &CandlestickPattern) -> usize {
    match pattern {
        CandlestickPattern::Hammer
        | CandlestickPattern::InvertedHammer
        | CandlestickPattern::Doji
        | CandlestickPattern::PinBar => 1,
        CandlestickPattern::BullishEngulfing
        | CandlestickPattern::BearishEngulfing
        | CandlestickPattern::BullishHarami
        | CandlestickPattern::BearishHarami => 2,
        CandlestickPattern::MorningStar
        | CandlestickPattern::EveningStar => 3,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use crate::models::Ohlc;

    /// Create a test candle with explicit OHLC values
    fn make_candle(open: Decimal, high: Decimal, low: Decimal, close: Decimal) -> Candle {
        Candle {
            time: Utc::now(),
            mid: Ohlc { open, high, low, close },
            volume: 1000,
            complete: true,
        }
    }

    // ========================================================================
    // Bullish Engulfing
    // ========================================================================

    #[test]
    fn test_bullish_engulfing_detected() {
        // Prev: bearish (open=1.1050, close=1.1000) body from 1.1000 to 1.1050
        // Curr: bullish (open=1.0990, close=1.1060) body from 1.0990 to 1.1060 - engulfs prev
        let candles = vec![
            make_candle(dec!(1.1050), dec!(1.1060), dec!(1.0990), dec!(1.1000)), // bearish
            make_candle(dec!(1.0990), dec!(1.1070), dec!(1.0980), dec!(1.1060)), // bullish, engulfs
        ];
        assert!(detect_bullish_engulfing(&candles));
    }

    #[test]
    fn test_bullish_engulfing_not_detected() {
        // Two bullish candles - not a bullish engulfing
        let candles = vec![
            make_candle(dec!(1.1000), dec!(1.1060), dec!(1.0990), dec!(1.1050)), // bullish
            make_candle(dec!(1.1050), dec!(1.1100), dec!(1.1040), dec!(1.1090)), // bullish
        ];
        assert!(!detect_bullish_engulfing(&candles));
    }

    // ========================================================================
    // Bearish Engulfing
    // ========================================================================

    #[test]
    fn test_bearish_engulfing_detected() {
        // Prev: bullish (open=1.1000, close=1.1050) body from 1.1000 to 1.1050
        // Curr: bearish (open=1.1060, close=1.0990) body from 1.0990 to 1.1060 - engulfs prev
        let candles = vec![
            make_candle(dec!(1.1000), dec!(1.1060), dec!(1.0990), dec!(1.1050)), // bullish
            make_candle(dec!(1.1060), dec!(1.1070), dec!(1.0980), dec!(1.0990)), // bearish, engulfs
        ];
        assert!(detect_bearish_engulfing(&candles));
    }

    #[test]
    fn test_bearish_engulfing_not_detected_same_direction() {
        // Both bearish - not a bearish engulfing
        let candles = vec![
            make_candle(dec!(1.1050), dec!(1.1060), dec!(1.0990), dec!(1.1000)), // bearish
            make_candle(dec!(1.1000), dec!(1.1010), dec!(1.0940), dec!(1.0950)), // bearish
        ];
        assert!(!detect_bearish_engulfing(&candles));
    }

    // ========================================================================
    // Hammer
    // ========================================================================

    #[test]
    fn test_hammer_detected() {
        // Hammer: small body near top, long lower wick, tiny upper wick
        // Range: 1.1100 - 1.0900 = 0.0200
        // Body: 1.1090 - 1.1080 = 0.0010 (< 0.0200/3 = 0.0067)
        // Lower wick: 1.1080 - 1.0900 = 0.0180 (> 0.0010*2 = 0.0020)
        // Upper wick: 1.1100 - 1.1090 = 0.0010 (< 0.0010 is false, = body)
        // Need upper wick < body, so adjust:
        // open=1.1095, close=1.1100, high=1.1100, low=1.0900
        // Body: 0.0005, upper wick: 0, lower wick: 1.0900 -> min(1.1095,1.1100)=1.1095 - 1.0900 = 0.0195
        let candles = vec![
            make_candle(dec!(1.1095), dec!(1.1100), dec!(1.0900), dec!(1.1100)),
        ];
        assert!(detect_hammer(&candles));
    }

    #[test]
    fn test_hammer_not_detected() {
        // Equal wicks, body in the middle - not a hammer
        // open=1.1000, close=1.1050, high=1.1100, low=1.0950
        // body=0.0050, range=0.0150, lower wick=0.0050, upper wick=0.0050
        // body < range/3 = 0.0050 < 0.005 -> false
        let candles = vec![
            make_candle(dec!(1.1000), dec!(1.1100), dec!(1.0950), dec!(1.1050)),
        ];
        assert!(!detect_hammer(&candles));
    }

    // ========================================================================
    // Inverted Hammer
    // ========================================================================

    #[test]
    fn test_inverted_hammer_detected() {
        // Inverted hammer: small body near bottom, long upper wick, tiny lower wick
        // open=1.0900, close=1.0905, high=1.1100, low=1.0900
        // body=0.0005, range=0.0200, upper wick=1.1100-1.0905=0.0195, lower wick=0
        let candles = vec![
            make_candle(dec!(1.0900), dec!(1.1100), dec!(1.0900), dec!(1.0905)),
        ];
        assert!(detect_inverted_hammer(&candles));
    }

    // ========================================================================
    // Doji
    // ========================================================================

    #[test]
    fn test_doji_detected() {
        // Doji: open approximately equals close, body < 10% of range
        // open=1.1000, close=1.1001, high=1.1050, low=1.0950
        // body=0.0001, range=0.0100, body/range=1% < 10%
        let candles = vec![
            make_candle(dec!(1.1000), dec!(1.1050), dec!(1.0950), dec!(1.1001)),
        ];
        assert!(detect_doji(&candles));
    }

    #[test]
    fn test_doji_zero_range() {
        // All values equal - technically a doji
        let candles = vec![
            make_candle(dec!(1.1000), dec!(1.1000), dec!(1.1000), dec!(1.1000)),
        ];
        assert!(detect_doji(&candles));
    }

    #[test]
    fn test_doji_not_detected() {
        // Normal candle with significant body
        // open=1.1000, close=1.1050, high=1.1060, low=1.0990
        // body=0.0050, range=0.0070, body/range=71% > 10%
        let candles = vec![
            make_candle(dec!(1.1000), dec!(1.1060), dec!(1.0990), dec!(1.1050)),
        ];
        assert!(!detect_doji(&candles));
    }

    // ========================================================================
    // Pin Bar
    // ========================================================================

    #[test]
    fn test_pin_bar_detected() {
        // Pin bar: one wick > 2/3 of range, body < 1/3 of range
        // Bullish pin bar (long lower wick):
        // open=1.1095, close=1.1100, high=1.1100, low=1.0900
        // range=0.0200, body=0.0005 (< 0.0200/3=0.0067)
        // lower wick=1.1095-1.0900=0.0195 (> 0.0200*2/3=0.0133)
        let candles = vec![
            make_candle(dec!(1.1095), dec!(1.1100), dec!(1.0900), dec!(1.1100)),
        ];
        assert!(detect_pin_bar(&candles));
    }

    #[test]
    fn test_pin_bar_bearish_detected() {
        // Bearish pin bar (long upper wick):
        // open=1.0905, close=1.0900, high=1.1100, low=1.0900
        // range=0.0200, body=0.0005 (< 0.0067)
        // upper wick=1.1100-1.0905=0.0195 (> 0.0133)
        let candles = vec![
            make_candle(dec!(1.0905), dec!(1.1100), dec!(1.0900), dec!(1.0900)),
        ];
        assert!(detect_pin_bar(&candles));
    }

    // ========================================================================
    // Morning Star
    // ========================================================================

    #[test]
    fn test_morning_star_detected() {
        // 3-candle pattern:
        // 1st: bearish (open=1.1100, close=1.1000) midpoint = 1.1050
        // 2nd: doji-like small body (open=1.0990, close=1.0995, high=1.1010, low=1.0980)
        //      body=0.0005, first_body=0.0100, 0.0005 < 0.0100/3=0.0033
        // 3rd: bullish closing above midpoint (open=1.1000, close=1.1060)
        //      1.1060 > 1.1050 (midpoint of first)
        let candles = vec![
            make_candle(dec!(1.1100), dec!(1.1110), dec!(1.0990), dec!(1.1000)), // bearish
            make_candle(dec!(1.0990), dec!(1.1010), dec!(1.0980), dec!(1.0995)), // small body
            make_candle(dec!(1.1000), dec!(1.1070), dec!(1.0990), dec!(1.1060)), // bullish
        ];
        assert!(detect_morning_star(&candles));
    }

    #[test]
    fn test_morning_star_not_detected_wrong_order() {
        // First candle is bullish instead of bearish
        let candles = vec![
            make_candle(dec!(1.1000), dec!(1.1110), dec!(1.0990), dec!(1.1100)), // bullish
            make_candle(dec!(1.0990), dec!(1.1010), dec!(1.0980), dec!(1.0995)), // small body
            make_candle(dec!(1.1000), dec!(1.1070), dec!(1.0990), dec!(1.1060)), // bullish
        ];
        assert!(!detect_morning_star(&candles));
    }

    // ========================================================================
    // Evening Star
    // ========================================================================

    #[test]
    fn test_evening_star_detected() {
        // 3-candle pattern:
        // 1st: bullish (open=1.1000, close=1.1100) midpoint = 1.1050
        // 2nd: doji-like small body (open=1.1110, close=1.1105)
        //      body=0.0005, first_body=0.0100, 0.0005 < 0.0100/3=0.0033
        // 3rd: bearish closing below midpoint (open=1.1090, close=1.1040)
        //      1.1040 < 1.1050 (midpoint of first)
        let candles = vec![
            make_candle(dec!(1.1000), dec!(1.1110), dec!(1.0990), dec!(1.1100)), // bullish
            make_candle(dec!(1.1110), dec!(1.1120), dec!(1.1090), dec!(1.1105)), // small body
            make_candle(dec!(1.1090), dec!(1.1100), dec!(1.1030), dec!(1.1040)), // bearish
        ];
        assert!(detect_evening_star(&candles));
    }

    #[test]
    fn test_evening_star_not_detected_wrong_order() {
        // First candle is bearish instead of bullish
        let candles = vec![
            make_candle(dec!(1.1100), dec!(1.1110), dec!(1.0990), dec!(1.1000)), // bearish
            make_candle(dec!(1.1010), dec!(1.1020), dec!(1.1000), dec!(1.1005)), // small body
            make_candle(dec!(1.1000), dec!(1.1010), dec!(1.0930), dec!(1.0940)), // bearish
        ];
        assert!(!detect_evening_star(&candles));
    }

    // ========================================================================
    // Bullish Harami
    // ========================================================================

    #[test]
    fn test_bullish_harami_detected() {
        // Prev: bearish with large body (open=1.1100, close=1.1000) body: 1.1000..1.1100
        // Curr: bullish with small body contained within (open=1.1020, close=1.1060) body: 1.1020..1.1060
        let candles = vec![
            make_candle(dec!(1.1100), dec!(1.1110), dec!(1.0990), dec!(1.1000)), // bearish
            make_candle(dec!(1.1020), dec!(1.1070), dec!(1.1010), dec!(1.1060)), // bullish, inside prev
        ];
        assert!(detect_bullish_harami(&candles));
    }

    #[test]
    fn test_bullish_harami_not_detected_body_exceeds() {
        // Curr body exceeds prev body
        let candles = vec![
            make_candle(dec!(1.1050), dec!(1.1060), dec!(1.0990), dec!(1.1000)), // bearish (body 1.1000..1.1050)
            make_candle(dec!(1.0990), dec!(1.1070), dec!(1.0980), dec!(1.1060)), // bullish, body 0.990..1.1060 exceeds
        ];
        assert!(!detect_bullish_harami(&candles));
    }

    // ========================================================================
    // Bearish Harami
    // ========================================================================

    #[test]
    fn test_bearish_harami_detected() {
        // Prev: bullish with large body (open=1.1000, close=1.1100) body: 1.1000..1.1100
        // Curr: bearish with small body contained within (open=1.1060, close=1.1020) body: 1.1020..1.1060
        let candles = vec![
            make_candle(dec!(1.1000), dec!(1.1110), dec!(1.0990), dec!(1.1100)), // bullish
            make_candle(dec!(1.1060), dec!(1.1070), dec!(1.1010), dec!(1.1020)), // bearish, inside prev
        ];
        assert!(detect_bearish_harami(&candles));
    }

    #[test]
    fn test_bearish_harami_not_detected_wrong_direction() {
        // Prev is bearish instead of bullish
        let candles = vec![
            make_candle(dec!(1.1100), dec!(1.1110), dec!(1.0990), dec!(1.1000)), // bearish
            make_candle(dec!(1.1060), dec!(1.1070), dec!(1.1010), dec!(1.1020)), // bearish
        ];
        assert!(!detect_bearish_harami(&candles));
    }

    // ========================================================================
    // Insufficient Candles
    // ========================================================================

    #[test]
    fn test_insufficient_candles() {
        let empty: Vec<Candle> = vec![];
        let single = vec![
            make_candle(dec!(1.1000), dec!(1.1050), dec!(1.0950), dec!(1.1020)),
        ];

        // Patterns needing 2 candles should return false with 1 or 0
        assert!(!detect_bullish_engulfing(&empty));
        assert!(!detect_bullish_engulfing(&single));
        assert!(!detect_bearish_engulfing(&empty));
        assert!(!detect_bearish_engulfing(&single));
        assert!(!detect_bullish_harami(&empty));
        assert!(!detect_bullish_harami(&single));
        assert!(!detect_bearish_harami(&empty));
        assert!(!detect_bearish_harami(&single));

        // Patterns needing 1 candle should return false with 0
        assert!(!detect_hammer(&empty));
        assert!(!detect_inverted_hammer(&empty));
        assert!(!detect_doji(&empty));
        assert!(!detect_pin_bar(&empty));

        // Patterns needing 3 candles should return false with 0, 1, or 2
        assert!(!detect_morning_star(&empty));
        assert!(!detect_morning_star(&single));
        assert!(!detect_evening_star(&empty));
        assert!(!detect_evening_star(&single));
        let two = vec![
            make_candle(dec!(1.1000), dec!(1.1050), dec!(1.0950), dec!(1.1020)),
            make_candle(dec!(1.1000), dec!(1.1050), dec!(1.0950), dec!(1.1020)),
        ];
        assert!(!detect_morning_star(&two));
        assert!(!detect_evening_star(&two));
    }

    // ========================================================================
    // Dispatch Function
    // ========================================================================

    #[test]
    fn test_detect_pattern_dispatch() {
        // Verify that detect_pattern dispatches correctly
        let doji_candle = vec![
            make_candle(dec!(1.1000), dec!(1.1050), dec!(1.0950), dec!(1.1001)),
        ];
        assert!(detect_pattern(&CandlestickPattern::Doji, &doji_candle));
        assert!(!detect_pattern(&CandlestickPattern::Hammer, &doji_candle));
    }

    #[test]
    fn test_candles_needed_values() {
        assert_eq!(candles_needed(&CandlestickPattern::Hammer), 1);
        assert_eq!(candles_needed(&CandlestickPattern::InvertedHammer), 1);
        assert_eq!(candles_needed(&CandlestickPattern::Doji), 1);
        assert_eq!(candles_needed(&CandlestickPattern::PinBar), 1);
        assert_eq!(candles_needed(&CandlestickPattern::BullishEngulfing), 2);
        assert_eq!(candles_needed(&CandlestickPattern::BearishEngulfing), 2);
        assert_eq!(candles_needed(&CandlestickPattern::BullishHarami), 2);
        assert_eq!(candles_needed(&CandlestickPattern::BearishHarami), 2);
        assert_eq!(candles_needed(&CandlestickPattern::MorningStar), 3);
        assert_eq!(candles_needed(&CandlestickPattern::EveningStar), 3);
    }
}
