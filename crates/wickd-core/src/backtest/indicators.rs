//! Technical indicator calculations
//!
//! Each indicator maintains its own state and produces outputs that rules can reference.

use chrono::Datelike;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::{HashMap, VecDeque};

use crate::models::Candle;

/// Indicator outputs are stored by name
pub type IndicatorOutputs = HashMap<String, Decimal>;

/// Trait for all indicators
pub trait Indicator: Send {
    /// Process a new candle and return updated outputs
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs;

    /// Get the indicator type name
    fn indicator_type(&self) -> &str;

    /// Get all output names this indicator produces
    fn output_names(&self) -> Vec<&str>;

    /// Reset indicator state
    fn reset(&mut self);
}

// ============================================================================
// Simple Moving Average
// ============================================================================

pub struct SmaIndicator {
    period: usize,
    prices: VecDeque<Decimal>,
}

impl SmaIndicator {
    pub fn new(period: usize) -> Self {
        Self {
            period,
            prices: VecDeque::with_capacity(period),
        }
    }
}

impl Indicator for SmaIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        self.prices.push_back(candle.mid.close);
        while self.prices.len() > self.period {
            self.prices.pop_front();
        }

        let mut outputs = HashMap::new();
        if self.prices.len() >= self.period {
            let sum: Decimal = self.prices.iter().sum();
            let value = sum / Decimal::from(self.period as u32);
            outputs.insert("value".to_string(), value);
        }
        outputs
    }

    fn indicator_type(&self) -> &str {
        "sma"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["value"]
    }

    fn reset(&mut self) {
        self.prices.clear();
    }
}

// ============================================================================
// Exponential Moving Average
// ============================================================================

pub struct EmaIndicator {
    period: usize,
    multiplier: Decimal,
    ema: Option<Decimal>,
    candle_count: usize,
    initial_sum: Decimal,
}

impl EmaIndicator {
    pub fn new(period: usize) -> Self {
        let multiplier = Decimal::from(2) / Decimal::from(period as u32 + 1);
        Self {
            period,
            multiplier,
            ema: None,
            candle_count: 0,
            initial_sum: Decimal::ZERO,
        }
    }
}

impl Indicator for EmaIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        let price = candle.mid.close;
        self.candle_count += 1;

        let mut outputs = HashMap::new();

        if self.ema.is_none() {
            // Use SMA for first EMA value
            self.initial_sum += price;
            if self.candle_count >= self.period {
                let sma = self.initial_sum / Decimal::from(self.period as u32);
                self.ema = Some(sma);
                outputs.insert("value".to_string(), sma);
            }
        } else {
            let prev_ema = self.ema.unwrap();
            let new_ema = (price - prev_ema) * self.multiplier + prev_ema;
            self.ema = Some(new_ema);
            outputs.insert("value".to_string(), new_ema);
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "ema"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["value"]
    }

    fn reset(&mut self) {
        self.ema = None;
        self.candle_count = 0;
        self.initial_sum = Decimal::ZERO;
    }
}

// ============================================================================
// RSI (Relative Strength Index)
// ============================================================================

pub struct RsiIndicator {
    period: usize,
    prices: VecDeque<Decimal>,
    prev_avg_gain: Option<Decimal>,
    prev_avg_loss: Option<Decimal>,
}

impl RsiIndicator {
    pub fn new(period: usize) -> Self {
        Self {
            period,
            prices: VecDeque::with_capacity(period + 1),
            prev_avg_gain: None,
            prev_avg_loss: None,
        }
    }
}

impl Indicator for RsiIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        self.prices.push_back(candle.mid.close);
        while self.prices.len() > self.period + 1 {
            self.prices.pop_front();
        }

        let mut outputs = HashMap::new();

        if self.prices.len() < self.period + 1 {
            return outputs;
        }

        if self.prev_avg_gain.is_none() {
            // First RSI calculation - use simple averages
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
            self.prev_avg_gain = Some(avg_gain);
            self.prev_avg_loss = Some(avg_loss);

            let rsi = if avg_loss == Decimal::ZERO {
                dec!(100)
            } else {
                let rs = avg_gain / avg_loss;
                dec!(100) - (dec!(100) / (dec!(1) + rs))
            };
            outputs.insert("value".to_string(), rsi);
        } else {
            // Smoothed RSI calculation
            let prices: Vec<_> = self.prices.iter().collect();
            let change = *prices[self.period] - *prices[self.period - 1];

            let (gain, loss) = if change > Decimal::ZERO {
                (change, Decimal::ZERO)
            } else {
                (Decimal::ZERO, change.abs())
            };

            let period_dec = Decimal::from(self.period as u32);
            let prev_gain = self.prev_avg_gain.unwrap();
            let prev_loss = self.prev_avg_loss.unwrap();

            let avg_gain = (prev_gain * (period_dec - dec!(1)) + gain) / period_dec;
            let avg_loss = (prev_loss * (period_dec - dec!(1)) + loss) / period_dec;

            self.prev_avg_gain = Some(avg_gain);
            self.prev_avg_loss = Some(avg_loss);

            let rsi = if avg_loss == Decimal::ZERO {
                dec!(100)
            } else {
                let rs = avg_gain / avg_loss;
                dec!(100) - (dec!(100) / (dec!(1) + rs))
            };
            outputs.insert("value".to_string(), rsi);
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "rsi"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["value"]
    }

    fn reset(&mut self) {
        self.prices.clear();
        self.prev_avg_gain = None;
        self.prev_avg_loss = None;
    }
}

// ============================================================================
// MFI (Money Flow Index)
// ============================================================================

pub struct MfiIndicator {
    period: usize,
    typical_prices: VecDeque<Decimal>,
    volumes: VecDeque<i32>,
    prev_avg_positive_flow: Option<Decimal>,
    prev_avg_negative_flow: Option<Decimal>,
}

impl MfiIndicator {
    pub fn new(period: usize) -> Self {
        Self {
            period,
            typical_prices: VecDeque::with_capacity(period + 1),
            volumes: VecDeque::with_capacity(period + 1),
            prev_avg_positive_flow: None,
            prev_avg_negative_flow: None,
        }
    }
}

impl Indicator for MfiIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        // Typical Price = (High + Low + Close) / 3
        let typical_price = (candle.mid.high + candle.mid.low + candle.mid.close) / dec!(3);

        self.typical_prices.push_back(typical_price);
        self.volumes.push_back(candle.volume);

        while self.typical_prices.len() > self.period + 1 {
            self.typical_prices.pop_front();
            self.volumes.pop_front();
        }

        let mut outputs = HashMap::new();

        if self.typical_prices.len() < self.period + 1 {
            return outputs;
        }

        if self.prev_avg_positive_flow.is_none() {
            // First MFI calculation - use simple averages
            let mut positive_flow = Decimal::ZERO;
            let mut negative_flow = Decimal::ZERO;

            let prices: Vec<_> = self.typical_prices.iter().collect();
            let vols: Vec<_> = self.volumes.iter().collect();

            for i in 1..=self.period {
                let money_flow = *prices[i] * Decimal::from(*vols[i]);
                if *prices[i] > *prices[i - 1] {
                    positive_flow += money_flow;
                } else if *prices[i] < *prices[i - 1] {
                    negative_flow += money_flow;
                }
                // If prices are equal, money flow is ignored (neither positive nor negative)
            }

            let avg_positive = positive_flow / Decimal::from(self.period as u32);
            let avg_negative = negative_flow / Decimal::from(self.period as u32);
            self.prev_avg_positive_flow = Some(avg_positive);
            self.prev_avg_negative_flow = Some(avg_negative);

            let mfi = if avg_negative == Decimal::ZERO {
                dec!(100)
            } else {
                let money_ratio = avg_positive / avg_negative;
                dec!(100) - (dec!(100) / (dec!(1) + money_ratio))
            };
            outputs.insert("value".to_string(), mfi);
        } else {
            // Smoothed MFI calculation (similar to RSI smoothing)
            let prices: Vec<_> = self.typical_prices.iter().collect();
            let vols: Vec<_> = self.volumes.iter().collect();

            let current_tp = *prices[self.period];
            let prev_tp = *prices[self.period - 1];
            let current_vol = *vols[self.period];
            let money_flow = current_tp * Decimal::from(current_vol);

            let (pos_flow, neg_flow) = if current_tp > prev_tp {
                (money_flow, Decimal::ZERO)
            } else if current_tp < prev_tp {
                (Decimal::ZERO, money_flow)
            } else {
                (Decimal::ZERO, Decimal::ZERO)
            };

            let period_dec = Decimal::from(self.period as u32);
            let prev_pos = self.prev_avg_positive_flow.unwrap();
            let prev_neg = self.prev_avg_negative_flow.unwrap();

            let avg_positive = (prev_pos * (period_dec - dec!(1)) + pos_flow) / period_dec;
            let avg_negative = (prev_neg * (period_dec - dec!(1)) + neg_flow) / period_dec;

            self.prev_avg_positive_flow = Some(avg_positive);
            self.prev_avg_negative_flow = Some(avg_negative);

            let mfi = if avg_negative == Decimal::ZERO {
                dec!(100)
            } else {
                let money_ratio = avg_positive / avg_negative;
                dec!(100) - (dec!(100) / (dec!(1) + money_ratio))
            };
            outputs.insert("value".to_string(), mfi);
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "mfi"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["value"]
    }

    fn reset(&mut self) {
        self.typical_prices.clear();
        self.volumes.clear();
        self.prev_avg_positive_flow = None;
        self.prev_avg_negative_flow = None;
    }
}

// ============================================================================
// Donchian Channel
// ============================================================================

pub struct DonchianIndicator {
    period: usize,
    highs: VecDeque<Decimal>,
    lows: VecDeque<Decimal>,
}

impl DonchianIndicator {
    pub fn new(period: usize) -> Self {
        Self {
            period,
            highs: VecDeque::with_capacity(period),
            lows: VecDeque::with_capacity(period),
        }
    }
}

impl Indicator for DonchianIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        self.highs.push_back(candle.mid.high);
        self.lows.push_back(candle.mid.low);

        while self.highs.len() > self.period {
            self.highs.pop_front();
            self.lows.pop_front();
        }

        let mut outputs = HashMap::new();

        if self.highs.len() >= self.period {
            let upper = self.highs.iter().max().copied().unwrap();
            let lower = self.lows.iter().min().copied().unwrap();
            let middle = (upper + lower) / dec!(2);

            outputs.insert("upper".to_string(), upper);
            outputs.insert("lower".to_string(), lower);
            outputs.insert("middle".to_string(), middle);
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "donchian"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["upper", "middle", "lower"]
    }

    fn reset(&mut self) {
        self.highs.clear();
        self.lows.clear();
    }
}

// ============================================================================
// ATR (Average True Range)
// ============================================================================

pub struct AtrIndicator {
    period: usize,
    prev_close: Option<Decimal>,
    tr_values: VecDeque<Decimal>,
    atr: Option<Decimal>,
}

impl AtrIndicator {
    pub fn new(period: usize) -> Self {
        Self {
            period,
            prev_close: None,
            tr_values: VecDeque::with_capacity(period),
            atr: None,
        }
    }
}

