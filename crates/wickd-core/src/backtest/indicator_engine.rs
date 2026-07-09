//! Indicator Engine
//!
//! Manages a collection of indicators and tracks their outputs over time.

use std::collections::HashMap;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::models::Candle;
use super::indicators::*;

pub use shared::{IndicatorConfig, IndicatorType};

/// Stores historical outputs for lookback
#[derive(Debug, Clone, Default)]
pub struct OutputHistory {
    /// Map of output name -> historical values (newest last)
    values: HashMap<String, Vec<Decimal>>,
    max_history: usize,
}

impl OutputHistory {
    pub fn new(max_history: usize) -> Self {
        Self {
            values: HashMap::new(),
            max_history,
        }
    }

    /// Add new outputs to history
    pub fn push(&mut self, outputs: &IndicatorOutputs) {
        for (name, value) in outputs {
            let history = self.values.entry(name.clone()).or_insert_with(Vec::new);
            history.push(*value);
            if history.len() > self.max_history {
                history.remove(0);
            }
        }
    }

    /// Get a value with optional offset (0 = current, 1 = previous, etc.)
    pub fn get(&self, output: &str, offset: usize) -> Option<Decimal> {
        let history = self.values.get(output)?;
        if offset >= history.len() {
            return None;
        }
        Some(history[history.len() - 1 - offset])
    }

    /// Get the latest value
    pub fn latest(&self, output: &str) -> Option<Decimal> {
        self.get(output, 0)
    }

    /// Check if we have enough history for a given offset
    pub fn has_history(&self, output: &str, offset: usize) -> bool {
        self.values.get(output).map(|h| h.len() > offset).unwrap_or(false)
    }

    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// Get all latest values as a snapshot
    /// Returns HashMap of output_name -> value as string
    pub fn get_all_latest(&self) -> HashMap<String, String> {
        self.values.iter()
            .filter_map(|(name, history)| {
                history.last().map(|v| (name.clone(), v.to_string()))
            })
            .collect()
    }
}

/// Manages all indicators for a strategy
pub struct IndicatorEngine {
    /// Indicators by their ID
    indicators: HashMap<String, Box<dyn Indicator>>,
    /// Historical outputs by indicator ID
    history: HashMap<String, OutputHistory>,
    /// Maximum history to keep
    max_history: usize,
}

impl IndicatorEngine {
    pub fn new(max_history: usize) -> Self {
        Self {
            indicators: HashMap::new(),
            history: HashMap::new(),
            max_history,
        }
    }

    /// Create an indicator engine from configuration
    /// Create from config without parameter resolution (params must be plain f64 values)
    pub fn from_config(configs: &[IndicatorConfig], max_history: usize) -> Result<Self, String> {
        Self::from_config_with_params(configs, max_history, &HashMap::new())
    }

    /// Create from config with parameter resolution
    /// `resolved_params` maps parameter IDs to their resolved values
    pub fn from_config_with_params(
        configs: &[IndicatorConfig],
        max_history: usize,
        resolved_params: &HashMap<String, f64>,
    ) -> Result<Self, String> {
        let mut engine = Self::new(max_history);

        for config in configs {
            // Resolve any parameter references in the indicator params
            let params = config.resolve_params(resolved_params);
            let indicator = create_indicator(config.indicator_type, &params)?;
            engine.add_indicator(&config.id, indicator);
        }

        Ok(engine)
    }

    /// Add an indicator to the engine
    pub fn add_indicator(&mut self, id: &str, indicator: Box<dyn Indicator>) {
        self.indicators.insert(id.to_string(), indicator);
        self.history.insert(id.to_string(), OutputHistory::new(self.max_history));
    }

    /// Process a new candle through all indicators
    pub fn on_candle(&mut self, candle: &Candle) -> HashMap<String, IndicatorOutputs> {
        let mut all_outputs = HashMap::new();

        for (id, indicator) in &mut self.indicators {
            let outputs = indicator.on_candle(candle);

            if let Some(history) = self.history.get_mut(id) {
                history.push(&outputs);
            }

            all_outputs.insert(id.clone(), outputs);
        }

        all_outputs
    }

