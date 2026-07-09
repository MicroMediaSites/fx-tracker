//! Built-in trading strategies for backtesting

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::VecDeque;

use crate::models::Candle;
use super::strategy::{Signal, Strategy};

/// Simple Moving Average Crossover Strategy
///
/// Generates buy signals when fast MA crosses above slow MA,
/// and sell signals when fast MA crosses below slow MA.
pub struct MovingAverageCrossover {
    fast_period: usize,
    slow_period: usize,
    fast_prices: VecDeque<Decimal>,
    slow_prices: VecDeque<Decimal>,
    prev_fast_ma: Option<Decimal>,
    prev_slow_ma: Option<Decimal>,
}

impl MovingAverageCrossover {
    pub fn new(fast_period: usize, slow_period: usize) -> Self {
        assert!(fast_period < slow_period, "Fast period must be less than slow period");
        Self {
            fast_period,
            slow_period,
            fast_prices: VecDeque::with_capacity(fast_period),
            slow_prices: VecDeque::with_capacity(slow_period),
            prev_fast_ma: None,
            prev_slow_ma: None,
        }
    }

    fn calculate_ma(prices: &VecDeque<Decimal>, period: usize) -> Option<Decimal> {
        if prices.len() < period {
            return None;
        }
        let sum: Decimal = prices.iter().rev().take(period).sum();
        Some(sum / Decimal::from(period as u32))
    }
}

impl Strategy for MovingAverageCrossover {
    fn on_candle(&mut self, candle: &Candle) -> Signal {
        let close = candle.mid.close;

        // Add to price buffers
        self.fast_prices.push_back(close);
        self.slow_prices.push_back(close);

        // Trim buffers
        while self.fast_prices.len() > self.fast_period {
            self.fast_prices.pop_front();
        }
        while self.slow_prices.len() > self.slow_period {
            self.slow_prices.pop_front();
        }

        // Calculate MAs
        let fast_ma = Self::calculate_ma(&self.fast_prices, self.fast_period);
        let slow_ma = Self::calculate_ma(&self.slow_prices, self.slow_period);

        let signal = match (fast_ma, slow_ma, self.prev_fast_ma, self.prev_slow_ma) {
            (Some(fast), Some(slow), Some(prev_fast), Some(prev_slow)) => {
                // Golden cross: fast crosses above slow
                if prev_fast <= prev_slow && fast > slow {
                    Signal::Buy
                }
                // Death cross: fast crosses below slow
                else if prev_fast >= prev_slow && fast < slow {
                    Signal::Sell
                } else {
                    Signal::Hold
                }
            }
            _ => Signal::Hold,
        };

        // Update previous MAs
        self.prev_fast_ma = fast_ma;
        self.prev_slow_ma = slow_ma;

        signal
    }

    fn name(&self) -> &str {
        "Moving Average Crossover"
    }

    fn reset(&mut self) {
        self.fast_prices.clear();
        self.slow_prices.clear();
        self.prev_fast_ma = None;
        self.prev_slow_ma = None;
    }
}

/// RSI (Relative Strength Index) Strategy
///
/// Generates buy signals when RSI crosses above oversold level,
/// and sell signals when RSI crosses below overbought level.
pub struct RsiStrategy {
    period: usize,
    overbought: Decimal,
    oversold: Decimal,
    prices: VecDeque<Decimal>,
    prev_rsi: Option<Decimal>,
}

impl RsiStrategy {
    pub fn new(period: usize, overbought: Decimal, oversold: Decimal) -> Self {
        Self {
            period,
            overbought,
            oversold,
            prices: VecDeque::with_capacity(period + 1),
            prev_rsi: None,
        }
    }

    fn calculate_rsi(&self) -> Option<Decimal> {
        if self.prices.len() < self.period + 1 {
            return None;
        }

        let mut gains = Decimal::ZERO;
        let mut losses = Decimal::ZERO;

        let prices: Vec<_> = self.prices.iter().collect();
        for i in 1..=self.period {
            let change = *prices[i] - *prices[i - 1];
            if change > Decimal::ZERO {
                gains += change;
            } else {
                losses += change.abs();
            }
        }

        let avg_gain = gains / Decimal::from(self.period as u32);
        let avg_loss = losses / Decimal::from(self.period as u32);

        if avg_loss == Decimal::ZERO {
            return Some(dec!(100));
        }

        let rs = avg_gain / avg_loss;
        let rsi = dec!(100) - (dec!(100) / (dec!(1) + rs));

        Some(rsi)
    }
}

impl Strategy for RsiStrategy {
    fn on_candle(&mut self, candle: &Candle) -> Signal {
        self.prices.push_back(candle.mid.close);

        while self.prices.len() > self.period + 1 {
            self.prices.pop_front();
        }

        let rsi = self.calculate_rsi();

        let signal = match (rsi, self.prev_rsi) {
            (Some(current), Some(prev)) => {
                // Cross above oversold -> Buy
                if prev <= self.oversold && current > self.oversold {
                    Signal::Buy
                }
                // Cross below overbought -> Sell
                else if prev >= self.overbought && current < self.overbought {
                    Signal::Sell
                } else {
                    Signal::Hold
                }
            }
            _ => Signal::Hold,
        };

        self.prev_rsi = rsi;
        signal
    }

    fn name(&self) -> &str {
        "RSI Strategy"
    }

    fn reset(&mut self) {
        self.prices.clear();
        self.prev_rsi = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Ohlc;
    use chrono::{DateTime, Utc, Duration};

    fn create_trending_candles() -> Vec<Candle> {
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let mut candles = Vec::new();
        let mut price = dec!(1.1000);

        // Create uptrend followed by downtrend
        for i in 0..50 {
            let change = if i < 25 {
                dec!(0.0010) // Uptrend
            } else {
                dec!(-0.0010) // Downtrend
            };

            price += change;
            candles.push(Candle {
                time: base_time + Duration::hours(i),
                mid: Ohlc {
                    open: price - change,
                    high: price + dec!(0.0005),
                    low: price - dec!(0.0015),
                    close: price,
                },
                volume: 1000,
                complete: true,
            });
        }

        candles
    }

    #[test]
    fn test_ma_crossover_signals() {
        // Use shorter periods so crossovers can occur with 50 candles
        let mut strategy = MovingAverageCrossover::new(3, 7);
        let candles = create_trending_candles();

        let mut signals = Vec::new();
        for candle in &candles {
            let signal = strategy.on_candle(candle);
            if signal != Signal::Hold {
                signals.push(signal);
            }
        }

        // With an uptrend followed by downtrend, we expect at least some signals
        // The exact signals depend on the crossover timing
        assert!(!signals.is_empty(), "Should generate at least one signal");
    }

    #[test]
    fn test_rsi_calculation() {
        let mut strategy = RsiStrategy::new(14, dec!(70), dec!(30));
        let candles = create_trending_candles();

        // Process all candles
        for candle in &candles {
            strategy.on_candle(candle);
        }

        // RSI should be calculable after enough candles
        assert!(strategy.prev_rsi.is_some());
        let rsi = strategy.prev_rsi.unwrap();
        assert!(rsi >= Decimal::ZERO && rsi <= dec!(100));
    }
}
