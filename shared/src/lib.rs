//! Shared types for CandleSight strategy definitions.
//!
//! This crate contains the type definitions used for strategy serialization/deserialization.
//! Both the Tauri backend and MCP server depend on this crate to ensure consistent parsing.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// Type-Safe Enums
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndicatorType {
    Sma,
    Ema,
    Rsi,
    Atr,
    Adx,
    Ichimoku,
    Chandelier,
    Bollinger,
    Macd,
    Stochastic,
    MaHistogram,
    MaBands,
    Dss,
    Adr,
    Daily,
    Swing,
    Mfi,
    Donchian,
    Vwap,
    ParabolicSar,
    SuperTrend,
}

impl IndicatorType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sma => "sma",
            Self::Ema => "ema",
            Self::Rsi => "rsi",
            Self::Atr => "atr",
            Self::Adx => "adx",
            Self::Ichimoku => "ichimoku",
            Self::Chandelier => "chandelier",
            Self::Bollinger => "bollinger",
            Self::Macd => "macd",
            Self::Stochastic => "stochastic",
            Self::MaHistogram => "ma_histogram",
            Self::MaBands => "ma_bands",
            Self::Dss => "dss",
            Self::Adr => "adr",
            Self::Daily => "daily",
            Self::Swing => "swing",
            Self::Mfi => "mfi",
            Self::Donchian => "donchian",
            Self::Vwap => "vwap",
            Self::ParabolicSar => "parabolic_sar",
            Self::SuperTrend => "super_trend",
        }
    }
}

impl std::fmt::Display for IndicatorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossDirection {
    Above,
    Below,
}

impl CrossDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Above => "above",
            Self::Below => "below",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonOperator {
    #[serde(rename = ">")]
    GreaterThan,
    #[serde(rename = ">=")]
    GreaterThanOrEqual,
    #[serde(rename = "<")]
    LessThan,
    #[serde(rename = "<=")]
    LessThanOrEqual,
    #[serde(rename = "==")]
    Equal,
    #[serde(rename = "is_within")]
    IsWithin,
}

impl ComparisonOperator {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GreaterThan => ">",
            Self::GreaterThanOrEqual => ">=",
            Self::LessThan => "<",
            Self::LessThanOrEqual => "<=",
            Self::Equal => "==",
            Self::IsWithin => "is_within",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceType {
    Open,
    High,
    Low,
    Close,
}

impl PriceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::High => "high",
            Self::Low => "low",
            Self::Close => "close",
        }
    }
}

impl std::fmt::Display for PriceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PivotLevel {
    Pp,
    R1,
    R2,
    R3,
    S1,
    S2,
    S3,
}