    /// Get a specific output value with optional offset
    pub fn get_output(&self, indicator_id: &str, output: &str, offset: usize) -> Option<Decimal> {
        self.history.get(indicator_id)?.get(output, offset)
    }

    /// Get the latest output value
    pub fn get_latest(&self, indicator_id: &str, output: &str) -> Option<Decimal> {
        self.get_output(indicator_id, output, 0)
    }

    /// Check if we have enough history for a cross detection
    pub fn can_detect_cross(&self, indicator_id: &str, output: &str) -> bool {
        self.history.get(indicator_id)
            .map(|h| h.has_history(output, 1))
            .unwrap_or(false)
    }

    /// Get output history
    pub fn get_history(&self, indicator_id: &str) -> Option<&OutputHistory> {
        self.history.get(indicator_id)
    }

    /// Reset all indicators and history
    pub fn reset(&mut self) {
        for indicator in self.indicators.values_mut() {
            indicator.reset();
        }
        for history in self.history.values_mut() {
            history.clear();
        }
    }

    /// Get a snapshot of all current indicator values
    /// Returns HashMap of indicator_id -> (output_name -> value as string)
    pub fn get_snapshot(&self) -> HashMap<String, HashMap<String, String>> {
        self.history.iter()
            .map(|(id, history)| (id.clone(), history.get_all_latest()))
            .collect()
    }
}

