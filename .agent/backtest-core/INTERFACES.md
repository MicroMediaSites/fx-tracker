# Backtest Core Interfaces

## Interfaces This Domain Exposes

### Strategy Trait (`strategy.rs`)

The core abstraction that all trading strategies must implement. The `BacktestEngine` is generic over this trait.

```rust
pub trait Strategy {
    fn prepare(&mut self, candles: &[Candle]) {}  // Optional pre-processing
    fn on_candle(&mut self, candle: &Candle) -> Signal;
    fn on_candle_extended(&mut self, candle: &Candle) -> ExtendedSignal; // Default wraps on_candle
    fn current_stop_loss(&self) -> Option<Decimal> { None }
    fn current_take_profit(&self) -> Option<Decimal> { None }
    fn name(&self) -> &str;
    fn reset(&mut self);
}
```

```rust
pub enum Signal { Buy, Sell, Hold, ClosePosition }

pub struct ExtendedSignal {
    pub signal: Signal,
    pub stop_loss: Option<Decimal>,
    pub take_profit: Option<Decimal>,
    pub entry_rule_id: Option<String>,
    pub entry_rule_name: Option<String>,
    pub exit_reason: Option<String>,
    pub entry_indicators: Option<HashMap<String, String>>,
}
```

### BacktestEngine (`engine.rs`)

```rust
pub struct BacktestEngine { /* config */ }

impl BacktestEngine {
    pub fn new(config: BacktestConfig) -> Self;
    pub fn run<S: Strategy>(&self, strategy: &mut S, candles: &[Candle]) -> BacktestResult;
}
```

Key config struct:
```rust
pub struct BacktestConfig {
    pub initial_balance: Decimal,
    pub position_size: Decimal,
    pub use_percentage: bool,
    pub risk_percent: Option<Decimal>,
    pub estimated_stop_pips: Decimal,
    pub spread_pips: Decimal,
    pub pip_value: Decimal,
}
```

Key result structs:
```rust
pub struct BacktestResult {
    pub metrics: BacktestMetrics,
    pub trades: Vec<SimulatedTrade>,
    pub equity_curve: Vec<Decimal>,
    pub final_balance: Decimal,
}

pub struct BacktestMetrics {
    pub total_pnl: Decimal,
    pub total_return_pct: Decimal,
    pub annualized_return_pct: Decimal,
    pub winning_trades: u32,
    pub losing_trades: u32,
    pub win_rate: Decimal,
    pub avg_win: Decimal,
    pub avg_loss: Decimal,
    pub profit_factor: Decimal,
    pub max_drawdown_pct: Decimal,
    pub sharpe_ratio: Decimal,
    pub total_trades: u32,
}

pub struct SimulatedTrade {
    pub entry_time: String,
    pub exit_time: Option<String>,
    pub entry_price: Decimal,
    pub exit_price: Option<Decimal>,
    pub units: Decimal,
    pub pnl: Decimal,
    pub is_long: bool,
    pub entry_rule_id: Option<String>,
    pub entry_rule_name: Option<String>,
    pub exit_reason: Option<String>,
    pub stop_loss: Option<Decimal>,
    pub take_profit: Option<Decimal>,
    pub entry_indicators: Option<HashMap<String, String>>,
}
```

### RulesBasedStrategy (`rules_strategy.rs`)

Adapter that implements `Strategy` using `RulesEngine`.

```rust
pub struct RulesBasedStrategy { /* wraps RulesEngine */ }

impl RulesBasedStrategy {
    pub fn new(definition: StrategyDefinition) -> Result<Self, String>;
    pub fn from_json(json: &str) -> Result<Self, String>;
    pub fn from_json_with_params(json: &str, params: HashMap<String, f64>) -> Result<Self, String>;
    pub fn get_resolved_params(&self) -> &HashMap<String, f64>;
    pub fn set_sr_zones(&mut self, zones: Vec<SRZone>);
    pub fn set_sr_zones_from_json(&mut self, json: &str) -> Result<(), String>;
    pub fn set_pivot_config(&mut self, config: PivotConfig);
    pub fn set_pivot_config_from_json(&mut self, json: &str) -> Result<(), String>;
    pub fn set_pip_value_for_instrument(&mut self, instrument: &str);
}
```