impl PivotLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pp => "pp",
            Self::R1 => "r1",
            Self::R2 => "r2",
            Self::R3 => "r3",
            Self::S1 => "s1",
            Self::S2 => "s2",
            Self::S3 => "s3",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PivotPeriod {
    #[default]
    Daily,
    Weekly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SRTarget {
    Upper,
    Lower,
    Midpoint,
}

impl SRTarget {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Upper => "upper",
            Self::Lower => "lower",
            Self::Midpoint => "midpoint",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistanceType {
    Pips,
    Atr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandlestickPattern {
    BullishEngulfing,
    BearishEngulfing,
    Hammer,
    InvertedHammer,
    Doji,
    PinBar,
    MorningStar,
    EveningStar,
    BullishHarami,
    BearishHarami,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeCondition {
    BarCount,
    Minutes,
    Hours,
}

impl TimeCondition {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BarCount => "bar_count",
            Self::Minutes => "minutes",
            Self::Hours => "hours",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterType {
    Number,
    Integer,
    Select,
    Boolean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskMethod {
    Percent,
    FixedAmount,
    FixedUnits,
}

impl RiskMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Percent => "percent",
            Self::FixedAmount => "fixed_amount",
            Self::FixedUnits => "fixed_units",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryLogicMode {
    All,
    Any,
    ScoreBased,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleDirection {
    Long,
    Short,
    Both,
}

impl RuleDirection {
    pub fn applies_to(&self, direction: PositionDirection) -> bool {
        match self {
            RuleDirection::Both => true,
            RuleDirection::Long => direction == PositionDirection::Long,
            RuleDirection::Short => direction == PositionDirection::Short,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SRCondition {
    Approaching,
    Testing,
    Broken,
    Bounced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PivotCondition {
    Approaching,
    Testing,
    Above,
    Below,
}

// ============================================================================
// Parameter Types
// ============================================================================

/// Definition of a strategy parameter for optimization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub param_type: ParameterType,
    pub default: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub options: Option<Vec<ParameterOption>>,
    pub group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterOption {
    pub value: f64,
    pub label: String,
}

/// Reference to a strategy parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterReference {
    #[serde(rename = "$param")]
    pub param_id: String,
}

/// A value that can be either a fixed number or a parameter reference.
/// Used for fields that users may want to optimize.
///
/// IMPORTANT: Order matters for serde untagged - try f64 first, then ParameterReference
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParameterizedValue {
    Fixed(f64),
    Reference(ParameterReference),
}

impl ParameterizedValue {
    /// Resolve the value to a concrete Decimal using the provided parameter map
    pub fn resolve(&self, params: &HashMap<String, f64>) -> Option<Decimal> {
        match self {
            ParameterizedValue::Fixed(v) => Decimal::try_from(*v).ok(),
            ParameterizedValue::Reference(r) => params
                .get(&r.param_id)
                .and_then(|v| Decimal::try_from(*v).ok()),
        }
    }

    /// Get the raw f64 value (for fixed) or None (for reference)
    pub fn as_fixed(&self) -> Option<f64> {
        match self {
            ParameterizedValue::Fixed(v) => Some(*v),
            ParameterizedValue::Reference(_) => None,
        }
    }
}

impl From<f64> for ParameterizedValue {
    fn from(v: f64) -> Self {
        ParameterizedValue::Fixed(v)
    }
}

impl From<i32> for ParameterizedValue {
    fn from(v: i32) -> Self {
        ParameterizedValue::Fixed(v as f64)
    }
}

pub fn default_lookback() -> ParameterizedValue {
    ParameterizedValue::Fixed(1.0)
}

// ============================================================================
// Indicator Config
// ============================================================================

/// Configuration for an indicator used in a strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorConfig {
    pub id: String,
    #[serde(rename = "type")]
    pub indicator_type: IndicatorType,
    pub params: HashMap<String, ParameterizedValue>,
    pub symbol: Option<String>,
    /// Timeframe for this indicator (e.g., "D", "H4", "W"). Defaults to strategy's primary timeframe.
    #[serde(default)]
    pub timeframe: Option<String>,
}

impl IndicatorConfig {
    /// Resolve all parameterized values to concrete f64 values
    pub fn resolve_params(&self, resolved_params: &HashMap<String, f64>) -> HashMap<String, f64> {
        self.params
            .iter()
            .filter_map(|(k, v)| {
                v.resolve(resolved_params)
                    .and_then(|d| d.to_string().parse::<f64>().ok())
                    .map(|val| (k.clone(), val))
            })
            .collect()
    }

    pub fn new_fixed(id: &str, indicator_type: IndicatorType, params: &[(&str, f64)]) -> Self {
        Self {
            id: id.to_string(),
            indicator_type,
            params: params
                .iter()
                .map(|(k, v)| (k.to_string(), ParameterizedValue::Fixed(*v)))
                .collect(),
            symbol: None,
            timeframe: None,
        }
    }
}

// ============================================================================
// Data Source Types
// ============================================================================

/// When to capture/evaluate a data source value
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    #[default]
    EachCandle, // Evaluate fresh on each candle (default, dynamic)
    AtEntry, // Capture value when trade opens, use as fixed reference
}

/// Trailing configuration for captured values
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrailConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Trail by this percentage in the favorable direction
    pub percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorSource {
    pub indicator: String,
    pub output: String,
    #[serde(default)]
    pub offset: usize,
    pub symbol: Option<String>,
    /// Timeframe for this data source (e.g., "D" for daily). Defaults to strategy's timeframe.
    pub timeframe: Option<String>,
    /// When to capture this value (default: each_candle)
    #[serde(default)]
    pub capture: CaptureMode,
    /// Trailing behavior (only applies when capture=at_entry)
    pub trail: Option<TrailConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceSource {
    pub source: String,
    pub value: PriceType,
    #[serde(default)]
    pub offset: usize,
    pub symbol: Option<String>,
    /// Timeframe for this data source (e.g., "D" for daily). Defaults to strategy's timeframe.
    pub timeframe: Option<String>,
    #[serde(default)]
    pub capture: CaptureMode,
    pub trail: Option<TrailConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixedSource {
    pub fixed: f64,
}

/// Parameter reference as a data source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterSource {
    #[serde(rename = "$param")]
    pub param_id: String,
}

// ============================================================================
// S/R Zone Types
// ============================================================================

/// S/R Zone data passed from frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SRZone {
    pub id: String,
    pub upper_price: Decimal,
    pub lower_price: Decimal,
}

/// Distance configuration for S/R zone triggers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SRZoneDistance {
    #[serde(rename = "type")]
    pub distance_type: DistanceType,
    pub value: ParameterizedValue,
    pub atr_period: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SRZoneSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub target: SRTarget,
    pub zone_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PivotSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub level: PivotLevel,
    pub period: PivotPeriod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatternSource {
    /// Must be "pattern" — used for DataSource variant matching
    pub source: String,
    pub pattern: CandlestickPattern,
    #[serde(default)]
    pub offset: usize,
}

// ============================================================================
// Market Regimes (Givens)
// ============================================================================

/// Predefined market regime conditions with hardcoded backend detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketRegime {
    // Trend/Volatility Regimes
    TrendingUp,
    TrendingDown,
    Ranging,
    HighVolatility,
    LowVolatility,
    // S/R Regimes
    SrTested,
    // Price Action - Gaps
    AtBullishGap,
    AtBearishGap,
    // Price Action - Supply/Demand Zones
    AtDemandZone,
    AtSupplyZone,
    // Price Action - Order Blocks
    AtBullishOb,
    AtBearishOb,
    // Price Action - Structure
    RetestingSupport,
    RetestingResistance,
    // Trading Sessions (UTC)
    LondonSession,      // 08:00-17:00 UTC
    UsSession,          // 13:00-22:00 UTC
    AsianSession,       // 00:00-09:00 UTC
    // Divergence (requires config in GivensTrigger)
    Divergence,
}

// ============================================================================
// Strategy Variables
// ============================================================================

/// Math operators for value expressions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MathOperator {
    #[serde(rename = "+")]
    Add,
    #[serde(rename = "-")]
    Subtract,
    #[serde(rename = "*")]
    Multiply,
    #[serde(rename = "/")]
    Divide,
    #[serde(rename = "**")]
    Pow,
    #[serde(rename = "%")]
    Mod,
}

/// A single math operation in a value expression chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MathOperation {
    pub operator: MathOperator,
    pub operand: Box<DataSource>,
}

/// Variable expression - the computation a variable performs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VariableExpression {
    /// Distance: left - right (or |left - right| if absolute)
    Distance {
        left: Box<DataSource>,
        right: Box<DataSource>,
        #[serde(default)]
        absolute: bool,
    },
    /// Ratio: numerator / denominator
    Ratio {
        numerator: Box<DataSource>,
        denominator: Box<DataSource>,
    },
    /// Change: source[bars] - source[0] (momentum measurement)
    Change { source: Box<DataSource>, bars: usize },
    /// Value: a data source with optional math operations (left-to-right evaluation)
    Value {
        source: Box<DataSource>,
        #[serde(default)]
        operations: Option<Vec<MathOperation>>,
    },
    /// Absolute value of a data source
    Abs { source: Box<DataSource> },
    /// Negate (flip sign) of a data source
    Negate { source: Box<DataSource> },
    /// Minimum of two data sources per bar
    Min { left: Box<DataSource>, right: Box<DataSource> },
    /// Maximum of two data sources per bar
    Max { left: Box<DataSource>, right: Box<DataSource> },
    /// Highest value of a source over N bars (rolling max)
    Highest { source: Box<DataSource>, period: ParameterizedValue },
    /// Lowest value of a source over N bars (rolling min)
    Lowest { source: Box<DataSource>, period: ParameterizedValue },
    /// Sum of a source over N bars (rolling sum)
    Sum { source: Box<DataSource>, period: ParameterizedValue },
    /// Average of a source over N bars (rolling mean)
    Average { source: Box<DataSource>, period: ParameterizedValue },
    /// Conditional: if condition_left op condition_right then true_value else false_value
    Conditional {
        condition_left: Box<DataSource>,
        operator: ComparisonOperator,
        condition_right: Box<DataSource>,
        true_value: Box<DataSource>,
        false_value: Box<DataSource>,
    },
}

/// A named variable definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyVariable {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub expression: VariableExpression,
}

/// Reference to a variable as a data source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableSource {
    #[serde(rename = "type")]
    pub source_type: String, // "variable"
    pub variable: String,    // ID of variable in strategy.variables
    #[serde(default)]
    pub offset: usize,
}

// ============================================================================
// V2 Data Sources
// ============================================================================

/// V2 Data source - supports indicator references, S/R zones, pivots, and variables
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DataSource {
    // Variable must come before other typed sources due to untagged enum ordering
    Variable(VariableSource),
    Indicator(IndicatorSource),
    Price(PriceSource),
    Fixed(FixedSource),
    Parameter(ParameterSource),
    SRZone(SRZoneSource),
    Pivot(PivotSource),
    Pattern(PatternSource),
    /// Bare numeric value — produced when resolveParams replaces a $param reference
    /// with its resolved value. Equivalent to Fixed but deserializes from a raw number.
    Numeric(f64),
}

// ============================================================================
// Distance Config (for "is_within" operator)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistanceUnit {
    Pips,
    Atr,
    Percent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistanceConfig {
    pub value: ParameterizedValue,
    pub unit: DistanceUnit,
    pub atr_period: Option<u32>,
}

// ============================================================================
// Triggers
// ============================================================================

/// Givens trigger - market regime condition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GivensTrigger {
    pub regime: MarketRegime,
    // Divergence config (only used when regime = Divergence)
    /// Type of divergence to detect
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divergence_type: Option<DivergenceType>,
    /// ID of the indicator to compare against price
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divergence_indicator: Option<String>,
    /// Output of the indicator (e.g., "value", "rsi", "macd")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divergence_output: Option<String>,
    /// Number of bars to look back for swing points (default: 50)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divergence_lookback: Option<u32>,
    /// Number of bars on each side to confirm a swing point (default: 5)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divergence_swing_strength: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossTrigger {
    pub left: DataSource,
    pub right: DataSource,
    pub direction: CrossDirection,
    #[serde(default = "default_lookback")]
    pub lookback: ParameterizedValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareTrigger {
    pub left: DataSource,
    pub operator: ComparisonOperator,
    pub right: DataSource,
    pub distance: Option<DistanceConfig>,
    #[serde(default = "default_lookback")]
    pub lookback: ParameterizedValue,
}

/// V2 Risk/Reward trigger
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskRewardTrigger {
    pub ratio: ParameterizedValue,
}

/// V2 Percent of TP trigger
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PercentOfTpTrigger {
    pub percent: ParameterizedValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeTrigger {
    pub condition: TimeCondition,
    pub value: ParameterizedValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdTrigger {
    pub source: DataSource,
    pub operator: ComparisonOperator,
    pub value: ParameterizedValue,
    #[serde(default = "default_lookback")]
    pub lookback: ParameterizedValue,
}

/// Time-in-range trigger for session filtering
/// Checks if the current candle's time falls within a specified time range (UTC)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeInRangeTrigger {
    /// Start hour (0-23, UTC)
    pub start_hour: u8,
    /// Start minute (0-59)
    #[serde(default)]
    pub start_minute: u8,
    /// End hour (0-23, UTC). If end time < start time, range wraps around midnight.
    pub end_hour: u8,
    /// End minute (0-59)
    #[serde(default)]
    pub end_minute: u8,
}

/// Day-of-week trigger for session filtering
/// Filters trading to specific days of the week
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DayOfWeekTrigger {
    /// Days of week: 0=Sun, 1=Mon, 2=Tue, 3=Wed, 4=Thu, 5=Fri, 6=Sat
    pub days: Vec<u8>,
    /// When true, the listed days are excluded (don't trade on these days)
    #[serde(default)]
    pub exclude: bool,
}

/// Divergence types for price vs indicator comparison
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceType {
    /// Price makes lower low, indicator makes higher low - signals reversal up
    Bullish,
    /// Price makes higher high, indicator makes lower high - signals reversal down
    Bearish,
    /// Price makes higher low, indicator makes lower low - signals continuation up
    HiddenBullish,
    /// Price makes lower high, indicator makes higher high - signals continuation down
    HiddenBearish,
}

/// Divergence trigger - detects when price and indicator swing points diverge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DivergenceTrigger {
    pub divergence_type: DivergenceType,
    /// ID of the indicator to compare against price
    pub indicator: String,
    /// Output of the indicator (e.g., "value", "rsi", "macd")
    pub output: String,
    /// Number of bars to look back for swing points (default: 50)
    #[serde(default = "default_divergence_lookback")]
    pub lookback: u32,
    /// Number of bars on each side to confirm a swing point (default: 5)
    #[serde(default = "default_swing_strength")]
    pub swing_strength: u32,
}

fn default_divergence_lookback() -> u32 {
    50
}

fn default_swing_strength() -> u32 {
    5
}

/// V2 Trigger types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Trigger {
    #[serde(rename = "givens")]
    Givens(GivensTrigger),
    #[serde(rename = "cross")]
    Cross(CrossTrigger),
    #[serde(rename = "compare")]
    Compare(CompareTrigger),
    #[serde(rename = "risk_reward_reached")]
    RiskReward(RiskRewardTrigger),
    #[serde(rename = "percent_of_tp_reached")]
    PercentOfTp(PercentOfTpTrigger),
    #[serde(rename = "time")]
    Time(TimeTrigger),
    #[serde(rename = "threshold")]
    Threshold(ThresholdTrigger),
    #[serde(rename = "time_in_range")]
    TimeInRange(TimeInRangeTrigger),
    #[serde(rename = "day_of_week")]
    DayOfWeek(DayOfWeekTrigger),
}

// ============================================================================
// Trigger Chains and Conditions
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainOperator {
    And,
    Or,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainedTrigger {
    pub operator: ChainOperator,
    pub trigger: Trigger,
}

/// A trigger chain consists of a primary trigger and optional chained triggers
/// DEPRECATED: Use Condition instead. Kept for backward compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerChain {
    pub primary: Trigger,
    #[serde(default)]
    pub chain: Vec<ChainedTrigger>,
}

/// A trigger wrapped with a negated flag for NOT logic
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerWithNot {
    pub trigger: Trigger,
    #[serde(default)]
    pub negated: bool,
}

/// A chained trigger with operator and NOT support
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainedTriggerWithNot {
    pub operator: ChainOperator,
    pub trigger: TriggerWithNot,
}

/// A condition consists of a primary trigger and optional chained triggers.
/// Multiple conditions in a rule are AND'd together.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    /// Optional name for the condition (for display purposes only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub primary: TriggerWithNot,
    #[serde(default)]
    pub chain: Vec<ChainedTriggerWithNot>,
    /// Skip this condition during evaluation. Can be parameterized for optimization sweeps.
    /// 0 = enabled (default), non-zero = disabled/skipped
    #[serde(default)]
    pub disabled: Option<ParameterizedValue>,
}

