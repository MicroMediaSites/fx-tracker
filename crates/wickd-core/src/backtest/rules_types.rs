//! Rules Engine Types
//!
//! Type definitions for strategy rules, triggers, and conditions.
//! Most types are re-exported from the shared crate for consistency with the MCP server.

use rust_decimal::Decimal;

pub use shared::{
    ParameterDefinition, ParameterOption, ParameterReference, ParameterizedValue, ParameterType,
    default_lookback,
    IndicatorConfig, IndicatorType,
    CaptureMode, TrailConfig, IndicatorSource, PriceSource, FixedSource, ParameterSource,
    PriceType,
    DataSource,
    SRZone, SRZoneDistance, SRZoneSource, PivotSource, PatternSource, SRTarget,
    CandlestickPattern,
    MarketRegime,
    MathOperator, MathOperation, VariableExpression, StrategyVariable, VariableSource,
    DistanceUnit, DistanceConfig,
    CrossDirection, ComparisonOperator, TimeCondition,
    GivensTrigger, CrossTrigger, CompareTrigger, RiskRewardTrigger,
    PercentOfTpTrigger, TimeTrigger, ThresholdTrigger, TimeInRangeTrigger, DayOfWeekTrigger, Trigger,
    ChainOperator, ChainedTrigger, TriggerChain,
    TriggerWithNot, ChainedTriggerWithNot, Condition,
    EntryRule, ExitRule, EntryLogic, EntryLogicMode, AnyEntryRule, AnyExitRule,
    RuleDirection,
    StopLossSource, StopLossEvaluationMode, RiskSettings, RiskMethod,
    StrategyDefinition,
    PositionDirection,
    EntryOrderType, PendingOrderConfig,
};

// ============================================================================
// Runtime Types (not shared - used only during backtest execution)
// ============================================================================

/// Captured value for at_entry data sources
#[derive(Debug, Clone)]
pub struct CapturedValue {
    pub initial_value: Decimal,
    pub current_value: Decimal, // May differ from initial if trailing
    pub trail_config: Option<TrailConfig>,
}

#[derive(Debug, Clone)]
pub struct PositionState {
    pub direction: PositionDirection,
    pub entry_price: Decimal,
    pub stop_loss: Decimal,
    pub take_profit: Decimal,
    pub remaining_percent: f64,
    pub bars_since_entry: usize,
    pub triggered_partials: Vec<String>, // IDs of partial exits already hit
    /// Captured indicator/price values at entry (key: "indicator.output" or "price.close")
    pub captured_values: std::collections::HashMap<String, CapturedValue>,
    /// Entry time for time-based exit calculations
    pub entry_time: chrono::DateTime<chrono::Utc>,
}
