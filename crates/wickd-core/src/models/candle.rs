use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use crate::oanda::types::{OandaCandle, OandaCandleData};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ohlc {
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
}

impl From<&OandaCandleData> for Ohlc {
    fn from(data: &OandaCandleData) -> Self {
        Self {
            open: Decimal::from_str(&data.o).unwrap_or_default(),
            high: Decimal::from_str(&data.h).unwrap_or_default(),
            low: Decimal::from_str(&data.l).unwrap_or_default(),
            close: Decimal::from_str(&data.c).unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    pub time: DateTime<Utc>,
    pub mid: Ohlc,
    pub volume: i32,
    pub complete: bool,
}

impl From<OandaCandle> for Candle {
    fn from(c: OandaCandle) -> Self {
        let mid = c.mid.as_ref().map(Ohlc::from).unwrap_or(Ohlc {
            open: Decimal::ZERO,
            high: Decimal::ZERO,
            low: Decimal::ZERO,
            close: Decimal::ZERO,
        });

        Self {
            time: DateTime::parse_from_rfc3339(&c.time)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            mid,
            volume: c.volume,
            complete: c.complete,
        }
    }
}

impl Candle {
    pub fn is_bullish(&self) -> bool {
        self.mid.close > self.mid.open
    }

    pub fn is_bearish(&self) -> bool {
        self.mid.close < self.mid.open
    }

    pub fn range(&self) -> Decimal {
        self.mid.high - self.mid.low
    }

    pub fn body_size(&self) -> Decimal {
        (self.mid.close - self.mid.open).abs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn create_oanda_candle_data(o: &str, h: &str, l: &str, c: &str) -> OandaCandleData {
        OandaCandleData {
            o: o.to_string(),
            h: h.to_string(),
            l: l.to_string(),
            c: c.to_string(),
        }
    }

    fn create_oanda_candle(time: &str, volume: i32, complete: bool, mid: Option<OandaCandleData>) -> OandaCandle {
        OandaCandle {
            time: time.to_string(),
            volume,
            complete,
            bid: None,
            ask: None,
            mid,
        }
    }

    #[test]
    fn test_ohlc_from_oanda_candle_data() {
        let data = create_oanda_candle_data("1.1000", "1.1050", "1.0950", "1.1020");
        let ohlc = Ohlc::from(&data);

        assert_eq!(ohlc.open, dec!(1.1000));
        assert_eq!(ohlc.high, dec!(1.1050));
        assert_eq!(ohlc.low, dec!(1.0950));
        assert_eq!(ohlc.close, dec!(1.1020));
    }

    #[test]
    fn test_ohlc_from_invalid_data() {
        let data = OandaCandleData {
            o: "invalid".to_string(),
            h: "also_invalid".to_string(),
            l: "bad".to_string(),
            c: "nope".to_string(),
        };
        let ohlc = Ohlc::from(&data);

        // Should default to zero for invalid values
        assert_eq!(ohlc.open, Decimal::ZERO);
        assert_eq!(ohlc.high, Decimal::ZERO);
        assert_eq!(ohlc.low, Decimal::ZERO);
        assert_eq!(ohlc.close, Decimal::ZERO);
    }

    #[test]
    fn test_candle_from_oanda_candle_with_mid() {
        let mid = create_oanda_candle_data("1.1000", "1.1050", "1.0950", "1.1020");
        let oanda_candle = create_oanda_candle("2024-01-15T10:00:00Z", 1000, true, Some(mid));
        let candle = Candle::from(oanda_candle);

        assert_eq!(candle.mid.open, dec!(1.1000));
        assert_eq!(candle.mid.close, dec!(1.1020));
        assert_eq!(candle.volume, 1000);
        assert!(candle.complete);
    }

    #[test]
    fn test_candle_from_oanda_candle_without_mid() {
        let oanda_candle = create_oanda_candle("2024-01-15T10:00:00Z", 500, false, None);
        let candle = Candle::from(oanda_candle);

        // Should default to zero OHLC
        assert_eq!(candle.mid.open, Decimal::ZERO);
        assert_eq!(candle.mid.high, Decimal::ZERO);
        assert_eq!(candle.mid.low, Decimal::ZERO);
        assert_eq!(candle.mid.close, Decimal::ZERO);
        assert_eq!(candle.volume, 500);
        assert!(!candle.complete);
    }

    #[test]
    fn test_candle_from_oanda_with_invalid_time() {
        let mid = create_oanda_candle_data("1.1000", "1.1050", "1.0950", "1.1020");
        let oanda_candle = create_oanda_candle("not-a-valid-time", 100, true, Some(mid));
        let candle = Candle::from(oanda_candle);

        // Should use current time for invalid timestamps
        // We can't assert exact time, but it should be close to now
        let now = Utc::now();
        let diff = (candle.time - now).num_seconds().abs();
        assert!(diff < 5); // Within 5 seconds of now
    }

    #[test]
    fn test_candle_is_bullish() {
        let candle = Candle {
            time: Utc::now(),
            mid: Ohlc {
                open: dec!(1.1000),
                high: dec!(1.1100),
                low: dec!(1.0950),
                close: dec!(1.1050), // close > open
            },
            volume: 1000,
            complete: true,
        };

        assert!(candle.is_bullish());
        assert!(!candle.is_bearish());
    }

    #[test]
    fn test_candle_is_bearish() {
        let candle = Candle {
            time: Utc::now(),
            mid: Ohlc {
                open: dec!(1.1050),
                high: dec!(1.1100),
                low: dec!(1.0950),
                close: dec!(1.1000), // close < open
            },
            volume: 1000,
            complete: true,
        };

        assert!(candle.is_bearish());
        assert!(!candle.is_bullish());
    }

    #[test]
    fn test_candle_doji() {
        // Doji: open == close
        let candle = Candle {
            time: Utc::now(),
            mid: Ohlc {
                open: dec!(1.1000),
                high: dec!(1.1100),
                low: dec!(1.0950),
                close: dec!(1.1000), // close == open
            },
            volume: 1000,
            complete: true,
        };

        assert!(!candle.is_bullish());
        assert!(!candle.is_bearish());
    }

    #[test]
    fn test_candle_range() {
        let candle = Candle {
            time: Utc::now(),
            mid: Ohlc {
                open: dec!(1.1000),
                high: dec!(1.1100),
                low: dec!(1.0900),
                close: dec!(1.1050),
            },
            volume: 1000,
            complete: true,
        };

        assert_eq!(candle.range(), dec!(0.0200)); // high - low
    }

    #[test]
    fn test_candle_body_size_bullish() {
        let candle = Candle {
            time: Utc::now(),
            mid: Ohlc {
                open: dec!(1.1000),
                high: dec!(1.1100),
                low: dec!(1.0950),
                close: dec!(1.1050),
            },
            volume: 1000,
            complete: true,
        };

        assert_eq!(candle.body_size(), dec!(0.0050)); // |close - open|
    }

    #[test]
    fn test_candle_body_size_bearish() {
        let candle = Candle {
            time: Utc::now(),
            mid: Ohlc {
                open: dec!(1.1050),
                high: dec!(1.1100),
                low: dec!(1.0950),
                close: dec!(1.1000),
            },
            volume: 1000,
            complete: true,
        };

        assert_eq!(candle.body_size(), dec!(0.0050)); // |close - open| (absolute)
    }
}
