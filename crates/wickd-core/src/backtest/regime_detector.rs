//! Market Regime Detection
//!
//! Detects predefined market conditions for strategy "givens" triggers.
//! Each regime has hardcoded detection logic that runs on the backtest engine's
//! indicator data.
//!
//! Supports three categories of regimes:
//! 1. Trend/Volatility - based on indicators (ADX, ATR, BB)
//! 2. S/R Zones - based on user-defined support/resistance
//! 3. Price Action Patterns - programmatically detected (gaps, order blocks, etc.)

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::VecDeque;

use crate::models::Candle;
use super::rules_engine::{MarketRegime, SRZone};

// ============================================================================
// Price Action Pattern Types
// ============================================================================

/// Detected gap zone (bullish = gap up, bearish = gap down)
#[derive(Debug, Clone)]
pub struct GapZone {
    pub gap_type: GapType,
    pub upper_price: Decimal,
    pub lower_price: Decimal,
    pub formation_index: usize,
    pub filled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapType {
    Bullish,  // Gap up - becomes support
    Bearish,  // Gap down - becomes resistance
}

/// Detected base/supply-demand zone
#[derive(Debug, Clone)]
pub struct BaseZone {
    pub zone_type: BaseZoneType,
    pub upper_price: Decimal,
    pub lower_price: Decimal,
    pub formation_index: usize,
    pub times_tested: u32,
    pub broken: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseZoneType {
    Supply,   // RBD or DBD
    Demand,   // DBR or RBR
}

/// Detected order block
#[derive(Debug, Clone)]
pub struct OrderBlock {
    pub ob_type: OrderBlockType,
    pub upper_price: Decimal,
    pub lower_price: Decimal,
    pub formation_index: usize,
    pub mitigated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderBlockType {
    Bullish,  // Last bearish candle before bullish impulse
    Bearish,  // Last bullish candle before bearish impulse
}

/// Detected structure level (for retest detection)
#[derive(Debug, Clone)]
pub struct StructureLevel {
    pub level_type: StructureLevelType,
    pub price: Decimal,
    pub formation_index: usize,
    pub break_index: Option<usize>,
    pub retested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureLevelType {
    PivotHigh,
    PivotLow,
}

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for regime detection thresholds
#[derive(Debug, Clone)]
pub struct RegimeConfig {
    // Trend/Volatility thresholds
    /// ADX threshold for trend detection (default: 25)
    pub adx_trend_threshold: Decimal,
    /// ADX threshold for ranging detection (default: 20)
    pub adx_range_threshold: Decimal,
    /// ATR multiplier for high volatility (default: 1.5)
    pub high_vol_multiplier: Decimal,
    /// ATR multiplier for low volatility (default: 0.5)
    pub low_vol_multiplier: Decimal,
    /// Number of periods for ATR rolling average (default: 14)
    pub atr_rolling_periods: usize,
    /// Bollinger Band width contraction threshold (default: 0.02 = 2%)
    pub bb_contraction_threshold: Decimal,
    /// Distance in pips for S/R zone tested (default: 20)
    pub sr_test_distance_pips: Decimal,

    // Price Action thresholds
    /// Minimum gap size in pips (default: 5)
    pub min_gap_pips: Decimal,
    /// Minimum impulse size as ATR multiple for base zones (default: 1.5)
    pub min_impulse_atr: Decimal,
    /// Maximum base range as ATR multiple (default: 0.8)
    pub max_base_range_atr: Decimal,
    /// Minimum impulse for order block (default: 2.0 ATR)
    pub min_ob_impulse_atr: Decimal,
    /// Pivot strength (bars on each side) (default: 3)
    pub pivot_strength: usize,
    /// Distance threshold for "at zone" detection in pips (default: 15)
    pub zone_distance_pips: Decimal,