### Multi-Timeframe Support (`mtf.rs`)

```rust
pub struct MtfCandleStore {
    htf_candles: HashMap<String, Vec<Candle>>,     // Per-timeframe sorted candles
    htf_indices: HashMap<String, usize>,           // Current position per timeframe
}

impl MtfCandleStore {
    pub fn new() -> Self;
    pub fn add_timeframe(&mut self, timeframe: &str, candles: Vec<Candle>);
    pub fn current_candle(&self, timeframe: &str) -> Option<&Candle>;
    pub fn advance(&mut self, timeframe: &str, primary_time: DateTime<Utc>) -> Option<&Candle>;
    pub fn filter_by_time_range(&self, start: &DateTime<Utc>, end: &DateTime<Utc>) -> Self;
    pub fn append_candle(&mut self, timeframe: &str, candle: Candle);
    pub fn reset(&mut self);
}

pub fn extract_htf_timeframes(definition: &StrategyDefinition) -> Vec<String>;
```

The `RulesBasedStrategy` exposes MTF setup:
```rust
impl RulesBasedStrategy {
    pub fn set_mtf_candle_store(&mut self, store: MtfCandleStore);
}
```

### RulesEngine (`rules_engine.rs`)

Core signal generation engine. Not usually called directly -- accessed through `RulesBasedStrategy`.

```rust
pub struct RulesEngine { /* indicators, position, zones, pivots, regime, params */ }

impl RulesEngine {
    pub fn new(strategy: StrategyDefinition) -> Result<Self, String>;
    pub fn with_params(strategy: StrategyDefinition, overrides: Option<HashMap<String, f64>>) -> Result<Self, String>;
    pub fn on_candle(&mut self, candle: &Candle) -> RulesSignal;          // Backtest mode
    pub fn on_candle_live(&mut self, candle: &Candle, pos: Option<PositionDirection>) -> RulesSignal; // Live mode
    pub fn warmup_candle(&mut self, candle: &Candle);                     // Indicator warmup only
    pub fn prepare_for_backtest(&mut self, candles: &[Candle]);           // Pattern detection
    pub fn reset(&mut self);
    pub fn has_position(&self) -> bool;
    pub fn get_risk_amount(&self, balance: Decimal, direction: PositionDirection) -> Decimal;
    pub fn calculate_position_size(&self, balance: Decimal, entry: Decimal, sl: Decimal, dir: PositionDirection) -> Option<Decimal>;
    pub fn get_indicator_snapshot(&self) -> HashMap<String, HashMap<String, String>>;
    pub fn get_resolved_params(&self) -> &HashMap<String, f64>;
    pub fn set_sr_zones(&mut self, zones: Vec<SRZone>);
    pub fn set_pivot_config(&mut self, config: PivotConfig);
    pub fn set_pip_value(&mut self, pip_value: Decimal);
    pub fn set_pip_value_for_instrument(&mut self, instrument: &str);
    pub fn set_balance(&mut self, balance: Decimal);
    pub fn evaluate_entry_rule_v2(&self, rule: &EntryRule, candle: &Candle) -> bool;   // Public for testing
    pub fn evaluate_trigger_chain(&self, chain: &TriggerChain, candle: &Candle) -> bool; // Deprecated
    pub fn evaluate_condition(&self, condition: &Condition, candle: &Candle) -> bool;
    pub fn evaluate_conditions(&self, conditions: &[Condition], candle: &Candle) -> bool;
    pub fn evaluate_trigger_v2(&self, trigger: &Trigger, candle: &Candle) -> bool;
}

pub enum RulesSignal {
    Hold,
    Entry { direction: PositionDirection, stop_loss: Option<Decimal>, take_profit: Option<Decimal>, triggered_rule_id: Option<String>, triggered_rule_name: Option<String> },
    Exit { reason: String, close_percent: f64 },
    PartialExit { reason: String, close_percent: f64, new_stop_loss: Option<Decimal> },
}
```

