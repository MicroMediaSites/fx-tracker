# Backtest Core Architecture

## Component Overview

The backtest-core domain has five major components arranged in a layered architecture:

```
commands/backtest.rs          (Tauri command handlers - entry points from frontend)
        |
        v
walk_forward.rs / optimizer.rs  (Orchestration layer - windowing, grid search)
        |
        v
engine.rs                       (Simulation loop - order execution, P&L, equity)
        |
        v
rules_strategy.rs               (Strategy adapter - bridges RulesEngine to Strategy trait)
        |
        v
rules_engine.rs + rules_triggers.rs  (Signal generation - evaluates triggers against indicators)
        |
        v
indicator_engine (owned by indicators domain)
```

## Data Flow

1. **Frontend** invokes a Tauri command (`run_custom_backtest`, `run_walk_forward`, `run_parameter_sweep`).
2. **Command handler** fetches candles from OANDA, parses strategy JSON into `StrategyDefinition`, and sets up configuration.
3. For **walk-forward**: windows are generated from date range, then for each window the optimizer runs grid search on the training slice, followed by an OOS backtest on the test slice.
4. For **optimization**: all parameter combinations are generated as a cartesian product, then each is backtested in parallel via `rayon::par_iter`.
5. The **BacktestEngine** iterates over candles, calling `strategy.on_candle_extended()` each bar. Signals are _deferred_ to the next candle's open price (no look-ahead bias). Intra-bar SL/TP checks happen on the current candle's high/low.
6. The **RulesBasedStrategy** wraps `RulesEngine`, translating `RulesSignal` into the `Strategy` trait interface.
7. The **RulesEngine** updates indicators, evaluates entry/exit conditions against the trigger tree, manages internal position state (for backtesting), and computes SL/TP from risk settings.

## Multi-Timeframe (MTF) Processing

The backtest engine supports strategies that use indicators on different timeframes than the primary trading timeframe. The flow:

1. **Extraction**: `mtf::extract_htf_timeframes(&strategy_definition)` scans indicator configs and rule data sources for any `timeframe` field that differs from the primary timeframe.
2. **Fetching**: Command handlers (`run_custom_backtest`, `run_walk_forward`, etc.) call `fetch_htf_candles()` which makes parallel OANDA API calls per higher timeframe.
3. **Storage**: `MtfCandleStore` (in `mtf.rs`) holds pre-fetched HTF candles per timeframe, with index-based advancement.
4. **Engine Integration**: `RulesEngine` maintains `htf_indicator_engines: HashMap<String, IndicatorEngine>` — one indicator engine per higher timeframe. When `set_mtf_candle_store()` is called, it seeds each HTF indicator engine with the pre-fetched candles.
5. **Advancement**: On each primary-timeframe candle, `advance_htf_engines()` checks if the primary candle's timestamp has crossed a higher-timeframe boundary. If so, it feeds the newly completed HTF candle to the corresponding HTF indicator engine.
6. **Rule Evaluation**: Triggers with `timeframe` on their data source resolve against the HTF indicator engine instead of the primary one.

For walk-forward analysis, `MtfCandleStore::filter_by_time_range()` slices HTF candles to each training/test window, ensuring no look-ahead bias across window boundaries.

## Key Design Decisions

### 1. Deferred Execution (Next-Bar Open)

Signals generated on candle N are executed at candle N+1's open price. This is implemented via a `pending_signal` variable in the engine loop. The reason: executing at the close of the signal candle would be look-ahead bias since you cannot know the close price until the candle completes. Executing at the next open is the most realistic simulation of a market order.

### 2. SL Priority Over TP on Same-Bar Breach

When both stop loss and take profit are breached on the same candle, stop loss wins. This is a conservative assumption because intra-bar order of price movement is unknown. Assuming the worst case protects against overfitting to favorable bar sequences.

### 3. Spread Simulation via Bid/Ask Reconstruction

The engine simulates spread by reconstructing bid/ask from mid prices. Longs enter at ASK (mid + half spread), exit at BID (mid - half spread). Shorts do the reverse. This avoids the need for separate bid/ask candle feeds while still accounting for the cost of crossing the spread.

### 4. Strategy Trait as Abstraction Boundary

The `Strategy` trait (`on_candle`, `on_candle_extended`, `prepare`, `reset`, `current_stop_loss`, `current_take_profit`) is the contract between the engine and any strategy implementation. `BacktestEngine::run` is generic over `S: Strategy`. This means the engine has zero knowledge of rules, indicators, or trigger evaluation. The `RulesBasedStrategy` adapter makes JSON-defined strategies compatible with this interface.

### 5. Rules Engine Dual Mode: Backtest vs Live

`RulesEngine` has two processing paths:
- `on_candle()` -- backtest mode. Tracks internal `PositionState`, manages captured values, trailing stops, partial exits.
- `on_candle_live()` -- live mode. Takes `position_direction: Option<PositionDirection>` from the broker. Does NOT maintain internal position state. Time-based triggers are unavailable in live mode (logged as warning once).