    /// Pip value for the instrument (0.0001 for most pairs, 0.01 for JPY pairs)
    pub pip_value: Decimal,
}

impl Default for RegimeConfig {
    fn default() -> Self {
        Self {
            // Trend/Volatility defaults
            adx_trend_threshold: dec!(25),
            adx_range_threshold: dec!(20),
            high_vol_multiplier: dec!(1.5),
            low_vol_multiplier: dec!(0.5),
            atr_rolling_periods: 14,
            bb_contraction_threshold: dec!(0.02),
            sr_test_distance_pips: dec!(20),
            // Price Action defaults
            min_gap_pips: dec!(5),
            min_impulse_atr: dec!(1.5),
            max_base_range_atr: dec!(0.8),
            min_ob_impulse_atr: dec!(2.0),
            pivot_strength: 3,
            zone_distance_pips: dec!(15),
            // Standard forex pip value (use 0.01 for JPY pairs)
            pip_value: dec!(0.0001),
        }
    }
}

impl RegimeConfig {
    /// Create config for JPY pairs (uses 0.01 pip value)
    pub fn for_jpy() -> Self {
        let mut config = Self::default();
        config.pip_value = dec!(0.01);
        config
    }

    /// Create config for a specific instrument
    pub fn for_instrument(instrument: &str) -> Self {
        if instrument.to_uppercase().ends_with("JPY") {
            Self::for_jpy()
        } else {
            Self::default()
        }
    }
}

// ============================================================================
// Regime Detector
// ============================================================================

/// Market regime detector - evaluates regime conditions based on indicator values
/// and detected price action patterns.
pub struct RegimeDetector {
    config: RegimeConfig,
    /// Rolling ATR values for average calculation
    atr_history: VecDeque<Decimal>,
    /// User-defined S/R zones
    sr_zones: Vec<SRZone>,
    /// Detected gap zones
    gap_zones: Vec<GapZone>,
    /// Detected supply/demand base zones
    base_zones: Vec<BaseZone>,
    /// Detected order blocks
    order_blocks: Vec<OrderBlock>,
    /// Detected structure levels (for retest detection)
    structure_levels: Vec<StructureLevel>,
    /// Current candle index in the backtest
    current_index: usize,
}

impl RegimeDetector {
    pub fn new(config: RegimeConfig) -> Self {
        Self {
            config,
            atr_history: VecDeque::with_capacity(50),
            sr_zones: Vec::new(),
            gap_zones: Vec::new(),
            base_zones: Vec::new(),
            order_blocks: Vec::new(),
            structure_levels: Vec::new(),
            current_index: 0,
        }
    }

    /// Set S/R zones for sr_tested regime detection
    pub fn set_sr_zones(&mut self, zones: Vec<SRZone>) {
        self.sr_zones = zones;
    }

    /// Update the current candle index
    pub fn set_current_index(&mut self, index: usize) {
        self.current_index = index;
    }

    /// Update ATR history for volatility regime detection
    pub fn update_atr(&mut self, atr: Decimal) {
        self.atr_history.push_back(atr);
        if self.atr_history.len() > 50 {
            self.atr_history.pop_front();
        }
    }

    /// Get rolling average ATR
    fn get_average_atr(&self) -> Option<Decimal> {
        if self.atr_history.len() < self.config.atr_rolling_periods {
            return None;
        }

        let sum: Decimal = self.atr_history
            .iter()
            .rev()
            .take(self.config.atr_rolling_periods)
            .sum();

        Some(sum / Decimal::from(self.config.atr_rolling_periods))
    }

