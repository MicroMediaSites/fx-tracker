//! AGT-604: Indicator-math correctness fixtures.
//!
//! Verifies SMA / EMA / RSI calculations directly against hand-computed values,
//! independent of the backtest engine (see correctness_fixture_tests.rs for the
//! engine-path fixtures). Each expected value is derived in the comments from the
//! documented formula so a reviewer can check it with a calculator, not just trust
//! that this file mirrors the implementation.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use chrono::{DateTime, Utc, Duration};

use crate::models::{Candle, Ohlc};
use super::indicators::{EmaIndicator, Indicator, RsiIndicator, SmaIndicator};

/// Builds a candle whose CLOSE is the given price (open/high/low are irrelevant to
/// SMA/EMA/RSI, which only read `candle.mid.close`).
fn candle_with_close(close: Decimal, hour_offset: i64) -> Candle {
    let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);

    Candle {
        time: base_time + Duration::hours(hour_offset),
        mid: Ohlc { open: close, high: close, low: close, close },
        volume: 100,
        complete: true,
    }
}

// ============================================================================
// SMA(3): sliding-window average, verified across a full window slide
// ============================================================================

#[test]
fn test_sma_sliding_window_hand_computed() {
    // Closes: 100, 102, 101, 105, 103, 108
    let closes = [dec!(100), dec!(102), dec!(101), dec!(105), dec!(103), dec!(108)];
    let mut sma = SmaIndicator::new(3);

    // Candle 0, 1: window not full yet (needs 3 values) -> no "value" output.
    let out0 = sma.on_candle(&candle_with_close(closes[0], 0));
    assert!(!out0.contains_key("value"));
    let out1 = sma.on_candle(&candle_with_close(closes[1], 1));
    assert!(!out1.contains_key("value"));

    // Candle 2: window = [100, 102, 101] -> SMA = 303 / 3 = 101
    let out2 = sma.on_candle(&candle_with_close(closes[2], 2));
    assert_eq!(out2.get("value"), Some(&dec!(101)));

    // Candle 3: 100 drops off, window = [102, 101, 105] -> SMA = 308 / 3
    let out3 = sma.on_candle(&candle_with_close(closes[3], 3));
    assert_eq!(out3.get("value"), Some(&(dec!(308) / dec!(3))));

    // Candle 4: 102 drops off, window = [101, 105, 103] -> SMA = 309 / 3 = 103
    let out4 = sma.on_candle(&candle_with_close(closes[4], 4));
    assert_eq!(out4.get("value"), Some(&dec!(103)));

    // Candle 5: 101 drops off, window = [105, 103, 108] -> SMA = 316 / 3
    let out5 = sma.on_candle(&candle_with_close(closes[5], 5));
    assert_eq!(out5.get("value"), Some(&(dec!(316) / dec!(3))));
}

// ============================================================================
// EMA(3): SMA-seeded exponential smoothing, verified across several steps
// ============================================================================

#[test]
fn test_ema_multi_step_hand_computed() {
    // Closes: 100, 102, 101, 105, 103, 108
    // multiplier = 2 / (period + 1) = 2 / 4 = 0.5 (chosen deliberately so every
    // step below is an exact, non-repeating decimal)
    let closes = [dec!(100), dec!(102), dec!(101), dec!(105), dec!(103), dec!(108)];
    let mut ema = EmaIndicator::new(3);

    ema.on_candle(&candle_with_close(closes[0], 0)); // accumulating (no output)
    ema.on_candle(&candle_with_close(closes[1], 1)); // accumulating (no output)

    // Candle 2: seed EMA = SMA(100, 102, 101) = 303 / 3 = 101
    let out2 = ema.on_candle(&candle_with_close(closes[2], 2));
    assert_eq!(out2.get("value"), Some(&dec!(101)));

    // Candle 3: EMA = (price - prev_ema) * 0.5 + prev_ema
    //               = (105 - 101) * 0.5 + 101 = 4 * 0.5 + 101 = 103
    let out3 = ema.on_candle(&candle_with_close(closes[3], 3));
    assert_eq!(out3.get("value"), Some(&dec!(103)));

    // Candle 4: EMA = (103 - 103) * 0.5 + 103 = 103
    let out4 = ema.on_candle(&candle_with_close(closes[4], 4));
    assert_eq!(out4.get("value"), Some(&dec!(103)));

    // Candle 5: EMA = (108 - 103) * 0.5 + 103 = 5 * 0.5 + 103 = 105.5
    let out5 = ema.on_candle(&candle_with_close(closes[5], 5));
    assert_eq!(out5.get("value"), Some(&dec!(105.5)));
}