This separation exists because in backtesting the engine IS the order management system, but in live trading the broker's position state is the source of truth.

### 6. Grouped AND/OR Condition Evaluation

Trigger chains use a grouping model: AND operators split the chain into groups, OR operators keep triggers in the same group. Groups are AND'd together; triggers within a group are OR'd. Example: `A OR B AND C` = `(A OR B) AND (C)`. This gives users an intuitive way to express "any of these conditions must hold" combined with "all of these broader requirements must hold."

### 7. Parameter Resolution via ParameterizedValue

Strategy values (indicator periods, RR ratios, risk values, thresholds) can be either fixed numbers or parameter references (`{"$param": "id"}`). `ParameterizedValue` is resolved at runtime against the resolved parameter map. This powers optimization: the optimizer creates `RulesBasedStrategy::from_json_with_params()` with different values for each grid point, without re-parsing the strategy structure.

### 8. Grid Search with Rayon (No Shared State)

Optimization uses exhaustive grid search parallelized via `rayon::par_iter`. Each thread creates its own `RulesBasedStrategy` and `BacktestEngine` instance. There is no shared mutable state between parallel runs. This eliminates synchronization overhead and makes the optimizer trivially correct.

### 9. Walk-Forward Window Generation

Two modes exist:
- **Rolling**: training window slides forward by `step_months`. Each window has the same training duration.
- **Anchored**: training always starts at data_start but the end expands forward. Each subsequent window trains on more data.

Window boundaries use calendar months (via `add_months()`) rather than fixed durations, so a "6-month training window" is always 6 calendar months regardless of month lengths.

### 10. Decimal Arithmetic Everywhere

All financial values use `rust_decimal::Decimal`. The engine implements its own `decimal_pow`, `decimal_ln`, `decimal_exp`, and `decimal_sqrt` via Taylor series / Newton's method to avoid converting to f64 for metric calculations (Sharpe ratio, annualized return). The profit factor is capped at `999.99` when there are no losing trades to avoid infinity.

The ONLY place f64 appears is in `OptimizationMetrics` and `WalkForwardResult` where values are serialized as strings for the frontend, and in `ParameterDefinition` where parameter ranges (min/max/step) are f64 by design from the shared crate.

## Invariants That Must Never Be Broken

1. **No f64 for financial values** in engine.rs, rules_engine.rs, or any P&L/price calculation path. Use `rust_decimal::Decimal` exclusively.
2. **Deferred execution** -- signals MUST execute on the next candle's open, never on the current candle's close.
3. **SL priority** -- when both SL and TP breach on the same candle, SL always wins.
4. **Strategy::reset()** must be called before each backtest run. The engine calls this in `run()`.
5. **Strategy::prepare()** must be called with full candle data before the loop. This is required for price action pattern detection in `RegimeDetector`.
6. **Only complete candles** are processed (`candle.complete == true` check in engine loop).
7. **No database migrations** in this domain. All schema changes go through `queries-service/src/migrate.ts`.
8. **Pip value must be set per instrument** before running any backtest. JPY pairs use 0.01, gold uses 0.01, standard forex uses 0.0001. The `set_pip_value_for_instrument()` call handles this.

## Known Technical Debt

1. **Decimal math functions**: `decimal_pow`, `decimal_ln`, `decimal_exp` use Taylor series with 50 iterations. This is accurate enough for financial metrics but is slower than necessary. Could use a dedicated math crate if performance becomes an issue.

2. **OptimizationMetrics stores values as Strings**: These were originally Decimal but were converted to String for JSON serialization to the frontend. The walk-forward aggregation then parses them back to f64 for calculations. This round-trip is lossy and fragile. A cleaner approach would be to keep Decimal internally and only convert to String at the Tauri command boundary.

3. **Legacy trigger_chain support**: Both `EntryRule` and `ExitRule` have both `conditions: Vec<Condition>` (V2) and `trigger_chain: Option<TriggerChain>` (V1). The V2 format is preferred and the V1 path is marked as deprecated but still evaluated as a fallback. This could be cleaned up once all existing strategies are migrated to V2.

4. **Duplicate candle filtering logic** in command handlers: The date range handling and candle fetching logic is repeated across `run_custom_backtest`, `run_backtest_debug`, `optimize_strategy`, and `run_walk_forward`. This should be extracted into a shared helper.

5. **Progress callback is unused in optimizer**: `run_optimization` accepts a `progress_callback` parameter but ignores it in parallel mode because rayon's `par_iter` doesn't support per-iteration callbacks cleanly. Progress is only reported at the walk-forward window level.

6. **Price history O(n) removal**: `self.price_history.remove(0)` is O(n). For 100-element cap this is acceptable, but if the cap increases significantly, this should use `VecDeque`.