    /// Check if a specific market regime is active
    pub fn is_regime_active(
        &self,
        regime: MarketRegime,
        candle: &Candle,
        adx: Option<Decimal>,
        sma20: Option<Decimal>,
        sma50: Option<Decimal>,
        atr: Option<Decimal>,
        bb_upper: Option<Decimal>,
        bb_lower: Option<Decimal>,
        bb_middle: Option<Decimal>,
    ) -> bool {
        match regime {
            // Trend/Volatility regimes
            MarketRegime::TrendingUp => {
                self.check_trending_up(candle, adx, sma20, sma50)
            }
            MarketRegime::TrendingDown => {
                self.check_trending_down(candle, adx, sma20, sma50)
            }
            MarketRegime::Ranging => {
                self.check_ranging(adx, bb_upper, bb_lower, bb_middle)
            }
            MarketRegime::HighVolatility => {
                self.check_high_volatility(atr)
            }
            MarketRegime::LowVolatility => {
                self.check_low_volatility(atr)
            }
            // S/R regime
            MarketRegime::SrTested => {
                self.check_sr_tested(candle)
            }
            // Price Action - Gaps
            MarketRegime::AtBullishGap => {
                self.check_at_gap(candle, GapType::Bullish)
            }
            MarketRegime::AtBearishGap => {
                self.check_at_gap(candle, GapType::Bearish)
            }
            // Price Action - Supply/Demand
            MarketRegime::AtDemandZone => {
                self.check_at_base_zone(candle, BaseZoneType::Demand)
            }
            MarketRegime::AtSupplyZone => {
                self.check_at_base_zone(candle, BaseZoneType::Supply)
            }
            // Price Action - Order Blocks
            MarketRegime::AtBullishOb => {
                self.check_at_order_block(candle, OrderBlockType::Bullish)
            }
            MarketRegime::AtBearishOb => {
                self.check_at_order_block(candle, OrderBlockType::Bearish)
            }
            // Price Action - Structure Retest
            MarketRegime::RetestingSupport => {
                self.check_retesting_support(candle)
            }
            MarketRegime::RetestingResistance => {
                self.check_retesting_resistance(candle)
            }
            // Trading Sessions - evaluated directly in rules_triggers.rs
            MarketRegime::LondonSession
            | MarketRegime::UsSession
            | MarketRegime::AsianSession => {
                // Sessions are evaluated by time check in evaluate_givens_trigger
                false
            }
            // Divergence - evaluated directly in rules_triggers.rs
            MarketRegime::Divergence => {
                // Divergence requires config from GivensTrigger, evaluated there
                false
            }
        }
    }

    /// Trending Up: ADX > 25, price > SMA20 > SMA50
    fn check_trending_up(
        &self,
        candle: &Candle,
        adx: Option<Decimal>,
        sma20: Option<Decimal>,
        sma50: Option<Decimal>,
    ) -> bool {
        match (adx, sma20, sma50) {
            (Some(adx_val), Some(sma20_val), Some(sma50_val)) => {
                let price = candle.mid.close;
                adx_val > self.config.adx_trend_threshold
                    && price > sma20_val
                    && sma20_val > sma50_val
            }
            _ => false,
        }
    }

    /// Trending Down: ADX > 25, price < SMA20 < SMA50
    fn check_trending_down(
        &self,
        candle: &Candle,
        adx: Option<Decimal>,
        sma20: Option<Decimal>,
        sma50: Option<Decimal>,
    ) -> bool {
        match (adx, sma20, sma50) {
            (Some(adx_val), Some(sma20_val), Some(sma50_val)) => {
                let price = candle.mid.close;
                adx_val > self.config.adx_trend_threshold
                    && price < sma20_val
                    && sma20_val < sma50_val
            }
            _ => false,
        }
    }

    /// Ranging: ADX < 20, BB width contracted
    fn check_ranging(
        &self,
        adx: Option<Decimal>,
        bb_upper: Option<Decimal>,
        bb_lower: Option<Decimal>,
        bb_middle: Option<Decimal>,
    ) -> bool {
        match (adx, bb_upper, bb_lower, bb_middle) {
            (Some(adx_val), Some(upper), Some(lower), Some(middle)) => {
                if adx_val >= self.config.adx_range_threshold {
                    return false;
                }

                // Calculate BB width as percentage of middle
                if middle == Decimal::ZERO {
                    return false;
                }

                let bb_width = (upper - lower) / middle;
                bb_width < self.config.bb_contraction_threshold
            }
            // If we don't have BB, just check ADX
            (Some(adx_val), _, _, _) => {
                adx_val < self.config.adx_range_threshold
            }
            _ => false,
        }
    }

