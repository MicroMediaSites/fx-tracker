# Indicators Domain Interfaces

This is the most critical document for this domain because it is cross-cutting -- 4+ other domains depend on its public API. Any change to these interfaces can break backtesting, strategy monitoring, charting, and AI analysis.

## Core Type: `IndicatorOutputs`

```rust
pub type IndicatorOutputs = HashMap<String, Decimal>;
```

This is the universal return type for all indicator calculations. Every consumer of this domain works with this type. Keys are output names (e.g., `"value"`, `"upper"`, `"macd"`), values are `rust_decimal::Decimal`.

**Consumed by**: backtest-core (rules engine), strategy-monitor, ai-analysis

---

## Trait: `Indicator`

```rust
pub trait Indicator: Send {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs;
    fn indicator_type(&self) -> &str;
    fn output_names(&self) -> Vec<&str>;
    fn reset(&mut self);
}
```

**Consumed by**:
- `IndicatorEngine` -- holds `HashMap<String, Box<dyn Indicator>>` and calls `on_candle()` on each
- `ai-analysis` (enrichment.rs, trade_review.rs) -- instantiates individual indicators directly and calls `on_candle()`

**Contract**:
- `on_candle()` returns empty HashMap when insufficient data; never panics
- `indicator_type()` returns a static string matching `IndicatorType::as_str()`
- `output_names()` returns all possible output keys that `on_candle()` may produce
- `reset()` restores the indicator to construction-time state

---

## Indicator Structs (Public)

All structs have `pub fn new(...)` constructors. Listed with their outputs.

| Struct | Params | Outputs | Direct Consumers |
|--------|--------|---------|-----------------|
| `SmaIndicator` | `period: usize` | `value` | ai-analysis (enrichment, trade_review) |
| `EmaIndicator` | `period: usize` | `value` | ai-analysis (enrichment, trade_review) |
| `RsiIndicator` | `period: usize` | `value` | ai-analysis (enrichment, trade_review) |
| `AtrIndicator` | `period: usize` | `value` | ai-analysis (enrichment, trade_review), ChandelierIndicator |
| `AdxIndicator` | `period: usize` | `value`, `plus_di`, `minus_di` | -- |
| `IchimokuIndicator` | `tenkan_period, kijun_period, senkou_b_period, displacement: usize` | `tenkan`, `kijun`, `senkou_a`, `senkou_b`, `cloud_top`, `cloud_bottom`, `chikou` | -- |
| `ChandelierIndicator` | `period: usize, multiplier: Decimal` | `exit_long`, `exit_short` | -- |
| `BollingerIndicator` | `period: usize, std_dev: Decimal` | `upper`, `middle`, `lower` | -- |
| `MacdIndicator` | `fast_period, slow_period, signal_period: usize` | `macd`, `signal`, `histogram` | -- |
| `StochasticIndicator` | `k_period, d_period: usize` | `k`, `d` | -- |
| `MaHistogramIndicator` | `fast_period, slow_period: usize` | `histogram`, `fast_ma`, `slow_ma` | -- |
| `MaBandsIndicator` | `period: usize, distance_pips: Decimal` | `upper`, `middle`, `lower` | -- |
| `DssIndicator` | `stoch_period, ema_period, signal_period: usize` | `dss`, `signal` | -- |
| `AdrIndicator` | `period: usize` | `value`, `ratio` | -- |
| `DailyIndicator` | (none) | `high`, `low`, `range`, `open` | -- |
| `SwingIndicator` | `strength: usize` | `recent_high`, `recent_high_bars`, `recent_low`, `recent_low_bars`, `prev_high`, `prev_high_bars`, `prev_low`, `prev_low_bars` | -- |
| `MfiIndicator` | `period: usize` | `value` | -- |
| `DonchianIndicator` | `period: usize` | `upper`, `middle`, `lower` | -- |

Indicators not listed as having direct consumers are accessed exclusively through `IndicatorEngine`.

---

## `IndicatorEngine`

```rust
pub struct IndicatorEngine {
    indicators: HashMap<String, Box<dyn Indicator>>,
    history: HashMap<String, OutputHistory>,
    max_history: usize,
}
```

### Public Methods

