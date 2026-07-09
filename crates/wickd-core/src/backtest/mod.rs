//! Backtesting module

pub mod strategy;
pub mod engine;
pub mod strategies;
pub mod indicators;
pub mod indicator_engine;
pub mod rules_types;
pub mod rules_engine;
pub mod rules_strategy;
pub mod pivots;
pub mod optimizer;
pub mod walk_forward;
pub mod regime_detector;
pub mod patterns;
pub mod mtf;
pub mod scripted_strategy;
pub mod surprise;
mod rules_triggers;

#[cfg(test)]
mod rules_engine_tests;

#[cfg(test)]
mod correctness_fixture_tests;

#[cfg(test)]
mod indicator_fixture_tests;

pub use strategy::{PositionSnapshot, Strategy, Signal};
pub use engine::{BacktestEngine, BacktestConfig, BacktestResult, BacktestMetrics, SimulatedTrade};
pub use strategies::{MovingAverageCrossover, RsiStrategy};
pub use indicators::*;
pub use indicator_engine::{IndicatorEngine, IndicatorConfig, OutputHistory};
pub use rules_engine::{
    RulesEngine, StrategyDefinition,
    EntryLogic, RiskSettings,
};
pub use rules_strategy::RulesBasedStrategy;
pub use pivots::{PivotLevels, PivotPeriod, PivotConfig, PivotPeriodTracker, calculate_standard_pivots};
pub use optimizer::{
    OptimizationConfig, OptimizationObjective, OptimizationResult, OptimizationRun,
    OptimizationMetrics, run_optimization,
};
pub use walk_forward::{
    WalkForwardConfig, WalkForwardResult, WalkForwardPeriod, WalkForwardWindow,
    ParameterStabilityInfo, run_walk_forward,
};
pub use regime_detector::{RegimeDetector, RegimeConfig};
pub use rules_engine::{
    MarketRegime, Trigger, TriggerChain, ChainedTrigger, ChainOperator,
    DataSource, DistanceConfig, DistanceUnit,
    EntryRule, ExitRule, GivensTrigger, CrossTrigger, CompareTrigger,
    AnyEntryRule, AnyExitRule,
};
pub use mtf::{MtfCandleStore, extract_htf_timeframes};
pub use scripted_strategy::{
    validate_script, validate_script_typed, ScriptMetadata, ScriptValidationError, ScriptedStrategy,
};
pub use surprise::{SurpriseCalendar, SurpriseRelease};