// ============================================================================
// Entry/Exit Rules
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryRule {
    pub id: String,
    pub name: Option<String>,
    pub direction: RuleDirection,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    #[serde(default)]
    pub trigger_chain: Option<TriggerChain>,
    /// Optional pending order configuration. When set, matching signals create
    /// pending orders (stop/limit) instead of immediate market entries.
    #[serde(default)]
    pub pending_order: Option<PendingOrderConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitRule {
    pub id: String,
    pub name: Option<String>,
    pub direction: RuleDirection,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    #[serde(default)]
    pub trigger_chain: Option<TriggerChain>,
    pub close_percent: ParameterizedValue,
    #[serde(default)]
    pub priority: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryLogic {
    #[serde(default = "default_entry_logic_mode")]
    pub mode: EntryLogicMode,
    pub min_score: Option<f64>,
}

fn default_entry_logic_mode() -> EntryLogicMode {
    EntryLogicMode::All
}

impl Default for EntryLogic {
    fn default() -> Self {
        Self {
            mode: EntryLogicMode::All,
            min_score: None,
        }
    }
}

/// Entry rule type alias
pub type AnyEntryRule = EntryRule;

/// Exit rule type alias
pub type AnyExitRule = ExitRule;

// ============================================================================
// Risk Settings
// ============================================================================

/// Evaluation mode for variable-based stop loss
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StopLossEvaluationMode {
    /// Calculate once at trade entry, use fixed value
    #[default]
    AtOpen,
    /// Re-evaluate each candle, update stop (favorable direction only)
    Trailing,
}

/// Stop loss source for custom R:R and position sizing calculations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StopLossSource {
    /// Use an indicator value as the stop level
    Indicator {
        indicator: String,
        output: String,
        #[serde(default)]
        capture: Option<String>,
    },
    /// Set stop at a fixed pip distance from entry
    FixedPips { pips: ParameterizedValue },
    /// Set stop at a percentage of account value from entry
    Percent { percent: ParameterizedValue },
    /// Use a computed variable as the stop level
    Variable {
        variable: String,
        #[serde(default)]
        evaluation: StopLossEvaluationMode,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskSettings {
    pub risk_method: RiskMethod,
    pub risk_value: ParameterizedValue,
    pub rr_ratio: ParameterizedValue,
    pub spread_buffer_pips: ParameterizedValue,
    #[serde(default)]
    pub stop_loss_source: Option<StopLossSource>,
    // Short trade overrides (when different from long)
    #[serde(default)]
    pub risk_method_short: Option<RiskMethod>,
    #[serde(default)]
    pub risk_value_short: Option<ParameterizedValue>,
    #[serde(default)]
    pub rr_ratio_short: Option<ParameterizedValue>,
    #[serde(default)]
    pub spread_buffer_pips_short: Option<ParameterizedValue>,
    #[serde(default)]
    pub stop_loss_source_short: Option<StopLossSource>,
}

// ============================================================================
// Strategy Definition
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyDefinition {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub parameters: Vec<ParameterDefinition>,
    #[serde(default)]
    pub indicators: Vec<IndicatorConfig>,
    #[serde(default)]
    pub variables: Vec<StrategyVariable>,
    pub entry_rules: Vec<AnyEntryRule>,
    #[serde(default)]
    pub entry_logic: EntryLogic,
    pub exit_rules: Vec<AnyExitRule>,
    pub risk_settings: RiskSettings,
    pub version: i32,
    pub is_active: bool,
    #[serde(default = "default_schema_version")]
    pub schema_version: i32,
    #[serde(default = "default_strategy_type")]
    pub strategy_type: String,           // "rules" | "scripted"
    pub script_content: Option<String>,  // Rhai source for scripted strategies
}

fn default_schema_version() -> i32 {
    2
}

fn default_strategy_type() -> String {
    "rules".to_string()
}

// ============================================================================
// Position Direction (used by both backtest and signals)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PositionDirection {
    Long,
    Short,
}

impl PositionDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            PositionDirection::Long => "long",
            PositionDirection::Short => "short",
        }
    }
}