| Method | Signature | Consumers | Description |
|--------|-----------|-----------|-------------|
| `new` | `(max_history: usize) -> Self` | backtest-core (tests) | Create empty engine |
| `from_config` | `(configs: &[IndicatorConfig], max_history: usize) -> Result<Self, String>` | backtest-core (tests) | Create from config without param resolution |
| `from_config_with_params` | `(configs: &[IndicatorConfig], max_history: usize, resolved_params: &HashMap<String, f64>) -> Result<Self, String>` | backtest-core (RulesEngine::with_params) | **Primary creation path**. Resolves parameterized values. |
| `add_indicator` | `(&mut self, id: &str, indicator: Box<dyn Indicator>)` | backtest-core (tests) | Manual indicator registration |
| `on_candle` | `(&mut self, candle: &Candle) -> HashMap<String, IndicatorOutputs>` | backtest-core (RulesEngine::evaluate) | **Core processing**. Feeds candle to all indicators, stores history. |
| `get_output` | `(&self, indicator_id: &str, output: &str, offset: usize) -> Option<Decimal>` | backtest-core (rules_triggers.rs) | Get value with lookback offset |
| `get_latest` | `(&self, indicator_id: &str, output: &str) -> Option<Decimal>` | backtest-core (rules_triggers.rs) | Get current value (offset=0) |
| `can_detect_cross` | `(&self, indicator_id: &str, output: &str) -> bool` | backtest-core (rules_triggers.rs) | Check if cross detection is possible (needs 2+ values) |
| `get_history` | `(&self, indicator_id: &str) -> Option<&OutputHistory>` | backtest-core | Get full history object |
| `reset` | `(&mut self)` | backtest-core (walk_forward) | Reset all indicators and history |
| `get_snapshot` | `(&self) -> HashMap<String, HashMap<String, String>>` | backtest-core (engine.rs) | Snapshot of all latest values (for trade entry metadata) |

### `OutputHistory`

```rust
pub struct OutputHistory {
    values: HashMap<String, Vec<Decimal>>,
    max_history: usize,
}
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `(max_history: usize) -> Self` | Create with capacity limit |
| `push` | `(&mut self, outputs: &IndicatorOutputs)` | Append outputs, trim to max |
| `get` | `(&self, output: &str, offset: usize) -> Option<Decimal>` | Get value at offset (0=newest) |
| `latest` | `(&self, output: &str) -> Option<Decimal>` | Convenience for `get(output, 0)` |
| `has_history` | `(&self, output: &str, offset: usize) -> bool` | Check if offset is available |
| `clear` | `(&mut self)` | Clear all history |
| `get_all_latest` | `(&self) -> HashMap<String, String>` | All latest values as strings |

---

## `RegimeDetector`

```rust
pub struct RegimeDetector {
    config: RegimeConfig,
    atr_history: VecDeque<Decimal>,
    sr_zones: Vec<SRZone>,
    gap_zones: Vec<GapZone>,
    base_zones: Vec<BaseZone>,
    order_blocks: Vec<OrderBlock>,
    structure_levels: Vec<StructureLevel>,
    current_index: usize,
}
```

### Public Methods

| Method | Signature | Consumers | Description |
|--------|-----------|-----------|-------------|
| `new` | `(config: RegimeConfig) -> Self` | backtest-core (RulesEngine) | Create with config |
| `set_sr_zones` | `(&mut self, zones: Vec<SRZone>)` | backtest-core (RulesEngine) | Set user-defined S/R zones |
| `set_current_index` | `(&mut self, index: usize)` | backtest-core (rules_triggers) | Update bar index for pattern lookback boundary |
| `update_atr` | `(&mut self, atr: Decimal)` | backtest-core (rules_triggers) | Feed ATR for volatility regime history |
| `is_regime_active` | `(&self, regime: MarketRegime, candle: &Candle, adx: Option<Decimal>, sma20: Option<Decimal>, sma50: Option<Decimal>, atr: Option<Decimal>, bb_upper: Option<Decimal>, bb_lower: Option<Decimal>, bb_middle: Option<Decimal>) -> bool` | backtest-core (rules_triggers) | **Core method**. Check if a regime is currently active. |
| `detect_patterns` | `(&mut self, candles: &[Candle], atr_values: &[Decimal])` | backtest-core (RulesEngine::prepare_for_backtest) | Pre-compute all price action patterns. Call once before backtest loop. |
| `reset` | `(&mut self)` | backtest-core (RulesEngine) | Clear all state and patterns |

### `RegimeConfig`

| Method | Signature | Description |
|--------|-----------|-------------|
| `default()` | `-> Self` | Standard forex defaults (pip_value=0.0001) |
| `for_jpy()` | `-> Self` | JPY pair defaults (pip_value=0.01) |
| `for_instrument(instrument: &str)` | `-> Self` | Auto-detect from instrument name |

Key config fields with defaults:
- `adx_trend_threshold`: 25
- `adx_range_threshold`: 20
- `high_vol_multiplier`: 1.5
- `low_vol_multiplier`: 0.5
- `atr_rolling_periods`: 14
- `bb_contraction_threshold`: 0.02
- `sr_test_distance_pips`: 20
- `min_gap_pips`: 5
- `zone_distance_pips`: 15
- `pivot_strength`: 3

---

## Pivot Calculations

### `PivotLevels`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PivotLevels {
    pub pp: Decimal,
    pub r1: Decimal, pub r2: Decimal, pub r3: Decimal,
    pub s1: Decimal, pub s2: Decimal, pub s3: Decimal,
}
```

| Method | Signature | Consumers |
|--------|-----------|-----------|
| `get_level` | `(&self, level: PivotLevel) -> Decimal` | backtest-core (rules_triggers) |

### `PivotPeriodTracker`