    /// S/R Tested: Price within X pips of user's S/R zone
    fn check_sr_tested(&self, candle: &Candle) -> bool {
        if self.sr_zones.is_empty() {
            return false;
        }

        let price = candle.mid.close;
        let pip_value = self.config.pip_value;
        let distance_threshold = self.config.sr_test_distance_pips * pip_value;

        for zone in &self.sr_zones {
            // Check if price is near upper boundary
            if (price - zone.upper_price).abs() <= distance_threshold {
                return true;
            }
            // Check if price is near lower boundary
            if (price - zone.lower_price).abs() <= distance_threshold {
                return true;
            }
        }

        false
    }

    /// High Volatility: ATR > 1.5x rolling average ATR
    fn check_high_volatility(&self, atr: Option<Decimal>) -> bool {
        match (atr, self.get_average_atr()) {
            (Some(current_atr), Some(avg_atr)) => {
                current_atr > avg_atr * self.config.high_vol_multiplier
            }
            _ => false,
        }
    }

    /// Low Volatility: ATR < 0.5x rolling average ATR
    fn check_low_volatility(&self, atr: Option<Decimal>) -> bool {
        match (atr, self.get_average_atr()) {
            (Some(current_atr), Some(avg_atr)) => {
                current_atr < avg_atr * self.config.low_vol_multiplier
            }
            _ => false,
        }
    }

    // ========================================================================
    // Price Action Pattern Detection
    // ========================================================================

    /// Update detected patterns from candle history.
    /// Should be called once at the start of a backtest with full candle data.
    pub fn detect_patterns(&mut self, candles: &[Candle], atr_values: &[Decimal]) {
        self.detect_gaps(candles);
        self.detect_base_zones(candles, atr_values);
        self.detect_order_blocks(candles, atr_values);
        self.detect_structure_levels(candles);
    }

    /// Detect gap zones in candle data
    fn detect_gaps(&mut self, candles: &[Candle]) {
        self.gap_zones.clear();

        if candles.len() < 2 {
            return;
        }

        let pip_value = self.config.pip_value;
        let min_gap = self.config.min_gap_pips * pip_value;

        for i in 1..candles.len() {
            let prev = &candles[i - 1];
            let curr = &candles[i];

            // Check for bullish gap (gap up): current low > previous high
            if curr.mid.low > prev.mid.high {
                let gap_size = curr.mid.low - prev.mid.high;
                if gap_size >= min_gap {
                    self.gap_zones.push(GapZone {
                        gap_type: GapType::Bullish,
                        upper_price: curr.mid.low,
                        lower_price: prev.mid.high,
                        formation_index: i,
                        filled: false,
                    });
                }
            }

            // Check for bearish gap (gap down): current high < previous low
            if curr.mid.high < prev.mid.low {
                let gap_size = prev.mid.low - curr.mid.high;
                if gap_size >= min_gap {
                    self.gap_zones.push(GapZone {
                        gap_type: GapType::Bearish,
                        upper_price: prev.mid.low,
                        lower_price: curr.mid.high,
                        formation_index: i,
                        filled: false,
                    });
                }
            }
        }

        // Update fill status
        self.update_gap_fill_status(candles);
    }

    fn update_gap_fill_status(&mut self, candles: &[Candle]) {
        for gap in &mut self.gap_zones {
            for candle in candles.iter().skip(gap.formation_index + 1) {
                let filled = match gap.gap_type {
                    GapType::Bullish => candle.mid.low <= gap.upper_price,
                    GapType::Bearish => candle.mid.high >= gap.lower_price,
                };
                if filled {
                    gap.filled = true;
                    break;
                }
            }
        }
    }

