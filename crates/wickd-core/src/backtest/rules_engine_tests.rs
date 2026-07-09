//! Tests for RulesEngine
//!
//! Separated from rules_engine.rs to reduce file size.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use chrono::{DateTime, Utc, Duration};

use crate::models::{Candle, Ohlc};
use super::indicator_engine::IndicatorConfig;
use super::rules_types::*;
use super::rules_engine::RulesEngine;
use shared::ParameterizedValue;

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

fn create_simple_strategy() -> StrategyDefinition {
    StrategyDefinition {
        id: "test".to_string(),
        user_id: "user1".to_string(),
        name: "Test Strategy".to_string(),
        description: "A simple test strategy".to_string(),
        parameters: vec![],
        indicators: vec![
            IndicatorConfig::new_fixed("sma_fast", IndicatorType::Sma, &[("period", 5.0)]),
            IndicatorConfig::new_fixed("sma_slow", IndicatorType::Sma, &[("period", 10.0)]),
        ],
        variables: vec![],
        entry_rules: vec![
            EntryRule {
                id: "cross_up".to_string(),
                name: Some("Fast crosses above slow".to_string()),
                direction: RuleDirection::Long,
                conditions: vec![
                    Condition {
                        name: None,
                        primary: TriggerWithNot {
                            trigger: Trigger::Cross(CrossTrigger {
                                left: DataSource::Indicator(IndicatorSource {
                                    indicator: "sma_fast".to_string(),
                                    output: "value".to_string(),
                                    offset: 0,
                                    symbol: None,
                                    timeframe: None,
                                    capture: CaptureMode::EachCandle,
                                    trail: None,
                                }),
                                right: DataSource::Indicator(IndicatorSource {
                                    indicator: "sma_slow".to_string(),
                                    output: "value".to_string(),
                                    offset: 0,
                                    symbol: None,
                                    timeframe: None,
                                    capture: CaptureMode::EachCandle,
                                    trail: None,
                                }),
                                direction: CrossDirection::Above,
                                lookback: ParameterizedValue::Fixed(1.0),
                            }),
                            negated: false,
                        },
                        chain: vec![],
                        disabled: None,
                    },
                ],
                trigger_chain: None,
                pending_order: None,
            },
        ],
        entry_logic: EntryLogic {
            mode: EntryLogicMode::All,
            min_score: None,
        },
        exit_rules: vec![
            ExitRule {
                id: "tp".to_string(),
                name: Some("Take profit at 2:1".to_string()),
                direction: RuleDirection::Both,
                conditions: vec![
                    Condition {
                        name: None,
                        primary: TriggerWithNot {
                            trigger: Trigger::RiskReward(RiskRewardTrigger {
                                ratio: ParameterizedValue::Fixed(2.0),
                            }),
                            negated: false,
                        },
                        chain: vec![],
                        disabled: None,
                    },
                ],
                trigger_chain: None,
                close_percent: ParameterizedValue::Fixed(100.0),
                priority: 100,
            },
        ],
        risk_settings: RiskSettings {
            risk_method: RiskMethod::Percent,
            risk_value: ParameterizedValue::Fixed(1.0),
            rr_ratio: ParameterizedValue::Fixed(2.0),
            spread_buffer_pips: ParameterizedValue::Fixed(1.0),
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
    }
}

#[test]
fn test_rules_engine_creation() {
    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy);
    assert!(engine.is_ok());
}

#[test]
fn test_rules_engine_processes_candles() {
    let strategy = create_simple_strategy();
    let mut engine = RulesEngine::new(strategy).unwrap();

    // Process some candles
    for i in 0..20 {
        let price = dec!(1.1000) + Decimal::from(i) * dec!(0.0010);
        engine.on_candle(&create_test_candle(price, i as i64));
    }

    // Engine should be functional
    assert!(!engine.has_position());
}

#[test]
fn test_rules_engine_set_balance() {
    let strategy = create_simple_strategy();
    let mut engine = RulesEngine::new(strategy).unwrap();

    engine.set_balance(dec!(50000));
    // Balance is private, but we can test get_risk_amount behavior
    let risk = engine.get_risk_amount(dec!(50000), PositionDirection::Long);
    assert_eq!(risk, dec!(500)); // 1% of 50000
}

#[test]
fn test_rules_engine_reset() {
    let strategy = create_simple_strategy();
    let mut engine = RulesEngine::new(strategy).unwrap();

    // Process some candles
    for i in 0..20 {
        engine.on_candle(&create_test_candle(dec!(1.1000), i));
    }

    engine.reset();
    assert!(!engine.has_position());
}

