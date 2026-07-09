//! Multi-Timeframe (MTF) Indicator Support
//!
//! Provides utilities for strategies that use indicators on different timeframes
//! (e.g., Daily EMA for trend + H1 RSI for entry).
//!
//! Key components:
//! - `extract_htf_timeframes`: Scans a strategy definition for non-primary timeframes
//! - `MtfCandleStore`: Stores pre-fetched HTF candles and provides timestamp-based lookup

use std::collections::{HashMap, HashSet};
use chrono::{DateTime, Utc};
use crate::models::Candle;

// ============================================================================
// Timeframe Extraction
// ============================================================================

/// Scan a strategy definition and return all unique timeframes required
/// beyond the primary timeframe.
///
/// Checks:
/// 1. `IndicatorConfig.timeframe` fields
/// 2. `DataSource::Indicator` and `DataSource::Price` timeframes in entry/exit rules
pub fn extract_htf_timeframes(strategy: &shared::StrategyDefinition, primary_timeframe: &str) -> HashSet<String> {
    let mut timeframes = HashSet::new();

    // 1. Check indicator configs for timeframe field
    for indicator in &strategy.indicators {
        if let Some(ref tf) = indicator.timeframe {
            if tf != primary_timeframe {
                timeframes.insert(tf.clone());
            }
        }
    }

    // 2. Scan all entry/exit rules for DataSource::Indicator and DataSource::Price with timeframe
    fn extract_from_datasource(ds: &shared::DataSource, primary: &str, set: &mut HashSet<String>) {
        match ds {
            shared::DataSource::Indicator(src) => {
                if let Some(ref tf) = src.timeframe {
                    if tf != primary {
                        set.insert(tf.clone());
                    }
                }
            }
            shared::DataSource::Price(src) => {
                if let Some(ref tf) = src.timeframe {
                    if tf != primary {
                        set.insert(tf.clone());
                    }
                }
            }
            _ => {}
        }
    }

    fn extract_from_trigger(trigger: &shared::Trigger, primary: &str, set: &mut HashSet<String>) {
        match trigger {
            shared::Trigger::Cross(t) => {
                extract_from_datasource(&t.left, primary, set);
                extract_from_datasource(&t.right, primary, set);
            }
            shared::Trigger::Compare(t) => {
                extract_from_datasource(&t.left, primary, set);
                extract_from_datasource(&t.right, primary, set);
            }
            shared::Trigger::Threshold(t) => {
                extract_from_datasource(&t.source, primary, set);
            }
            _ => {} // Other trigger types don't have data sources with timeframes
        }
    }

    // Scan entry rules
    for rule in &strategy.entry_rules {
        for condition in &rule.conditions {
            extract_from_trigger(&condition.primary.trigger, primary_timeframe, &mut timeframes);
            for chained in &condition.chain {
                extract_from_trigger(&chained.trigger.trigger, primary_timeframe, &mut timeframes);
            }
        }
    }

    // Scan exit rules
    for rule in &strategy.exit_rules {
        for condition in &rule.conditions {
            extract_from_trigger(&condition.primary.trigger, primary_timeframe, &mut timeframes);
            for chained in &condition.chain {
                extract_from_trigger(&chained.trigger.trigger, primary_timeframe, &mut timeframes);
            }
        }
    }

    timeframes
}

// ============================================================================
// HTF Candle Store
// ============================================================================

/// Stores pre-fetched HTF candles and provides timestamp-based lookup.
/// For each HTF timeframe, holds sorted candles and tracks the "current" HTF candle
/// based on the primary candle's timestamp.
#[derive(Debug, Clone)]
pub struct MtfCandleStore {
    /// HTF timeframe -> sorted candles
    candles: HashMap<String, Vec<Candle>>,
    /// HTF timeframe -> index of current HTF candle (the last COMPLETED one)
    current_index: HashMap<String, usize>,
}

impl MtfCandleStore {
    pub fn new() -> Self {
        Self {
            candles: HashMap::new(),
            current_index: HashMap::new(),
        }
    }

    /// Add candles for an HTF timeframe
    pub fn add_timeframe(&mut self, timeframe: String, candles: Vec<Candle>) {
        self.current_index.insert(timeframe.clone(), 0);
        self.candles.insert(timeframe, candles);
    }

    /// Check if store has any HTF timeframes
    pub fn is_empty(&self) -> bool {
        self.candles.is_empty()
    }