    /// Detect base (supply/demand) zones
    /// Pattern: Impulse → Base (consolidation) → Impulse
    fn detect_base_zones(&mut self, candles: &[Candle], atr_values: &[Decimal]) {
        self.base_zones.clear();

        if candles.len() < 5 || atr_values.is_empty() {
            return;
        }

        let mut i = 0;
        while i < candles.len().saturating_sub(4) {
            let atr = if i < atr_values.len() { atr_values[i] } else { continue; };
            let min_impulse = atr * self.config.min_impulse_atr;
            let max_base_range = atr * self.config.max_base_range_atr;

            // Look for impulse move (1-3 candles)
            if let Some((impulse_end, impulse_dir)) = self.find_impulse(candles, i, min_impulse) {
                // Look for base after impulse (2-4 candles)
                if let Some((base_end, base_high, base_low)) =
                    self.find_base(candles, impulse_end + 1, max_base_range)
                {
                    // Look for departure impulse
                    if let Some((_, departure_dir)) =
                        self.find_impulse(candles, base_end + 1, min_impulse)
                    {
                        let zone_type = match (impulse_dir, departure_dir) {
                            (true, false) | (false, false) => BaseZoneType::Supply, // RBD or DBD
                            (false, true) | (true, true) => BaseZoneType::Demand,   // DBR or RBR
                        };

                        self.base_zones.push(BaseZone {
                            zone_type,
                            upper_price: base_high,
                            lower_price: base_low,
                            formation_index: base_end,
                            times_tested: 0,
                            broken: false,
                        });

                        i = base_end + 1;
                        continue;
                    }
                }
            }
            i += 1;
        }

        // Update zone status
        self.update_base_zone_status(candles);
    }

    /// Find an impulse move starting at index
    /// Returns (end_index, is_bullish) or None
    fn find_impulse(&self, candles: &[Candle], start: usize, min_move: Decimal) -> Option<(usize, bool)> {
        if start >= candles.len() {
            return None;
        }

        for len in 1..=3 {
            let end = start + len;
            if end > candles.len() {
                break;
            }

            let first = &candles[start];
            let last = &candles[end - 1];

            // Bullish impulse
            let bullish_move = last.mid.close - first.mid.open;
            if bullish_move >= min_move {
                return Some((end - 1, true));
            }

            // Bearish impulse
            let bearish_move = first.mid.open - last.mid.close;
            if bearish_move >= min_move {
                return Some((end - 1, false));
            }
        }

        None
    }

    /// Find a consolidation base starting at index
    /// Returns (end_index, high, low) or None
    fn find_base(&self, candles: &[Candle], start: usize, max_range: Decimal) -> Option<(usize, Decimal, Decimal)> {
        if start >= candles.len() {
            return None;
        }

        for len in 2..=4 {
            let end = start + len;
            if end > candles.len() {
                break;
            }

            let window = &candles[start..end];
            let high = window.iter().map(|c| c.mid.high).max()?;
            let low = window.iter().map(|c| c.mid.low).min()?;
            let range = high - low;

            if range <= max_range {
                return Some((end - 1, high, low));
            }
        }

        None
    }