#[test]
fn test_get_risk_amount_percent() {
    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy).unwrap();

    // Strategy has risk_value = 1.0 (1%)
    let risk = engine.get_risk_amount(dec!(10000), PositionDirection::Long);
    assert_eq!(risk, dec!(100)); // 1% of 10000
}

#[test]
fn test_get_risk_amount_fixed() {
    let mut strategy = create_simple_strategy();
    strategy.risk_settings.risk_method = RiskMethod::FixedAmount;
    strategy.risk_settings.risk_value = ParameterizedValue::Fixed(250.0);

    let engine = RulesEngine::new(strategy).unwrap();
    let risk = engine.get_risk_amount(dec!(10000), PositionDirection::Long);

    assert_eq!(risk, dec!(250));
}

// V1 trigger tests removed - V1 evaluation methods no longer exist
// All evaluation now happens through methods in rules_triggers.rs

#[test]
fn test_rules_signal_to_signal_conversion() {
    use super::strategy::Signal;
    use super::rules_engine::RulesSignal;

    let hold = RulesSignal::Hold;
    assert!(matches!(Signal::from(hold), Signal::Hold));

    let entry_long = RulesSignal::Entry {
        direction: PositionDirection::Long,
        stop_loss: None,
        take_profit: None,
        triggered_rule_id: None,
        triggered_rule_name: None,
        pending_order: None,
    };
    assert!(matches!(Signal::from(entry_long), Signal::Buy));

    let entry_short = RulesSignal::Entry {
        direction: PositionDirection::Short,
        stop_loss: None,
        take_profit: None,
        triggered_rule_id: None,
        triggered_rule_name: None,
        pending_order: None,
    };
    assert!(matches!(Signal::from(entry_short), Signal::Sell));

    let exit = RulesSignal::Exit {
        reason: "Test".to_string(),
        close_percent: 100.0,
    };
    assert!(matches!(Signal::from(exit), Signal::ClosePosition));

    let partial = RulesSignal::PartialExit {
        reason: "Test".to_string(),
        close_percent: 50.0,
        new_stop_loss: None,
    };
    assert!(matches!(Signal::from(partial), Signal::ClosePosition));
}

#[test]
fn test_position_direction_equality() {
    assert_eq!(PositionDirection::Long, PositionDirection::Long);
    assert_eq!(PositionDirection::Short, PositionDirection::Short);
    assert_ne!(PositionDirection::Long, PositionDirection::Short);
}

#[test]
fn test_disabled_condition_is_skipped() {
    // Create a strategy with one disabled condition and one enabled condition
    let mut strategy = create_simple_strategy();

    // Create an impossible cross trigger (fast SMA crosses ABOVE slow SMA with negated=true)
    // This means: fast does NOT cross above slow - which blocks all entries
    // But we'll DISABLE it, so it should pass anyway
    let impossible_condition = Condition {
        name: None,
        primary: TriggerWithNot {
            trigger: Trigger::Cross(CrossTrigger {
                left: DataSource::Indicator(IndicatorSource {
                    indicator: "sma_fast".to_string(),
                    output: "value".to_string(),
                    offset: 0,
                    symbol: None,
                    timeframe: None,
                    capture: CaptureMode::EachCandle,
                    trail: None,
                }),
                right: DataSource::Indicator(IndicatorSource {
                    indicator: "sma_slow".to_string(),
                    output: "value".to_string(),
                    offset: 0,
                    symbol: None,
                    timeframe: None,
                    capture: CaptureMode::EachCandle,
                    trail: None,
                }),
                direction: CrossDirection::Above,
                lookback: ParameterizedValue::Fixed(1.0),
            }),
            negated: true, // Negated = this will almost never pass
        },
        chain: vec![],
        disabled: Some(ParameterizedValue::Fixed(1.0)), // DISABLED - should be skipped
    };

    // Normal condition - fast crosses above slow (should trigger during uptrend)
    let normal_condition = Condition {
        name: None,
        primary: TriggerWithNot {
            trigger: Trigger::Cross(CrossTrigger {
                left: DataSource::Indicator(IndicatorSource {
                    indicator: "sma_fast".to_string(),
                    output: "value".to_string(),
                    offset: 0,
                    symbol: None,
                    timeframe: None,
                    capture: CaptureMode::EachCandle,
                    trail: None,
                }),
                right: DataSource::Indicator(IndicatorSource {
                    indicator: "sma_slow".to_string(),
                    output: "value".to_string(),
                    offset: 0,
                    symbol: None,
                    timeframe: None,
                    capture: CaptureMode::EachCandle,
                    trail: None,
                }),
                direction: CrossDirection::Above,
                lookback: ParameterizedValue::Fixed(1.0),
            }),
            negated: false,
        },
        chain: vec![],
        disabled: None, // ENABLED
    };

    // Add both conditions - the impossible one (disabled) and normal one
    strategy.entry_rules[0].conditions = vec![impossible_condition, normal_condition];

    let mut engine = RulesEngine::new(strategy).unwrap();

    // Process uptrending candles - fast SMA should eventually cross above slow
    for i in 0..30 {
        let price = dec!(1.1000) + Decimal::from(i) * dec!(0.0020);
        engine.on_candle(&create_test_candle(price, i as i64));
    }

    // The impossible condition is disabled, so it passes
    // The normal condition should trigger during the uptrend
    // This test verifies the engine doesn't crash and processes correctly
}