impl Indicator for AtrIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        let mut outputs = HashMap::new();

        // Calculate True Range
        let tr = match self.prev_close {
            Some(prev_close) => {
                let hl = candle.mid.high - candle.mid.low;
                let hc = (candle.mid.high - prev_close).abs();
                let lc = (candle.mid.low - prev_close).abs();
                hl.max(hc).max(lc)
            }
            None => candle.mid.high - candle.mid.low,
        };

        self.prev_close = Some(candle.mid.close);
        self.tr_values.push_back(tr);

        while self.tr_values.len() > self.period {
            self.tr_values.pop_front();
        }

        if self.tr_values.len() >= self.period {
            match self.atr {
                None => {
                    // Initial ATR = simple average
                    let sum: Decimal = self.tr_values.iter().sum();
                    let atr = sum / Decimal::from(self.period as u32);
                    self.atr = Some(atr);
                    outputs.insert("value".to_string(), atr);
                }
                Some(prev_atr) => {
                    // Smoothed ATR
                    let period_dec = Decimal::from(self.period as u32);
                    let atr = (prev_atr * (period_dec - dec!(1)) + tr) / period_dec;
                    self.atr = Some(atr);
                    outputs.insert("value".to_string(), atr);
                }
            }
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "atr"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["value"]
    }

    fn reset(&mut self) {
        self.prev_close = None;
        self.tr_values.clear();
        self.atr = None;
    }
}

// ============================================================================
// ADX - Average Directional Index
// ============================================================================

/// ADX (Average Directional Index) - measures trend strength
/// Outputs: value (ADX), plus_di (+DI), minus_di (-DI)
pub struct AdxIndicator {
    period: usize,
    prev_high: Option<Decimal>,
    prev_low: Option<Decimal>,
    prev_close: Option<Decimal>,
    /// Smoothed +DM
    smoothed_plus_dm: Option<Decimal>,
    /// Smoothed -DM
    smoothed_minus_dm: Option<Decimal>,
    /// Smoothed TR
    smoothed_tr: Option<Decimal>,
    /// DX values for ADX smoothing
    dx_values: VecDeque<Decimal>,
    /// Smoothed ADX
    adx: Option<Decimal>,
    /// Counter for initial warmup
    count: usize,
}

impl AdxIndicator {
    pub fn new(period: usize) -> Self {
        Self {
            period,
            prev_high: None,
            prev_low: None,
            prev_close: None,
            smoothed_plus_dm: None,
            smoothed_minus_dm: None,
            smoothed_tr: None,
            dx_values: VecDeque::with_capacity(period),
            adx: None,
            count: 0,
        }
    }
}

impl Indicator for AdxIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        let mut outputs = HashMap::new();
        self.count += 1;

        // Need at least one previous candle for directional movement
        if self.prev_high.is_none() {
            self.prev_high = Some(candle.mid.high);
            self.prev_low = Some(candle.mid.low);
            self.prev_close = Some(candle.mid.close);
            return outputs;
        }

        let prev_high = self.prev_high.unwrap();
        let prev_low = self.prev_low.unwrap();
        let prev_close = self.prev_close.unwrap();

        // Calculate directional movement
        let up_move = candle.mid.high - prev_high;
        let down_move = prev_low - candle.mid.low;

        let plus_dm = if up_move > down_move && up_move > Decimal::ZERO {
            up_move
        } else {
            Decimal::ZERO
        };

        let minus_dm = if down_move > up_move && down_move > Decimal::ZERO {
            down_move
        } else {
            Decimal::ZERO
        };

        // Calculate True Range
        let hl = candle.mid.high - candle.mid.low;
        let hc = (candle.mid.high - prev_close).abs();
        let lc = (candle.mid.low - prev_close).abs();
        let tr = hl.max(hc).max(lc);

        // Update previous values
        self.prev_high = Some(candle.mid.high);
        self.prev_low = Some(candle.mid.low);
        self.prev_close = Some(candle.mid.close);

        let period_dec = Decimal::from(self.period as u32);

        // Wilder's smoothing for +DM, -DM, and TR
        match (self.smoothed_plus_dm, self.smoothed_minus_dm, self.smoothed_tr) {
            (Some(s_plus_dm), Some(s_minus_dm), Some(s_tr)) => {
                // Wilder's smoothing: new = prev - (prev / period) + current
                let new_plus_dm = s_plus_dm - (s_plus_dm / period_dec) + plus_dm;
                let new_minus_dm = s_minus_dm - (s_minus_dm / period_dec) + minus_dm;
                let new_tr = s_tr - (s_tr / period_dec) + tr;

                self.smoothed_plus_dm = Some(new_plus_dm);
                self.smoothed_minus_dm = Some(new_minus_dm);
                self.smoothed_tr = Some(new_tr);

                // Calculate +DI and -DI
                if new_tr > Decimal::ZERO {
                    let plus_di = Decimal::from(100) * new_plus_dm / new_tr;
                    let minus_di = Decimal::from(100) * new_minus_dm / new_tr;

                    outputs.insert("plus_di".to_string(), plus_di);
                    outputs.insert("minus_di".to_string(), minus_di);

                    // Calculate DX
                    let di_sum = plus_di + minus_di;
                    if di_sum > Decimal::ZERO {
                        let dx = Decimal::from(100) * (plus_di - minus_di).abs() / di_sum;
                        self.dx_values.push_back(dx);

                        while self.dx_values.len() > self.period {
                            self.dx_values.pop_front();
                        }

                        // Calculate ADX using Wilder's smoothing
                        if self.dx_values.len() >= self.period {
                            match self.adx {
                                None => {
                                    // Initial ADX = simple average of DX
                                    let sum: Decimal = self.dx_values.iter().sum();
                                    let adx_val = sum / period_dec;
                                    self.adx = Some(adx_val);
                                    outputs.insert("value".to_string(), adx_val);
                                }
                                Some(prev_adx) => {
                                    // Wilder's smoothing for ADX
                                    let adx_val = (prev_adx * (period_dec - Decimal::ONE) + dx) / period_dec;
                                    self.adx = Some(adx_val);
                                    outputs.insert("value".to_string(), adx_val);
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                // Initial period: accumulate values
                // After `period` candles, calculate initial smoothed values
                if self.count >= self.period {
                    // For initial smoothing, just use the current values
                    // (In practice, we should accumulate and average, but this is simpler)
                    self.smoothed_plus_dm = Some(plus_dm * period_dec);
                    self.smoothed_minus_dm = Some(minus_dm * period_dec);
                    self.smoothed_tr = Some(tr * period_dec);
                }
            }
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "adx"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["value", "plus_di", "minus_di"]
    }

    fn reset(&mut self) {
        self.prev_high = None;
        self.prev_low = None;
        self.prev_close = None;
        self.smoothed_plus_dm = None;
        self.smoothed_minus_dm = None;
        self.smoothed_tr = None;
        self.dx_values.clear();
        self.adx = None;
        self.count = 0;
    }
}

// ============================================================================
// Ichimoku Cloud
// ============================================================================

pub struct IchimokuIndicator {
    tenkan_period: usize,
    kijun_period: usize,
    senkou_b_period: usize,
    displacement: usize,
    highs: VecDeque<Decimal>,
    lows: VecDeque<Decimal>,
    close_buffer: VecDeque<Decimal>,
    senkou_a_buffer: VecDeque<Option<Decimal>>,
    senkou_b_buffer: VecDeque<Option<Decimal>>,
}

impl IchimokuIndicator {
    pub fn new(
        tenkan_period: usize,
        kijun_period: usize,
        senkou_b_period: usize,
        displacement: usize,
    ) -> Self {
        Self {
            tenkan_period,
            kijun_period,
            senkou_b_period,
            displacement,
            highs: VecDeque::with_capacity(senkou_b_period),
            lows: VecDeque::with_capacity(senkou_b_period),
            close_buffer: VecDeque::with_capacity(displacement + 1),
            senkou_a_buffer: VecDeque::with_capacity(displacement + 1),
            senkou_b_buffer: VecDeque::with_capacity(displacement + 1),
        }
    }

    fn period_high_low(&self, period: usize) -> Option<(Decimal, Decimal)> {
        if self.highs.len() < period {
            return None;
        }
        let high = self.highs.iter().rev().take(period).max().copied()?;
        let low = self.lows.iter().rev().take(period).min().copied()?;
        Some((high, low))
    }
}

impl Indicator for IchimokuIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        self.highs.push_back(candle.mid.high);
        self.lows.push_back(candle.mid.low);
        self.close_buffer.push_back(candle.mid.close);

        while self.highs.len() > self.senkou_b_period {
            self.highs.pop_front();
        }
        while self.lows.len() > self.senkou_b_period {
            self.lows.pop_front();
        }

        let mut outputs = HashMap::new();

        // Tenkan-sen (Conversion Line)
        if let Some((high, low)) = self.period_high_low(self.tenkan_period) {
            let tenkan = (high + low) / dec!(2);
            outputs.insert("tenkan".to_string(), tenkan);
        }

        // Kijun-sen (Base Line)
        if let Some((high, low)) = self.period_high_low(self.kijun_period) {
            let kijun = (high + low) / dec!(2);
            outputs.insert("kijun".to_string(), kijun);
        }

        // Calculate current senkou values
        let current_senkou_a = match (
            self.period_high_low(self.tenkan_period),
            self.period_high_low(self.kijun_period),
        ) {
            (Some((th, tl)), Some((kh, kl))) => {
                let tenkan = (th + tl) / dec!(2);
                let kijun = (kh + kl) / dec!(2);
                Some((tenkan + kijun) / dec!(2))
            }
            _ => None,
        };

        let current_senkou_b = self.period_high_low(self.senkou_b_period).map(|(h, l)| (h + l) / dec!(2));

        // Buffer senkou values for displacement delay
        self.senkou_a_buffer.push_back(current_senkou_a);
        self.senkou_b_buffer.push_back(current_senkou_b);

        // Pop displaced values when buffer exceeds displacement
        let displaced_senkou_a = if self.senkou_a_buffer.len() > self.displacement {
            self.senkou_a_buffer.pop_front().flatten()
        } else {
            None
        };

        let displaced_senkou_b = if self.senkou_b_buffer.len() > self.displacement {
            self.senkou_b_buffer.pop_front().flatten()
        } else {
            None
        };

        // Senkou Span A - output displaced value (for backtesting: "what cloud is at candle N?")
        if let Some(senkou_a) = displaced_senkou_a {
            outputs.insert("senkou_a".to_string(), senkou_a);
        }

        // Senkou Span B - output displaced value (for backtesting)
        if let Some(senkou_b) = displaced_senkou_b {
            outputs.insert("senkou_b".to_string(), senkou_b);
        }

        // Cloud top/bottom from displaced values
        if let (Some(&senkou_a), Some(&senkou_b)) = (
            outputs.get("senkou_a"),
            outputs.get("senkou_b"),
        ) {
            outputs.insert("cloud_top".to_string(), senkou_a.max(senkou_b));
            outputs.insert("cloud_bottom".to_string(), senkou_a.min(senkou_b));
        }

        // Raw (undisplaced) senkou outputs for charting — frontend applies displacement offset
        if let Some(sa) = current_senkou_a {
            outputs.insert("senkou_a_raw".to_string(), sa);
        }
        if let Some(sb) = current_senkou_b {
            outputs.insert("senkou_b_raw".to_string(), sb);
        }

        // Chikou Span - close from displacement candles ago (for backtesting)
        if self.close_buffer.len() > self.displacement {
            let chikou = self.close_buffer.pop_front().unwrap();
            outputs.insert("chikou".to_string(), chikou);
        }

        // Raw chikou for charting — current close, frontend shifts it left by displacement
        outputs.insert("chikou_raw".to_string(), candle.mid.close);

        outputs
    }

    fn indicator_type(&self) -> &str {
        "ichimoku"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["tenkan", "kijun", "senkou_a", "senkou_b", "cloud_top", "cloud_bottom", "chikou",
             "senkou_a_raw", "senkou_b_raw", "chikou_raw"]
    }

    fn reset(&mut self) {
        self.highs.clear();
        self.lows.clear();
        self.close_buffer.clear();
        self.senkou_a_buffer.clear();
        self.senkou_b_buffer.clear();
    }
}

// ============================================================================
// Chandelier Exit
// ============================================================================

pub struct ChandelierIndicator {
    period: usize,
    multiplier: Decimal,
    highs: VecDeque<Decimal>,
    lows: VecDeque<Decimal>,
    atr: AtrIndicator,
}

impl ChandelierIndicator {
    pub fn new(period: usize, multiplier: Decimal) -> Self {
        Self {
            period,
            multiplier,
            highs: VecDeque::with_capacity(period),
            lows: VecDeque::with_capacity(period),
            atr: AtrIndicator::new(period),
        }
    }
}

impl Indicator for ChandelierIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        self.highs.push_back(candle.mid.high);
        self.lows.push_back(candle.mid.low);

        while self.highs.len() > self.period {
            self.highs.pop_front();
        }
        while self.lows.len() > self.period {
            self.lows.pop_front();
        }

        let atr_outputs = self.atr.on_candle(candle);
        let mut outputs = HashMap::new();

        if let Some(&atr) = atr_outputs.get("value") {
            if self.highs.len() >= self.period {
                let highest = self.highs.iter().max().copied().unwrap();
                let lowest = self.lows.iter().min().copied().unwrap();

                let exit_long = highest - self.multiplier * atr;
                let exit_short = lowest + self.multiplier * atr;

                outputs.insert("exit_long".to_string(), exit_long);
                outputs.insert("exit_short".to_string(), exit_short);
            }
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "chandelier"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["exit_long", "exit_short"]
    }

    fn reset(&mut self) {
        self.highs.clear();
        self.lows.clear();
        self.atr.reset();
    }
}

// ============================================================================
// Bollinger Bands
// ============================================================================

pub struct BollingerIndicator {
    period: usize,
    std_dev: Decimal,
    prices: VecDeque<Decimal>,
}

impl BollingerIndicator {
    pub fn new(period: usize, std_dev: Decimal) -> Self {
        Self {
            period,
            std_dev,
            prices: VecDeque::with_capacity(period),
        }
    }
}

impl Indicator for BollingerIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        self.prices.push_back(candle.mid.close);
        while self.prices.len() > self.period {
            self.prices.pop_front();
        }

        let mut outputs = HashMap::new();

        if self.prices.len() >= self.period {
            let sum: Decimal = self.prices.iter().sum();
            let mean = sum / Decimal::from(self.period as u32);

            // Calculate standard deviation
            let variance_sum: Decimal = self.prices.iter()
                .map(|p| (*p - mean) * (*p - mean))
                .sum();
            let variance = variance_sum / Decimal::from(self.period as u32);

            // Approximate square root using Newton's method
            let std = decimal_sqrt(variance);

            let upper = mean + self.std_dev * std;
            let lower = mean - self.std_dev * std;

            outputs.insert("upper".to_string(), upper);
            outputs.insert("middle".to_string(), mean);
            outputs.insert("lower".to_string(), lower);
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "bollinger"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["upper", "middle", "lower"]
    }

    fn reset(&mut self) {
        self.prices.clear();
    }
}

