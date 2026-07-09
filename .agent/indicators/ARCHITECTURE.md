# Indicators Domain Architecture

## Overview

The indicators domain provides a stateful, streaming computation pipeline for technical indicators. Each indicator maintains its own internal state and processes candles one at a time, producing named output values. The `IndicatorEngine` orchestrates multiple indicators and tracks their output history for lookback access.

## Computation Pipeline

### 1. Individual Indicator Calculation (`indicators.rs`)

Every indicator implements the `Indicator` trait:

```rust
pub trait Indicator: Send {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs;
    fn indicator_type(&self) -> &str;
    fn output_names(&self) -> Vec<&str>;
    fn reset(&mut self);
}
```

- `on_candle()` is the core method. It receives a single candle, updates internal state, and returns a `HashMap<String, Decimal>` of named outputs.
- Indicators return an **empty HashMap** when they don't have enough data (warmup period not met). This is the standard graceful degradation pattern -- consumers must always handle missing outputs.
- All internal state uses `VecDeque<Decimal>` for rolling windows, with capacity pre-allocated at construction.
- The `Send` bound enables indicators to be used across threads (required for parallel optimization in backtest-core).

### 2. Indicator Engine Orchestration (`indicator_engine.rs`)

The `IndicatorEngine` manages multiple indicators and their output history:

```
IndicatorConfig[] --> IndicatorEngine::from_config_with_params()
                          |
                          v
                    HashMap<String, Box<dyn Indicator>>  (indicators by ID)
                    HashMap<String, OutputHistory>        (history by ID)
```

**Creation flow:**
1. `from_config_with_params()` receives `IndicatorConfig[]` and resolved parameter values
2. For each config, `resolve_params()` substitutes `$param` references with concrete values
3. `create_indicator()` factory function matches on `IndicatorType` enum and instantiates the correct struct
4. Indicator + history slot registered under the config's `id`

**Processing flow (per candle):**
1. `on_candle(candle)` iterates all registered indicators
2. Each indicator's `on_candle()` called, outputs collected
3. Outputs pushed to `OutputHistory` for that indicator
4. Returns `HashMap<indicator_id, IndicatorOutputs>`

**History access:**
- `get_output(id, output, offset)` -- offset 0 = current, 1 = previous, etc.
- `get_latest(id, output)` -- convenience for offset 0
- `can_detect_cross(id, output)` -- checks if at least 2 values exist (for cross detection)
- `get_snapshot()` -- all latest values for all indicators (used for trade entry snapshots)

The `OutputHistory` struct caps stored values at `max_history` (default 100), removing oldest values when exceeded.

### 3. Parameter Resolution

Indicator parameters can be either fixed values or references to strategy-level parameters (for optimization). The resolution chain:

```
IndicatorConfig.params: HashMap<String, ParameterizedValue>
    |
    v  ParameterizedValue::Fixed(f64) --> direct value
    v  ParameterizedValue::Reference { $param: "id" } --> lookup in resolved_params map
    |
    v
HashMap<String, f64> --> passed to create_indicator()
```

Helper functions convert f64 params to the correct types:
- `get_param_usize()` -- for period values (with sensible defaults)
- `get_param_decimal()` -- for multiplier/distance values (converts f64 -> Decimal)

## Regime Detection (`regime_detector.rs`)

The `RegimeDetector` evaluates predefined market conditions (regimes) used as "givens" in strategy rules. It operates in three categories:

### Trend/Volatility Regimes
- **TrendingUp**: ADX > 25 AND price > SMA20 > SMA50
- **TrendingDown**: ADX > 25 AND price < SMA20 < SMA50
- **Ranging**: ADX < 20 AND Bollinger Band width contracted (< 2% of middle)
- **HighVolatility**: Current ATR > 1.5x rolling average ATR
- **LowVolatility**: Current ATR < 0.5x rolling average ATR

These consume indicator values passed in from the rules engine -- the regime detector does NOT own or compute indicators itself.

### S/R Zone Regimes
- **SrTested**: Price within configurable pip distance of user-defined S/R zone boundaries