#[test]
fn test_disabled_zero_means_enabled() {
    // Verify that disabled: Some(0.0) means the condition is ENABLED (not skipped)
    let mut strategy = create_simple_strategy();

    // Create an impossible condition: fast SMA crosses BELOW slow SMA (in an uptrend)
    // Since we're feeding rising prices, fast should always be above slow after warmup
    // So "crosses below" should never happen
    let impossible_condition = Condition {
        name: None,
        primary: TriggerWithNot {
            trigger: Trigger::Cross(CrossTrigger {
                left: DataSource::Indicator(IndicatorSource {
                    indicator: "sma_fast".to_string(),
                    output: "value".to_string(),
                    offset: 0,
                    symbol: None,
                    timeframe: None,
                    capture: CaptureMode::EachCandle,
                    trail: None,
                }),
                right: DataSource::Indicator(IndicatorSource {
                    indicator: "sma_slow".to_string(),
                    output: "value".to_string(),
                    offset: 0,
                    symbol: None,
                    timeframe: None,
                    capture: CaptureMode::EachCandle,
                    trail: None,
                }),
                direction: CrossDirection::Below, // Fast crosses BELOW slow - won't happen in uptrend
                lookback: ParameterizedValue::Fixed(1.0),
            }),
            negated: false,
        },
        chain: vec![],
        disabled: Some(ParameterizedValue::Fixed(0.0)), // 0 = ENABLED, should block
    };

    strategy.entry_rules[0].conditions = vec![impossible_condition];

    let mut engine = RulesEngine::new(strategy).unwrap();

    // Process rising candles - fast SMA should stay above slow SMA
    for i in 0..30 {
        let price = dec!(1.1000) + Decimal::from(i) * dec!(0.0020);
        engine.on_candle(&create_test_candle(price, i as i64));
    }

    // The condition has disabled: 0 which means it's still evaluated
    // "Fast crosses below slow" should never happen in uptrend, blocking all entries
    assert!(!engine.has_position());
}

// ============================================================================
// Grouped AND Evaluation Tests
// ============================================================================
//
// These tests verify that trigger chains with AND/OR operators are evaluated
// with proper grouping:
// - Groups are AND'd together (all groups must pass)
// - Triggers within a group are OR'd (any trigger in group must pass)
// - Groups are determined by splitting the chain at AND operators
//
// Example: A OR B AND C = (A OR B) AND (C)

/// Helper to create a Threshold trigger that tests if price is above/below a threshold.
/// This allows us to create triggers that pass or fail based on the test candle price.
fn create_threshold_trigger(_value: f64, threshold: f64, above: bool) -> Trigger {
    Trigger::Threshold(ThresholdTrigger {
        source: DataSource::Price(PriceSource {
            source: "price".to_string(),
            value: PriceType::Close,
            offset: 0,
            symbol: None,
            timeframe: None,
            capture: CaptureMode::EachCandle,
            trail: None,
        }),
        operator: if above { ComparisonOperator::GreaterThan } else { ComparisonOperator::LessThan },
        value: ParameterizedValue::Fixed(threshold),
        lookback: ParameterizedValue::Fixed(1.0),
    })
}