/// Create an indicator from type name and parameters
pub(crate) fn create_indicator(
    indicator_type: IndicatorType,
    params: &HashMap<String, f64>,
) -> Result<Box<dyn Indicator>, String> {
    match indicator_type {
        IndicatorType::Sma => {
            let period = get_param_usize(params, "period", 20)?;
            Ok(Box::new(SmaIndicator::new(period)))
        }
        IndicatorType::Ema => {
            let period = get_param_usize(params, "period", 20)?;
            Ok(Box::new(EmaIndicator::new(period)))
        }
        IndicatorType::Rsi => {
            let period = get_param_usize(params, "period", 14)?;
            Ok(Box::new(RsiIndicator::new(period)))
        }
        IndicatorType::Atr => {
            let period = get_param_usize(params, "period", 14)?;
            Ok(Box::new(AtrIndicator::new(period)))
        }
        IndicatorType::Adx => {
            let period = get_param_usize(params, "period", 14)?;
            Ok(Box::new(AdxIndicator::new(period)))
        }
        IndicatorType::Ichimoku => {
            let tenkan = get_param_usize(params, "tenkan_period", 9)?;
            let kijun = get_param_usize(params, "kijun_period", 26)?;
            let senkou_b = get_param_usize(params, "senkou_b_period", 52)?;
            let displacement = get_param_usize(params, "displacement", 26)?;
            Ok(Box::new(IchimokuIndicator::new(tenkan, kijun, senkou_b, displacement)))
        }
        IndicatorType::Chandelier => {
            let period = get_param_usize(params, "period", 22)?;
            let multiplier = get_param_decimal(params, "multiplier", dec!(3))?;
            Ok(Box::new(ChandelierIndicator::new(period, multiplier)))
        }
        IndicatorType::Bollinger => {
            let period = get_param_usize(params, "period", 20)?;
            let std_dev = get_param_decimal(params, "std_dev", dec!(2))?;
            Ok(Box::new(BollingerIndicator::new(period, std_dev)))
        }
        IndicatorType::Macd => {
            let fast = get_param_usize(params, "fast_period", 12)?;
            let slow = get_param_usize(params, "slow_period", 26)?;
            let signal = get_param_usize(params, "signal_period", 9)?;
            Ok(Box::new(MacdIndicator::new(fast, slow, signal)))
        }
        IndicatorType::Stochastic => {
            let k = get_param_usize(params, "k_period", 14)?;
            let d = get_param_usize(params, "d_period", 3)?;
            Ok(Box::new(StochasticIndicator::new(k, d)))
        }
        IndicatorType::MaHistogram => {
            let fast = get_param_usize(params, "fast_period", 5)?;
            let slow = get_param_usize(params, "slow_period", 13)?;
            Ok(Box::new(MaHistogramIndicator::new(fast, slow)))
        }
        IndicatorType::MaBands => {
            let period = get_param_usize(params, "period", 20)?;
            let distance = get_param_decimal(params, "distance", dec!(20))?;
            Ok(Box::new(MaBandsIndicator::new(period, distance)))
        }
        IndicatorType::Dss => {
            let stoch = get_param_usize(params, "stoch_period", 13)?;
            let ema = get_param_usize(params, "ema_period", 8)?;
            let signal = get_param_usize(params, "signal_period", 8)?;
            Ok(Box::new(DssIndicator::new(stoch, ema, signal)))
        }
        IndicatorType::Adr => {
            let period = get_param_usize(params, "period", 14)?;
            Ok(Box::new(AdrIndicator::new(period)))
        }
        IndicatorType::Daily => {
            Ok(Box::new(DailyIndicator::new()))
        }
        IndicatorType::Swing => {
            let strength = get_param_usize(params, "strength", 5)?;
            Ok(Box::new(SwingIndicator::new(strength)))
        }
        IndicatorType::Mfi => {
            let period = get_param_usize(params, "period", 14)?;
            Ok(Box::new(MfiIndicator::new(period)))
        }
        IndicatorType::Donchian => {
            let period = get_param_usize(params, "period", 20)?;
            Ok(Box::new(DonchianIndicator::new(period)))
        }
        IndicatorType::Vwap => {
            Ok(Box::new(VwapIndicator::new()))
        }
        IndicatorType::ParabolicSar => {
            let af_start = get_param_decimal(params, "af_start", dec!(0.02))?;
            let af_increment = get_param_decimal(params, "af_increment", dec!(0.02))?;
            let af_max = get_param_decimal(params, "af_max", dec!(0.20))?;
            Ok(Box::new(ParabolicSarIndicator::new(af_start, af_increment, af_max)))
        }
        IndicatorType::SuperTrend => {
            let period = get_param_usize(params, "period", 10)?;
            let multiplier = get_param_decimal(params, "multiplier", dec!(3.0))?;
            Ok(Box::new(SuperTrendIndicator::new(period, multiplier)))
        }
    }
}

fn get_param_usize(params: &HashMap<String, f64>, key: &str, default: usize) -> Result<usize, String> {
    Ok(params.get(key).map(|v| *v as usize).unwrap_or(default))
}