// ============================================================================
// Entry Order Types (for pending/limit/stop orders)
// ============================================================================

/// Type of entry order. Market orders execute immediately at next candle open.
/// Pending orders (stop/limit) wait until price reaches a specified level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EntryOrderType {
    /// Execute immediately at next candle open (current/default behavior)
    #[default]
    Market,
    /// Buy when price rises to level (breakout long)
    BuyStop,
    /// Sell when price falls to level (breakout short)
    SellStop,
    /// Buy when price falls to level (pullback long)
    BuyLimit,
    /// Sell when price rises to level (pullback short)
    SellLimit,
}

/// Configuration for a pending entry order (stop or limit).
/// Attached to an EntryRule to specify that matched signals should create
/// pending orders instead of immediate market entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingOrderConfig {
    /// The type of pending order
    pub order_type: EntryOrderType,
    /// The trigger price level (resolved from a DataSource)
    pub price: DataSource,
    /// Cancel after N bars if not filled (None = no expiry)
    #[serde(default)]
    pub expiry_bars: Option<u32>,
}

// ============================================================================
// Validation Helper
// ============================================================================

impl StrategyDefinition {
    /// Validate a strategy JSON string by attempting to parse it.
    /// Returns Ok(StrategyDefinition) if valid, Err with parse error if invalid.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Serialize the strategy to JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Validate that all indicator references in conditions exist in the indicators array.
    /// Returns Ok(()) if valid, Err with list of missing indicators if invalid.
    pub fn validate_indicator_references(&self) -> Result<(), Vec<String>> {
        use std::collections::HashSet;

        // Collect defined indicator IDs
        let defined: HashSet<&str> = self.indicators.iter().map(|i| i.id.as_str()).collect();

        // Collect all referenced indicator IDs
        let mut referenced: HashSet<String> = HashSet::new();

        // Helper to extract indicator refs from DataSource
        fn collect_from_data_source(ds: &DataSource, refs: &mut HashSet<String>) {
            if let DataSource::Indicator(ind) = ds {
                refs.insert(ind.indicator.clone());
            }
        }

        // Helper to extract indicator refs from a Trigger
        fn collect_from_trigger(trigger: &Trigger, refs: &mut HashSet<String>) {
            match trigger {
                Trigger::Cross(t) => {
                    collect_from_data_source(&t.left, refs);
                    collect_from_data_source(&t.right, refs);
                }
                Trigger::Compare(t) => {
                    collect_from_data_source(&t.left, refs);
                    collect_from_data_source(&t.right, refs);
                }
                Trigger::Threshold(t) => {
                    collect_from_data_source(&t.source, refs);
                }
                Trigger::Givens(t) => {
                    // Givens can have divergence indicator reference
                    if let Some(ref ind) = t.divergence_indicator {
                        refs.insert(ind.clone());
                    }
                }
                _ => {}
            }
        }

        // Collect from entry rules
        for rule in &self.entry_rules {
            for condition in &rule.conditions {
                collect_from_trigger(&condition.primary.trigger, &mut referenced);
                for chained in &condition.chain {
                    collect_from_trigger(&chained.trigger.trigger, &mut referenced);
                }
            }
            // Legacy trigger_chain support
            if let Some(ref tc) = rule.trigger_chain {
                collect_from_trigger(&tc.primary, &mut referenced);
                for chained in &tc.chain {
                    collect_from_trigger(&chained.trigger, &mut referenced);
                }
            }
        }

        // Collect from exit rules
        for rule in &self.exit_rules {
            for condition in &rule.conditions {
                collect_from_trigger(&condition.primary.trigger, &mut referenced);
                for chained in &condition.chain {
                    collect_from_trigger(&chained.trigger.trigger, &mut referenced);
                }
            }
            // Legacy trigger_chain support
            if let Some(ref tc) = rule.trigger_chain {
                collect_from_trigger(&tc.primary, &mut referenced);
                for chained in &tc.chain {
                    collect_from_trigger(&chained.trigger, &mut referenced);
                }
            }
        }

        // Also check variables for indicator references
        for var in &self.variables {
            fn collect_from_expr(expr: &VariableExpression, refs: &mut HashSet<String>) {
                match expr {
                    VariableExpression::Distance { left, right, .. } => {
                        collect_from_data_source(left, refs);
                        collect_from_data_source(right, refs);
                    }
                    VariableExpression::Ratio { numerator, denominator } => {
                        collect_from_data_source(numerator, refs);
                        collect_from_data_source(denominator, refs);
                    }
                    VariableExpression::Change { source, .. } => {
                        collect_from_data_source(source, refs);
                    }
                    VariableExpression::Value { source, operations } => {
                        collect_from_data_source(source, refs);
                        if let Some(ops) = operations {
                            for op in ops {
                                collect_from_data_source(&op.operand, refs);
                            }
                        }
                    }
                    VariableExpression::Abs { source } | VariableExpression::Negate { source } => {
                        collect_from_data_source(source, refs);
                    }
                    VariableExpression::Min { left, right } | VariableExpression::Max { left, right } => {
                        collect_from_data_source(left, refs);
                        collect_from_data_source(right, refs);
                    }
                    VariableExpression::Highest { source, .. }
                    | VariableExpression::Lowest { source, .. }
                    | VariableExpression::Sum { source, .. }
                    | VariableExpression::Average { source, .. } => {
                        collect_from_data_source(source, refs);
                    }
                    VariableExpression::Conditional { condition_left, condition_right, true_value, false_value, .. } => {
                        collect_from_data_source(condition_left, refs);
                        collect_from_data_source(condition_right, refs);
                        collect_from_data_source(true_value, refs);
                        collect_from_data_source(false_value, refs);
                    }
                }
            }
            collect_from_expr(&var.expression, &mut referenced);
        }

        // Find missing indicators
        let missing: Vec<String> = referenced
            .into_iter()
            .filter(|r| !defined.contains(r.as_str()))
            .collect();

        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }
}