#[test]
fn test_grouped_and_all_groups_pass() {
    // Test: (A OR B) AND (C)
    // Chain: A OR B AND C
    // Group 1: price > 1.0 OR price > 0.5 (both true, group passes)
    // Group 2: price > 0.9 (true, group passes)
    // Expected: condition passes because ALL groups pass

    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy).unwrap();

    let condition = Condition {
        name: None,
        primary: TriggerWithNot {
            trigger: create_threshold_trigger(0.0, 1.0, true), // price > 1.0 (true with 1.1)
            negated: false,
        },
        chain: vec![
            ChainedTriggerWithNot {
                operator: ChainOperator::Or, // Same group as primary
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 0.5, true), // price > 0.5 (true)
                    negated: false,
                },
            },
            ChainedTriggerWithNot {
                operator: ChainOperator::And, // Start new group
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 0.9, true), // price > 0.9 (true)
                    negated: false,
                },
            },
        ],
        disabled: None,
    };

    let candle = create_test_candle(dec!(1.1000), 0);
    let result = engine.evaluate_condition(&condition, &candle);

    // Group 1: (1.1 > 1.0) OR (1.1 > 0.5) = true OR true = true
    // Group 2: (1.1 > 0.9) = true
    // Result: true AND true = true
    assert!(result, "Condition should pass because all groups pass");
}

#[test]
fn test_grouped_and_one_group_fails() {
    // Test: (A OR B) AND (C)
    // Chain: A OR B AND C
    // Group 1: price > 1.0 OR price > 0.5 (both true, group passes)
    // Group 2: price > 2.0 (false, group fails)
    // Expected: condition FAILS because one group fails (AND'd together)

    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy).unwrap();

    let condition = Condition {
        name: None,
        primary: TriggerWithNot {
            trigger: create_threshold_trigger(0.0, 1.0, true), // price > 1.0 (true)
            negated: false,
        },
        chain: vec![
            ChainedTriggerWithNot {
                operator: ChainOperator::Or, // Same group as primary
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 0.5, true), // price > 0.5 (true)
                    negated: false,
                },
            },
            ChainedTriggerWithNot {
                operator: ChainOperator::And, // Start new group
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 2.0, true), // price > 2.0 (false)
                    negated: false,
                },
            },
        ],
        disabled: None,
    };

    let candle = create_test_candle(dec!(1.1000), 0);
    let result = engine.evaluate_condition(&condition, &candle);

    // Group 1: (1.1 > 1.0) OR (1.1 > 0.5) = true OR true = true
    // Group 2: (1.1 > 2.0) = false
    // Result: true AND false = false
    assert!(!result, "Condition should FAIL because Group 2 fails (groups are AND'd)");
}

#[test]
fn test_grouped_and_or_within_group() {
    // Test: A OR B OR C (all in one group)
    // No AND operators, so all triggers are OR'd in single group
    // Group 1: price > 2.0 OR price > 3.0 OR price > 1.0
    // Expected: passes because at least one trigger passes

    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy).unwrap();

    let condition = Condition {
        name: None,
        primary: TriggerWithNot {
            trigger: create_threshold_trigger(0.0, 2.0, true), // price > 2.0 (false)
            negated: false,
        },
        chain: vec![
            ChainedTriggerWithNot {
                operator: ChainOperator::Or, // Same group
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 3.0, true), // price > 3.0 (false)
                    negated: false,
                },
            },
            ChainedTriggerWithNot {
                operator: ChainOperator::Or, // Same group
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 1.0, true), // price > 1.0 (true)
                    negated: false,
                },
            },
        ],
        disabled: None,
    };

    let candle = create_test_candle(dec!(1.1000), 0);
    let result = engine.evaluate_condition(&condition, &candle);

    // Single group: (1.1 > 2.0) OR (1.1 > 3.0) OR (1.1 > 1.0) = false OR false OR true = true
    assert!(result, "Condition should pass because one trigger in OR group passes");
}

#[test]
fn test_grouped_and_all_or_fail() {
    // Test: A OR B OR C (all in one group, all fail)
    // Group 1: price > 2.0 OR price > 3.0 OR price > 4.0
    // Expected: fails because no trigger passes

    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy).unwrap();

    let condition = Condition {
        name: None,
        primary: TriggerWithNot {
            trigger: create_threshold_trigger(0.0, 2.0, true), // price > 2.0 (false)
            negated: false,
        },
        chain: vec![
            ChainedTriggerWithNot {
                operator: ChainOperator::Or,
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 3.0, true), // price > 3.0 (false)
                    negated: false,
                },
            },
            ChainedTriggerWithNot {
                operator: ChainOperator::Or,
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 4.0, true), // price > 4.0 (false)
                    negated: false,
                },
            },
        ],
        disabled: None,
    };

    let candle = create_test_candle(dec!(1.1000), 0);
    let result = engine.evaluate_condition(&condition, &candle);

    // Single group: (1.1 > 2.0) OR (1.1 > 3.0) OR (1.1 > 4.0) = false OR false OR false = false
    assert!(!result, "Condition should fail because all triggers in OR group fail");
}