// ============================================================================
// MACD
// ============================================================================

pub struct MacdIndicator {
    fast_ema: EmaIndicator,
    slow_ema: EmaIndicator,
    signal_ema_period: usize,
    macd_values: VecDeque<Decimal>,
    signal: Option<Decimal>,
}

impl MacdIndicator {
    pub fn new(fast_period: usize, slow_period: usize, signal_period: usize) -> Self {
        Self {
            fast_ema: EmaIndicator::new(fast_period),
            slow_ema: EmaIndicator::new(slow_period),
            signal_ema_period: signal_period,
            macd_values: VecDeque::with_capacity(signal_period),
            signal: None,
        }
    }
}

impl Indicator for MacdIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        let fast_out = self.fast_ema.on_candle(candle);
        let slow_out = self.slow_ema.on_candle(candle);

        let mut outputs = HashMap::new();

        if let (Some(&fast), Some(&slow)) = (fast_out.get("value"), slow_out.get("value")) {
            let macd = fast - slow;
            outputs.insert("macd".to_string(), macd);

            self.macd_values.push_back(macd);
            while self.macd_values.len() > self.signal_ema_period {
                self.macd_values.pop_front();
            }

            // Calculate signal line (EMA of MACD)
            if self.macd_values.len() >= self.signal_ema_period {
                let multiplier = Decimal::from(2) / Decimal::from(self.signal_ema_period as u32 + 1);

                match self.signal {
                    None => {
                        let sum: Decimal = self.macd_values.iter().sum();
                        let signal = sum / Decimal::from(self.signal_ema_period as u32);
                        self.signal = Some(signal);
                        outputs.insert("signal".to_string(), signal);
                        outputs.insert("histogram".to_string(), macd - signal);
                    }
                    Some(prev_signal) => {
                        let signal = (macd - prev_signal) * multiplier + prev_signal;
                        self.signal = Some(signal);
                        outputs.insert("signal".to_string(), signal);
                        outputs.insert("histogram".to_string(), macd - signal);
                    }
                }
            }
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "macd"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["macd", "signal", "histogram"]
    }

    fn reset(&mut self) {
        self.fast_ema.reset();
        self.slow_ema.reset();
        self.macd_values.clear();
        self.signal = None;
    }
}

// ============================================================================
// Stochastic Oscillator
// ============================================================================

pub struct StochasticIndicator {
    k_period: usize,
    d_period: usize,
    highs: VecDeque<Decimal>,
    lows: VecDeque<Decimal>,
    closes: VecDeque<Decimal>,
    k_values: VecDeque<Decimal>,
}

impl StochasticIndicator {
    pub fn new(k_period: usize, d_period: usize) -> Self {
        Self {
            k_period,
            d_period,
            highs: VecDeque::with_capacity(k_period),
            lows: VecDeque::with_capacity(k_period),
            closes: VecDeque::with_capacity(k_period),
            k_values: VecDeque::with_capacity(d_period),
        }
    }
}

impl Indicator for StochasticIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        self.highs.push_back(candle.mid.high);
        self.lows.push_back(candle.mid.low);
        self.closes.push_back(candle.mid.close);

        while self.highs.len() > self.k_period {
            self.highs.pop_front();
        }
        while self.lows.len() > self.k_period {
            self.lows.pop_front();
        }
        while self.closes.len() > self.k_period {
            self.closes.pop_front();
        }

        let mut outputs = HashMap::new();

        if self.highs.len() >= self.k_period {
            let highest = self.highs.iter().max().copied().unwrap();
            let lowest = self.lows.iter().min().copied().unwrap();
            let close = self.closes.back().copied().unwrap();

            let range = highest - lowest;
            let k = if range == Decimal::ZERO {
                dec!(50)
            } else {
                (close - lowest) / range * dec!(100)
            };

            outputs.insert("k".to_string(), k);

            self.k_values.push_back(k);
            while self.k_values.len() > self.d_period {
                self.k_values.pop_front();
            }

            if self.k_values.len() >= self.d_period {
                let d: Decimal = self.k_values.iter().sum::<Decimal>() / Decimal::from(self.d_period as u32);
                outputs.insert("d".to_string(), d);
            }
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "stochastic"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["k", "d"]
    }

    fn reset(&mut self) {
        self.highs.clear();
        self.lows.clear();
        self.closes.clear();
        self.k_values.clear();
    }
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Approximate square root using Newton's method
fn decimal_sqrt(n: Decimal) -> Decimal {
    if n <= Decimal::ZERO {
        return Decimal::ZERO;
    }

    let mut x = n;
    let two = Decimal::from(2);

    // Newton's method iterations
    for _ in 0..20 {
        let next = (x + n / x) / two;
        if (next - x).abs() < dec!(0.0000001) {
            return next;
        }
        x = next;
    }

    x
}

// ============================================================================
// MA Histogram (difference between fast and slow MA)
// ============================================================================

pub struct MaHistogramIndicator {
    fast_period: usize,
    slow_period: usize,
    fast_ema: Option<Decimal>,
    slow_ema: Option<Decimal>,
    fast_multiplier: Decimal,
    slow_multiplier: Decimal,
    candle_count: usize,
    fast_initial_sum: Decimal,
    slow_initial_sum: Decimal,
}

impl MaHistogramIndicator {
    pub fn new(fast_period: usize, slow_period: usize) -> Self {
        let fast_multiplier = Decimal::from(2) / Decimal::from(fast_period as u32 + 1);
        let slow_multiplier = Decimal::from(2) / Decimal::from(slow_period as u32 + 1);
        Self {
            fast_period,
            slow_period,
            fast_ema: None,
            slow_ema: None,
            fast_multiplier,
            slow_multiplier,
            candle_count: 0,
            fast_initial_sum: Decimal::ZERO,
            slow_initial_sum: Decimal::ZERO,
        }
    }
}

impl Indicator for MaHistogramIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        let price = candle.mid.close;
        self.candle_count += 1;

        let mut outputs = HashMap::new();

        // Update fast EMA
        self.fast_initial_sum += price;
        if self.fast_ema.is_none() && self.candle_count >= self.fast_period {
            self.fast_ema = Some(self.fast_initial_sum / Decimal::from(self.fast_period as u32));
        } else if let Some(prev) = self.fast_ema {
            self.fast_ema = Some((price - prev) * self.fast_multiplier + prev);
        }

        // Update slow EMA
        self.slow_initial_sum += price;
        if self.slow_ema.is_none() && self.candle_count >= self.slow_period {
            self.slow_ema = Some(self.slow_initial_sum / Decimal::from(self.slow_period as u32));
        } else if let Some(prev) = self.slow_ema {
            self.slow_ema = Some((price - prev) * self.slow_multiplier + prev);
        }

        // Output values when both EMAs are ready
        if let (Some(fast), Some(slow)) = (self.fast_ema, self.slow_ema) {
            outputs.insert("fast_ma".to_string(), fast);
            outputs.insert("slow_ma".to_string(), slow);
            outputs.insert("histogram".to_string(), fast - slow);
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "ma_histogram"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["histogram", "fast_ma", "slow_ma"]
    }

    fn reset(&mut self) {
        self.fast_ema = None;
        self.slow_ema = None;
        self.candle_count = 0;
        self.fast_initial_sum = Decimal::ZERO;
        self.slow_initial_sum = Decimal::ZERO;
    }
}

// ============================================================================
// MA Bands (upper/lower bands around a moving average)
// ============================================================================

pub struct MaBandsIndicator {
    period: usize,
    distance_pips: Decimal,
    ema: Option<Decimal>,
    multiplier: Decimal,
    candle_count: usize,
    initial_sum: Decimal,
}

impl MaBandsIndicator {
    pub fn new(period: usize, distance_pips: Decimal) -> Self {
        let multiplier = Decimal::from(2) / Decimal::from(period as u32 + 1);
        Self {
            period,
            distance_pips,
            ema: None,
            multiplier,
            candle_count: 0,
            initial_sum: Decimal::ZERO,
        }
    }
}