// ============================================================================
// RSI(3): initial simple-average RSI, then one Wilder-smoothed step
// ============================================================================

#[test]
fn test_rsi_initial_and_smoothed_step_hand_computed() {
    // Closes: 100, 103, 100, 106, 102 (period = 3, so the first RSI needs
    // period + 1 = 4 closes)
    let closes = [dec!(100), dec!(103), dec!(100), dec!(106), dec!(102)];
    let mut rsi = RsiIndicator::new(3);

    let out0 = rsi.on_candle(&candle_with_close(closes[0], 0));
    assert!(!out0.contains_key("value"));
    let out1 = rsi.on_candle(&candle_with_close(closes[1], 1));
    assert!(!out1.contains_key("value"));
    let out2 = rsi.on_candle(&candle_with_close(closes[2], 2));
    assert!(!out2.contains_key("value"));

    // Candle 3: prices = [100, 103, 100, 106]. Changes over the 3 steps:
    //   100 -> 103 = +3 (gain)
    //   103 -> 100 = -3 (loss 3)
    //   100 -> 106 = +6 (gain)
    // gains = 3 + 6 = 9, losses = 3
    // avg_gain = 9 / 3 = 3, avg_loss = 3 / 3 = 1
    // rs  = avg_gain / avg_loss = 3 / 1 = 3
    // rsi = 100 - 100 / (1 + rs) = 100 - 100 / 4 = 100 - 25 = 75
    let out3 = rsi.on_candle(&candle_with_close(closes[3], 3));
    assert_eq!(out3.get("value"), Some(&dec!(75)));

    // Candle 4: window slides to prices = [103, 100, 106, 102] (oldest 100
    // dropped). New change is prices[3] - prices[2] = 102 - 106 = -4 (loss 4,
    // gain 0). Wilder smoothing with period=3 (period-1=2):
    //   avg_gain = (prev_avg_gain * 2 + gain) / 3 = (3 * 2 + 0) / 3 = 6 / 3 = 2
    //   avg_loss = (prev_avg_loss * 2 + loss) / 3 = (1 * 2 + 4) / 3 = 6 / 3 = 2
    //   rs  = avg_gain / avg_loss = 2 / 2 = 1
    //   rsi = 100 - 100 / (1 + rs) = 100 - 100 / 2 = 100 - 50 = 50
    let out4 = rsi.on_candle(&candle_with_close(closes[4], 4));
    assert_eq!(out4.get("value"), Some(&dec!(50)));
}

#[test]
fn test_rsi_all_gains_saturates_at_100() {
    // A strictly increasing series has zero losses, so avg_loss stays 0 and the
    // formula's explicit `if avg_loss == ZERO { 100 }` branch fires -- RSI must
    // be exactly 100, not merely "close to" 100.
    let closes = [dec!(100), dec!(101), dec!(102), dec!(103)];
    let mut rsi = RsiIndicator::new(3);

    for (i, &c) in closes.iter().enumerate() {
        rsi.on_candle(&candle_with_close(c, i as i64));
    }
    let out = rsi.on_candle(&candle_with_close(dec!(104), closes.len() as i64));
    assert_eq!(out.get("value"), Some(&dec!(100)));
}