### Price Action Regimes (Pattern-Based)
Pre-computed from full candle history via `detect_patterns()`:
- **AtBullishGap / AtBearishGap**: Unfilled price gaps (gap > min_gap_pips)
- **AtDemandZone / AtSupplyZone**: Impulse-Base-Impulse supply/demand zones
- **AtBullishOb / AtBearishOb**: Order blocks (last opposing candle before impulse)
- **RetestingSupport / RetestingResistance**: Broken structure levels being retested

Key design decision: Price action patterns are pre-computed once before a backtest (`detect_patterns(candles, atr_values)`) rather than incrementally. This is because patterns require full history context. The `current_index` is tracked so only patterns formed before the current bar are evaluated (preventing look-ahead bias).

### Session and Divergence Regimes
- **LondonSession / UsSession / AsianSession**: Evaluated by time check directly in `rules_triggers.rs`, not by `RegimeDetector`
- **Divergence**: Requires per-trigger config, evaluated directly in `rules_triggers.rs`

### Configuration
`RegimeConfig` has sensible defaults and auto-adjusts for JPY pairs (0.01 pip value vs 0.0001). Created via `RegimeConfig::for_instrument(instrument)`.

## Pivot Calculations (`pivots.rs`)

Standard pivot point calculations from previous period's High/Low/Close:
- PP = (H + L + C) / 3
- R1 = 2*PP - L, S1 = 2*PP - H
- R2 = PP + range, S2 = PP - range
- R3 = H + 2*(PP-L), S3 = L - 2*(H-PP)

**Period tracking** is handled by `PivotPeriodTracker`:
1. Tracks current period's running H/L/C
2. On period boundary crossing (new day or new week), saves current as "previous" and resets
3. `calculate_pivots()` returns `PivotLevels` from previous period's data
4. Returns `None` until at least one full period has completed

Supports `PivotPeriod::Daily` and `PivotPeriod::Weekly`. Weekly boundaries are Monday-based.

## Multi-Timeframe Indicator Processing

The indicators domain itself is timeframe-agnostic — each `IndicatorEngine` processes candles sequentially regardless of their timeframe. Multi-timeframe support is achieved through the architecture above it:

1. **Per-timeframe engines**: The `RulesEngine` in backtest-core creates separate `IndicatorEngine` instances for each higher timeframe (`htf_indicator_engines: HashMap<String, IndicatorEngine>`).
2. **Seeding**: When `set_mtf_candle_store()` is called, each HTF indicator engine is fed its pre-fetched candles to build up indicator state.
3. **Advancement**: As the primary-timeframe backtest loop runs, newly completed HTF candles are fed to their corresponding indicator engine via `advance_htf_engines()`.
4. **Resolution**: When a rule trigger references `{"indicator": "ema_daily", "output": "value", "timeframe": "D"}`, the rules engine looks up the value from `htf_indicator_engines["D"]` instead of the primary indicator engine.

The `IndicatorConfig` struct in `shared/src/lib.rs` has an optional `timeframe: Option<String>` field. When set (e.g., `"D"`, `"H4"`, `"W"`), the indicator is computed on that timeframe's candles. Similarly, `IndicatorSource` and `PriceSource` have optional `timeframe` fields for trigger data sources.

This design means adding MTF support required zero changes to the indicators domain — it's purely an orchestration concern in backtest-core and strategy-monitor.

## Key Design Decisions

### Why `Decimal` Instead of `f64`
All indicator outputs are `rust_decimal::Decimal`. This prevents floating-point drift in financial calculations, especially for EMA smoothing chains (MACD uses EMAs of EMAs) and multi-period running averages. The `decimal_sqrt()` helper uses Newton's method (20 iterations, convergence threshold 0.0000001) for Bollinger Band standard deviation.

### Why Stateful Structs Instead of Pure Functions
Indicators maintain rolling state (VecDeque buffers, smoothed averages) because they process candles one at a time in a streaming fashion. The backtest engine feeds candles sequentially; pre-computing all indicator values would require O(n * periods) memory for n candles. The streaming approach uses O(period) memory per indicator.

### Why the `Indicator` Trait Is Object-Safe
The `Box<dyn Indicator>` pattern in `IndicatorEngine` allows heterogeneous collections of indicators. The `Send` bound is required because `IndicatorEngine` instances can be moved across threads during parallel optimization.