impl Indicator for MaBandsIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        let price = candle.mid.close;
        self.candle_count += 1;

        let mut outputs = HashMap::new();

        // Update EMA
        self.initial_sum += price;
        if self.ema.is_none() && self.candle_count >= self.period {
            self.ema = Some(self.initial_sum / Decimal::from(self.period as u32));
        } else if let Some(prev) = self.ema {
            self.ema = Some((price - prev) * self.multiplier + prev);
        }

        // Output bands when EMA is ready
        if let Some(ma) = self.ema {
            // Convert pips to price distance (assuming standard forex: 1 pip = 0.0001)
            let distance = self.distance_pips * dec!(0.0001);
            outputs.insert("middle".to_string(), ma);
            outputs.insert("upper".to_string(), ma + distance);
            outputs.insert("lower".to_string(), ma - distance);
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "ma_bands"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["upper", "middle", "lower"]
    }

    fn reset(&mut self) {
        self.ema = None;
        self.candle_count = 0;
        self.initial_sum = Decimal::ZERO;
    }
}

// ============================================================================
// DSS (Double Smoothed Stochastic)
// ============================================================================

pub struct DssIndicator {
    stoch_period: usize,
    ema_period: usize,
    #[allow(dead_code)]
    signal_period: usize,
    highs: VecDeque<Decimal>,
    lows: VecDeque<Decimal>,
    closes: VecDeque<Decimal>,
    raw_stoch_ema: Option<Decimal>,
    dss_ema: Option<Decimal>,
    signal_ema: Option<Decimal>,
    ema_multiplier: Decimal,
    signal_multiplier: Decimal,
    raw_stoch_values: VecDeque<Decimal>,
    dss_values: VecDeque<Decimal>,
}

impl DssIndicator {
    pub fn new(stoch_period: usize, ema_period: usize, signal_period: usize) -> Self {
        let ema_multiplier = Decimal::from(2) / Decimal::from(ema_period as u32 + 1);
        let signal_multiplier = Decimal::from(2) / Decimal::from(signal_period as u32 + 1);
        Self {
            stoch_period,
            ema_period,
            signal_period,
            highs: VecDeque::with_capacity(stoch_period),
            lows: VecDeque::with_capacity(stoch_period),
            closes: VecDeque::with_capacity(stoch_period),
            raw_stoch_ema: None,
            dss_ema: None,
            signal_ema: None,
            ema_multiplier,
            signal_multiplier,
            raw_stoch_values: VecDeque::with_capacity(ema_period),
            dss_values: VecDeque::with_capacity(signal_period),
        }
    }
}

impl Indicator for DssIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        self.highs.push_back(candle.mid.high);
        self.lows.push_back(candle.mid.low);
        self.closes.push_back(candle.mid.close);

        while self.highs.len() > self.stoch_period {
            self.highs.pop_front();
        }
        while self.lows.len() > self.stoch_period {
            self.lows.pop_front();
        }
        while self.closes.len() > self.stoch_period {
            self.closes.pop_front();
        }

        let mut outputs = HashMap::new();

        if self.highs.len() >= self.stoch_period {
            let highest = self.highs.iter().max().copied().unwrap();
            let lowest = self.lows.iter().min().copied().unwrap();
            let close = self.closes.back().copied().unwrap();

            let range = highest - lowest;
            let raw_stoch = if range == Decimal::ZERO {
                dec!(50)
            } else {
                (close - lowest) / range * dec!(100)
            };

            // First smoothing: EMA of raw stochastic
            self.raw_stoch_values.push_back(raw_stoch);
            while self.raw_stoch_values.len() > self.ema_period {
                self.raw_stoch_values.pop_front();
            }

            if self.raw_stoch_ema.is_none() && self.raw_stoch_values.len() >= self.ema_period {
                let sum: Decimal = self.raw_stoch_values.iter().sum();
                self.raw_stoch_ema = Some(sum / Decimal::from(self.ema_period as u32));
            } else if let Some(prev) = self.raw_stoch_ema {
                self.raw_stoch_ema = Some((raw_stoch - prev) * self.ema_multiplier + prev);
            }

            // Second smoothing: EMA of first EMA (this is the DSS value)
            if let Some(first_smooth) = self.raw_stoch_ema {
                self.dss_values.push_back(first_smooth);
                while self.dss_values.len() > self.ema_period {
                    self.dss_values.pop_front();
                }

                if self.dss_ema.is_none() && self.dss_values.len() >= self.ema_period {
                    let sum: Decimal = self.dss_values.iter().sum();
                    self.dss_ema = Some(sum / Decimal::from(self.ema_period as u32));
                } else if let Some(prev) = self.dss_ema {
                    self.dss_ema = Some((first_smooth - prev) * self.ema_multiplier + prev);
                }

                // Output DSS value
                if let Some(dss) = self.dss_ema {
                    outputs.insert("dss".to_string(), dss);

                    // Signal line: EMA of DSS
                    if self.signal_ema.is_none() {
                        self.signal_ema = Some(dss);
                    } else if let Some(prev) = self.signal_ema {
                        self.signal_ema = Some((dss - prev) * self.signal_multiplier + prev);
                    }

                    if let Some(signal) = self.signal_ema {
                        outputs.insert("signal".to_string(), signal);
                    }
                }
            }
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "dss"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["dss", "signal"]
    }

    fn reset(&mut self) {
        self.highs.clear();
        self.lows.clear();
        self.closes.clear();
        self.raw_stoch_ema = None;
        self.dss_ema = None;
        self.signal_ema = None;
        self.raw_stoch_values.clear();
        self.dss_values.clear();
    }
}

// ============================================================================
// ADR (Average Daily Range)
// ============================================================================

pub struct AdrIndicator {
    period: usize,
    daily_ranges: VecDeque<Decimal>,
    current_day_high: Option<Decimal>,
    current_day_low: Option<Decimal>,
    last_day: Option<u32>,
}

impl AdrIndicator {
    pub fn new(period: usize) -> Self {
        Self {
            period,
            daily_ranges: VecDeque::with_capacity(period),
            current_day_high: None,
            current_day_low: None,
            last_day: None,
        }
    }
}

impl Indicator for AdrIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        let mut outputs = HashMap::new();

        // Get the day of year from the candle time
        let day = candle.time.ordinal();

        // Check if we've moved to a new day
        if let Some(last) = self.last_day {
            if day != last {
                // Close out the previous day's range
                if let (Some(high), Some(low)) = (self.current_day_high, self.current_day_low) {
                    let range = high - low;
                    self.daily_ranges.push_back(range);
                    while self.daily_ranges.len() > self.period {
                        self.daily_ranges.pop_front();
                    }
                }
                // Reset for new day
                self.current_day_high = Some(candle.mid.high);
                self.current_day_low = Some(candle.mid.low);
            } else {
                // Same day - update high/low
                if let Some(high) = self.current_day_high {
                    if candle.mid.high > high {
                        self.current_day_high = Some(candle.mid.high);
                    }
                }
                if let Some(low) = self.current_day_low {
                    if candle.mid.low < low {
                        self.current_day_low = Some(candle.mid.low);
                    }
                }
            }
        } else {
            // First candle
            self.current_day_high = Some(candle.mid.high);
            self.current_day_low = Some(candle.mid.low);
        }
        self.last_day = Some(day);

        // Calculate ADR if we have enough data
        if !self.daily_ranges.is_empty() {
            let sum: Decimal = self.daily_ranges.iter().sum();
            let adr = sum / Decimal::from(self.daily_ranges.len() as u32);
            outputs.insert("value".to_string(), adr);

            // Calculate current day's range ratio to ADR
            if let (Some(high), Some(low)) = (self.current_day_high, self.current_day_low) {
                let current_range = high - low;
                if adr > Decimal::ZERO {
                    let ratio = current_range / adr * dec!(100);
                    outputs.insert("ratio".to_string(), ratio);
                }
            }
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "adr"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["value", "ratio"]
    }

    fn reset(&mut self) {
        self.daily_ranges.clear();
        self.current_day_high = None;
        self.current_day_low = None;
        self.last_day = None;
    }
}

// ============================================================================
// Daily (Current Day's Stats)
// ============================================================================

/// Tracks the current trading day's high, low, range, and opening price.
/// Day boundaries are determined by the candle timestamps (forex typically uses 5pm EST / 22:00 UTC).
pub struct DailyIndicator {
    current_day_high: Option<Decimal>,
    current_day_low: Option<Decimal>,
    current_day_open: Option<Decimal>,
    last_day: Option<u32>,
}

impl DailyIndicator {
    pub fn new() -> Self {
        Self {
            current_day_high: None,
            current_day_low: None,
            current_day_open: None,
            last_day: None,
        }
    }
}

impl Indicator for DailyIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        let mut outputs = HashMap::new();

        // Get the day of year from the candle time
        let day = candle.time.ordinal();

        // Check if we've moved to a new day
        if let Some(last) = self.last_day {
            if day != last {
                // New day - reset stats
                self.current_day_high = Some(candle.mid.high);
                self.current_day_low = Some(candle.mid.low);
                self.current_day_open = Some(candle.mid.open);
            } else {
                // Same day - update high/low
                if let Some(high) = self.current_day_high {
                    if candle.mid.high > high {
                        self.current_day_high = Some(candle.mid.high);
                    }
                }
                if let Some(low) = self.current_day_low {
                    if candle.mid.low < low {
                        self.current_day_low = Some(candle.mid.low);
                    }
                }
            }
        } else {
            // First candle
            self.current_day_high = Some(candle.mid.high);
            self.current_day_low = Some(candle.mid.low);
            self.current_day_open = Some(candle.mid.open);
        }
        self.last_day = Some(day);

        // Output current day's values
        if let Some(high) = self.current_day_high {
            outputs.insert("high".to_string(), high);
        }
        if let Some(low) = self.current_day_low {
            outputs.insert("low".to_string(), low);
        }
        if let Some(open) = self.current_day_open {
            outputs.insert("open".to_string(), open);
        }
        if let (Some(high), Some(low)) = (self.current_day_high, self.current_day_low) {
            outputs.insert("range".to_string(), high - low);
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "daily"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["high", "low", "range", "open"]
    }

    fn reset(&mut self) {
        self.current_day_high = None;
        self.current_day_low = None;
        self.current_day_open = None;
        self.last_day = None;
    }
}

// ============================================================================
// Swing (Swing High/Low Detection)
// ============================================================================

/// Detected swing point with price and bar index
#[derive(Debug, Clone, Copy)]
struct SwingPoint {
    price: Decimal,
    bar_index: usize,  // absolute bar index when the swing occurred
}

/// Detects swing highs and swing lows in price action.
/// A swing high/low is confirmed when price on both sides (strength bars each) is lower/higher.
/// Outputs recent swing prices and how many bars ago they occurred.
pub struct SwingIndicator {
    strength: usize,
    highs: VecDeque<Decimal>,
    lows: VecDeque<Decimal>,
    swing_highs: Vec<SwingPoint>,
    swing_lows: Vec<SwingPoint>,
    bar_count: usize,
    max_swings: usize,  // how many swing points to keep
}