### Optimizer (`optimizer.rs`)

```rust
pub fn run_optimization(
    strategy_json: &str,
    parameters: &[ParameterDefinition],
    candles: &[Candle],
    initial_balance: Decimal,
    sr_zones: Option<&[SRZone]>,
    pivot_config: Option<&PivotConfig>,
    config: &OptimizationConfig,
    instrument: &str,
    progress_callback: Option<&dyn Fn(usize, usize)>,
) -> Result<OptimizationResult, String>;

pub fn extract_param_ranges(parameters: &[ParameterDefinition], filter_ids: Option<&[String]>) -> Vec<ParameterRange>;
pub fn generate_combinations(ranges: &[ParameterRange]) -> Vec<HashMap<String, f64>>;
pub fn count_combinations(ranges: &[ParameterRange]) -> usize;
pub fn calculate_score(result: &BacktestResult, objective: OptimizationObjective) -> f64;
```

```rust
pub enum OptimizationObjective {
    SharpeRatio, ProfitFactor, TotalReturn, WinRate, MinDrawdown, TradeCount,
}

pub struct OptimizationResult {
    pub total_combinations: usize,
    pub valid_results: usize,
    pub runs: Vec<OptimizationRun>,      // Sorted by score descending
    pub best_params: Option<HashMap<String, f64>>,
    pub objective: OptimizationObjective,
}
```

### Walk-Forward (`walk_forward.rs`)

```rust
pub fn run_walk_forward(
    strategy_json: &str,
    parameters: &[ParameterDefinition],
    candles: &[Candle],
    initial_balance: Decimal,
    sr_zones: Option<&[SRZone]>,
    pivot_config: Option<&PivotConfig>,
    config: &WalkForwardConfig,
    instrument: &str,
    progress_callback: Option<&dyn Fn(WalkForwardProgress)>,
    cancel_token: Option<&Arc<AtomicBool>>,
) -> Result<WalkForwardResult, String>;

pub fn generate_windows(start: DateTime<Utc>, end: DateTime<Utc>, config: &WalkForwardConfig) -> Vec<WalkForwardWindow>;
pub fn generate_anchored_windows(start: DateTime<Utc>, end: DateTime<Utc>, config: &WalkForwardConfig) -> Vec<WalkForwardWindow>;
```

```rust
pub struct WalkForwardResult {
    pub config: WalkForwardConfig,
    pub periods: Vec<WalkForwardPeriod>,
    pub total_periods: usize,
    pub valid_periods: usize,
    pub profitable_periods: usize,
    pub oos_total_pnl: String,
    pub oos_total_return_pct: String,
    pub oos_avg_sharpe: f64,
    pub oos_win_rate: String,
    pub oos_max_drawdown_pct: String,
    pub oos_total_trades: u32,
    pub sharpe_efficiency: f64,         // OOS Sharpe / IS Sharpe (%)
    pub return_efficiency: f64,         // OOS Return / IS Return (%)
    pub robustness_score: u32,          // 0-100
    pub parameter_stability: Vec<ParameterStabilityInfo>,
    pub oos_equity_curve: Vec<String>,
}
```

### Tauri Commands (`commands/backtest.rs`)

These are the frontend-facing entry points:

| Command | Description |
|---------|-------------|
| `run_backtest` | Run with built-in strategy (MA crossover, RSI) |
| `run_custom_backtest` | Run with JSON-defined rules-based strategy |
| `run_backtest_debug` | Run with debug output (sample candles, detailed trades) |
| `optimize_strategy` | Grid search parameter optimization |
| `run_walk_forward` | Walk-forward analysis with job tracking |
| `cancel_walk_forward` | Cancel a running walk-forward via AtomicBool token |
| `run_parameter_sweep` | Fixed-value sweep: for each value of a param, run full walk-forward |
| `validate_strategy_json` | Validate strategy JSON without running a backtest |