#[test]
fn test_grouped_and_chain_multiple_groups() {
    // Test: A AND B AND C (three separate groups, each must pass)
    // Group 1: price > 1.0 (true)
    // Group 2: price > 0.9 (true)
    // Group 3: price > 0.5 (true)
    // Expected: passes because ALL groups pass

    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy).unwrap();

    let condition = Condition {
        name: None,
        primary: TriggerWithNot {
            trigger: create_threshold_trigger(0.0, 1.0, true), // price > 1.0 (true)
            negated: false,
        },
        chain: vec![
            ChainedTriggerWithNot {
                operator: ChainOperator::And, // New group
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 0.9, true), // price > 0.9 (true)
                    negated: false,
                },
            },
            ChainedTriggerWithNot {
                operator: ChainOperator::And, // New group
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 0.5, true), // price > 0.5 (true)
                    negated: false,
                },
            },
        ],
        disabled: None,
    };

    let candle = create_test_candle(dec!(1.1000), 0);
    let result = engine.evaluate_condition(&condition, &candle);

    // Group 1: (1.1 > 1.0) = true
    // Group 2: (1.1 > 0.9) = true
    // Group 3: (1.1 > 0.5) = true
    // Result: true AND true AND true = true
    assert!(result, "Condition should pass because all groups pass");
}

#[test]
fn test_grouped_and_chain_one_fails() {
    // Test: A AND B AND C (three separate groups, one fails)
    // Group 1: price > 1.0 (true)
    // Group 2: price > 2.0 (false) <-- fails
    // Group 3: price > 0.5 (true)
    // Expected: FAILS because one group fails

    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy).unwrap();

    let condition = Condition {
        name: None,
        primary: TriggerWithNot {
            trigger: create_threshold_trigger(0.0, 1.0, true), // price > 1.0 (true)
            negated: false,
        },
        chain: vec![
            ChainedTriggerWithNot {
                operator: ChainOperator::And, // New group
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 2.0, true), // price > 2.0 (false)
                    negated: false,
                },
            },
            ChainedTriggerWithNot {
                operator: ChainOperator::And, // New group
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 0.5, true), // price > 0.5 (true)
                    negated: false,
                },
            },
        ],
        disabled: None,
    };

    let candle = create_test_candle(dec!(1.1000), 0);
    let result = engine.evaluate_condition(&condition, &candle);

    // Group 1: (1.1 > 1.0) = true
    // Group 2: (1.1 > 2.0) = false
    // Group 3: (1.1 > 0.5) = true
    // Result: true AND false AND true = false
    assert!(!result, "Condition should FAIL because one group fails");
}

#[test]
fn test_grouped_and_mixed_complex() {
    // Test: A OR B AND C OR D AND E
    // This becomes: (A OR B) AND (C OR D) AND (E)
    // Group 1: price > 1.0 OR price > 2.0 = true OR false = true
    // Group 2: price > 0.9 OR price > 3.0 = true OR false = true
    // Group 3: price > 0.5 = true
    // Expected: passes because ALL groups pass

    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy).unwrap();

    let condition = Condition {
        name: None,
        primary: TriggerWithNot {
            trigger: create_threshold_trigger(0.0, 1.0, true), // price > 1.0 (true)
            negated: false,
        },
        chain: vec![
            ChainedTriggerWithNot {
                operator: ChainOperator::Or, // Same group as primary
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 2.0, true), // price > 2.0 (false)
                    negated: false,
                },
            },
            ChainedTriggerWithNot {
                operator: ChainOperator::And, // Start Group 2
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 0.9, true), // price > 0.9 (true)
                    negated: false,
                },
            },
            ChainedTriggerWithNot {
                operator: ChainOperator::Or, // Same group as Group 2
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 3.0, true), // price > 3.0 (false)
                    negated: false,
                },
            },
            ChainedTriggerWithNot {
                operator: ChainOperator::And, // Start Group 3
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 0.5, true), // price > 0.5 (true)
                    negated: false,
                },
            },
        ],
        disabled: None,
    };

    let candle = create_test_candle(dec!(1.1000), 0);
    let result = engine.evaluate_condition(&condition, &candle);

    // Group 1: (1.1 > 1.0) OR (1.1 > 2.0) = true OR false = true
    // Group 2: (1.1 > 0.9) OR (1.1 > 3.0) = true OR false = true
    // Group 3: (1.1 > 0.5) = true
    // Result: true AND true AND true = true
    assert!(result, "Condition should pass because all groups pass");
}