### Why Regime Detection Is Separate from Indicators
Regimes are boolean conditions composed from multiple indicator values, not standalone calculations. They also incorporate non-indicator data (S/R zones, price action patterns, candle timestamps). Keeping them separate avoids coupling the simple indicator trait with complex multi-source logic.

### Why Pattern Detection Is Pre-Computed
Price action patterns (gaps, order blocks, supply/demand zones, structure levels) require looking at candle sequences that may be far apart. Computing these incrementally would require maintaining large sliding windows. Pre-computation with forward-pass status updates (fill, break, mitigation, retest) is simpler and prevents look-ahead bias through `current_index` tracking.

## How to Add a New Indicator

Full checklist (see `docs/patterns/adding-indicators.md` for details):

1. **`shared/src/lib.rs`**: Add variant to `IndicatorType` enum and `as_str()` match
2. **`src-tauri/src/backtest/indicators.rs`**: Create struct implementing `Indicator` trait
   - Use `Decimal` for all values
   - Return empty HashMap during warmup
   - Implement `reset()` to clear all state
   - Add unit tests
3. **`src-tauri/src/backtest/indicator_engine.rs`**: Add match arm in `create_indicator()` factory
   - Use `get_param_usize()` / `get_param_decimal()` with sensible defaults
4. **`src/types/strategy.ts`**: Add to `IndicatorType` union, `INDICATOR_METADATA`, `INDICATOR_OUTPUTS`, `INDICATOR_DEFAULTS`
5. **`src/components/charts/chartConstants.ts`**: Add colors and chart preset
6. **Run tests**: `npm run test:be` to verify all indicator tests pass

## Known Technical Debt

1. **ADX initial smoothing approximation**: The ADX indicator's initial Wilder smoothing uses a simplified approach (`plus_dm * period`) instead of accumulating and averaging the first N directional movement values. The code has a comment acknowledging this: "In practice, we should accumulate and average, but this is simpler." This affects the first few ADX values but converges to correct values after warmup.

2. **MA Bands hardcoded pip value**: `MaBandsIndicator` hardcodes `0.0001` as the pip-to-price conversion factor. This means it only works correctly for standard forex pairs, not JPY pairs or metals.

3. **EMA initial_sum accumulation**: The `EmaIndicator` continues adding to `initial_sum` even after the EMA is seeded. This has no effect on outputs (the branch checking `self.ema.is_none()` is no longer taken) but is wasted computation.

4. **Direct indicator instantiation in ai-analysis**: `enrichment.rs` and `trade_review.rs` bypass `IndicatorEngine` and instantiate indicators directly. This creates a second path for indicator usage that won't benefit from any future `IndicatorEngine` improvements (caching, multi-timeframe, etc.).

5. **Pattern detection recomputation**: `detect_patterns()` clears and recomputes all patterns each time it's called. For walk-forward analysis where the same instrument's patterns are needed across multiple windows, this is redundant work.

6. **No Ichimoku displacement in backtest**: The Ichimoku indicator outputs current senkou values; displacement (shifting values 26 periods forward) is delegated to the frontend. In backtesting, this means cloud comparison triggers use current-bar cloud values, not the displaced ones. A comment in the code notes: "Displacement (plotting 26 periods ahead) is handled by the frontend."

## Invariants

1. **All outputs are `Decimal`** -- never `f64` in the output HashMap
2. **Missing data returns empty HashMap** -- indicators NEVER panic or return partial results; they return `{}` until warmup is complete
3. **`reset()` restores to construction state** -- after `reset()`, the indicator behaves exactly as if newly constructed
4. **`indicator_type()` matches `IndicatorType::as_str()`** -- the string returned by the trait method must match the enum's `as_str()` for the corresponding variant
5. **Parameter defaults are defined in `create_indicator()`** -- if a param is missing from config, the factory provides a sensible default (e.g., period=20 for SMA, period=14 for RSI)
6. **`IndicatorEngine` is the sole factory** -- all indicator instantiation in the strategy pipeline goes through `create_indicator()`. Direct instantiation is only used in ai-analysis (documented as tech debt)
