//! Rules-Based Strategy Wrapper
//!
//! Implements the Strategy trait using a RulesEngine for compatibility with BacktestEngine.

use crate::models::Candle;
use rust_decimal::Decimal;
use super::strategy::{Strategy, Signal, ExtendedSignal};
use super::rules_engine::{RulesEngine, RulesSignal, StrategyDefinition, PositionDirection, SRZone};
use super::pivots::PivotConfig;
use super::mtf::MtfCandleStore;

/// A strategy implementation that uses the rules engine
pub struct RulesBasedStrategy {
    engine: RulesEngine,
}

impl RulesBasedStrategy {
    /// Create a new rules-based strategy from a strategy definition
    pub fn new(definition: StrategyDefinition) -> Result<Self, String> {
        let engine = RulesEngine::new(definition)?;
        Ok(Self { engine })
    }

    /// Create from JSON string
    pub fn from_json(json: &str) -> Result<Self, String> {
        let definition: StrategyDefinition = serde_json::from_str(json)
            .map_err(|e| format!("Failed to parse strategy JSON: {}", e))?;
        Self::new(definition)
    }

    /// Create from JSON string with parameter overrides for optimization
    pub fn from_json_with_params(
        json: &str,
        params: std::collections::HashMap<String, f64>,
    ) -> Result<Self, String> {
        let definition: StrategyDefinition = serde_json::from_str(json)
            .map_err(|e| format!("Failed to parse strategy JSON: {}", e))?;
        let engine = RulesEngine::with_params(definition, Some(params))?;
        Ok(Self { engine })
    }

    /// Get the resolved parameter values for this strategy instance
    pub fn get_resolved_params(&self) -> &std::collections::HashMap<String, f64> {
        self.engine.get_resolved_params()
    }

    /// Set S/R zones for zone-based trigger evaluation
    pub fn set_sr_zones(&mut self, zones: Vec<SRZone>) {
        self.engine.set_sr_zones(zones);
    }

    /// Set S/R zones from JSON string
    pub fn set_sr_zones_from_json(&mut self, json: &str) -> Result<(), String> {
        let zones: Vec<SRZone> = serde_json::from_str(json)
            .map_err(|e| format!("Failed to parse S/R zones JSON: {}", e))?;
        self.set_sr_zones(zones);
        Ok(())
    }

    /// Set pivot point configuration
    pub fn set_pivot_config(&mut self, config: PivotConfig) {
        self.engine.set_pivot_config(config);
    }

    /// Set pivot configuration from JSON string
    pub fn set_pivot_config_from_json(&mut self, json: &str) -> Result<(), String> {
        let config: PivotConfig = serde_json::from_str(json)
            .map_err(|e| format!("Failed to parse pivot config JSON: {}", e))?;
        self.set_pivot_config(config);
        Ok(())
    }

    /// Set pip value based on instrument name.
    /// This should be called before running backtest to ensure correct stop loss calculations
    /// for non-standard instruments (JPY pairs, gold, silver, indices).
    pub fn set_pip_value_for_instrument(&mut self, instrument: &str) {
        self.engine.set_pip_value_for_instrument(instrument);
    }

    /// Reclassify indicators whose explicit timeframe matches the chart's primary
    /// granularity from HTF to primary engine. Must be called BEFORE `set_mtf_candle_store`.
    pub fn set_primary_granularity(&mut self, granularity: &str) {
        self.engine.set_primary_granularity(granularity);
    }

    /// Set the MTF candle store for multi-timeframe indicator support.
    /// Should be called after construction and before running the backtest.
    pub fn set_mtf_candle_store(&mut self, store: MtfCandleStore) {
        self.engine.set_mtf_candle_store(store);
    }
}

impl Strategy for RulesBasedStrategy {
    fn prepare(&mut self, candles: &[Candle]) {
        self.engine.prepare_for_backtest(candles);
    }

    fn on_candle(&mut self, candle: &Candle) -> Signal {
        let rules_signal = self.engine.on_candle(candle);
        rules_signal.into()
    }

    fn current_stop_loss(&self) -> Option<Decimal> {
        self.engine.position.as_ref().map(|p| p.stop_loss)
    }

    fn current_take_profit(&self) -> Option<Decimal> {
        self.engine.position.as_ref().map(|p| p.take_profit)
    }

    fn on_candle_extended(&mut self, candle: &Candle) -> ExtendedSignal {
        let rules_signal = self.engine.on_candle(candle);

        let signal: Signal = match &rules_signal {
            RulesSignal::Hold => Signal::Hold,
            RulesSignal::Entry { direction, .. } => match direction {
                PositionDirection::Long => Signal::Buy,
                PositionDirection::Short => Signal::Sell,
            },
            RulesSignal::Exit { .. } | RulesSignal::PartialExit { .. } => Signal::ClosePosition,
        };

        // Extract SL/TP, rule info, and pending order from entry signals
        let (stop_loss, take_profit, entry_rule_id, entry_rule_name, pending_order) = match &rules_signal {
            RulesSignal::Entry { stop_loss, take_profit, triggered_rule_id, triggered_rule_name, pending_order, .. } => {
                (*stop_loss, *take_profit, triggered_rule_id.clone(), triggered_rule_name.clone(), pending_order.clone())
            }
            _ => (None, None, None, None, None),
        };

        // Extract exit reason from exit signals
        let exit_reason = match &rules_signal {
            RulesSignal::Exit { reason, .. } | RulesSignal::PartialExit { reason, .. } => {
                Some(reason.clone())
            }
            _ => None,
        };

        // Capture indicator values for entry signals
        let entry_indicators = match &rules_signal {
            RulesSignal::Entry { .. } => {
                // Flatten the nested HashMap to "indicator_id.output" -> value
                let snapshot = self.engine.get_indicator_snapshot();
                let mut flat: std::collections::HashMap<String, String> = std::collections::HashMap::new();
                for (indicator_id, outputs) in snapshot {
                    for (output_name, value) in outputs {
                        let key = if output_name == "value" {
                            // For single-output indicators, just use the indicator_id
                            indicator_id.clone()
                        } else {
                            // For multi-output, use indicator_id.output format
                            format!("{}.{}", indicator_id, output_name)
                        };
                        flat.insert(key, value);
                    }
                }
                Some(flat)
            }
            _ => None,
        };

        ExtendedSignal {
            signal,
            stop_loss,
            take_profit,
            entry_rule_id,
            entry_rule_name,
            exit_reason,
            entry_indicators,
            pending_order,
        }
    }

    fn notify_position_closed(&mut self) {
        self.engine.position = None;
    }