| Method | Signature | Consumers |
|--------|-----------|-----------|
| `new` | `() -> Self` | backtest-core (RulesEngine) |
| `update` | `(&mut self, time: DateTime<Utc>, high: Decimal, low: Decimal, close: Decimal, period: PivotPeriod) -> bool` | backtest-core (RulesEngine) |
| `can_calculate` | `(&self) -> bool` | backtest-core |
| `calculate_pivots` | `(&self) -> Option<PivotLevels>` | backtest-core (RulesEngine) |
| `reset` | `(&mut self)` | backtest-core |

### `PivotConfig`

```rust
pub struct PivotConfig {
    pub enabled: bool,
    pub period: PivotPeriod,  // Daily or Weekly
}
```

### Standalone Function

| Function | Signature | Consumers |
|----------|-----------|-----------|
| `calculate_standard_pivots` | `(high: Decimal, low: Decimal, close: Decimal) -> PivotLevels` | PivotPeriodTracker, backtest-core |

---

## Utility Function

| Function | Signature | Location | Description |
|----------|-----------|----------|-------------|
| `decimal_sqrt` | `(n: Decimal) -> Decimal` | indicators.rs (private) | Newton's method square root. Used by BollingerIndicator. Not publicly exported. |

---

## The `IndicatorType` Enum Contract

Defined in `shared/src/lib.rs`, this enum is the single source of truth for what indicators exist. It must stay in sync across:

1. **`shared/src/lib.rs`**: Enum definition + `as_str()` + `Display`
2. **`indicators.rs`**: Corresponding `XxxIndicator` struct implementing `Indicator`
3. **`indicator_engine.rs`**: `create_indicator()` match arm
4. **`src/types/strategy.ts`**: TypeScript `IndicatorType` union + metadata

Current variants (18 total):
```
Sma, Ema, Rsi, Atr, Adx, Ichimoku, Chandelier, Bollinger, Macd,
Stochastic, MaHistogram, MaBands, Dss, Adr, Daily, Swing, Mfi, Donchian
```

The enum uses `#[serde(rename_all = "snake_case")]` for JSON serialization. This means:
- `MaHistogram` serializes to `"ma_histogram"`
- `MaBands` serializes to `"ma_bands"`

---

## How Indicator Results Flow to Consumers

```
Candle
  |
  v
IndicatorEngine::on_candle()
  |
  +-> HashMap<indicator_id, HashMap<output_name, Decimal>>
  |     |
  |     +-> Stored in OutputHistory (per-indicator rolling buffer)
  |
  v
Rules Engine (in backtest-core) queries via:
  - get_output(indicator_id, output_name, offset) -> for compare/threshold triggers
  - get_latest(indicator_id, output_name)          -> for regime detection helpers
  - can_detect_cross(indicator_id, output_name)    -> before cross trigger evaluation

Regime Detector receives indicator values as function args:
  - is_regime_active(regime, candle, adx, sma20, sma50, atr, bb_upper, bb_lower, bb_middle)
  - The rules engine extracts these from IndicatorEngine before calling

Pivot Tracker receives raw candle HLC:
  - update(time, high, low, close, period) -> returns bool if new period
  - calculate_pivots() -> PivotLevels used in rules evaluation

Trade Entry Metadata:
  - get_snapshot() returns all latest values for recording indicator state at entry
```

---

## Backwards Compatibility Rules

### Adding a New Indicator
- **Safe**: Add new variant to `IndicatorType`, new struct, new factory match arm. Existing strategies are unaffected.
- Requires: Frontend updates to TypeScript types and chart constants.

### Adding a New Output to an Existing Indicator
- **Safe**: Adding a new key to the HashMap returned by `on_candle()`. Existing strategies that don't reference the new output are unaffected. Consumers use `HashMap::get()` which returns `None` for unknown keys.
- Consider: Updating `output_names()` to include the new output (for documentation/introspection).

### Renaming an Output
- **BREAKING**: Any strategy with a `DataSource::Indicator { output: "old_name" }` will silently fail (get `None` instead of a value). Rules will not trigger. Do not rename outputs.

### Removing an Indicator
- **BREAKING**: Strategies referencing it will fail at `create_indicator()` time (returns `Err`). The `IndicatorType` enum provides compile-time exhaustiveness checking, but existing serialized strategies will fail deserialization if the variant is removed.

### Changing Default Parameters
- **Non-breaking but behavior-changing**: Strategies that rely on defaults (omitting the param from config) will compute different values. This is acceptable for new strategies but may change backtest results for existing ones that omit the parameter.

### Changing Calculation Logic
- **Non-breaking in API, breaking in behavior**: Output values will differ. All backtests with the affected indicator will produce different results. This should be done intentionally and documented.

### Changing `RegimeDetector` Thresholds
- **Behavior-changing**: Strategies using regime-based givens triggers will see different signal generation. Changes to `RegimeConfig` defaults affect all strategies that don't override thresholds.

### Adding a New `MarketRegime` Variant
- **Safe for existing strategies**: New variant won't be referenced by existing strategies. Requires adding detection logic to `is_regime_active()` and frontend UI for the new regime.