impl SwingIndicator {
    pub fn new(strength: usize) -> Self {
        Self {
            strength,
            highs: VecDeque::with_capacity(strength * 2 + 1),
            lows: VecDeque::with_capacity(strength * 2 + 1),
            swing_highs: Vec::new(),
            swing_lows: Vec::new(),
            bar_count: 0,
            max_swings: 10,  // keep last 10 swing points of each type
        }
    }

    /// Check if the bar at center_idx (in our buffer) is a swing high
    fn is_swing_high(&self, center_idx: usize) -> bool {
        if self.highs.len() < self.strength * 2 + 1 {
            return false;
        }
        let center_high = self.highs[center_idx];
        // Check bars before
        for i in (center_idx.saturating_sub(self.strength))..center_idx {
            if self.highs[i] >= center_high {
                return false;
            }
        }
        // Check bars after
        for i in (center_idx + 1)..=(center_idx + self.strength).min(self.highs.len() - 1) {
            if self.highs[i] >= center_high {
                return false;
            }
        }
        true
    }

    /// Check if the bar at center_idx (in our buffer) is a swing low
    fn is_swing_low(&self, center_idx: usize) -> bool {
        if self.lows.len() < self.strength * 2 + 1 {
            return false;
        }
        let center_low = self.lows[center_idx];
        // Check bars before
        for i in (center_idx.saturating_sub(self.strength))..center_idx {
            if self.lows[i] <= center_low {
                return false;
            }
        }
        // Check bars after
        for i in (center_idx + 1)..=(center_idx + self.strength).min(self.lows.len() - 1) {
            if self.lows[i] <= center_low {
                return false;
            }
        }
        true
    }
}

impl Indicator for SwingIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        let mut outputs = HashMap::new();

        // Add current candle to buffers
        self.highs.push_back(candle.mid.high);
        self.lows.push_back(candle.mid.low);
        self.bar_count += 1;

        // Keep buffer size manageable
        let buffer_size = self.strength * 2 + 1 + 10;  // some extra for lookback
        while self.highs.len() > buffer_size {
            self.highs.pop_front();
        }
        while self.lows.len() > buffer_size {
            self.lows.pop_front();
        }

        // Check for swing at the bar that's now strength bars behind current
        // (because we need strength bars after it to confirm)
        if self.highs.len() >= self.strength * 2 + 1 {
            let check_idx = self.highs.len() - 1 - self.strength;
            let check_bar = self.bar_count - 1 - self.strength;

            if self.is_swing_high(check_idx) {
                self.swing_highs.push(SwingPoint {
                    price: self.highs[check_idx],
                    bar_index: check_bar,
                });
                // Keep only recent swings
                if self.swing_highs.len() > self.max_swings {
                    self.swing_highs.remove(0);
                }
            }

            if self.is_swing_low(check_idx) {
                self.swing_lows.push(SwingPoint {
                    price: self.lows[check_idx],
                    bar_index: check_bar,
                });
                // Keep only recent swings
                if self.swing_lows.len() > self.max_swings {
                    self.swing_lows.remove(0);
                }
            }
        }

        // Output most recent swing high
        if let Some(sh) = self.swing_highs.last() {
            outputs.insert("recent_high".to_string(), sh.price);
            let bars_ago = self.bar_count.saturating_sub(sh.bar_index + 1);
            outputs.insert("recent_high_bars".to_string(), Decimal::from(bars_ago as i64));
        }

        // Output second most recent swing high (for divergence)
        if self.swing_highs.len() >= 2 {
            let sh = &self.swing_highs[self.swing_highs.len() - 2];
            outputs.insert("prev_high".to_string(), sh.price);
            let bars_ago = self.bar_count.saturating_sub(sh.bar_index + 1);
            outputs.insert("prev_high_bars".to_string(), Decimal::from(bars_ago as i64));
        }

        // Output most recent swing low
        if let Some(sl) = self.swing_lows.last() {
            outputs.insert("recent_low".to_string(), sl.price);
            let bars_ago = self.bar_count.saturating_sub(sl.bar_index + 1);
            outputs.insert("recent_low_bars".to_string(), Decimal::from(bars_ago as i64));
        }

        // Output second most recent swing low (for divergence)
        if self.swing_lows.len() >= 2 {
            let sl = &self.swing_lows[self.swing_lows.len() - 2];
            outputs.insert("prev_low".to_string(), sl.price);
            let bars_ago = self.bar_count.saturating_sub(sl.bar_index + 1);
            outputs.insert("prev_low_bars".to_string(), Decimal::from(bars_ago as i64));
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "swing"
    }

    fn output_names(&self) -> Vec<&str> {
        vec![
            "recent_high", "recent_high_bars",
            "recent_low", "recent_low_bars",
            "prev_high", "prev_high_bars",
            "prev_low", "prev_low_bars",
        ]
    }

    fn reset(&mut self) {
        self.highs.clear();
        self.lows.clear();
        self.swing_highs.clear();
        self.swing_lows.clear();
        self.bar_count = 0;
    }
}

// ============================================================================
// VWAP (Volume Weighted Average Price)
// ============================================================================

/// VWAP tracks cumulative (typical_price × volume) / cumulative volume.
/// Resets each trading day (detected via NaiveDate comparison).
/// OANDA doesn't provide real volume — uses tick count from candle.volume.
pub struct VwapIndicator {
    cumulative_tp_vol: Decimal,
    cumulative_vol: Decimal,
    last_date: Option<chrono::NaiveDate>,
}

impl VwapIndicator {
    pub fn new() -> Self {
        Self {
            cumulative_tp_vol: Decimal::ZERO,
            cumulative_vol: Decimal::ZERO,
            last_date: None,
        }
    }
}

impl Indicator for VwapIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        let mut outputs = HashMap::new();

        let date = candle.time.date_naive();

        // Reset on new day
        if let Some(last) = self.last_date {
            if date != last {
                self.cumulative_tp_vol = Decimal::ZERO;
                self.cumulative_vol = Decimal::ZERO;
            }
        }
        self.last_date = Some(date);

        let typical_price = (candle.mid.high + candle.mid.low + candle.mid.close) / dec!(3);
        let volume = Decimal::from(candle.volume);

        self.cumulative_tp_vol += typical_price * volume;
        self.cumulative_vol += volume;

        if self.cumulative_vol > Decimal::ZERO {
            let vwap = self.cumulative_tp_vol / self.cumulative_vol;
            outputs.insert("vwap".to_string(), vwap);
        } else {
            // Zero-volume candle on a new day: fall back to typical price
            outputs.insert("vwap".to_string(), typical_price);
        }

        outputs
    }

    fn indicator_type(&self) -> &str {
        "vwap"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["vwap"]
    }

    fn reset(&mut self) {
        self.cumulative_tp_vol = Decimal::ZERO;
        self.cumulative_vol = Decimal::ZERO;
        self.last_date = None;
    }
}

// ============================================================================
// Parabolic SAR (Wilder's Parabolic Stop and Reverse)
// ============================================================================

/// Classic Parabolic SAR with acceleration factor.
/// Outputs: "sar" (the SAR value), "trend" (1 = bullish, -1 = bearish)
pub struct ParabolicSarIndicator {
    af_start: Decimal,
    af_increment: Decimal,
    af_max: Decimal,
    // State
    trend_up: bool,
    sar: Decimal,
    ep: Decimal,       // extreme point
    af: Decimal,       // current acceleration factor
    candle_count: usize,
    prev_high: Decimal,
    prev_low: Decimal,
    prev_prev_high: Decimal,
    prev_prev_low: Decimal,
}

impl ParabolicSarIndicator {
    pub fn new(af_start: Decimal, af_increment: Decimal, af_max: Decimal) -> Self {
        Self {
            af_start,
            af_increment,
            af_max,
            trend_up: true,
            sar: Decimal::ZERO,
            ep: Decimal::ZERO,
            af: af_start,
            candle_count: 0,
            prev_high: Decimal::ZERO,
            prev_low: Decimal::ZERO,
            prev_prev_high: Decimal::ZERO,
            prev_prev_low: Decimal::ZERO,
        }
    }
}

impl Indicator for ParabolicSarIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        self.candle_count += 1;

        if self.candle_count == 1 {
            // First candle: store values, no output yet
            self.prev_high = candle.mid.high;
            self.prev_low = candle.mid.low;
            return HashMap::new();
        }

        if self.candle_count == 2 {
            // Second candle: initialize SAR
            self.trend_up = candle.mid.close > self.prev_low;
            if self.trend_up {
                self.sar = self.prev_low;
                self.ep = candle.mid.high;
            } else {
                self.sar = self.prev_high;
                self.ep = candle.mid.low;
            }
            self.af = self.af_start;
            self.prev_prev_high = self.prev_high;
            self.prev_prev_low = self.prev_low;
            self.prev_high = candle.mid.high;
            self.prev_low = candle.mid.low;

            let mut outputs = HashMap::new();
            outputs.insert("sar".to_string(), self.sar);
            outputs.insert("trend".to_string(), if self.trend_up { dec!(1) } else { dec!(-1) });
            return outputs;
        }

        // Calculate new SAR
        let mut new_sar = self.sar + self.af * (self.ep - self.sar);

        if self.trend_up {
            // In uptrend, SAR cannot be above the prior two lows
            new_sar = new_sar.min(self.prev_low).min(self.prev_prev_low);

            // Check for reversal: low crosses below SAR
            if candle.mid.low < new_sar {
                // Reverse to downtrend
                self.trend_up = false;
                new_sar = self.ep; // SAR = previous extreme point
                self.ep = candle.mid.low;
                self.af = self.af_start;
            } else {
                // Continue uptrend
                if candle.mid.high > self.ep {
                    self.ep = candle.mid.high;
                    self.af = (self.af + self.af_increment).min(self.af_max);
                }
            }
        } else {
            // In downtrend, SAR cannot be below the prior two highs
            new_sar = new_sar.max(self.prev_high).max(self.prev_prev_high);

            // Check for reversal: high crosses above SAR
            if candle.mid.high > new_sar {
                // Reverse to uptrend
                self.trend_up = true;
                new_sar = self.ep; // SAR = previous extreme point
                self.ep = candle.mid.high;
                self.af = self.af_start;
            } else {
                // Continue downtrend
                if candle.mid.low < self.ep {
                    self.ep = candle.mid.low;
                    self.af = (self.af + self.af_increment).min(self.af_max);
                }
            }
        }

        self.sar = new_sar;
        self.prev_prev_high = self.prev_high;
        self.prev_prev_low = self.prev_low;
        self.prev_high = candle.mid.high;
        self.prev_low = candle.mid.low;

        let mut outputs = HashMap::new();
        outputs.insert("sar".to_string(), self.sar);
        outputs.insert("trend".to_string(), if self.trend_up { dec!(1) } else { dec!(-1) });
        outputs
    }

    fn indicator_type(&self) -> &str {
        "parabolic_sar"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["sar", "trend"]
    }

    fn reset(&mut self) {
        self.trend_up = true;
        self.sar = Decimal::ZERO;
        self.ep = Decimal::ZERO;
        self.af = self.af_start;
        self.candle_count = 0;
        self.prev_high = Decimal::ZERO;
        self.prev_low = Decimal::ZERO;
        self.prev_prev_high = Decimal::ZERO;
        self.prev_prev_low = Decimal::ZERO;
    }
}

// ============================================================================
// SuperTrend
// ============================================================================