    /// Get the current HTF candle for a given timeframe (without advancing)
    pub fn current_candle(&self, timeframe: &str) -> Option<&Candle> {
        let candles = self.candles.get(timeframe)?;
        let idx = *self.current_index.get(timeframe)?;
        candles.get(idx)
    }

    /// Advance the HTF candle pointer for a given primary candle timestamp.
    /// Returns all newly completed HTF candles (may be multiple if primary data has gaps).
    /// A new HTF candle is "complete" when primary_time >= next_htf_candle.time
    pub fn advance(&mut self, timeframe: &str, primary_time: &DateTime<Utc>) -> Vec<Candle> {
        let candles = match self.candles.get(timeframe) {
            Some(c) => c,
            None => return Vec::new(),
        };
        let idx = match self.current_index.get_mut(timeframe) {
            Some(i) => i,
            None => return Vec::new(),
        };

        let mut newly_completed = Vec::new();
        // Advance through all HTF candles that have completed by primary_time
        loop {
            let next_idx = *idx + 1;
            if next_idx < candles.len() && primary_time >= &candles[next_idx].time {
                *idx = next_idx;
                newly_completed.push(candles[next_idx].clone());
            } else {
                break;
            }
        }
        newly_completed
    }

    /// Get all HTF timeframes in this store
    pub fn timeframes(&self) -> Vec<&String> {
        self.candles.keys().collect()
    }

    /// Filter HTF candles by time range (for walk-forward window splitting).
    /// Returns a fresh store with all indices at 0 — safe to use without calling reset().
    pub fn filter_by_time_range(&self, start: &DateTime<Utc>, end: &DateTime<Utc>) -> MtfCandleStore {
        let mut filtered = MtfCandleStore::new();
        for (tf, candles) in &self.candles {
            let filtered_candles: Vec<Candle> = candles.iter()
                .filter(|c| c.time >= *start && c.time < *end)
                .cloned()
                .collect();
            if !filtered_candles.is_empty() {
                filtered.add_timeframe(tf.clone(), filtered_candles);
            }
        }
        filtered
    }

    /// Append a new candle to a timeframe (for live updates).
    /// Only appends if the candle is newer than the last one in the store.
    pub fn append_candle(&mut self, timeframe: &str, candle: Candle) {
        if let Some(candles) = self.candles.get_mut(timeframe) {
            if candles.last().map(|c| candle.time > c.time).unwrap_or(true) {
                candles.push(candle);
            }
        }
    }