fn get_param_decimal(params: &HashMap<String, f64>, key: &str, default: Decimal) -> Result<Decimal, String> {
    Ok(params.get(key)
        .map(|v| Decimal::try_from(*v).unwrap_or(default))
        .unwrap_or(default))
}

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
    fn test_indicator_engine_creation() {
        let configs = vec![
            IndicatorConfig::new_fixed("sma20", IndicatorType::Sma, &[("period", 20.0)]),
            IndicatorConfig::new_fixed("rsi14", IndicatorType::Rsi, &[("period", 14.0)]),
        ];

        let engine = IndicatorEngine::from_config(&configs, 100);
        assert!(engine.is_ok());
    }

    #[test]
    fn test_indicator_engine_processing() {
        let configs = vec![
            IndicatorConfig::new_fixed("sma3", IndicatorType::Sma, &[("period", 3.0)]),
        ];

        let mut engine = IndicatorEngine::from_config(&configs, 100).unwrap();

        // Process 3 candles
        engine.on_candle(&create_test_candle(dec!(1.1000), 0));
        engine.on_candle(&create_test_candle(dec!(1.1100), 1));
        engine.on_candle(&create_test_candle(dec!(1.1200), 2));

        let value = engine.get_latest("sma3", "value");
        assert!(value.is_some());

        let expected = (dec!(1.1000) + dec!(1.1100) + dec!(1.1200)) / dec!(3);
        assert_eq!(value.unwrap(), expected);
    }

    #[test]
    fn test_output_history() {
        let mut history = OutputHistory::new(5);

        let outputs1: IndicatorOutputs = [("value".to_string(), dec!(1.0))].into_iter().collect();
        let outputs2: IndicatorOutputs = [("value".to_string(), dec!(2.0))].into_iter().collect();
        let outputs3: IndicatorOutputs = [("value".to_string(), dec!(3.0))].into_iter().collect();

        history.push(&outputs1);
        history.push(&outputs2);
        history.push(&outputs3);

        assert_eq!(history.get("value", 0), Some(dec!(3.0))); // latest
        assert_eq!(history.get("value", 1), Some(dec!(2.0))); // previous
        assert_eq!(history.get("value", 2), Some(dec!(1.0))); // oldest
    }

    #[test]
    fn test_output_history_max_limit() {
        let mut history = OutputHistory::new(3);

        for i in 1..=5 {
            let outputs: IndicatorOutputs = [("value".to_string(), Decimal::from(i))].into_iter().collect();
            history.push(&outputs);
        }

        // Should only keep last 3 values: 3, 4, 5
        assert_eq!(history.get("value", 0), Some(dec!(5)));
        assert_eq!(history.get("value", 1), Some(dec!(4)));
        assert_eq!(history.get("value", 2), Some(dec!(3)));
        assert_eq!(history.get("value", 3), None); // Beyond max
    }

    #[test]
    fn test_output_history_latest() {
        let mut history = OutputHistory::new(5);
        let outputs: IndicatorOutputs = [("value".to_string(), dec!(42.0))].into_iter().collect();
        history.push(&outputs);

        assert_eq!(history.latest("value"), Some(dec!(42.0)));
        assert_eq!(history.latest("nonexistent"), None);
    }

    #[test]
    fn test_output_history_has_history() {
        let mut history = OutputHistory::new(5);

        assert!(!history.has_history("value", 0));

        let outputs: IndicatorOutputs = [("value".to_string(), dec!(1.0))].into_iter().collect();
        history.push(&outputs);

        assert!(history.has_history("value", 0));
        assert!(!history.has_history("value", 1));
    }

    #[test]
    fn test_output_history_clear() {
        let mut history = OutputHistory::new(5);
        let outputs: IndicatorOutputs = [("value".to_string(), dec!(1.0))].into_iter().collect();
        history.push(&outputs);

        assert!(history.has_history("value", 0));

        history.clear();

        assert!(!history.has_history("value", 0));
        assert_eq!(history.get("value", 0), None);
    }

    #[test]
    fn test_create_all_indicator_types() {
        let test_cases: Vec<(IndicatorType, Vec<(&str, f64)>)> = vec![
            (IndicatorType::Sma, vec![("period", 20.0)]),
            (IndicatorType::Ema, vec![("period", 20.0)]),
            (IndicatorType::Rsi, vec![("period", 14.0)]),
            (IndicatorType::Atr, vec![("period", 14.0)]),
            (IndicatorType::Ichimoku, vec![
                ("tenkan_period", 9.0),
                ("kijun_period", 26.0),
                ("senkou_b_period", 52.0),
                ("displacement", 26.0),
            ]),
            (IndicatorType::Chandelier, vec![("period", 22.0), ("multiplier", 3.0)]),
            (IndicatorType::Bollinger, vec![("period", 20.0), ("std_dev", 2.0)]),
            (IndicatorType::Macd, vec![("fast_period", 12.0), ("slow_period", 26.0), ("signal_period", 9.0)]),
            (IndicatorType::Stochastic, vec![("k_period", 14.0), ("d_period", 3.0)]),
            (IndicatorType::Vwap, vec![]),
            (IndicatorType::ParabolicSar, vec![("af_start", 0.02), ("af_increment", 0.02), ("af_max", 0.2)]),
            (IndicatorType::SuperTrend, vec![("period", 10.0), ("multiplier", 3.0)]),
        ];

        for (indicator_type, params_list) in test_cases {
            let configs = vec![IndicatorConfig::new_fixed("test", indicator_type, &params_list)];

            let engine = IndicatorEngine::from_config(&configs, 100);
            assert!(engine.is_ok(), "Failed to create {:?} indicator", indicator_type);
        }
    }

    // Note: test_create_unknown_indicator_type is no longer needed - with enums,
    // invalid indicator types are caught at compile time. The only runtime error
    // case is for ADX which is not yet implemented.

    #[test]
    fn test_indicator_engine_get_output_with_offset() {
        let mut engine = IndicatorEngine::new(100);
        engine.add_indicator("sma3", Box::new(SmaIndicator::new(3)));

        // Process 5 candles
        for i in 0..5 {
            let price = dec!(1.1000) + Decimal::from(i) * dec!(0.01);
            engine.on_candle(&create_test_candle(price, i));
        }

        // Test offset access
        let latest = engine.get_output("sma3", "value", 0);
        let previous = engine.get_output("sma3", "value", 1);

        assert!(latest.is_some());
        assert!(previous.is_some());
        assert_ne!(latest, previous);
    }

    #[test]
    fn test_indicator_engine_can_detect_cross() {
        let mut engine = IndicatorEngine::new(100);
        engine.add_indicator("sma3", Box::new(SmaIndicator::new(3)));

        // Initially can't detect cross - no history
        assert!(!engine.can_detect_cross("sma3", "value"));
        assert!(!engine.can_detect_cross("nonexistent", "value"));

        // Process enough candles
        for i in 0..5 {
            engine.on_candle(&create_test_candle(dec!(1.1000), i));
        }

        // Now should be able to detect cross
        assert!(engine.can_detect_cross("sma3", "value"));
    }

    #[test]
    fn test_indicator_engine_get_history() {
        let mut engine = IndicatorEngine::new(100);
        engine.add_indicator("sma3", Box::new(SmaIndicator::new(3)));

        assert!(engine.get_history("sma3").is_some());
        assert!(engine.get_history("nonexistent").is_none());
    }

    #[test]
    fn test_indicator_engine_reset() {
        let mut engine = IndicatorEngine::new(100);
        engine.add_indicator("sma3", Box::new(SmaIndicator::new(3)));

        // Process some candles
        for i in 0..5 {
            engine.on_candle(&create_test_candle(dec!(1.1000), i));
        }

        assert!(engine.get_latest("sma3", "value").is_some());

        // Reset
        engine.reset();

        // History should be cleared
        assert!(engine.get_latest("sma3", "value").is_none());
    }

    #[test]
    fn test_get_param_with_defaults() {
        let params: HashMap<String, f64> = HashMap::new();

        // Test that defaults are used when params are missing
        let result = get_param_usize(&params, "missing", 42);
        assert_eq!(result.unwrap(), 42);

        let result = get_param_decimal(&params, "missing", dec!(3.5));
        assert_eq!(result.unwrap(), dec!(3.5));
    }

    #[test]
    fn test_get_param_with_values() {
        let params: HashMap<String, f64> = [("period".to_string(), 50.0)].into_iter().collect();

        let result = get_param_usize(&params, "period", 20);
        assert_eq!(result.unwrap(), 50);

        let params: HashMap<String, f64> = [("multiplier".to_string(), 2.5)].into_iter().collect();
        let result = get_param_decimal(&params, "multiplier", dec!(3.0));
        assert!((result.unwrap() - dec!(2.5)).abs() < dec!(0.001));
    }
}