### Built-in Strategies (`strategies.rs`)

```rust
pub struct MovingAverageCrossover { /* fast/slow period */ }
pub struct RsiStrategy { /* period, overbought, oversold */ }
```

Both implement `Strategy` trait. These are legacy strategies used by `run_backtest` but not by the main product flow (which uses `RulesBasedStrategy`).

## Interfaces This Domain Consumes

### From `shared` crate (strategy type definitions)

All strategy schema types are defined in `shared/src/lib.rs` and re-exported through `rules_types.rs`:

- `StrategyDefinition` -- top-level strategy schema
- `ParameterDefinition`, `ParameterizedValue`, `ParameterReference` -- parameterization
- `IndicatorConfig`, `IndicatorType` -- indicator configuration
- `Trigger` (enum: `Givens`, `Cross`, `Compare`, `Threshold`, `RiskReward`, `PercentOfTp`, `Time`)
- `DataSource` (untagged enum: `Indicator`, `Price`, `Fixed`, `Parameter`, `SRZone`, `Pivot`, `Variable`)
- `Condition`, `TriggerWithNot`, `ChainedTriggerWithNot`, `ChainOperator`
- `EntryRule`, `ExitRule`, `EntryLogic`, `RuleDirection`
- `RiskSettings`, `RiskMethod`, `StopLossSource`, `StopLossEvaluationMode`
- `PositionDirection`, `CaptureMode`, `TrailConfig`
- `MarketRegime`, `DistanceUnit`, `DistanceConfig`
- `StrategyVariable`, `VariableExpression`, `MathOperator`, `MathOperation`
- `SRZone`, `PivotSource`, `SRTarget`

### From `indicators` domain

- `IndicatorEngine::from_config_with_params(configs, history_size, params)` -- create indicator set
- `IndicatorEngine::on_candle(candle)` -- update all indicators with new candle
- `IndicatorEngine::get_output(id, output_name, offset)` -- get indicator value at offset bars ago
- `IndicatorEngine::get_latest(id, output_name)` -- get current indicator value (offset 0)
- `IndicatorEngine::get_snapshot()` -- get all current values for entry indicator capture
- `IndicatorEngine::reset()` -- clear all indicator state

### From `models` domain

- `Candle` -- `{ time: DateTime<Utc>, mid: Ohlc, volume: u32, complete: bool }`
- `Ohlc` -- `{ open: Decimal, high: Decimal, low: Decimal, close: Decimal }`

### From `oanda` domain (in command handlers only)

- `endpoints::get_candles()` -- fetch candles by count
- `endpoints::get_candles_paginated()` -- fetch candles by date range with pagination
- `Granularity::from_str()` -- parse timeframe string

### From Tauri framework

- `State<'_, AppState>` -- access to OANDA client and cancel token
- `AppHandle` + `Emitter` -- emit progress events to frontend
- `#[tauri::command]` -- command registration

## Interface Evolution Rules

1. **Never remove fields from result structs** sent to the frontend. The frontend may depend on any field. Add new fields with `#[serde(default)]` or `Option<T>`.

2. **Never change `StrategyDefinition` without updating the shared crate**. The shared crate is the single source of truth for strategy schema. Copy the schema to queries-service after changes: `cp shared/schema.ts queries-service/schema.ts`.

3. **Keep `Strategy` trait minimal**. New capabilities should be expressed through `ExtendedSignal` fields or through the `RulesEngine` directly, not by adding methods to the trait. The trait is the stable interface between engine and strategy.

4. **OptimizationMetrics and WalkForwardResult use String for Decimal values**. This is for JSON serialization precision. Do not change these to f64 -- the frontend parses them to maintain precision.

5. **Tauri commands must validate all inputs before OANDA calls**. Network calls are expensive; fail fast on bad inputs. Always validate instrument format, date format, and JSON parseability before fetching candles.

6. **The `RulesSignal` enum must remain convertible to `Signal`**. The `From<RulesSignal> for Signal` implementation must stay in sync. If new `RulesSignal` variants are added, update the conversion.