// ============================================================================
// Forex Utilities
// ============================================================================

/// Calculate pip value from OANDA's pip_location metadata.
///
/// pip_location indicates the decimal position of the pip:
/// - `-4` → 10^-4 = 0.0001 (standard forex: EUR/USD, GBP/USD, etc.)
/// - `-2` → 10^-2 = 0.01 (JPY pairs, Gold XAU)
/// - `-3` → 10^-3 = 0.001 (Silver XAG)
/// - `-1` → 10^-1 = 0.1 (some indices)
///
/// This is the preferred method when OANDA instrument data is available.
pub fn pip_value_from_location(pip_location: i32) -> Decimal {
    use rust_decimal_macros::dec;

    // pip_location is typically negative, indicating 10^(pip_location)
    // e.g., -4 means 10^-4 = 0.0001
    match pip_location {
        -4 => dec!(0.0001),
        -3 => dec!(0.001),
        -2 => dec!(0.01),
        -1 => dec!(0.1),
        0 => dec!(1),
        1 => dec!(10),
        _ => {
            // For any other value, calculate dynamically
            if pip_location < 0 {
                let divisor = 10i64.pow(pip_location.unsigned_abs());
                Decimal::ONE / Decimal::from(divisor)
            } else {
                Decimal::from(10i64.pow(pip_location as u32))
            }
        }
    }
}