#[test]
fn test_negated_triggers_in_groups() {
    // Test: A OR NOT B AND C
    // Group 1: price > 1.0 OR NOT(price > 2.0) = true OR NOT(false) = true OR true = true
    // Group 2: price > 0.9 = true
    // Expected: passes because all groups pass

    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy).unwrap();

    let condition = Condition {
        name: None,
        primary: TriggerWithNot {
            trigger: create_threshold_trigger(0.0, 1.0, true), // price > 1.0 (true)
            negated: false,
        },
        chain: vec![
            ChainedTriggerWithNot {
                operator: ChainOperator::Or, // Same group
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 2.0, true), // price > 2.0 (false)
                    negated: true, // NOT applied - so NOT false = true
                },
            },
            ChainedTriggerWithNot {
                operator: ChainOperator::And, // New group
                trigger: TriggerWithNot {
                    trigger: create_threshold_trigger(0.0, 0.9, true), // price > 0.9 (true)
                    negated: false,
                },
            },
        ],
        disabled: None,
    };

    let candle = create_test_candle(dec!(1.1000), 0);
    let result = engine.evaluate_condition(&condition, &candle);

    // Group 1: (1.1 > 1.0) OR NOT(1.1 > 2.0) = true OR NOT(false) = true OR true = true
    // Group 2: (1.1 > 0.9) = true
    // Result: true AND true = true
    assert!(result, "Condition should pass with negated trigger in group");
}

// ============================================================================
// Session Filter Trigger Tests (TimeInRange + DayOfWeek)
// ============================================================================

/// Create a test candle at a specific date/time
fn create_candle_at(datetime_str: &str) -> Candle {
    let time = DateTime::parse_from_rfc3339(datetime_str)
        .unwrap()
        .with_timezone(&Utc);
    Candle {
        time,
        mid: Ohlc {
            open: dec!(1.1000),
            high: dec!(1.1010),
            low: dec!(1.0990),
            close: dec!(1.1005),
        },
        volume: 1000,
        complete: true,
    }
}

#[test]
fn test_time_in_range_within() {
    // Candle at 10:00 UTC should match 08:00-16:00 range
    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy).unwrap();
    let candle = create_candle_at("2024-03-15T10:00:00Z"); // Friday 10:00

    let trigger = Trigger::TimeInRange(shared::TimeInRangeTrigger {
        start_hour: 8,
        start_minute: 0,
        end_hour: 16,
        end_minute: 0,
    });

    assert!(engine.evaluate_trigger_v2(&trigger, &candle),
        "10:00 UTC should be within 08:00-16:00 range");
}

#[test]
fn test_time_in_range_outside() {
    // Candle at 18:00 UTC should NOT match 08:00-16:00 range
    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy).unwrap();
    let candle = create_candle_at("2024-03-15T18:00:00Z"); // Friday 18:00

    let trigger = Trigger::TimeInRange(shared::TimeInRangeTrigger {
        start_hour: 8,
        start_minute: 0,
        end_hour: 16,
        end_minute: 0,
    });

    assert!(!engine.evaluate_trigger_v2(&trigger, &candle),
        "18:00 UTC should NOT be within 08:00-16:00 range");
}

#[test]
fn test_time_in_range_midnight_wrap() {
    // Candle at 23:00 UTC should match 22:00-02:00 range (wraps midnight)
    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy).unwrap();
    let candle = create_candle_at("2024-03-15T23:00:00Z");

    let trigger = Trigger::TimeInRange(shared::TimeInRangeTrigger {
        start_hour: 22,
        start_minute: 0,
        end_hour: 2,
        end_minute: 0,
    });

    assert!(engine.evaluate_trigger_v2(&trigger, &candle),
        "23:00 UTC should be within 22:00-02:00 midnight-wrapping range");
}