    fn notify_entry_rejected(&mut self) {
        // The RulesEngine opened an internal position during signal generation
        // (in evaluate_entry_rules_v2_with_position → open_position), but the
        // BacktestEngine rejected the actual trade. Clear the stale position so
        // the engine can evaluate new entries on subsequent candles.
        self.engine.position = None;
    }

    fn name(&self) -> &str {
        "Rules-Based Strategy"
    }

    fn reset(&mut self) {
        self.engine.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Ohlc;
    use chrono::{DateTime, Utc, Duration};
    use rust_decimal_macros::dec;

    fn create_test_candle(price: rust_decimal::Decimal, time_offset: i64) -> Candle {
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
    fn test_rules_strategy_from_json() {
        let json = r#"{
            "id": "test",
            "user_id": "user1",
            "name": "Test Strategy",
            "description": "A test",
            "schema_version": 2,
            "indicators": [
                {
                    "id": "sma_fast",
                    "type": "sma",
                    "params": {"period": 5}
                },
                {
                    "id": "sma_slow",
                    "type": "sma",
                    "params": {"period": 10}
                }
            ],
            "entry_rules": [
                {
                    "id": "cross_up",
                    "direction": "long",
                    "conditions": [
                        {
                            "primary": {
                                "trigger": {
                                    "type": "cross",
                                    "left": {"indicator": "sma_fast", "output": "value"},
                                    "right": {"indicator": "sma_slow", "output": "value"},
                                    "direction": "above"
                                },
                                "negated": false
                            },
                            "chain": []
                        }
                    ]
                }
            ],
            "entry_logic": {"mode": "all"},
            "exit_rules": [
                {
                    "id": "tp",
                    "direction": "both",
                    "conditions": [
                        {
                            "primary": {
                                "trigger": {"type": "risk_reward_reached", "ratio": 2.0},
                                "negated": false
                            },
                            "chain": []
                        }
                    ],
                    "close_percent": 100,
                    "priority": 100
                }
            ],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": 2.0,
                "spread_buffer_pips": 1.0
            },
            "version": 1,
            "is_active": true
        }"#;

        let strategy = RulesBasedStrategy::from_json(json);
        assert!(strategy.is_ok());

        let mut strategy = strategy.unwrap();

        // Process some candles
        for i in 0..20 {
            let price = dec!(1.1000) + rust_decimal::Decimal::from(i) * dec!(0.0010);
            let _signal = strategy.on_candle(&create_test_candle(price, i as i64));
        }
    }