/// Returns the pip value for a given forex instrument by name.
/// This is a fallback when OANDA metadata is not available.
///
/// Known instrument types:
/// - JPY pairs (USDJPY, EURJPY, etc.): 0.01
/// - Gold (XAU*): 0.01
/// - Silver (XAG*): 0.001
/// - Standard forex: 0.0001
pub fn get_pip_value(instrument: &str) -> Decimal {
    use rust_decimal_macros::dec;

    let upper = instrument.to_uppercase();

    // JPY pairs
    if upper.ends_with("JPY") {
        return dec!(0.01);
    }

    // Gold (XAU/USD, XAU/EUR, etc.)
    if upper.starts_with("XAU") {
        return dec!(0.01);
    }

    // Silver (XAG/USD, XAG/EUR, etc.)
    if upper.starts_with("XAG") {
        return dec!(0.001);
    }

    // Default for standard forex
    dec!(0.0001)
}

/// Converts pips to price distance using pip_location.
pub fn pips_to_price_with_location(pips: Decimal, pip_location: i32) -> Decimal {
    pips * pip_value_from_location(pip_location)
}

/// Converts pips to price distance for a given instrument (by name fallback).
/// For JPY pairs: 20 pips = 0.20
/// For other pairs: 20 pips = 0.0020
pub fn pips_to_price(pips: Decimal, instrument: &str) -> Decimal {
    pips * get_pip_value(instrument)
}

/// Converts price distance to pips using pip_location.
pub fn price_to_pips_with_location(price_distance: Decimal, pip_location: i32) -> Decimal {
    price_distance / pip_value_from_location(pip_location)
}