#[test]
fn test_time_in_range_midnight_wrap_outside() {
    // Candle at 15:00 UTC should NOT match 22:00-02:00 range
    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy).unwrap();
    let candle = create_candle_at("2024-03-15T15:00:00Z");

    let trigger = Trigger::TimeInRange(shared::TimeInRangeTrigger {
        start_hour: 22,
        start_minute: 0,
        end_hour: 2,
        end_minute: 0,
    });

    assert!(!engine.evaluate_trigger_v2(&trigger, &candle),
        "15:00 UTC should NOT be within 22:00-02:00 midnight-wrapping range");
}

#[test]
fn test_day_of_week_include() {
    // Monday candle should match days=[1] (include mode)
    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy).unwrap();
    let candle = create_candle_at("2024-03-18T10:00:00Z"); // Monday

    let trigger = Trigger::DayOfWeek(shared::DayOfWeekTrigger {
        days: vec![1], // Monday
        exclude: false,
    });

    assert!(engine.evaluate_trigger_v2(&trigger, &candle),
        "Monday candle should match days=[1] in include mode");
}

#[test]
fn test_day_of_week_exclude() {
    // Friday candle should NOT match when days=[5] and exclude=true
    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy).unwrap();
    let candle = create_candle_at("2024-03-22T10:00:00Z"); // Friday

    let trigger = Trigger::DayOfWeek(shared::DayOfWeekTrigger {
        days: vec![5], // Friday
        exclude: true,
    });

    assert!(!engine.evaluate_trigger_v2(&trigger, &candle),
        "Friday candle should NOT pass when days=[5] and exclude=true");
}

#[test]
fn test_day_of_week_sunday() {
    // Sunday candle should match days=[0]
    let strategy = create_simple_strategy();
    let engine = RulesEngine::new(strategy).unwrap();
    let candle = create_candle_at("2024-03-17T10:00:00Z"); // Sunday

    let trigger = Trigger::DayOfWeek(shared::DayOfWeekTrigger {
        days: vec![0], // Sunday
        exclude: false,
    });

    assert!(engine.evaluate_trigger_v2(&trigger, &candle),
        "Sunday candle should match days=[0]");
}

// ============================================================================
// Stop Loss Validation Tests
// ============================================================================
//
// These tests verify that stop losses on the wrong side of entry are rejected.
// This prevents guaranteed-profit trades where the SL is immediately hit for a
// profit (e.g., swing low above entry for a long).
//
// Tests use Variable SL sources with Fixed values for deterministic behavior —
// the variable resolves to a known price level, guaranteeing wrong-side or
// correct-side SL without depending on indicator state.

/// Create a strategy with a Variable SL source that resolves to a fixed price.
/// This gives deterministic control over where the SL is placed.
fn create_strategy_with_fixed_sl(sl_price: f64) -> StrategyDefinition {
    let mut strategy = create_simple_strategy();
    strategy.variables.push(StrategyVariable {
        id: "fixed_sl".to_string(),
        name: "Fixed SL Level".to_string(),
        description: None,
        expression: VariableExpression::Value {
            source: Box::new(DataSource::Fixed(FixedSource {
                fixed: sl_price,
            })),
            operations: None,
        },
    });
    strategy.risk_settings.stop_loss_source = Some(StopLossSource::Variable {
        variable: "fixed_sl".to_string(),
        evaluation: StopLossEvaluationMode::AtOpen,
    });
    strategy
}

#[test]
fn test_sl_wrong_side_long_skips_trade() {
    // SL at 1.1100 is ABOVE entry at 1.1000 for a long → must return None
    let strategy = create_strategy_with_fixed_sl(1.1100);
    let engine = RulesEngine::new(strategy).unwrap();

    let candle = create_test_candle(dec!(1.1000), 0);
    let result = engine.calculate_sl_tp_for_signal(PositionDirection::Long, &candle);

    assert!(result.is_none(),
        "Long trade with SL above entry must be rejected");
}

#[test]
fn test_sl_wrong_side_short_skips_trade() {
    // SL at 1.0900 is BELOW entry at 1.1000 for a short → must return None
    let strategy = create_strategy_with_fixed_sl(1.0900);
    let mut engine = RulesEngine::new(strategy).unwrap();
    // Set short-specific SL to the same variable
    engine.strategy.risk_settings.stop_loss_source_short = Some(StopLossSource::Variable {
        variable: "fixed_sl".to_string(),
        evaluation: StopLossEvaluationMode::AtOpen,
    });

    let candle = create_test_candle(dec!(1.1000), 0);
    let result = engine.calculate_sl_tp_for_signal(PositionDirection::Short, &candle);

    assert!(result.is_none(),
        "Short trade with SL below entry must be rejected");
}