    #[test]
    fn test_price_source_in_trigger() {
        // Test that PriceSource format {"source": "price", "value": "close"} works
        let json = r#"{
            "id": "price_test",
            "user_id": "user1",
            "name": "Price Source Test",
            "description": "Test PriceSource parsing",
            "schema_version": 2,
            "indicators": [
                {
                    "id": "ichimoku",
                    "type": "ichimoku",
                    "params": {"tenkan_period": 9, "kijun_period": 26, "senkou_b_period": 52, "displacement": 26}
                }
            ],
            "entry_rules": [
                {
                    "id": "price_above_tenkan",
                    "direction": "long",
                    "conditions": [
                        {
                            "primary": {
                                "trigger": {
                                    "type": "cross",
                                    "left": {"source": "price", "value": "close"},
                                    "right": {"indicator": "ichimoku", "output": "tenkan"},
                                    "direction": "above"
                                },
                                "negated": false
                            },
                            "chain": []
                        }
                    ]
                }
            ],
            "entry_logic": {"mode": "all"},
            "exit_rules": [
                {
                    "id": "price_below_tenkan",
                    "direction": "long",
                    "conditions": [
                        {
                            "primary": {
                                "trigger": {
                                    "type": "cross",
                                    "left": {"source": "price", "value": "close"},
                                    "right": {"indicator": "ichimoku", "output": "tenkan"},
                                    "direction": "below"
                                },
                                "negated": false
                            },
                            "chain": []
                        }
                    ],
                    "close_percent": 100,
                    "priority": 100
                }
            ],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": 2.0,
                "spread_buffer_pips": 1.0
            },
            "version": 1,
            "is_active": true
        }"#;

        let strategy = RulesBasedStrategy::from_json(json);
        assert!(strategy.is_ok(), "Failed to parse strategy with PriceSource: {:?}", strategy.err());
    }

    #[test]
    fn test_wrong_price_source_format() {
        // Test common mistake: using "type": "price" instead of "source": "price"
        // This is what an MCP client might incorrectly generate
        let json = r#"{
            "id": "wrong_format_test",
            "user_id": "user1",
            "name": "Wrong Format Test",
            "description": "Test wrong PriceSource format",
            "schema_version": 2,
            "indicators": [
                {
                    "id": "ichimoku",
                    "type": "ichimoku",
                    "params": {"tenkan_period": 9, "kijun_period": 26, "senkou_b_period": 52, "displacement": 26}
                }
            ],
            "entry_rules": [
                {
                    "id": "wrong_price",
                    "direction": "long",
                    "conditions": [
                        {
                            "primary": {
                                "trigger": {
                                    "type": "cross",
                                    "left": {"type": "price", "value": "close"},
                                    "right": {"indicator": "ichimoku", "output": "tenkan"},
                                    "direction": "above"
                                },
                                "negated": false
                            },
                            "chain": []
                        }
                    ]
                }
            ],
            "entry_logic": {"mode": "all"},
            "exit_rules": [],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": 2.0,
                "spread_buffer_pips": 1.0
            },
            "version": 1,
            "is_active": true
        }"#;

        let result = RulesBasedStrategy::from_json(json);
        // This should fail because "type": "price" is wrong - it should be "source": "price"
        match result {
            Ok(_) => println!("Unexpectedly parsed successfully!"),
            Err(e) => println!("Expected error: {}", e),
        }
        // We expect an error containing "DataSource" since serde can't match the untagged enum
    }

    #[test]
    fn test_stop_loss_source_with_indicator() {
        // StopLossSource is a TAGGED enum - it NEEDS "type": "indicator"
        // This is DIFFERENT from DataSource in triggers which is UNTAGGED
        let json = r#"{
            "id": "stop_loss_test",
            "user_id": "user1",
            "name": "StopLoss Test",
            "description": "Test StopLossSource format with type:indicator",
            "schema_version": 2,
            "indicators": [
                {
                    "id": "ichimoku",
                    "type": "ichimoku",
                    "params": {"tenkan_period": 9, "kijun_period": 26, "senkou_b_period": 52, "displacement": 26}
                }
            ],
            "entry_rules": [
                {
                    "id": "basic",
                    "direction": "long",
                    "conditions": [
                        {
                            "primary": {
                                "trigger": {
                                    "type": "compare",
                                    "left": {"indicator": "ichimoku", "output": "tenkan"},
                                    "operator": ">",
                                    "right": {"indicator": "ichimoku", "output": "kijun"}
                                },
                                "negated": false
                            },
                            "chain": []
                        }
                    ]
                }
            ],
            "entry_logic": {"mode": "all"},
            "exit_rules": [],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": 3.0,
                "spread_buffer_pips": 1.0,
                "stop_loss_source": {
                    "type": "indicator",
                    "indicator": "ichimoku",
                    "output": "kijun"
                }
            },
            "version": 1,
            "is_active": true
        }"#;

        let result = RulesBasedStrategy::from_json(json);
        assert!(result.is_ok(), "StopLossSource with type:indicator should parse: {:?}", result.err());
    }

    #[test]
    fn test_mtf_strategy_with_daily_indicators() {
        // Exact user strategy that produces zero trades — daily EMA + daily MACD + H1 MACD cross
        let json = r#"{
            "name": "MTF Test",
            "description": "test",
            "schema_version": 2,
            "id": "test",
            "user_id": "user1",
            "version": 1,
            "is_active": true,
            "indicators": [
                {"id": "daily_ema", "params": {"period": 5}, "timeframe": "D", "type": "ema"},
                {"id": "daily_macd", "params": {"fast_period": 3, "signal_period": 3, "slow_period": 5}, "timeframe": "D", "type": "macd"},
                {"id": "h1_macd", "params": {"fast_period": 3, "signal_period": 3, "slow_period": 5}, "type": "macd"},
                {"id": "chandelier", "params": {"multiplier": 2, "period": 5}, "type": "chandelier"}
            ],
            "parameters": [],
            "entry_rules": [
                {
                    "id": "long_entry",
                    "direction": "long",
                    "conditions": [
                        {
                            "primary": {
                                "negated": false,
                                "trigger": {
                                    "type": "compare",
                                    "left": {"source": "price", "value": "close"},
                                    "operator": ">",
                                    "right": {"indicator": "daily_ema", "output": "value"}
                                }
                            },
                            "chain": []
                        },
                        {
                            "primary": {
                                "negated": false,
                                "trigger": {
                                    "type": "cross",
                                    "direction": "above",
                                    "left": {"indicator": "h1_macd", "output": "macd"},
                                    "right": {"indicator": "h1_macd", "output": "signal"}
                                }
                            },
                            "chain": []
                        }
                    ]
                }
            ],
            "exit_rules": [],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": 2.0,
                "spread_buffer_pips": 1.0,
                "stop_loss_source": {"indicator": "h1_atr", "output": "value", "type": "indicator"}
            }
        }"#;

        // Step 1: Verify deserialization
        let strategy_result = RulesBasedStrategy::from_json(json);
        assert!(strategy_result.is_ok(), "MTF strategy should parse: {:?}", strategy_result.err());
        let mut strategy = strategy_result.unwrap();

        // Step 2: Build daily candles (trending up to satisfy EMA + MACD conditions)
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let mut daily_candles = Vec::new();
        for i in 0..30 {
            let price = dec!(1.1000) + rust_decimal::Decimal::from(i) * dec!(0.0020);
            daily_candles.push(Candle {
                time: base_time + Duration::days(i),
                mid: Ohlc {
                    open: price - dec!(0.0005),
                    high: price + dec!(0.0020),
                    low: price - dec!(0.0010),
                    close: price,
                },
                volume: 1000,
                complete: true,
            });
        }

        // Step 3: Build MTF candle store with daily candles
        let mut mtf_store = crate::backtest::mtf::MtfCandleStore::new();
        mtf_store.add_timeframe("D".to_string(), daily_candles);
        strategy.set_mtf_candle_store(mtf_store);

        // Step 4: Run a mini backtest with H1 candles
        use crate::backtest::engine::{BacktestEngine, BacktestConfig};
        let config = BacktestConfig {
            warmup_bars: 0,
            initial_balance: dec!(10000),
            position_size: dec!(1000),
            use_percentage: false,
            risk_percent: Some(dec!(1)),
            estimated_stop_pips: dec!(20),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            instrument: String::new(),
        };
        let engine = BacktestEngine::new(config);

        // Generate H1 candles spanning the 30 days (720 H1 candles)
        let mut h1_candles = Vec::new();
        for i in 0..720 {
            let day = i / 24;
            let base_price = dec!(1.1000) + rust_decimal::Decimal::from(day) * dec!(0.0020);
            // Add some intraday variation to get RSI oscillation
            let hour_offset = if (i % 24) < 12 {
                dec!(0.0005) * rust_decimal::Decimal::from(i % 12)
            } else {
                dec!(0.0005) * rust_decimal::Decimal::from(24 - (i % 24))
            };
            let price = base_price + hour_offset;
            h1_candles.push(Candle {
                time: base_time + Duration::hours(i),
                mid: Ohlc {
                    open: price - dec!(0.0003),
                    high: price + dec!(0.0010),
                    low: price - dec!(0.0008),
                    close: price,
                },
                volume: 500,
                complete: true,
            });
        }

        let result = engine.run(&mut strategy, &h1_candles);

        // The key assertion: with 30 days of trending data and low RSI periods,
        // we should get at least some trades. Zero trades means the MTF indicators
        // aren't resolving.
        assert!(result.metrics.total_trades > 0,
            "MTF strategy produced 0 trades — HTF indicators likely not resolving. \
             Check resolve_data_source_v2 fallback and reset() re-seeding.");
    }

    #[test]
    fn test_user_reverse_engineered_mtf_strategy() {
        // Exact user strategy JSON — "2023 Reverse Engineered"
        // Has givens triggers (trending_up/trending_down) that require ADX + SMA(20) + SMA(50),
        // which are NOT in the indicator list. Tests both:
        // 1. That the JSON parses and the strategy runs without panicking
        // 2. That with givens blocking entries, we get zero trades
        // 3. With the position sync fix, if givens were satisfied, trades would work correctly
        let json = r#"{
            "name": "2023 Reverse Engineered",
            "description": "MTF strategy",
            "schema_version": 2,
            "id": "test_re",
            "user_id": "user1",
            "version": 1,
            "is_active": true,
            "indicators": [
                {"id": "daily_ema", "params": {"period": {"$param": "daily_ema_period"}}, "timeframe": "D", "type": "ema"},
                {"id": "daily_macd", "params": {"fast_period": 12, "signal_period": 9, "slow_period": 26}, "timeframe": "D", "type": "macd"},
                {"id": "h1_macd", "params": {"fast_period": 12, "signal_period": 9, "slow_period": 26}, "type": "macd"},
                {"id": "chandelier", "params": {"multiplier": {"$param": "atr_mult"}, "period": 22}, "type": "chandelier"}
            ],
            "parameters": [
                {"default": 50, "id": "daily_ema_period", "max": 100, "min": 20, "name": "Daily EMA Period", "step": 10, "type": "integer"},
                {"default": 2, "id": "atr_mult", "max": 3.5, "min": 1.5, "name": "ATR Multiplier", "step": 0.5, "type": "number"},
                {"default": 2, "id": "rr_ratio", "max": 3, "min": 1.5, "name": "Risk:Reward Ratio", "step": 0.5, "type": "number"}
            ],
            "entry_rules": [
                {
                    "id": "long_entry",
                    "direction": "long",
                    "name": "Long: Daily Trend + H1 MACD Cross Up",
                    "conditions": [
                        {"primary": {"negated": false, "trigger": {"regime": "trending_up", "type": "givens"}}, "chain": []},
                        {"primary": {"negated": false, "trigger": {"type": "compare", "left": {"source": "price", "value": "close"}, "operator": ">", "right": {"indicator": "daily_ema", "output": "value"}}}, "chain": []},
                        {"primary": {"negated": false, "trigger": {"type": "threshold", "operator": ">", "source": {"indicator": "daily_macd", "output": "histogram"}, "value": 0}}, "chain": []},
                        {"primary": {"negated": false, "trigger": {"type": "cross", "direction": "above", "left": {"indicator": "h1_macd", "output": "macd"}, "right": {"indicator": "h1_macd", "output": "signal"}}}, "chain": []}
                    ]
                },
                {
                    "id": "short_entry",
                    "direction": "short",
                    "name": "Short: Daily Trend + H1 MACD Cross Down",
                    "conditions": [
                        {"primary": {"negated": false, "trigger": {"regime": "trending_down", "type": "givens"}}, "chain": []},
                        {"primary": {"negated": false, "trigger": {"type": "compare", "left": {"source": "price", "value": "close"}, "operator": "<", "right": {"indicator": "daily_ema", "output": "value"}}}, "chain": []},
                        {"primary": {"negated": false, "trigger": {"type": "threshold", "operator": "<", "source": {"indicator": "daily_macd", "output": "histogram"}, "value": 0}}, "chain": []},
                        {"primary": {"negated": false, "trigger": {"type": "cross", "direction": "below", "left": {"indicator": "h1_macd", "output": "macd"}, "right": {"indicator": "h1_macd", "output": "signal"}}}, "chain": []}
                    ]
                }
            ],
            "exit_rules": [],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": {"$param": "rr_ratio"},
                "spread_buffer_pips": 1,
                "stop_loss_source": {"indicator": "chandelier", "output": "exit_long", "type": "indicator"},
                "stop_loss_source_short": {"indicator": "chandelier", "output": "exit_short", "type": "indicator"}
            }
        }"#;

        // Step 1: Verify deserialization
        let strategy_result = RulesBasedStrategy::from_json(json);
        assert!(strategy_result.is_ok(), "Strategy should parse: {:?}", strategy_result.err());
        let mut strategy = strategy_result.unwrap();

        // Step 2: Build daily candles (trending up then down for variety)
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T03:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let mut daily_candles = Vec::new();
        for i in 0..120 {
            // 60 days up, 60 days down
            let price = if i < 60 {
                dec!(1.1000) + rust_decimal::Decimal::from(i) * dec!(0.0015)
            } else {
                dec!(1.1000) + dec!(0.0900) - rust_decimal::Decimal::from(i - 60) * dec!(0.0015)
            };
            daily_candles.push(Candle {
                time: base_time + Duration::days(i),
                mid: Ohlc {
                    open: price - dec!(0.0005),
                    high: price + dec!(0.0030),
                    low: price - dec!(0.0020),
                    close: price,
                },
                volume: 1000,
                complete: true,
            });
        }

        // Step 3: Build MTF candle store
        let mut mtf_store = crate::backtest::mtf::MtfCandleStore::new();
        mtf_store.add_timeframe("D".to_string(), daily_candles);
        strategy.set_mtf_candle_store(mtf_store);

        // Step 4: Generate H1 candles spanning 120 days (2880 candles)
        let mut h1_candles = Vec::new();
        for i in 0..2880i64 {
            let day = i / 24;
            let base_price = if day < 60 {
                dec!(1.1000) + rust_decimal::Decimal::from(day) * dec!(0.0015)
            } else {
                dec!(1.1000) + dec!(0.0900) - rust_decimal::Decimal::from(day - 60) * dec!(0.0015)
            };
            // Intraday oscillation for MACD crosses
            let hour = i % 24;
            let intraday = if hour < 8 {
                dec!(0.0003) * rust_decimal::Decimal::from(hour)
            } else if hour < 16 {
                dec!(0.0003) * rust_decimal::Decimal::from(16 - hour)
            } else {
                -dec!(0.0002) * rust_decimal::Decimal::from(hour - 16)
            };
            let price = base_price + intraday;

            h1_candles.push(Candle {
                time: base_time + Duration::hours(i),
                mid: Ohlc {
                    open: price - dec!(0.0003),
                    high: price + dec!(0.0012),
                    low: price - dec!(0.0010),
                    close: price,
                },
                volume: 500,
                complete: true,
            });
        }

        // Step 5: Run backtest
        use crate::backtest::engine::{BacktestEngine, BacktestConfig};
        let config = BacktestConfig {
            warmup_bars: 0,
            initial_balance: dec!(10000),
            position_size: dec!(1000),
            use_percentage: false,
            risk_percent: Some(dec!(1)),
            estimated_stop_pips: dec!(20),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            instrument: String::new(),
        };
        let engine = BacktestEngine::new(config);
        let result = engine.run(&mut strategy, &h1_candles);

        // With givens triggers (trending_up/trending_down) requiring ADX + SMA(20) + SMA(50)
        // which are NOT defined in the strategy, the givens condition always returns false.
        // This means ZERO trades — the givens block is the root cause, not the backtest engine.
        // The strategy needs ADX, SMA(20), and SMA(50) indicators added to work with givens.
        assert_eq!(result.metrics.total_trades, 0,
            "Expected 0 trades because givens triggers require ADX + SMA(20) + SMA(50) \
             which are not in the indicator list. Got {} trades.",
            result.metrics.total_trades);
    }

    /// Same strategy as above but with givens triggers removed — verifies the MTF
    /// indicator pipeline produces trades when the blocking givens condition is gone.
    #[test]
    fn test_user_reverse_engineered_without_givens() {
        let json = r#"{
            "name": "2023 Reverse Engineered (no givens)",
            "description": "MTF strategy without givens",
            "schema_version": 2,
            "id": "test_re_ng",
            "user_id": "user1",
            "version": 1,
            "is_active": true,
            "indicators": [
                {"id": "daily_ema", "params": {"period": {"$param": "daily_ema_period"}}, "timeframe": "D", "type": "ema"},
                {"id": "daily_macd", "params": {"fast_period": 12, "signal_period": 9, "slow_period": 26}, "timeframe": "D", "type": "macd"},
                {"id": "h1_macd", "params": {"fast_period": 12, "signal_period": 9, "slow_period": 26}, "type": "macd"},
                {"id": "chandelier", "params": {"multiplier": {"$param": "atr_mult"}, "period": 22}, "type": "chandelier"}
            ],
            "parameters": [
                {"default": 50, "id": "daily_ema_period", "max": 100, "min": 20, "name": "Daily EMA Period", "step": 10, "type": "integer"},
                {"default": 2, "id": "atr_mult", "max": 3.5, "min": 1.5, "name": "ATR Multiplier", "step": 0.5, "type": "number"},
                {"default": 2, "id": "rr_ratio", "max": 3, "min": 1.5, "name": "Risk:Reward Ratio", "step": 0.5, "type": "number"}
            ],
            "entry_rules": [
                {
                    "id": "long_entry",
                    "direction": "long",
                    "name": "Long entry",
                    "conditions": [
                        {"primary": {"negated": false, "trigger": {"type": "compare", "left": {"source": "price", "value": "close"}, "operator": ">", "right": {"indicator": "daily_ema", "output": "value"}}}, "chain": []},
                        {"primary": {"negated": false, "trigger": {"type": "threshold", "operator": ">", "source": {"indicator": "daily_macd", "output": "histogram"}, "value": 0}}, "chain": []},
                        {"primary": {"negated": false, "trigger": {"type": "cross", "direction": "above", "left": {"indicator": "h1_macd", "output": "macd"}, "right": {"indicator": "h1_macd", "output": "signal"}}}, "chain": []}
                    ]
                },
                {
                    "id": "short_entry",
                    "direction": "short",
                    "name": "Short entry",
                    "conditions": [
                        {"primary": {"negated": false, "trigger": {"type": "compare", "left": {"source": "price", "value": "close"}, "operator": "<", "right": {"indicator": "daily_ema", "output": "value"}}}, "chain": []},
                        {"primary": {"negated": false, "trigger": {"type": "threshold", "operator": "<", "source": {"indicator": "daily_macd", "output": "histogram"}, "value": 0}}, "chain": []},
                        {"primary": {"negated": false, "trigger": {"type": "cross", "direction": "below", "left": {"indicator": "h1_macd", "output": "macd"}, "right": {"indicator": "h1_macd", "output": "signal"}}}, "chain": []}
                    ]
                }
            ],
            "exit_rules": [],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": {"$param": "rr_ratio"},
                "spread_buffer_pips": 1,
                "stop_loss_source": {"indicator": "chandelier", "output": "exit_long", "type": "indicator"},
                "stop_loss_source_short": {"indicator": "chandelier", "output": "exit_short", "type": "indicator"}
            }
        }"#;

        let strategy_result = RulesBasedStrategy::from_json(json);
        assert!(strategy_result.is_ok(), "Strategy should parse: {:?}", strategy_result.err());
        let mut strategy = strategy_result.unwrap();

        let base_time = DateTime::parse_from_rfc3339("2024-01-01T03:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // Build daily candles — trending up
        let mut daily_candles = Vec::new();
        for i in 0..120 {
            let price = if i < 60 {
                dec!(1.1000) + rust_decimal::Decimal::from(i) * dec!(0.0015)
            } else {
                dec!(1.1000) + dec!(0.0900) - rust_decimal::Decimal::from(i - 60) * dec!(0.0015)
            };
            daily_candles.push(Candle {
                time: base_time + Duration::days(i),
                mid: Ohlc {
                    open: price - dec!(0.0005),
                    high: price + dec!(0.0030),
                    low: price - dec!(0.0020),
                    close: price,
                },
                volume: 1000,
                complete: true,
            });
        }

        let mut mtf_store = crate::backtest::mtf::MtfCandleStore::new();
        mtf_store.add_timeframe("D".to_string(), daily_candles);
        strategy.set_mtf_candle_store(mtf_store);

        // Generate H1 candles with oscillation for MACD crosses
        let mut h1_candles = Vec::new();
        for i in 0..2880i64 {
            let day = i / 24;
            let base_price = if day < 60 {
                dec!(1.1000) + rust_decimal::Decimal::from(day) * dec!(0.0015)
            } else {
                dec!(1.1000) + dec!(0.0900) - rust_decimal::Decimal::from(day - 60) * dec!(0.0015)
            };
            let hour = i % 24;
            let intraday = if hour < 8 {
                dec!(0.0003) * rust_decimal::Decimal::from(hour)
            } else if hour < 16 {
                dec!(0.0003) * rust_decimal::Decimal::from(16 - hour)
            } else {
                -dec!(0.0002) * rust_decimal::Decimal::from(hour - 16)
            };
            let price = base_price + intraday;
            h1_candles.push(Candle {
                time: base_time + Duration::hours(i),
                mid: Ohlc {
                    open: price - dec!(0.0003),
                    high: price + dec!(0.0012),
                    low: price - dec!(0.0010),
                    close: price,
                },
                volume: 500,
                complete: true,
            });
        }

        use crate::backtest::engine::{BacktestEngine, BacktestConfig};
        let config = BacktestConfig {
            warmup_bars: 0,
            initial_balance: dec!(10000),
            position_size: dec!(1000),
            use_percentage: false,
            risk_percent: Some(dec!(1)),
            estimated_stop_pips: dec!(20),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            instrument: String::new(),
        };
        let engine = BacktestEngine::new(config);
        let result = engine.run(&mut strategy, &h1_candles);

        // Without givens, the remaining conditions (daily EMA + daily MACD + H1 MACD cross)
        // should produce trades. This proves the MTF pipeline works and the position sync
        // fix lets new entries fire after SL/TP closes.
        assert!(result.metrics.total_trades > 0,
            "Expected trades with givens removed — MTF pipeline should work. Got 0 trades.");

        // With the position sync fix, we should get MORE than 1 trade
        // (the old bug would lock the engine after the first SL/TP close)
        assert!(result.metrics.total_trades > 1,
            "Expected multiple trades (position sync fix). Got only {} trade(s).",
            result.metrics.total_trades);

        println!(
            "Without givens: {} trades (longs: {}, shorts: {})",
            result.metrics.total_trades,
            result.trades.iter().filter(|t| t.is_long).count(),
            result.trades.iter().filter(|t| !t.is_long).count()
        );
    }

    #[test]
    fn test_fixed_macd_crossover_with_givens() {
        // Exact user MACD Crossover strategy with ADX + SMA(20) + SMA(50) added
        // so the givens triggers (trending_up/trending_down) can evaluate.
        let json = r#"{
            "name": "MACD Crossover",
            "description": "MACD crossover with givens trend filter",
            "schema_version": 2,
            "id": "test_macd",
            "user_id": "user1",
            "version": 1,
            "is_active": true,
            "indicators": [
                {"id": "macd", "params": {"fast_period": {"$param": "fast_period"}, "signal_period": {"$param": "signal_period"}, "slow_period": {"$param": "slow_period"}}, "type": "macd"},
                {"id": "adx", "params": {"period": 14}, "type": "adx"},
                {"id": "sma_20", "params": {"period": 20}, "type": "sma"},
                {"id": "sma_50", "params": {"period": 50}, "type": "sma"}
            ],
            "parameters": [
                {"default": 12, "id": "fast_period", "max": 16, "min": 8, "name": "Fast Period", "step": 2, "type": "integer"},
                {"default": 26, "id": "slow_period", "max": 32, "min": 20, "name": "Slow Period", "step": 2, "type": "integer"},
                {"default": 9, "id": "signal_period", "max": 13, "min": 5, "name": "Signal Period", "step": 2, "type": "integer"}
            ],
            "entry_rules": [
                {
                    "id": "long_entry",
                    "direction": "long",
                    "name": "MACD Cross Above Signal",
                    "conditions": [
                        {"chain": [], "primary": {"negated": false, "trigger": {"direction": "above", "left": {"indicator": "macd", "output": "macd"}, "right": {"indicator": "macd", "output": "signal"}, "type": "cross"}}},
                        {"chain": [], "primary": {"negated": false, "trigger": {"regime": "trending_up", "type": "givens"}}}
                    ]
                },
                {
                    "id": "short_entry",
                    "direction": "short",
                    "name": "MACD Cross Below Signal",
                    "conditions": [
                        {"chain": [], "primary": {"negated": false, "trigger": {"direction": "below", "left": {"indicator": "macd", "output": "macd"}, "right": {"indicator": "macd", "output": "signal"}, "type": "cross"}}},
                        {"chain": [], "primary": {"negated": false, "trigger": {"regime": "trending_down", "type": "givens"}}}
                    ]
                }
            ],
            "exit_rules": [],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": 2,
                "spread_buffer_pips": 1,
                "stop_loss_source": {"pips": 30, "type": "fixed_pips"}
            }
        }"#;

        let strategy_result = RulesBasedStrategy::from_json(json);
        assert!(strategy_result.is_ok(), "Strategy should parse: {:?}", strategy_result.err());
        let mut strategy = strategy_result.unwrap();

        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // Generate 200 days of H1 candles (4800 candles).
        // First 100 days: strong uptrend (for trending_up givens).
        // Next 100 days: strong downtrend (for trending_down givens).
        // Need enough data for SMA(50) warmup + ADX(14) warmup.
        // Trend must dominate oscillation so ADX > 25.
        let mut h1_candles = Vec::new();
        for i in 0..4800i64 {
            let day = i / 24;
            let base_price = if day < 100 {
                // Strong uptrend: +0.0040/day (>> oscillation amplitude)
                dec!(1.0500) + rust_decimal::Decimal::from(day) * dec!(0.0040)
            } else {
                // Strong downtrend
                dec!(1.4500) - rust_decimal::Decimal::from(day - 100) * dec!(0.0040)
            };

            // Small intraday oscillation — just enough for MACD crosses
            // but much smaller than the daily trend move
            let hour = i % 24;
            let cycle = (hour as f64 * std::f64::consts::PI / 12.0).sin();
            let intraday = rust_decimal::Decimal::from_f64_retain(cycle * 0.0004)
                .unwrap_or(Decimal::ZERO);
            let price = base_price + intraday;

            h1_candles.push(Candle {
                time: base_time + Duration::hours(i),
                mid: Ohlc {
                    open: price - dec!(0.0005),
                    high: price + dec!(0.0015),
                    low: price - dec!(0.0012),
                    close: price,
                },
                volume: 500,
                complete: true,
            });
        }

        use crate::backtest::engine::{BacktestEngine, BacktestConfig};
        let config = BacktestConfig {
            warmup_bars: 0,
            initial_balance: dec!(10000),
            position_size: dec!(1000),
            use_percentage: false,
            risk_percent: Some(dec!(1)),
            estimated_stop_pips: dec!(30),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            instrument: String::new(),
        };
        let engine = BacktestEngine::new(config);
        let result = engine.run(&mut strategy, &h1_candles);

        println!(
            "MACD Crossover with givens: {} trades (longs: {}, shorts: {})",
            result.metrics.total_trades,
            result.trades.iter().filter(|t| t.is_long).count(),
            result.trades.iter().filter(|t| !t.is_long).count()
        );

        // With proper indicators (ADX, SMA20, SMA50) for givens, plus MACD cross,
        // we should get trades in a strongly trending market.
        assert!(result.metrics.total_trades > 0,
            "MACD Crossover with givens + required indicators should produce trades. \
             Got 0. Check ADX threshold (>25), SMA ordering, and MACD cross timing.");
    }

    #[test]
    fn test_givens_trending_without_adx_uses_sma_alignment() {
        // Regression test: givens trending_up/trending_down must work when the
        // strategy has SMA(20) + SMA(50) but NO ADX indicator. Before this fix,
        // missing ADX returned false (blocking all trend entries). After: SMA
        // alignment alone is sufficient when ADX is genuinely unconfigured.
        let json = r#"{
            "name": "MACD no ADX",
            "description": "test",
            "schema_version": 2,
            "id": "test_no_adx",
            "user_id": "user1",
            "version": 1,
            "is_active": true,
            "indicators": [
                {"id": "macd", "params": {"fast_period": 12, "signal_period": 9, "slow_period": 26}, "type": "macd"},
                {"id": "sma_20", "params": {"period": 20}, "type": "sma"},
                {"id": "sma_50", "params": {"period": 50}, "type": "sma"}
            ],
            "parameters": [],
            "entry_rules": [
                {
                    "id": "long_entry",
                    "direction": "long",
                    "conditions": [
                        {"chain": [], "primary": {"negated": false, "trigger": {"direction": "above", "left": {"indicator": "macd", "output": "macd"}, "right": {"indicator": "macd", "output": "signal"}, "type": "cross"}}},
                        {"chain": [], "primary": {"negated": false, "trigger": {"regime": "trending_up", "type": "givens"}}}
                    ]
                },
                {
                    "id": "short_entry",
                    "direction": "short",
                    "conditions": [
                        {"chain": [], "primary": {"negated": false, "trigger": {"direction": "below", "left": {"indicator": "macd", "output": "macd"}, "right": {"indicator": "macd", "output": "signal"}, "type": "cross"}}},
                        {"chain": [], "primary": {"negated": false, "trigger": {"regime": "trending_down", "type": "givens"}}}
                    ]
                }
            ],
            "exit_rules": [],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": 2,
                "spread_buffer_pips": 1,
                "stop_loss_source": {"pips": 30, "type": "fixed_pips"}
            }
        }"#;

        let mut strategy = RulesBasedStrategy::from_json(json).unwrap();

        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // Generate trending H1 data: 100 days up, 100 days down (4800 candles)
        let mut h1_candles = Vec::new();
        for i in 0..4800i64 {
            let day = i / 24;
            let base_price = if day < 100 {
                dec!(1.0500) + Decimal::from(day) * dec!(0.0040)
            } else {
                dec!(1.4500) - Decimal::from(day - 100) * dec!(0.0040)
            };
            let hour = i % 24;
            let cycle = Decimal::try_from(
                (hour as f64 * std::f64::consts::PI / 12.0).sin() * 0.0004
            ).unwrap_or(Decimal::ZERO);
            let price = base_price + cycle;
            h1_candles.push(Candle {
                time: base_time + Duration::hours(i),
                mid: Ohlc {
                    open: price - dec!(0.0005),
                    high: price + dec!(0.0015),
                    low: price - dec!(0.0012),
                    close: price,
                },
                volume: 500,
                complete: true,
            });
        }

        use crate::backtest::engine::{BacktestEngine, BacktestConfig};
        let config = BacktestConfig {
            warmup_bars: 0,
            initial_balance: dec!(10000),
            position_size: dec!(1000),
            use_percentage: false,
            risk_percent: Some(dec!(1)),
            estimated_stop_pips: dec!(30),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            instrument: String::new(),
        };
        let result = BacktestEngine::new(config).run(&mut strategy, &h1_candles);

        assert!(result.metrics.total_trades > 0,
            "Strategy with SMA but no ADX should produce trades via SMA alignment alone. \
             Got 0 trades — givens may still be blocking when ADX is unconfigured.");
    }

    #[test]
    fn test_givens_trending_blocks_during_adx_warmup() {
        // Ensure that when ADX IS configured but hasn't warmed up yet (returns None),
        // the givens trigger returns false (conservative). This prevents spurious
        // early trades that would be filtered once ADX is ready.
        let json = r#"{
            "name": "MACD with ADX",
            "description": "test",
            "schema_version": 2,
            "id": "test_warmup",
            "user_id": "user1",
            "version": 1,
            "is_active": true,
            "indicators": [
                {"id": "macd", "params": {"fast_period": 3, "signal_period": 3, "slow_period": 5}, "type": "macd"},
                {"id": "adx", "params": {"period": 14}, "type": "adx"},
                {"id": "sma_20", "params": {"period": 20}, "type": "sma"},
                {"id": "sma_50", "params": {"period": 50}, "type": "sma"}
            ],
            "parameters": [],
            "entry_rules": [
                {
                    "id": "long_entry",
                    "direction": "long",
                    "conditions": [
                        {"chain": [], "primary": {"negated": false, "trigger": {"direction": "above", "left": {"indicator": "macd", "output": "macd"}, "right": {"indicator": "macd", "output": "signal"}, "type": "cross"}}},
                        {"chain": [], "primary": {"negated": false, "trigger": {"regime": "trending_up", "type": "givens"}}}
                    ]
                }
            ],
            "exit_rules": [],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": 2,
                "spread_buffer_pips": 1,
                "stop_loss_source": {"pips": 30, "type": "fixed_pips"}
            }
        }"#;

        let def: crate::backtest::rules_engine::StrategyDefinition = serde_json::from_str(json).unwrap();
        let mut engine = crate::backtest::rules_engine::RulesEngine::new(def).unwrap();

        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // Feed only 10 candles — ADX(14) won't have warmed up yet.
        // Use a strong uptrend so SMA alignment would pass if ADX weren't blocking.
        let mut trades_during_warmup = 0u32;
        for i in 0..10i64 {
            let price = dec!(1.0500) + Decimal::from(i) * dec!(0.0040);
            let candle = Candle {
                time: base_time + Duration::hours(i),
                mid: Ohlc {
                    open: price - dec!(0.0005),
                    high: price + dec!(0.0015),
                    low: price - dec!(0.0012),
                    close: price,
                },
                volume: 500,
                complete: true,
            };
            let signal = engine.on_candle(&candle);
            if !matches!(signal, crate::backtest::rules_engine::RulesSignal::Hold) {
                trades_during_warmup += 1;
            }
        }

        assert_eq!(trades_during_warmup, 0,
            "No trades should fire during ADX warmup period (ADX configured but None). \
             Got {} signals — warmup bypass bug.", trades_during_warmup);
    }

    /// Regression test: indicators with an explicit timeframe matching the chart timeframe
    /// were being routed to an HTF engine that never received candles (extract_htf_timeframes
    /// skips the primary timeframe). set_primary_granularity reclassifies them to the
    /// primary engine so their values resolve correctly.
    #[test]
    fn test_same_timeframe_indicators_reclassified_to_primary() {
        // Strategy with an H4 indicator that has explicit timeframe "H4"
        // When run on an H4 chart, this indicator must be reclassified to the primary engine
        let json = r#"{
            "id": "same_tf_test",
            "user_id": "test",
            "name": "Same-TF Reclassification Test",
            "description": "Test same-timeframe indicator reclassification",
            "version": 1,
            "is_active": true,
            "schema_version": 2,
            "indicators": [
                {"id": "sma_explicit_h4", "type": "sma", "timeframe": "H4", "params": {"period": 20}},
                {"id": "sma_no_tf", "type": "sma", "params": {"period": 50}}
            ],
            "parameters": [],
            "entry_rules": [{
                "id": "long_entry",
                "name": "Long",
                "direction": "long",
                "conditions": [{
                    "primary": {
                        "trigger": {
                            "type": "cross",
                            "left": {"indicator": "sma_explicit_h4", "output": "value"},
                            "right": {"indicator": "sma_no_tf", "output": "value"},
                            "direction": "above"
                        },
                        "negated": false
                    },
                    "chain": []
                }]
            }],
            "exit_rules": [],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": 2,
                "spread_buffer_pips": 1
            }
        }"#;

        let mut strategy = RulesBasedStrategy::from_json(json).expect("Strategy should parse");

        // Before reclassification: sma_explicit_h4 is in the HTF engine for "H4"
        // and will return None on every candle (no HTF candles provided).
        // Generate candles: flat/declining first (SMA20 < SMA50), then rising sharply
        // so SMA(20) crosses above SMA(50).
        let mut candles = Vec::new();
        for i in 0..120i64 {
            let price = if i < 70 {
                // Declining: SMA(20) tracks lower, SMA(50) stays higher
                dec!(1.2000) - rust_decimal::Decimal::from(i) * dec!(0.0005)
            } else {
                // Sharp rally: SMA(20) rises fast, crosses above SMA(50)
                dec!(1.1650) + rust_decimal::Decimal::from(i - 70) * dec!(0.0020)
            };
            candles.push(create_test_candle(price, i));
        }

        // Run WITHOUT reclassification - the explicit-TF indicator returns None
        strategy.prepare(&candles);
        let mut signals_without: Vec<Signal> = Vec::new();
        for candle in &candles {
            signals_without.push(strategy.on_candle(candle));
        }
        let entries_without = signals_without.iter().filter(|s| matches!(s, Signal::Buy | Signal::Sell)).count();

        // Now create a fresh strategy WITH reclassification
        let mut strategy2 = RulesBasedStrategy::from_json(json).expect("Strategy should parse");
        strategy2.set_primary_granularity("H4"); // chart is H4, reclassify H4 indicators
        strategy2.prepare(&candles);
        let mut signals_with: Vec<Signal> = Vec::new();
        for candle in &candles {
            signals_with.push(strategy2.on_candle(candle));
        }
        let entries_with = signals_with.iter().filter(|s| matches!(s, Signal::Buy | Signal::Sell)).count();

        // Without reclassification: the explicit-TF SMA never resolves, so cross trigger
        // can never fire — expect zero entries.
        assert_eq!(entries_without, 0,
            "Without reclassification, explicit-TF indicator should never resolve → 0 entries");

        // With reclassification: both SMAs resolve on the primary engine, crosses can fire
        assert!(entries_with > 0,
            "With reclassification, same-TF indicator should resolve and produce entries. Got 0.");
    }

    #[test]
    fn test_user_ichimoku_strategy_json() {
        // Exact JSON from user that fails - find where the error is
        let json = r#"{"name":"Ichimoku - Personal Customization","description":"Ichimoku strategy with Tenkan/Kijun crossover entry, cloud confirmation, stop loss at Kijun, and 3:1 take profit. Exit on SL/TP or momentum reversal (opposite crossover).","schema_version":2,"indicators":[{"id":"ichimoku","type":"ichimoku","params":{"tenkan_period":{"$param":"tenkan"},"kijun_period":{"$param":"kijun"},"senkou_b_period":{"$param":"senkou_b"},"displacement":26}},{"id":"sma","type":"sma","params":{"period":20}}],"parameters":[{"id":"tenkan","name":"tenkan","description":"Tenkan","type":"integer","default":7,"group":"indicator","min":4,"max":18,"step":1},{"id":"kijun","name":"kijun","description":"kijun","type":"integer","default":22,"group":"indicator","min":18,"max":24,"step":1},{"id":"senkou_b","name":"senkou b","description":"senkou b","type":"integer","default":44,"group":"indicator","min":40,"max":48,"step":1},{"id":"risk_reward","name":"risk reward","description":"Risk reward","type":"integer","default":3,"group":"indicator","min":3,"max":3,"step":1}],"entry_rules":[{"id":"long_entry","name":"Long - Tenkan Cross Above Kijun + Cloud Support","direction":"long","conditions":[{"primary":{"trigger":{"type":"cross","left":{"indicator":"ichimoku","output":"tenkan"},"right":{"indicator":"ichimoku","output":"kijun"},"direction":"above"},"negated":false},"chain":[]},{"primary":{"trigger":{"type":"compare","left":{"source":"price","value":"close"},"operator":">","right":{"indicator":"ichimoku","output":"tenkan"}},"negated":false},"chain":[{"operator":"and","trigger":{"trigger":{"type":"compare","left":{"source":"price","value":"close"},"operator":">","right":{"indicator":"ichimoku","output":"kijun"}},"negated":false}}]},{"primary":{"trigger":{"type":"compare","left":{"source":"price","value":"close"},"operator":">","right":{"indicator":"ichimoku","output":"cloud_top"}},"negated":false},"chain":[]},{"primary":{"trigger":{"type":"givens","regime":"ranging"},"negated":true},"chain":[]},{"primary":{"trigger":{"type":"givens","regime":"low_volatility"},"negated":true},"chain":[]}]},{"id":"short_entry","name":"Short - Tenkan Cross Below Kijun + Cloud Resistance","direction":"short","conditions":[{"primary":{"trigger":{"type":"cross","left":{"indicator":"ichimoku","output":"tenkan"},"right":{"indicator":"ichimoku","output":"kijun"},"direction":"below"},"negated":false},"chain":[]},{"primary":{"trigger":{"type":"compare","left":{"source":"price","value":"close"},"operator":"<","right":{"indicator":"ichimoku","output":"tenkan"}},"negated":false},"chain":[{"operator":"and","trigger":{"trigger":{"type":"compare","left":{"source":"price","value":"close"},"operator":"<","right":{"indicator":"ichimoku","output":"kijun"}},"negated":false}}]},{"primary":{"trigger":{"type":"compare","left":{"source":"price","value":"close"},"operator":"<","right":{"indicator":"ichimoku","output":"cloud_bottom"}},"negated":false},"chain":[]},{"primary":{"trigger":{"type":"givens","regime":"ranging"},"negated":true},"chain":[]},{"primary":{"trigger":{"type":"givens","regime":"low_volatility"},"negated":true},"chain":[]}]}],"exit_rules":[{"id":"long_momentum_shift","name":"Long Exit - Tenkan Crosses Below Kijun","direction":"long","close_percent":100,"priority":2,"conditions":[{"primary":{"trigger":{"type":"cross","left":{"indicator":"ichimoku","output":"tenkan"},"right":{"indicator":"ichimoku","output":"kijun"},"direction":"below"},"negated":false},"chain":[]}]},{"id":"short_momentum_shift","name":"Short Exit - Tenkan Crosses Above Kijun","direction":"short","close_percent":100,"priority":2,"conditions":[{"primary":{"trigger":{"type":"cross","left":{"indicator":"ichimoku","output":"tenkan"},"right":{"indicator":"ichimoku","output":"kijun"},"direction":"above"},"negated":false},"chain":[]}]}],"risk_settings":{"risk_method":"percent","risk_value":1,"rr_ratio":{"$param":"risk_reward"},"spread_buffer_pips":1,"stop_loss_source":{"type":"indicator","indicator":"ichimoku","output":"kijun"}},"id":"user1","user_id":"user1","version":1,"is_active":true}"#;

        let result = RulesBasedStrategy::from_json(json);
        match result {
            Ok(_) => println!("Strategy parsed successfully!"),
            Err(e) => println!("Parse error: {}", e),
        }
    }
}