/// Converts price distance to pips for a given instrument (by name fallback).
/// For JPY pairs: 0.20 = 20 pips
/// For other pairs: 0.0020 = 20 pips
pub fn price_to_pips(price_distance: Decimal, instrument: &str) -> Decimal {
    price_distance / get_pip_value(instrument)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ema_crossover_strategy() {
        let json = r#"{
            "id": "test-id",
            "user_id": "test-user",
            "name": "RohaGod Valid Test",
            "description": "EMA crossover test strategy",
            "indicators": [
                {"id": "fast_ema", "type": "ema", "params": {"period": 9}},
                {"id": "slow_ema", "type": "ema", "params": {"period": 21}}
            ],
            "entry_rules": [
                {
                    "id": "rule_1",
                    "name": "EMA Cross",
                    "direction": "long",
                    "conditions": [
                        {
                            "primary": {
                                "trigger": {
                                    "type": "cross",
                                    "left": {"indicator": "fast_ema", "output": "value"},
                                    "right": {"indicator": "slow_ema", "output": "value"},
                                    "direction": "above"
                                },
                                "negated": false
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
                "spread_buffer_pips": 1
            },
            "version": 1,
            "is_active": true
        }"#;

        match StrategyDefinition::from_json(json) {
            Ok(s) => {
                assert_eq!(s.name, "RohaGod Valid Test");
                assert_eq!(s.indicators.len(), 2);
                assert_eq!(s.indicators[0].indicator_type, IndicatorType::Ema);
                assert_eq!(s.entry_rules.len(), 1);
                assert_eq!(s.entry_rules[0].direction, RuleDirection::Long);
                assert_eq!(s.risk_settings.risk_method, RiskMethod::Percent);
            }
            Err(e) => panic!("Parse error: {}", e),
        }
    }

    #[test]
    fn test_reject_invalid_indicator_type() {
        let json = r#"{
            "id": "test-id",
            "user_id": "test-user",
            "name": "Invalid Test",
            "description": "Test with invalid indicator type",
            "indicators": [
                {"id": "bad", "type": "not_a_real_indicator", "params": {"period": 9}}
            ],
            "entry_rules": [],
            "exit_rules": [],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": 2.0,
                "spread_buffer_pips": 1
            },
            "version": 1,
            "is_active": true
        }"#;

        assert!(StrategyDefinition::from_json(json).is_err());
    }

    #[test]
    fn test_reject_invalid_direction() {
        let json = r#"{
            "id": "test-id",
            "user_id": "test-user",
            "name": "Invalid Test",
            "description": "Test with invalid direction",
            "indicators": [],
            "entry_rules": [
                {
                    "id": "rule_1",
                    "direction": "sideways",
                    "conditions": []
                }
            ],
            "exit_rules": [],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": 2.0,
                "spread_buffer_pips": 1
            },
            "version": 1,
            "is_active": true
        }"#;

        assert!(StrategyDefinition::from_json(json).is_err());
    }

    #[test]
    fn test_time_trigger_with_param() {
        // Test that $param works in time trigger value
        let json = r#"{
            "id": "test-id",
            "user_id": "test-user",
            "name": "Time Trigger Param Test",
            "description": "Test $param in time trigger",
            "parameters": [
                {"id": "max_bars", "name": "Max Bars", "type": "integer", "default": 10}
            ],
            "indicators": [],
            "entry_rules": [],
            "exit_rules": [
                {
                    "id": "time_exit",
                    "name": "Time-based exit",
                    "direction": "both",
                    "close_percent": 100,
                    "priority": 1,
                    "conditions": [{
                        "primary": {
                            "trigger": {
                                "type": "time",
                                "condition": "bar_count",
                                "value": {"$param": "max_bars"}
                            },
                            "negated": false
                        },
                        "chain": []
                    }]
                }
            ],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": 2.0,
                "spread_buffer_pips": 1
            },
            "version": 1,
            "is_active": true
        }"#;

        match StrategyDefinition::from_json(json) {
            Ok(s) => {
                assert_eq!(s.exit_rules.len(), 1);
            }
            Err(e) => panic!("Time trigger with $param should parse: {}", e),
        }
    }

    #[test]
    fn test_param_in_trigger_right_value() {
        // Test that $param works directly (not wrapped in fixed)
        let json = r#"{
            "id": "test-id",
            "user_id": "test-user",
            "name": "Param Right Value Test",
            "description": "Test $param as trigger right value",
            "parameters": [
                {"id": "threshold", "name": "Threshold", "type": "number", "default": 70}
            ],
            "indicators": [
                {"id": "rsi", "type": "rsi", "params": {"period": 14}}
            ],
            "entry_rules": [
                {
                    "id": "rsi_entry",
                    "direction": "long",
                    "conditions": [{
                        "primary": {
                            "trigger": {
                                "type": "compare",
                                "left": {"indicator": "rsi", "output": "value"},
                                "operator": "<",
                                "right": {"$param": "threshold"}
                            },
                            "negated": false
                        },
                        "chain": []
                    }]
                }
            ],
            "exit_rules": [],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": 2.0,
                "spread_buffer_pips": 1
            },
            "version": 1,
            "is_active": true
        }"#;

        match StrategyDefinition::from_json(json) {
            Ok(s) => {
                assert_eq!(s.entry_rules.len(), 1);
            }
            Err(e) => panic!("$param as trigger right value should parse: {}", e),
        }
    }

    #[test]
    fn test_reject_param_wrapped_in_fixed() {
        // Test that {"fixed": {"$param": ...}} is REJECTED
        let json = r#"{
            "id": "test-id",
            "user_id": "test-user",
            "name": "Wrong Param Syntax Test",
            "description": "Test that fixed cannot wrap $param",
            "parameters": [
                {"id": "threshold", "name": "Threshold", "type": "number", "default": 70}
            ],
            "indicators": [
                {"id": "rsi", "type": "rsi", "params": {"period": 14}}
            ],
            "entry_rules": [
                {
                    "id": "rsi_entry",
                    "direction": "long",
                    "conditions": [{
                        "primary": {
                            "trigger": {
                                "type": "compare",
                                "left": {"indicator": "rsi", "output": "value"},
                                "operator": "<",
                                "right": {"fixed": {"$param": "threshold"}}
                            },
                            "negated": false
                        },
                        "chain": []
                    }]
                }
            ],
            "exit_rules": [],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": 2.0,
                "spread_buffer_pips": 1
            },
            "version": 1,
            "is_active": true
        }"#;

        // This SHOULD fail because fixed expects f64, not an object
        assert!(StrategyDefinition::from_json(json).is_err(),
            "fixed wrapper around $param should be rejected");
    }

    #[test]
    fn test_validate_indicator_references_missing() {
        // Strategy that references rsi and mfi but doesn't define them
        let json = r#"{
            "id": "test-id",
            "user_id": "test-user",
            "name": "Missing Indicators Test",
            "description": "References indicators that are not defined",
            "indicators": [
                {"id": "ema_7", "type": "ema", "params": {"period": 7}}
            ],
            "entry_rules": [
                {
                    "id": "rule_1",
                    "direction": "long",
                    "conditions": [
                        {
                            "primary": {
                                "trigger": {
                                    "type": "threshold",
                                    "source": {"indicator": "rsi", "output": "value"},
                                    "operator": ">",
                                    "value": 50
                                },
                                "negated": false
                            },
                            "chain": []
                        },
                        {
                            "primary": {
                                "trigger": {
                                    "type": "threshold",
                                    "source": {"indicator": "mfi", "output": "value"},
                                    "operator": ">",
                                    "value": 50
                                },
                                "negated": false
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
                "spread_buffer_pips": 1
            },
            "version": 1,
            "is_active": true
        }"#;

        let strategy = StrategyDefinition::from_json(json).expect("JSON should parse");
        let result = strategy.validate_indicator_references();

        assert!(result.is_err(), "Should detect missing indicators");
        let missing = result.unwrap_err();
        assert!(missing.contains(&"rsi".to_string()), "Should report rsi as missing");
        assert!(missing.contains(&"mfi".to_string()), "Should report mfi as missing");
        assert_eq!(missing.len(), 2, "Should report exactly 2 missing indicators");
    }

    #[test]
    fn test_validate_indicator_references_valid() {
        // Strategy that properly defines all referenced indicators
        let json = r#"{
            "id": "test-id",
            "user_id": "test-user",
            "name": "Valid Indicators Test",
            "description": "All referenced indicators are defined",
            "indicators": [
                {"id": "ema_7", "type": "ema", "params": {"period": 7}},
                {"id": "rsi", "type": "rsi", "params": {"period": 14}}
            ],
            "entry_rules": [
                {
                    "id": "rule_1",
                    "direction": "long",
                    "conditions": [
                        {
                            "primary": {
                                "trigger": {
                                    "type": "threshold",
                                    "source": {"indicator": "rsi", "output": "value"},
                                    "operator": ">",
                                    "value": 50
                                },
                                "negated": false
                            },
                            "chain": []
                        },
                        {
                            "primary": {
                                "trigger": {
                                    "type": "compare",
                                    "left": {"source": "price", "value": "close"},
                                    "operator": ">",
                                    "right": {"indicator": "ema_7", "output": "value"}
                                },
                                "negated": false
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
                "spread_buffer_pips": 1
            },
            "version": 1,
            "is_active": true
        }"#;

        let strategy = StrategyDefinition::from_json(json).expect("JSON should parse");
        let result = strategy.validate_indicator_references();

        assert!(result.is_ok(), "Should pass when all indicators are defined");
    }

    #[test]
    fn test_ai_conversion_rsi_strategy_parses() {
        // This is the exact JSON the conversion prompt should produce for an RSI Pine Script.
        // If this test fails, the conversion prompt has a schema mismatch.
        let json = r#"{
            "id": "import-placeholder",
            "user_id": "import-placeholder",
            "schema_version": 2,
            "name": "RSI Strategy",
            "description": "RSI-based entry/exit strategy converted from Pine Script",
            "indicators": [
                {"id": "rsi_14", "type": "rsi", "params": {"period": 14}}
            ],
            "parameters": [],
            "variables": [],
            "entry_rules": [
                {
                    "id": "entry_long_1",
                    "direction": "long",
                    "conditions": [
                        {
                            "primary": {
                                "trigger": {
                                    "type": "threshold",
                                    "source": {"indicator": "rsi_14", "output": "value"},
                                    "operator": "<",
                                    "value": 30,
                                    "lookback": 1
                                },
                                "negated": false
                            },
                            "chain": []
                        }
                    ]
                }
            ],
            "exit_rules": [
                {
                    "id": "exit_long_1",
                    "direction": "long",
                    "close_percent": 100,
                    "conditions": [
                        {
                            "primary": {
                                "trigger": {
                                    "type": "threshold",
                                    "source": {"indicator": "rsi_14", "output": "value"},
                                    "operator": ">",
                                    "value": 70,
                                    "lookback": 1
                                },
                                "negated": false
                            },
                            "chain": []
                        }
                    ]
                }
            ],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": 2.0,
                "spread_buffer_pips": 1
            },
            "version": 1,
            "is_active": true
        }"#;

        match StrategyDefinition::from_json(json) {
            Ok(s) => {
                assert_eq!(s.name, "RSI Strategy");
                assert_eq!(s.indicators.len(), 1);
                assert_eq!(s.indicators[0].indicator_type, IndicatorType::Rsi);
                assert_eq!(s.entry_rules.len(), 1);
                assert_eq!(s.entry_rules[0].direction, RuleDirection::Long);
                assert_eq!(s.exit_rules.len(), 1);
                assert_eq!(s.exit_rules[0].direction, RuleDirection::Long);
                assert_eq!(s.risk_settings.risk_method, RiskMethod::Percent);
            }
            Err(e) => panic!("AI conversion JSON should parse but failed: {}", e),
        }
    }

    #[test]
    fn test_ai_conversion_cross_trigger_parses() {
        // Test cross trigger with fixed value (RSI crossing above 30)
        let json = r#"{
            "id": "import-placeholder",
            "user_id": "import-placeholder",
            "schema_version": 2,
            "name": "Cross Test",
            "description": "Tests cross trigger with fixed data source",
            "indicators": [
                {"id": "rsi_14", "type": "rsi", "params": {"period": 14}},
                {"id": "ema_20", "type": "ema", "params": {"period": 20}},
                {"id": "ema_50", "type": "ema", "params": {"period": 50}}
            ],
            "parameters": [],
            "variables": [],
            "entry_rules": [
                {
                    "id": "entry_long_1",
                    "direction": "long",
                    "conditions": [
                        {
                            "primary": {
                                "trigger": {
                                    "type": "cross",
                                    "left": {"indicator": "rsi_14", "output": "value"},
                                    "right": {"fixed": 30},
                                    "direction": "above",
                                    "lookback": 1
                                },
                                "negated": false
                            },
                            "chain": [
                                {
                                    "operator": "and",
                                    "trigger": {
                                        "trigger": {
                                            "type": "compare",
                                            "left": {"indicator": "ema_20", "output": "value"},
                                            "operator": ">",
                                            "right": {"indicator": "ema_50", "output": "value"},
                                            "lookback": 1
                                        },
                                        "negated": false
                                    }
                                }
                            ]
                        }
                    ]
                }
            ],
            "exit_rules": [],
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1,
                "rr_ratio": 2.0,
                "spread_buffer_pips": 1
            },
            "version": 1,
            "is_active": true
        }"#;

        match StrategyDefinition::from_json(json) {
            Ok(s) => {
                assert_eq!(s.entry_rules.len(), 1);
                // Verify the chain has one item
                assert_eq!(s.entry_rules[0].conditions[0].chain.len(), 1);
            }
            Err(e) => panic!("Cross trigger JSON should parse but failed: {}", e),
        }
    }

    #[test]
    fn test_ai_conversion_price_source_parses() {
        // Test compare trigger with price data source
        let json = r#"{
            "id": "import-placeholder",
            "user_id": "import-placeholder",
            "schema_version": 2,
            "name": "Price Source Test",
            "description": "Tests price data source in compare trigger",
            "indicators": [
                {"id": "ema_20", "type": "ema", "params": {"period": 20}}
            ],
            "parameters": [],
            "variables": [],
            "entry_rules": [
                {
                    "id": "entry_long_1",
                    "direction": "long",
                    "conditions": [
                        {
                            "primary": {
                                "trigger": {
                                    "type": "compare",
                                    "left": {"indicator": "ema_20", "output": "value"},
                                    "operator": ">",
                                    "right": {"source": "price", "value": "close"},
                                    "lookback": 1
                                },
                                "negated": false
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
                "spread_buffer_pips": 1
            },
            "version": 1,
            "is_active": true
        }"#;

        match StrategyDefinition::from_json(json) {
            Ok(s) => {
                assert_eq!(s.entry_rules.len(), 1);
            }
            Err(e) => panic!("Price source JSON should parse but failed: {}", e),
        }
    }
}