    /// Reset all current indices to 0
    pub fn reset(&mut self) {
        for idx in self.current_index.values_mut() {
            *idx = 0;
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Ohlc;
    use chrono::TimeZone;
    use rust_decimal_macros::dec;

    fn make_candle(year: i32, month: u32, day: u32, hour: u32) -> Candle {
        Candle {
            time: Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap(),
            mid: Ohlc {
                open: dec!(1.1000),
                high: dec!(1.1100),
                low: dec!(1.0900),
                close: dec!(1.1050),
            },
            volume: 100,
            complete: true,
        }
    }

    #[test]
    fn test_mtf_candle_store_advance() {
        let mut store = MtfCandleStore::new();
        let daily_candles = vec![
            make_candle(2024, 1, 1, 0),
            make_candle(2024, 1, 2, 0),
            make_candle(2024, 1, 3, 0),
        ];
        store.add_timeframe("D".to_string(), daily_candles);

        // H1 candle at 2024-01-01 03:00 - should not advance (still day 1)
        let h1_time = Utc.with_ymd_and_hms(2024, 1, 1, 3, 0, 0).unwrap();
        assert!(store.advance("D", &h1_time).is_empty());

        // H1 candle at 2024-01-02 00:00 - should advance to day 2
        let h1_time = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
        let advanced = store.advance("D", &h1_time);
        assert_eq!(advanced.len(), 1);

        // Current candle should now be day 2
        let current = store.current_candle("D").unwrap();
        assert_eq!(current.time, Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap());

        // Test multi-step advance: jump from day 2 to day 3 in one call
        // (simulates gap in primary data)
        let mut store2 = MtfCandleStore::new();
        store2.add_timeframe("D".to_string(), vec![
            make_candle(2024, 1, 1, 0),
            make_candle(2024, 1, 2, 0),
            make_candle(2024, 1, 3, 0),
        ]);
        let jump_time = Utc.with_ymd_and_hms(2024, 1, 3, 12, 0, 0).unwrap();
        let multi = store2.advance("D", &jump_time);
        assert_eq!(multi.len(), 2); // Should catch up both day 2 and day 3
    }

    #[test]
    fn test_mtf_candle_store_filter_by_time_range() {
        let mut store = MtfCandleStore::new();
        let daily_candles = vec![
            make_candle(2024, 1, 1, 0),
            make_candle(2024, 1, 2, 0),
            make_candle(2024, 1, 3, 0),
            make_candle(2024, 1, 4, 0),
        ];
        store.add_timeframe("D".to_string(), daily_candles);

        let start = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 1, 4, 0, 0, 0).unwrap();
        let filtered = store.filter_by_time_range(&start, &end);

        // Should contain days 2 and 3 (4 is excluded by < end)
        let candles = filtered.candles.get("D").unwrap();
        assert_eq!(candles.len(), 2);
    }

    #[test]
    fn test_mtf_candle_store_reset() {
        let mut store = MtfCandleStore::new();
        let daily_candles = vec![
            make_candle(2024, 1, 1, 0),
            make_candle(2024, 1, 2, 0),
        ];
        store.add_timeframe("D".to_string(), daily_candles);

        // Advance to day 2
        let h1_time = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
        store.advance("D", &h1_time);
        assert_eq!(*store.current_index.get("D").unwrap(), 1);

        // Reset
        store.reset();
        assert_eq!(*store.current_index.get("D").unwrap(), 0);
    }

    #[test]
    fn test_mtf_candle_store_empty() {
        let store = MtfCandleStore::new();
        assert!(store.is_empty());
        assert!(store.current_candle("D").is_none());
    }

    #[test]
    fn test_mtf_candle_store_append_candle() {
        let mut store = MtfCandleStore::new();
        let daily_candles = vec![
            make_candle(2024, 1, 1, 0),
            make_candle(2024, 1, 2, 0),
        ];
        store.add_timeframe("D".to_string(), daily_candles);

        // Append a newer candle
        let new_candle = make_candle(2024, 1, 3, 0);
        store.append_candle("D", new_candle);
        assert_eq!(store.candles.get("D").unwrap().len(), 3);

        // Append a duplicate (same time) - should NOT be added
        let dup_candle = make_candle(2024, 1, 3, 0);
        store.append_candle("D", dup_candle);
        assert_eq!(store.candles.get("D").unwrap().len(), 3);

        // Append an older candle - should NOT be added
        let old_candle = make_candle(2024, 1, 2, 0);
        store.append_candle("D", old_candle);
        assert_eq!(store.candles.get("D").unwrap().len(), 3);

        // Append to non-existent timeframe - should be a no-op
        store.append_candle("W", make_candle(2024, 1, 7, 0));
        assert!(store.candles.get("W").is_none());
    }

    #[test]
    fn test_extract_htf_timeframes_from_indicators() {
        // Build a minimal strategy with an HTF indicator
        let strategy = shared::StrategyDefinition {
            id: "test".to_string(),
            user_id: "u1".to_string(),
            name: "Test".to_string(),
            description: "Test".to_string(),
            parameters: vec![],
            indicators: vec![
                shared::IndicatorConfig {
                    id: "ema_daily".to_string(),
                    indicator_type: shared::IndicatorType::Ema,
                    params: std::collections::HashMap::new(),
                    symbol: None,
                    timeframe: Some("D".to_string()),
                },
                shared::IndicatorConfig {
                    id: "rsi_h1".to_string(),
                    indicator_type: shared::IndicatorType::Rsi,
                    params: std::collections::HashMap::new(),
                    symbol: None,
                    timeframe: None, // primary
                },
            ],
            variables: vec![],
            entry_rules: vec![],
            entry_logic: shared::EntryLogic::default(),
            exit_rules: vec![],
            risk_settings: shared::RiskSettings {
                risk_method: shared::RiskMethod::Percent,
                risk_value: shared::ParameterizedValue::Fixed(1.0),
                rr_ratio: shared::ParameterizedValue::Fixed(2.0),
                spread_buffer_pips: shared::ParameterizedValue::Fixed(1.0),
                stop_loss_source: None,
                risk_method_short: None,
                risk_value_short: None,
                rr_ratio_short: None,
                spread_buffer_pips_short: None,
                stop_loss_source_short: None,
            },
            version: 1,
            is_active: true,
            schema_version: 2,
            strategy_type: "rules".to_string(),
            script_content: None,
        };

        let htf = extract_htf_timeframes(&strategy, "H1");
        assert!(htf.contains("D"));
        assert_eq!(htf.len(), 1);
    }
}