    fn update_base_zone_status(&mut self, candles: &[Candle]) {
        for zone in &mut self.base_zones {
            for candle in candles.iter().skip(zone.formation_index + 1) {
                let touched = candle.mid.low <= zone.upper_price && candle.mid.high >= zone.lower_price;

                if touched {
                    match zone.zone_type {
                        BaseZoneType::Supply => {
                            if candle.mid.close > zone.upper_price {
                                zone.broken = true;
                                break;
                            } else {
                                zone.times_tested += 1;
                            }
                        }
                        BaseZoneType::Demand => {
                            if candle.mid.close < zone.lower_price {
                                zone.broken = true;
                                break;
                            } else {
                                zone.times_tested += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Detect order blocks (last opposing candle before impulse)
    fn detect_order_blocks(&mut self, candles: &[Candle], atr_values: &[Decimal]) {
        self.order_blocks.clear();

        if candles.len() < 3 || atr_values.is_empty() {
            return;
        }

        for i in 1..candles.len() {
            let atr = if i < atr_values.len() { atr_values[i] } else { continue; };
            let min_impulse = atr * self.config.min_ob_impulse_atr;
            let candle = &candles[i];

            // Check for bullish impulse candle
            let bullish_move = candle.mid.close - candle.mid.open;
            if bullish_move >= min_impulse {
                // Look back for last bearish candle
                if let Some(ob_idx) = self.find_last_opposing_candle(candles, i, false) {
                    let ob_candle = &candles[ob_idx];
                    self.order_blocks.push(OrderBlock {
                        ob_type: OrderBlockType::Bullish,
                        upper_price: ob_candle.mid.high,
                        lower_price: ob_candle.mid.low,
                        formation_index: ob_idx,
                        mitigated: false,
                    });
                }
            }

            // Check for bearish impulse candle
            let bearish_move = candle.mid.open - candle.mid.close;
            if bearish_move >= min_impulse {
                // Look back for last bullish candle
                if let Some(ob_idx) = self.find_last_opposing_candle(candles, i, true) {
                    let ob_candle = &candles[ob_idx];
                    self.order_blocks.push(OrderBlock {
                        ob_type: OrderBlockType::Bearish,
                        upper_price: ob_candle.mid.high,
                        lower_price: ob_candle.mid.low,
                        formation_index: ob_idx,
                        mitigated: false,
                    });
                }
            }
        }

        // Deduplicate (same candle shouldn't be OB for multiple impulses)
        self.order_blocks.sort_by_key(|ob| ob.formation_index);
        self.order_blocks.dedup_by_key(|ob| ob.formation_index);

        // Update mitigation status
        self.update_ob_mitigation_status(candles);
    }

    fn find_last_opposing_candle(&self, candles: &[Candle], impulse_idx: usize, looking_for_bullish: bool) -> Option<usize> {
        let start = impulse_idx.saturating_sub(5);

        for i in (start..impulse_idx).rev() {
            let candle = &candles[i];
            let is_bullish = candle.mid.close > candle.mid.open;

            if is_bullish == looking_for_bullish {
                return Some(i);
            }
        }

        None
    }

    fn update_ob_mitigation_status(&mut self, candles: &[Candle]) {
        for ob in &mut self.order_blocks {
            for candle in candles.iter().skip(ob.formation_index + 1) {
                let touched = candle.mid.low <= ob.upper_price && candle.mid.high >= ob.lower_price;

                if touched {
                    ob.mitigated = true;
                    break;
                }
            }
        }
    }

    /// Detect structure levels (pivot highs/lows) for retest detection
    fn detect_structure_levels(&mut self, candles: &[Candle]) {
        self.structure_levels.clear();

        let strength = self.config.pivot_strength;
        if candles.len() < strength * 2 + 1 {
            return;
        }

        for i in strength..(candles.len() - strength) {
            let candle = &candles[i];

            // Check for pivot high
            let is_pivot_high = (0..strength).all(|j| candles[i - j - 1].mid.high < candle.mid.high)
                && (0..strength).all(|j| candles[i + j + 1].mid.high < candle.mid.high);

            if is_pivot_high {
                self.structure_levels.push(StructureLevel {
                    level_type: StructureLevelType::PivotHigh,
                    price: candle.mid.high,
                    formation_index: i,
                    break_index: None,
                    retested: false,
                });
            }

            // Check for pivot low
            let is_pivot_low = (0..strength).all(|j| candles[i - j - 1].mid.low > candle.mid.low)
                && (0..strength).all(|j| candles[i + j + 1].mid.low > candle.mid.low);

            if is_pivot_low {
                self.structure_levels.push(StructureLevel {
                    level_type: StructureLevelType::PivotLow,
                    price: candle.mid.low,
                    formation_index: i,
                    break_index: None,
                    retested: false,
                });
            }
        }

        // Detect breaks and retests
        self.update_structure_level_status(candles);
    }

    fn update_structure_level_status(&mut self, candles: &[Candle]) {
        let pip_value = self.config.pip_value;
        let tolerance = self.config.zone_distance_pips * pip_value;

        for level in &mut self.structure_levels {
            let mut broke = false;

            for (idx, candle) in candles.iter().enumerate().skip(level.formation_index + 1) {
                if !broke {
                    // Check for break
                    match level.level_type {
                        StructureLevelType::PivotHigh => {
                            if candle.mid.close > level.price + tolerance {
                                level.break_index = Some(idx);
                                broke = true;
                            }
                        }
                        StructureLevelType::PivotLow => {
                            if candle.mid.close < level.price - tolerance {
                                level.break_index = Some(idx);
                                broke = true;
                            }
                        }
                    }
                } else {
                    // After break, look for retest
                    let retested = (candle.mid.low - level.price).abs() <= tolerance
                        || (candle.mid.high - level.price).abs() <= tolerance;

                    if retested {
                        level.retested = true;
                        break;
                    }
                }
            }
        }
    }

    // ========================================================================
    // Price Action Regime Checks
    // ========================================================================

    /// Check if price is at an unfilled gap of specified type
    fn check_at_gap(&self, candle: &Candle, gap_type: GapType) -> bool {
        let price = candle.mid.close;
        let pip_value = self.config.pip_value;
        let distance_threshold = self.config.zone_distance_pips * pip_value;

        for gap in &self.gap_zones {
            if gap.gap_type != gap_type || gap.filled {
                continue;
            }

            // Only consider gaps formed before current index
            if gap.formation_index >= self.current_index {
                continue;
            }

            // Check if price is within the gap or near its boundaries
            if price >= gap.lower_price - distance_threshold
                && price <= gap.upper_price + distance_threshold
            {
                return true;
            }
        }

        false
    }

    /// Check if price is at a base zone of specified type
    fn check_at_base_zone(&self, candle: &Candle, zone_type: BaseZoneType) -> bool {
        let price = candle.mid.close;
        let pip_value = self.config.pip_value;
        let distance_threshold = self.config.zone_distance_pips * pip_value;

        for zone in &self.base_zones {
            if zone.zone_type != zone_type || zone.broken {
                continue;
            }

            if zone.formation_index >= self.current_index {
                continue;
            }

            if price >= zone.lower_price - distance_threshold
                && price <= zone.upper_price + distance_threshold
            {
                return true;
            }
        }

        false
    }

    /// Check if price is at an order block of specified type
    fn check_at_order_block(&self, candle: &Candle, ob_type: OrderBlockType) -> bool {
        let price = candle.mid.close;
        let pip_value = self.config.pip_value;
        let distance_threshold = self.config.zone_distance_pips * pip_value;

        for ob in &self.order_blocks {
            if ob.ob_type != ob_type {
                continue;
            }

            if ob.formation_index >= self.current_index {
                continue;
            }

            // For order blocks, we typically want unmitigated ones
            // But price could be approaching for first test
            if price >= ob.lower_price - distance_threshold
                && price <= ob.upper_price + distance_threshold
            {
                return true;
            }
        }

        false
    }

    /// Check if price is retesting a broken resistance (now support)
    fn check_retesting_support(&self, candle: &Candle) -> bool {
        let price = candle.mid.close;
        let pip_value = self.config.pip_value;
        let tolerance = self.config.zone_distance_pips * pip_value;

        for level in &self.structure_levels {
            // Looking for pivot highs that were broken above
            if level.level_type != StructureLevelType::PivotHigh {
                continue;
            }

            // Must have been broken
            let break_idx = match level.break_index {
                Some(idx) => idx,
                None => continue,
            };

            // Break must be before current index
            if break_idx >= self.current_index {
                continue;
            }

            // Check if price is retesting from above (support)
            if (price - level.price).abs() <= tolerance && price >= level.price - tolerance {
                return true;
            }
        }

        false
    }

    /// Check if price is retesting a broken support (now resistance)
    fn check_retesting_resistance(&self, candle: &Candle) -> bool {
        let price = candle.mid.close;
        let pip_value = self.config.pip_value;
        let tolerance = self.config.zone_distance_pips * pip_value;

        for level in &self.structure_levels {
            // Looking for pivot lows that were broken below
            if level.level_type != StructureLevelType::PivotLow {
                continue;
            }

            // Must have been broken
            let break_idx = match level.break_index {
                Some(idx) => idx,
                None => continue,
            };

            // Break must be before current index
            if break_idx >= self.current_index {
                continue;
            }

            // Check if price is retesting from below (resistance)
            if (price - level.price).abs() <= tolerance && price <= level.price + tolerance {
                return true;
            }
        }

        false
    }

    /// Reset detector state
    pub fn reset(&mut self) {
        self.atr_history.clear();
        self.gap_zones.clear();
        self.base_zones.clear();
        self.order_blocks.clear();
        self.structure_levels.clear();
        self.current_index = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Ohlc;
    use chrono::{DateTime, Utc};

    fn create_test_candle(price: Decimal) -> Candle {
        Candle {
            time: DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            mid: Ohlc {
                open: price,
                high: price + dec!(0.001),
                low: price - dec!(0.001),
                close: price,
            },
            volume: 1000,
            complete: true,
        }
    }

    #[test]
    fn test_trending_up() {
        let detector = RegimeDetector::new(RegimeConfig::default());
        let candle = create_test_candle(dec!(1.1000));

        // Trending up: ADX > 25, price > SMA20 > SMA50
        assert!(detector.check_trending_up(
            &candle,
            Some(dec!(30)),
            Some(dec!(1.0950)), // SMA20 below price
            Some(dec!(1.0900)), // SMA50 below SMA20
        ));

        // Not trending: ADX too low
        assert!(!detector.check_trending_up(
            &candle,
            Some(dec!(20)),
            Some(dec!(1.0950)),
            Some(dec!(1.0900)),
        ));

        // Not trending: price below SMA20
        assert!(!detector.check_trending_up(
            &candle,
            Some(dec!(30)),
            Some(dec!(1.1050)), // SMA20 above price
            Some(dec!(1.0900)),
        ));
    }

    #[test]
    fn test_trending_down() {
        let detector = RegimeDetector::new(RegimeConfig::default());
        let candle = create_test_candle(dec!(1.1000));

        // Trending down: ADX > 25, price < SMA20 < SMA50
        assert!(detector.check_trending_down(
            &candle,
            Some(dec!(30)),
            Some(dec!(1.1050)), // SMA20 above price
            Some(dec!(1.1100)), // SMA50 above SMA20
        ));
    }

    #[test]
    fn test_ranging() {
        let detector = RegimeDetector::new(RegimeConfig::default());

        // Ranging: ADX < 20, BB contracted
        assert!(detector.check_ranging(
            Some(dec!(15)),
            Some(dec!(1.1010)), // Upper
            Some(dec!(1.0990)), // Lower (0.002 width)
            Some(dec!(1.1000)), // Middle (width/middle = 0.0018 < 0.02)
        ));

        // Not ranging: ADX too high
        assert!(!detector.check_ranging(
            Some(dec!(25)),
            Some(dec!(1.1010)),
            Some(dec!(1.0990)),
            Some(dec!(1.1000)),
        ));
    }

    #[test]
    fn test_sr_tested() {
        let mut detector = RegimeDetector::new(RegimeConfig::default());
        let candle = create_test_candle(dec!(1.1000));

        // No zones - should be false
        assert!(!detector.check_sr_tested(&candle));

        // Add a zone near price
        detector.set_sr_zones(vec![
            SRZone {
                id: "zone1".to_string(),
                upper_price: dec!(1.1010), // 10 pips away
                lower_price: dec!(1.0950),
            }
        ]);

        // Price is within 20 pips of upper boundary
        assert!(detector.check_sr_tested(&candle));

        // Price far from zone
        let far_candle = create_test_candle(dec!(1.1500));
        assert!(!detector.check_sr_tested(&far_candle));
    }

    #[test]
    fn test_high_volatility() {
        let mut detector = RegimeDetector::new(RegimeConfig::default());

        // Build up ATR history
        for _ in 0..14 {
            detector.update_atr(dec!(0.0050)); // Normal ATR
        }

        // Current ATR is 1.5x+ average
        assert!(detector.check_high_volatility(Some(dec!(0.0080)))); // 0.008 > 0.005 * 1.5

        // Current ATR is normal
        assert!(!detector.check_high_volatility(Some(dec!(0.0050))));
    }

    #[test]
    fn test_low_volatility() {
        let mut detector = RegimeDetector::new(RegimeConfig::default());

        // Build up ATR history
        for _ in 0..14 {
            detector.update_atr(dec!(0.0050)); // Normal ATR
        }

        // Current ATR is below 0.5x average
        assert!(detector.check_low_volatility(Some(dec!(0.0020)))); // 0.002 < 0.005 * 0.5

        // Current ATR is normal
        assert!(!detector.check_low_volatility(Some(dec!(0.0050))));
    }
}