#[test]
fn test_sl_equal_to_entry_skips_trade() {
    // SL exactly at entry price → zero risk → must return None
    let strategy = create_strategy_with_fixed_sl(1.1000);
    let engine = RulesEngine::new(strategy).unwrap();

    let candle = create_test_candle(dec!(1.1000), 0);
    let result = engine.calculate_sl_tp_for_signal(PositionDirection::Long, &candle);

    assert!(result.is_none(),
        "Trade with SL == entry must be rejected (zero risk)");
}

#[test]
fn test_sl_correct_side_long_allows_trade() {
    // SL at 1.0900 is below entry at 1.1000 for a long → valid
    let strategy = create_strategy_with_fixed_sl(1.0900);
    let engine = RulesEngine::new(strategy).unwrap();

    let candle = create_test_candle(dec!(1.1000), 0);
    let result = engine.calculate_sl_tp_for_signal(PositionDirection::Long, &candle);

    assert!(result.is_some(), "Long with SL below entry should be allowed");
    let (sl, tp) = result.unwrap();
    assert!(sl < dec!(1.1000), "Long SL should be below entry");
    assert!(tp > dec!(1.1000), "Long TP should be above entry");
}

#[test]
fn test_sl_correct_side_short_allows_trade() {
    // SL at 1.1100 is above entry at 1.1000 for a short → valid
    let strategy = create_strategy_with_fixed_sl(1.1100);
    let mut engine = RulesEngine::new(strategy).unwrap();
    engine.strategy.risk_settings.stop_loss_source_short = Some(StopLossSource::Variable {
        variable: "fixed_sl".to_string(),
        evaluation: StopLossEvaluationMode::AtOpen,
    });

    let candle = create_test_candle(dec!(1.1000), 0);
    let result = engine.calculate_sl_tp_for_signal(PositionDirection::Short, &candle);

    assert!(result.is_some(), "Short with SL above entry should be allowed");
    let (sl, tp) = result.unwrap();
    assert!(sl > dec!(1.1000), "Short SL should be above entry");
    assert!(tp < dec!(1.1000), "Short TP should be below entry");
}

#[test]
fn test_fixed_pips_sl_always_correct_side() {
    // FixedPips SL is always placed on the correct side by construction
    let mut strategy = create_simple_strategy();
    strategy.risk_settings.stop_loss_source = Some(StopLossSource::FixedPips {
        pips: ParameterizedValue::Fixed(20.0),
    });
    let engine = RulesEngine::new(strategy).unwrap();

    let candle = create_test_candle(dec!(1.1000), 0);

    let long_result = engine.calculate_sl_tp_for_signal(PositionDirection::Long, &candle);
    assert!(long_result.is_some(), "FixedPips long should always produce valid SL");
    let (sl, _) = long_result.unwrap();
    assert!(sl < dec!(1.1000), "FixedPips long SL should be below entry");

    let short_result = engine.calculate_sl_tp_for_signal(PositionDirection::Short, &candle);
    assert!(short_result.is_some(), "FixedPips short should always produce valid SL");
    let (sl, _) = short_result.unwrap();
    assert!(sl > dec!(1.1000), "FixedPips short SL should be above entry");
}

#[test]
fn test_open_position_skips_when_sl_wrong_side() {
    // SL at 1.1100 is ABOVE entry at 1.1000 for a long → open_position should not create position
    let strategy = create_strategy_with_fixed_sl(1.1100);
    let mut engine = RulesEngine::new(strategy).unwrap();

    let candle = create_test_candle(dec!(1.1000), 0);
    engine.open_position(PositionDirection::Long, &candle);

    assert!(!engine.has_position(),
        "Position should NOT be opened when SL is on wrong side of entry");
}

#[test]
fn test_open_position_succeeds_when_sl_correct_side() {
    // SL at 1.0900 is below entry at 1.1000 for a long → should create position
    let strategy = create_strategy_with_fixed_sl(1.0900);
    let mut engine = RulesEngine::new(strategy).unwrap();

    let candle = create_test_candle(dec!(1.1000), 0);
    engine.open_position(PositionDirection::Long, &candle);

    assert!(engine.has_position(),
        "Position should be opened when SL is on correct side of entry");
}