/// SuperTrend indicator using ATR-based bands.
/// Outputs: "supertrend" (the active band value), "trend" (1 = bullish, -1 = bearish)
pub struct SuperTrendIndicator {
    multiplier: Decimal,
    atr: AtrIndicator,
    // State
    prev_close: Option<Decimal>,
    prev_upper_band: Option<Decimal>,
    prev_lower_band: Option<Decimal>,
    prev_supertrend: Option<Decimal>,
    trend_up: bool,
}

impl SuperTrendIndicator {
    pub fn new(period: usize, multiplier: Decimal) -> Self {
        Self {
            multiplier,
            atr: AtrIndicator::new(period),
            prev_close: None,
            prev_upper_band: None,
            prev_lower_band: None,
            prev_supertrend: None,
            trend_up: true,
        }
    }
}

impl Indicator for SuperTrendIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        let atr_outputs = self.atr.on_candle(candle);

        let atr_value = match atr_outputs.get("value") {
            Some(&v) => v,
            None => {
                self.prev_close = Some(candle.mid.close);
                return HashMap::new();
            }
        };

        let hl2 = (candle.mid.high + candle.mid.low) / dec!(2);
        let basic_upper = hl2 + self.multiplier * atr_value;
        let basic_lower = hl2 - self.multiplier * atr_value;

        let final_upper = match (self.prev_upper_band, self.prev_close) {
            (Some(prev_upper), Some(prev_close)) => {
                if basic_upper < prev_upper || prev_close > prev_upper {
                    basic_upper
                } else {
                    prev_upper
                }
            }
            _ => basic_upper,
        };

        let final_lower = match (self.prev_lower_band, self.prev_close) {
            (Some(prev_lower), Some(prev_close)) => {
                if basic_lower > prev_lower || prev_close < prev_lower {
                    basic_lower
                } else {
                    prev_lower
                }
            }
            _ => basic_lower,
        };

        // Determine trend direction
        let supertrend = match self.prev_supertrend {
            Some(prev_st) => {
                let prev_upper = self.prev_upper_band.unwrap_or(final_upper);

                if prev_st == prev_upper {
                    // Previous was bearish (upper band)
                    if candle.mid.close > final_upper {
                        self.trend_up = true;
                        final_lower
                    } else {
                        self.trend_up = false;
                        final_upper
                    }
                } else {
                    // Previous was bullish (lower band)
                    if candle.mid.close < final_lower {
                        self.trend_up = false;
                        final_upper
                    } else {
                        self.trend_up = true;
                        final_lower
                    }
                }
            }
            None => {
                // First calculation: default to bullish if close > hl2
                self.trend_up = candle.mid.close > hl2;
                if self.trend_up { final_lower } else { final_upper }
            }
        };

        self.prev_close = Some(candle.mid.close);
        self.prev_upper_band = Some(final_upper);
        self.prev_lower_band = Some(final_lower);
        self.prev_supertrend = Some(supertrend);

        let mut outputs = HashMap::new();
        outputs.insert("supertrend".to_string(), supertrend);
        outputs.insert("trend".to_string(), if self.trend_up { dec!(1) } else { dec!(-1) });
        outputs
    }

    fn indicator_type(&self) -> &str {
        "super_trend"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["supertrend", "trend"]
    }

    fn reset(&mut self) {
        self.atr.reset();
        self.prev_close = None;
        self.prev_upper_band = None;
        self.prev_lower_band = None;
        self.prev_supertrend = None;
        self.trend_up = true;
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Ohlc;
    use chrono::{DateTime, Utc, Duration};

    fn create_test_candle(price: Decimal, time_offset: i64) -> Candle {
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        Candle {
            time: base_time + Duration::hours(time_offset),
            mid: Ohlc {
                open: price - dec!(0.0010),
                high: price + dec!(0.0010),
                low: price - dec!(0.0020),
                close: price,
            },
            volume: 1000,
            complete: true,
        }
    }

    #[test]
    fn test_sma_calculation() {
        let mut sma = SmaIndicator::new(3);

        sma.on_candle(&create_test_candle(dec!(1.1000), 0));
        sma.on_candle(&create_test_candle(dec!(1.1100), 1));
        let outputs = sma.on_candle(&create_test_candle(dec!(1.1200), 2));

        assert!(outputs.contains_key("value"));
        let expected = (dec!(1.1000) + dec!(1.1100) + dec!(1.1200)) / dec!(3);
        assert_eq!(outputs.get("value").unwrap(), &expected);
    }

    #[test]
    fn test_rsi_bounds() {
        let mut rsi = RsiIndicator::new(14);

        // Process enough candles with uptrend
        for i in 0..20 {
            let price = dec!(1.1000) + Decimal::from(i) * dec!(0.0010);
            rsi.on_candle(&create_test_candle(price, i as i64));
        }

        // RSI should be between 0 and 100
        let outputs = rsi.on_candle(&create_test_candle(dec!(1.1200), 20));
        if let Some(&value) = outputs.get("value") {
            assert!(value >= Decimal::ZERO && value <= dec!(100));
        }
    }

    #[test]
    fn test_ichimoku_outputs() {
        let mut ichi = IchimokuIndicator::new(9, 26, 52, 26);

        // Process enough candles
        for i in 0..60 {
            let price = dec!(1.1000) + Decimal::from(i % 10) * dec!(0.0010);
            ichi.on_candle(&create_test_candle(price, i as i64));
        }

        let outputs = ichi.on_candle(&create_test_candle(dec!(1.1050), 60));

        // Should have tenkan and kijun at minimum
        assert!(outputs.contains_key("tenkan"));
        assert!(outputs.contains_key("kijun"));
    }

    #[test]
    fn test_decimal_sqrt() {
        // Use approximate equality since Newton's method converges to close but not exact values
        assert!((decimal_sqrt(dec!(4)) - dec!(2)).abs() < dec!(0.0001));
        assert!((decimal_sqrt(dec!(2)) - dec!(1.4142135)).abs() < dec!(0.0001));
    }

    #[test]
    fn test_decimal_sqrt_zero_and_negative() {
        assert_eq!(decimal_sqrt(Decimal::ZERO), Decimal::ZERO);
        assert_eq!(decimal_sqrt(dec!(-1)), Decimal::ZERO);
    }

    #[test]
    fn test_sma_reset() {
        let mut sma = SmaIndicator::new(3);
        sma.on_candle(&create_test_candle(dec!(1.1000), 0));
        sma.on_candle(&create_test_candle(dec!(1.1100), 1));
        sma.on_candle(&create_test_candle(dec!(1.1200), 2));

        sma.reset();

        // After reset, should need 3 candles again
        let outputs = sma.on_candle(&create_test_candle(dec!(1.1000), 0));
        assert!(!outputs.contains_key("value"));
    }

    #[test]
    fn test_sma_indicator_type_and_output_names() {
        let sma = SmaIndicator::new(20);
        assert_eq!(sma.indicator_type(), "sma");
        assert_eq!(sma.output_names(), vec!["value"]);
    }

    #[test]
    fn test_ema_calculation() {
        let mut ema = EmaIndicator::new(3);

        // First 3 candles to get initial EMA (SMA)
        ema.on_candle(&create_test_candle(dec!(1.1000), 0));
        ema.on_candle(&create_test_candle(dec!(1.1100), 1));
        let outputs = ema.on_candle(&create_test_candle(dec!(1.1200), 2));

        assert!(outputs.contains_key("value"));
        // Initial EMA should equal SMA
        let expected_sma = (dec!(1.1000) + dec!(1.1100) + dec!(1.1200)) / dec!(3);
        assert_eq!(outputs.get("value").unwrap(), &expected_sma);

        // Fourth candle uses EMA formula
        let outputs = ema.on_candle(&create_test_candle(dec!(1.1300), 3));
        assert!(outputs.contains_key("value"));
    }

    #[test]
    fn test_ema_reset() {
        let mut ema = EmaIndicator::new(3);
        for i in 0..5 {
            ema.on_candle(&create_test_candle(dec!(1.1000), i));
        }

        ema.reset();

        // Should need period candles again
        let outputs = ema.on_candle(&create_test_candle(dec!(1.1000), 0));
        assert!(!outputs.contains_key("value"));
    }

    #[test]
    fn test_ema_indicator_type_and_output_names() {
        let ema = EmaIndicator::new(20);
        assert_eq!(ema.indicator_type(), "ema");
        assert_eq!(ema.output_names(), vec!["value"]);
    }

    #[test]
    fn test_rsi_reset() {
        let mut rsi = RsiIndicator::new(14);
        for i in 0..20 {
            rsi.on_candle(&create_test_candle(dec!(1.1000), i));
        }

        rsi.reset();

        let outputs = rsi.on_candle(&create_test_candle(dec!(1.1000), 0));
        assert!(!outputs.contains_key("value"));
    }

    #[test]
    fn test_rsi_indicator_type_and_output_names() {
        let rsi = RsiIndicator::new(14);
        assert_eq!(rsi.indicator_type(), "rsi");
        assert_eq!(rsi.output_names(), vec!["value"]);
    }

    #[test]
    fn test_rsi_downtrend() {
        let mut rsi = RsiIndicator::new(14);

        // Process candles with downtrend
        for i in 0..20 {
            let price = dec!(1.2000) - Decimal::from(i) * dec!(0.0010);
            rsi.on_candle(&create_test_candle(price, i as i64));
        }

        let outputs = rsi.on_candle(&create_test_candle(dec!(1.1500), 20));
        if let Some(&value) = outputs.get("value") {
            // In a downtrend, RSI should be low
            assert!(value < dec!(50));
        }
    }

    #[test]
    fn test_atr_calculation() {
        let mut atr = AtrIndicator::new(3);

        // Process enough candles
        for i in 0..5 {
            let price = dec!(1.1000) + Decimal::from(i) * dec!(0.0005);
            atr.on_candle(&create_test_candle(price, i as i64));
        }

        let outputs = atr.on_candle(&create_test_candle(dec!(1.1025), 5));
        assert!(outputs.contains_key("value"));
        // ATR should be positive
        assert!(*outputs.get("value").unwrap() > Decimal::ZERO);
    }

    #[test]
    fn test_atr_reset() {
        let mut atr = AtrIndicator::new(3);
        for i in 0..5 {
            atr.on_candle(&create_test_candle(dec!(1.1000), i));
        }

        atr.reset();

        let outputs = atr.on_candle(&create_test_candle(dec!(1.1000), 0));
        assert!(!outputs.contains_key("value"));
    }

    #[test]
    fn test_atr_indicator_type_and_output_names() {
        let atr = AtrIndicator::new(14);
        assert_eq!(atr.indicator_type(), "atr");
        assert_eq!(atr.output_names(), vec!["value"]);
    }

    #[test]
    fn test_ichimoku_reset() {
        let mut ichi = IchimokuIndicator::new(9, 26, 52, 26);
        for i in 0..60 {
            ichi.on_candle(&create_test_candle(dec!(1.1000), i));
        }

        ichi.reset();

        let outputs = ichi.on_candle(&create_test_candle(dec!(1.1000), 0));
        assert!(!outputs.contains_key("tenkan"));
    }

    #[test]
    fn test_ichimoku_indicator_type_and_output_names() {
        let ichi = IchimokuIndicator::new(9, 26, 52, 26);
        assert_eq!(ichi.indicator_type(), "ichimoku");
        let names = ichi.output_names();
        assert!(names.contains(&"tenkan"));
        assert!(names.contains(&"kijun"));
        assert!(names.contains(&"senkou_a"));
        assert!(names.contains(&"senkou_b"));
    }

    #[test]
    fn test_ichimoku_displacement_delays_cloud() {
        // Use small periods: tenkan=2, kijun=3, senkou_b=4, displacement=3
        let mut ichi = IchimokuIndicator::new(2, 3, 4, 3);

        // Feed enough candles to warm up senkou_b (needs 4 candles) plus displacement (3 more)
        // With varying prices so senkou values are meaningful
        let prices = vec![
            dec!(1.1000), dec!(1.1020), dec!(1.1040), dec!(1.1010),
            dec!(1.1030), dec!(1.1050), dec!(1.1060), dec!(1.1080),
            dec!(1.1070), dec!(1.1090),
        ];

        let mut all_outputs = Vec::new();
        for (i, &price) in prices.iter().enumerate() {
            let outputs = ichi.on_candle(&create_test_candle(price, i as i64));
            all_outputs.push(outputs);
        }

        // senkou_a requires tenkan (period=2) and kijun (period=3), so first computed at index 2.
        // senkou_b requires period=4, so first computed at index 3.
        // With displacement=3, displaced senkou_a appears at index 2+3=5, senkou_b at 3+3=6.
        // Cloud (needs both) first appears at index 6.

        // Before senkou_a displacement kicks in (indices 0-4): not yet displaced
        for i in 0..5 {
            assert!(
                !all_outputs[i].contains_key("senkou_a"),
                "senkou_a should not appear at candle {} (displacement not reached)",
                i
            );
        }

        // senkou_a should appear at index 5 (displaced from index 2)
        assert!(
            all_outputs[5].contains_key("senkou_a"),
            "senkou_a should appear at candle 5 (displaced from candle 2)"
        );
        // But senkou_b should NOT yet appear at index 5
        assert!(
            !all_outputs[5].contains_key("senkou_b"),
            "senkou_b should not appear at candle 5 (displacement not reached)"
        );

        // At index 6: both senkou_a and senkou_b should appear, enabling cloud
        assert!(
            all_outputs[6].contains_key("senkou_a"),
            "senkou_a should appear at candle 6"
        );
        assert!(
            all_outputs[6].contains_key("senkou_b"),
            "senkou_b should appear at candle 6 (displaced from candle 3)"
        );
        assert!(
            all_outputs[6].contains_key("cloud_top"),
            "cloud_top should appear at candle 6"
        );
        assert!(
            all_outputs[6].contains_key("cloud_bottom"),
            "cloud_bottom should appear at candle 6"
        );

        // The displaced senkou_a at index 6 should equal the senkou_a that was computed
        // at index 3 (6 - displacement=3), NOT the current senkou_a value.
        // Verify this by computing what senkou_a should have been at index 3:
        // At index 3, the last 2 highs/lows for tenkan (period=2): candles 2,3
        //   highs: 1.1040+0.001, 1.1010+0.001 => tenkan = (1.1050 + 1.1000) / 2 = 1.1025
        //   Wait - create_test_candle uses high=price+0.001, low=price-0.002
        // Let's just verify the displaced value differs from current-candle value
        // by checking that index 6's senkou_a != what we'd get without displacement
        // We'll do this by running a second indicator with displacement=0 and comparing
        let mut ichi_no_disp = IchimokuIndicator::new(2, 3, 4, 0);
        let mut no_disp_outputs = Vec::new();
        for (i, &price) in prices.iter().enumerate() {
            let outputs = ichi_no_disp.on_candle(&create_test_candle(price, i as i64));
            no_disp_outputs.push(outputs);
        }

        // With displacement=0, senkou_a at index 6 reflects current candle 6's computation
        // With displacement=3, senkou_a at index 6 reflects candle 3's computation
        // These should differ because prices change between candle 3 and 6
        let displaced_val = all_outputs[6].get("senkou_a").unwrap();
        let current_val = no_disp_outputs[6].get("senkou_a").unwrap();
        assert_ne!(
            displaced_val, current_val,
            "Displaced senkou_a should differ from current-candle senkou_a"
        );
    }

    #[test]
    fn test_ichimoku_chikou_displacement() {
        // Chikou span should output close from `displacement` candles ago
        let mut ichi = IchimokuIndicator::new(2, 3, 4, 3);

        let prices = vec![
            dec!(1.1000), dec!(1.1020), dec!(1.1040), dec!(1.1010),
            dec!(1.1030), dec!(1.1050), dec!(1.1060),
        ];

        let mut all_outputs = Vec::new();
        for (i, &price) in prices.iter().enumerate() {
            let outputs = ichi.on_candle(&create_test_candle(price, i as i64));
            all_outputs.push(outputs);
        }

        // With displacement=3, chikou should not appear for the first 3 candles
        // (indices 0, 1, 2) since we don't have enough history
        for i in 0..3 {
            assert!(
                !all_outputs[i].contains_key("chikou"),
                "chikou should not appear at candle {} (not enough history for displacement)",
                i
            );
        }

        // At index 3 (4th candle), chikou should be the close from 3 candles ago (index 0)
        // create_test_candle sets close = price, so close at index 0 = 1.1000
        assert_eq!(
            all_outputs[3].get("chikou"),
            Some(&dec!(1.1000)),
            "chikou at candle 3 should be close from candle 0"
        );

        // At index 4, chikou should be close from index 1 = 1.1020
        assert_eq!(
            all_outputs[4].get("chikou"),
            Some(&dec!(1.1020)),
            "chikou at candle 4 should be close from candle 1"
        );

        // At index 5, chikou should be close from index 2 = 1.1040
        assert_eq!(
            all_outputs[5].get("chikou"),
            Some(&dec!(1.1040)),
            "chikou at candle 5 should be close from candle 2"
        );

        // At index 6, chikou should be close from index 3 = 1.1010
        assert_eq!(
            all_outputs[6].get("chikou"),
            Some(&dec!(1.1010)),
            "chikou at candle 6 should be close from candle 3"
        );
    }

    #[test]
    fn test_ichimoku_displacement_zero() {
        // With displacement=0, all values should pass through immediately (no delay)
        let mut ichi = IchimokuIndicator::new(2, 3, 4, 0);

        let prices = vec![
            dec!(1.1000), dec!(1.1020), dec!(1.1040), dec!(1.1010),
            dec!(1.1030), dec!(1.1050),
        ];

        let mut all_outputs = Vec::new();
        for (i, &price) in prices.iter().enumerate() {
            let outputs = ichi.on_candle(&create_test_candle(price, i as i64));
            all_outputs.push(outputs);
        }

        // With displacement=0, senkou values should appear as soon as periods are met
        // senkou_b needs 4 candles, so first available at index 3
        assert!(
            all_outputs[3].contains_key("senkou_a"),
            "senkou_a should appear immediately at candle 3 with displacement=0"
        );
        assert!(
            all_outputs[3].contains_key("senkou_b"),
            "senkou_b should appear immediately at candle 3 with displacement=0"
        );
        assert!(
            all_outputs[3].contains_key("cloud_top"),
            "cloud_top should appear immediately at candle 3 with displacement=0"
        );
        assert!(
            all_outputs[3].contains_key("cloud_bottom"),
            "cloud_bottom should appear immediately at candle 3 with displacement=0"
        );

        // Chikou should also appear immediately (close of current candle, 0 displacement)
        assert!(
            all_outputs[0].contains_key("chikou"),
            "chikou should appear at candle 0 with displacement=0"
        );
        // Chikou value should be the current close
        assert_eq!(
            all_outputs[0].get("chikou"),
            Some(&dec!(1.1000)),
            "chikou with displacement=0 should equal current close"
        );
        assert_eq!(
            all_outputs[3].get("chikou"),
            Some(&dec!(1.1010)),
            "chikou with displacement=0 should equal current close at candle 3"
        );
    }

    #[test]
    fn test_chandelier_calculation() {
        let mut chandelier = ChandelierIndicator::new(3, dec!(2));

        for i in 0..5 {
            let price = dec!(1.1000) + Decimal::from(i) * dec!(0.0005);
            chandelier.on_candle(&create_test_candle(price, i as i64));
        }

        let outputs = chandelier.on_candle(&create_test_candle(dec!(1.1025), 5));
        assert!(outputs.contains_key("exit_long"));
        assert!(outputs.contains_key("exit_short"));
    }

    #[test]
    fn test_chandelier_reset() {
        let mut chandelier = ChandelierIndicator::new(3, dec!(2));
        for i in 0..5 {
            chandelier.on_candle(&create_test_candle(dec!(1.1000), i));
        }

        chandelier.reset();

        let outputs = chandelier.on_candle(&create_test_candle(dec!(1.1000), 0));
        assert!(!outputs.contains_key("exit_long"));
    }

    #[test]
    fn test_chandelier_indicator_type_and_output_names() {
        let chandelier = ChandelierIndicator::new(22, dec!(3));
        assert_eq!(chandelier.indicator_type(), "chandelier");
        assert_eq!(chandelier.output_names(), vec!["exit_long", "exit_short"]);
    }

    #[test]
    fn test_bollinger_calculation() {
        let mut bb = BollingerIndicator::new(3, dec!(2));

        bb.on_candle(&create_test_candle(dec!(1.1000), 0));
        bb.on_candle(&create_test_candle(dec!(1.1010), 1));
        let outputs = bb.on_candle(&create_test_candle(dec!(1.1020), 2));

        assert!(outputs.contains_key("upper"));
        assert!(outputs.contains_key("middle"));
        assert!(outputs.contains_key("lower"));

        // Upper should be greater than middle, middle greater than lower
        let upper = *outputs.get("upper").unwrap();
        let middle = *outputs.get("middle").unwrap();
        let lower = *outputs.get("lower").unwrap();
        assert!(upper > middle);
        assert!(middle > lower);
    }

    #[test]
    fn test_bollinger_reset() {
        let mut bb = BollingerIndicator::new(3, dec!(2));
        for i in 0..5 {
            bb.on_candle(&create_test_candle(dec!(1.1000), i));
        }

        bb.reset();

        let outputs = bb.on_candle(&create_test_candle(dec!(1.1000), 0));
        assert!(!outputs.contains_key("upper"));
    }

    #[test]
    fn test_bollinger_indicator_type_and_output_names() {
        let bb = BollingerIndicator::new(20, dec!(2));
        assert_eq!(bb.indicator_type(), "bollinger");
        assert_eq!(bb.output_names(), vec!["upper", "middle", "lower"]);
    }

    #[test]
    fn test_macd_calculation() {
        let mut macd = MacdIndicator::new(3, 5, 3);

        // Process enough candles for slow EMA + signal
        for i in 0..15 {
            let price = dec!(1.1000) + Decimal::from(i % 5) * dec!(0.0010);
            macd.on_candle(&create_test_candle(price, i as i64));
        }

        let outputs = macd.on_candle(&create_test_candle(dec!(1.1050), 15));
        assert!(outputs.contains_key("macd"));
        assert!(outputs.contains_key("signal"));
        assert!(outputs.contains_key("histogram"));
    }

    #[test]
    fn test_macd_reset() {
        let mut macd = MacdIndicator::new(3, 5, 3);
        for i in 0..15 {
            macd.on_candle(&create_test_candle(dec!(1.1000), i));
        }

        macd.reset();

        let outputs = macd.on_candle(&create_test_candle(dec!(1.1000), 0));
        assert!(!outputs.contains_key("macd"));
    }

    #[test]
    fn test_macd_indicator_type_and_output_names() {
        let macd = MacdIndicator::new(12, 26, 9);
        assert_eq!(macd.indicator_type(), "macd");
        assert_eq!(macd.output_names(), vec!["macd", "signal", "histogram"]);
    }

    #[test]
    fn test_stochastic_calculation() {
        let mut stoch = StochasticIndicator::new(3, 3);

        // Create varying prices
        stoch.on_candle(&create_test_candle(dec!(1.1000), 0));
        stoch.on_candle(&create_test_candle(dec!(1.1050), 1));
        stoch.on_candle(&create_test_candle(dec!(1.0950), 2));
        stoch.on_candle(&create_test_candle(dec!(1.1025), 3));
        stoch.on_candle(&create_test_candle(dec!(1.1000), 4));
        let outputs = stoch.on_candle(&create_test_candle(dec!(1.1010), 5));

        assert!(outputs.contains_key("k"));
        assert!(outputs.contains_key("d"));

        // K and D should be between 0 and 100
        let k = *outputs.get("k").unwrap();
        let d = *outputs.get("d").unwrap();
        assert!(k >= Decimal::ZERO && k <= dec!(100));
        assert!(d >= Decimal::ZERO && d <= dec!(100));
    }

    #[test]
    fn test_stochastic_flat_range() {
        let mut stoch = StochasticIndicator::new(3, 3);

        // Create a truly flat candle (open = high = low = close)
        fn create_flat_candle(price: Decimal, time_offset: i64) -> Candle {
            let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc);

            Candle {
                time: base_time + Duration::hours(time_offset),
                mid: Ohlc {
                    open: price,
                    high: price,
                    low: price,
                    close: price,
                },
                volume: 1000,
                complete: true,
            }
        }

        for i in 0..5 {
            stoch.on_candle(&create_flat_candle(dec!(1.1000), i as i64));
        }

        let outputs = stoch.on_candle(&create_flat_candle(dec!(1.1000), 5));
        if let Some(&k) = outputs.get("k") {
            // When range is zero, stochastic returns 50
            assert_eq!(k, dec!(50));
        }
    }

    #[test]
    fn test_stochastic_reset() {
        let mut stoch = StochasticIndicator::new(3, 3);
        for i in 0..10 {
            stoch.on_candle(&create_test_candle(dec!(1.1000), i));
        }

        stoch.reset();

        let outputs = stoch.on_candle(&create_test_candle(dec!(1.1000), 0));
        assert!(!outputs.contains_key("k"));
    }

    #[test]
    fn test_stochastic_indicator_type_and_output_names() {
        let stoch = StochasticIndicator::new(14, 3);
        assert_eq!(stoch.indicator_type(), "stochastic");
        assert_eq!(stoch.output_names(), vec!["k", "d"]);
    }

    // ====================================================================
    // VWAP Tests
    // ====================================================================

    #[test]
    fn test_vwap_calculation() {
        let mut vwap = VwapIndicator::new();

        // First candle: VWAP = typical price (since single candle)
        let c1 = create_test_candle(dec!(1.1000), 0);
        let outputs = vwap.on_candle(&c1);
        assert!(outputs.contains_key("vwap"));

        // Typical price = (high + low + close) / 3 = (1.1010 + 1.0980 + 1.1000) / 3
        let tp1 = (dec!(1.1010) + dec!(1.0980) + dec!(1.1000)) / dec!(3);
        assert_eq!(outputs.get("vwap").unwrap(), &tp1);

        // Second candle with different price
        let c2 = create_test_candle(dec!(1.1100), 1);
        let outputs = vwap.on_candle(&c2);
        assert!(outputs.contains_key("vwap"));

        // VWAP should be weighted average of both candles
        let tp2 = (dec!(1.1110) + dec!(1.1080) + dec!(1.1100)) / dec!(3);
        let vol = dec!(1000);
        let expected = (tp1 * vol + tp2 * vol) / (vol + vol);
        assert_eq!(outputs.get("vwap").unwrap(), &expected);
    }

    #[test]
    fn test_vwap_daily_reset() {
        let mut vwap = VwapIndicator::new();

        // Feed a candle on day 1 (offset 0 = 2024-01-01T00:00)
        let c1 = create_test_candle(dec!(1.1000), 0);
        vwap.on_candle(&c1);

        // Feed another candle on day 1 (offset 4 = 2024-01-01T04:00)
        let c2 = create_test_candle(dec!(1.1100), 4);
        vwap.on_candle(&c2);

        // Feed a candle on day 2 (offset 24 = 2024-01-02T00:00)
        let c3 = create_test_candle(dec!(1.1200), 24);
        let outputs = vwap.on_candle(&c3);

        // After reset, VWAP should equal the typical price of just the day-2 candle
        let tp3 = (dec!(1.1210) + dec!(1.1180) + dec!(1.1200)) / dec!(3);
        assert_eq!(outputs.get("vwap").unwrap(), &tp3);
    }

    #[test]
    fn test_vwap_indicator_type_and_output_names() {
        let vwap = VwapIndicator::new();
        assert_eq!(vwap.indicator_type(), "vwap");
        assert_eq!(vwap.output_names(), vec!["vwap"]);
    }

    #[test]
    fn test_vwap_reset() {
        let mut vwap = VwapIndicator::new();
        vwap.on_candle(&create_test_candle(dec!(1.1000), 0));
        vwap.on_candle(&create_test_candle(dec!(1.1100), 1));

        vwap.reset();

        // After reset, cumulative values should be cleared
        let outputs = vwap.on_candle(&create_test_candle(dec!(1.1200), 24));
        let tp = (dec!(1.1210) + dec!(1.1180) + dec!(1.1200)) / dec!(3);
        assert_eq!(outputs.get("vwap").unwrap(), &tp);
    }

    // ====================================================================
    // Parabolic SAR Tests
    // ====================================================================

    #[test]
    fn test_parabolic_sar_basic() {
        let mut psar = ParabolicSarIndicator::new(dec!(0.02), dec!(0.02), dec!(0.20));

        // Feed uptrending candles
        let mut outputs = HashMap::new();
        for i in 0..10 {
            let price = dec!(1.1000) + Decimal::from(i) * dec!(0.0020);
            outputs = psar.on_candle(&create_test_candle(price, i as i64));
        }

        // In an uptrend, SAR should be below price
        assert!(outputs.contains_key("sar"));
        assert!(outputs.contains_key("trend"));

        let sar = *outputs.get("sar").unwrap();
        let trend = *outputs.get("trend").unwrap();

        // Price is around 1.118, SAR should be well below
        assert!(sar < dec!(1.1180));
        assert_eq!(trend, dec!(1)); // bullish
    }

    #[test]
    fn test_parabolic_sar_reversal() {
        let mut psar = ParabolicSarIndicator::new(dec!(0.02), dec!(0.02), dec!(0.20));

        // Feed uptrending candles first
        for i in 0..8 {
            let price = dec!(1.1000) + Decimal::from(i) * dec!(0.0020);
            psar.on_candle(&create_test_candle(price, i as i64));
        }

        // Now feed sharply downtrending candles to trigger reversal
        let mut last_outputs = HashMap::new();
        for i in 8..16 {
            let price = dec!(1.1140) - Decimal::from(i - 8) * dec!(0.0030);
            last_outputs = psar.on_candle(&create_test_candle(price, i as i64));
        }

        // After sharp reversal, trend should have flipped to bearish
        let trend = *last_outputs.get("trend").unwrap();
        assert_eq!(trend, dec!(-1)); // bearish
    }

    #[test]
    fn test_parabolic_sar_warmup() {
        let mut psar = ParabolicSarIndicator::new(dec!(0.02), dec!(0.02), dec!(0.20));

        // First candle should return empty
        let outputs = psar.on_candle(&create_test_candle(dec!(1.1000), 0));
        assert!(outputs.is_empty());

        // Second candle should have values
        let outputs = psar.on_candle(&create_test_candle(dec!(1.1020), 1));
        assert!(outputs.contains_key("sar"));
        assert!(outputs.contains_key("trend"));
    }

    #[test]
    fn test_parabolic_sar_indicator_type_and_output_names() {
        let psar = ParabolicSarIndicator::new(dec!(0.02), dec!(0.02), dec!(0.20));
        assert_eq!(psar.indicator_type(), "parabolic_sar");
        assert_eq!(psar.output_names(), vec!["sar", "trend"]);
    }

    #[test]
    fn test_parabolic_sar_reset() {
        let mut psar = ParabolicSarIndicator::new(dec!(0.02), dec!(0.02), dec!(0.20));
        for i in 0..5 {
            psar.on_candle(&create_test_candle(dec!(1.1000), i));
        }

        psar.reset();

        // After reset, first candle should return empty again
        let outputs = psar.on_candle(&create_test_candle(dec!(1.1000), 0));
        assert!(outputs.is_empty());
    }

    // ====================================================================
    // SuperTrend Tests
    // ====================================================================

    #[test]
    fn test_supertrend_bullish() {
        let mut st = SuperTrendIndicator::new(3, dec!(2));

        // Feed uptrending candles
        let mut outputs = HashMap::new();
        for i in 0..10 {
            let price = dec!(1.1000) + Decimal::from(i) * dec!(0.0020);
            outputs = st.on_candle(&create_test_candle(price, i as i64));
        }

        // After warmup and uptrend, should be bullish
        assert!(outputs.contains_key("supertrend"));
        assert!(outputs.contains_key("trend"));
        assert_eq!(*outputs.get("trend").unwrap(), dec!(1)); // bullish
    }

    #[test]
    fn test_supertrend_bearish() {
        let mut st = SuperTrendIndicator::new(3, dec!(2));

        // Feed downtrending candles
        let mut outputs = HashMap::new();
        for i in 0..10 {
            let price = dec!(1.2000) - Decimal::from(i) * dec!(0.0020);
            outputs = st.on_candle(&create_test_candle(price, i as i64));
        }

        // After warmup and downtrend, should be bearish
        assert!(outputs.contains_key("supertrend"));
        assert!(outputs.contains_key("trend"));
        assert_eq!(*outputs.get("trend").unwrap(), dec!(-1)); // bearish
    }

    #[test]
    fn test_supertrend_warmup() {
        let mut st = SuperTrendIndicator::new(5, dec!(3));

        // First few candles should return empty (ATR needs period candles)
        for i in 0..4 {
            let outputs = st.on_candle(&create_test_candle(dec!(1.1000), i as i64));
            assert!(outputs.is_empty(), "SuperTrend should not output during ATR warmup at candle {}", i);
        }

        // After enough candles, should produce output
        let outputs = st.on_candle(&create_test_candle(dec!(1.1050), 4));
        assert!(outputs.contains_key("supertrend"));
    }

    #[test]
    fn test_supertrend_reset() {
        let mut st = SuperTrendIndicator::new(3, dec!(2));
        for i in 0..10 {
            st.on_candle(&create_test_candle(dec!(1.1000), i));
        }

        st.reset();

        // After reset, should need warmup again
        let outputs = st.on_candle(&create_test_candle(dec!(1.1000), 0));
        assert!(outputs.is_empty());
    }

    #[test]
    fn test_supertrend_indicator_type_and_output_names() {
        let st = SuperTrendIndicator::new(10, dec!(3));
        assert_eq!(st.indicator_type(), "super_trend");
        assert_eq!(st.output_names(), vec!["supertrend", "trend"]);
    }
}
