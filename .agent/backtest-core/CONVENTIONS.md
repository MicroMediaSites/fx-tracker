# Backtest Core Conventions

## Naming Conventions

### Structs and Enums
- Configuration types: `*Config` (e.g., `BacktestConfig`, `WalkForwardConfig`, `OptimizationConfig`)
- Result types: `*Result` (e.g., `BacktestResult`, `WalkForwardResult`, `OptimizationResult`)
- Metric types: `*Metrics` (e.g., `BacktestMetrics`, `OptimizationMetrics`)
- Frontend-facing types in commands: `*Data` (e.g., `BacktestResultData`, `TradeData`, `BacktestDebugData`)

### Functions
- Entry points: `run_*` (e.g., `run_walk_forward`, `run_optimization`, `run_backtest_with_strategy`)
- Evaluation: `evaluate_*` (e.g., `evaluate_entry_rules_v2`, `evaluate_condition`, `evaluate_trigger_v2`)
- Resolution: `resolve_*` (e.g., `resolve_data_source_v2`, `resolve_parameterized`, `resolve_variable_expression`)
- Calculation: `calculate_*` (e.g., `calculate_stop_loss`, `calculate_position_size`, `calculate_metrics`)

### Trigger-Related Methods
- Trigger evaluators follow the pattern `evaluate_{trigger_type}_v2` (e.g., `evaluate_cross_v2`, `evaluate_compare_v2`, `evaluate_threshold_v2`)
- Position-context variants add `_with_position` suffix (e.g., `evaluate_trigger_v2_with_position`)
- Live-trading variants add `_live` suffix (e.g., `evaluate_exit_rules_live`, `evaluate_entry_rules_live`)

### V2 Suffix Convention
Methods with `_v2` suffix indicate the V2 schema evaluation path (conditions-based). The V1 path (trigger_chain-based) still exists as fallback but is deprecated. New code should always use the V2 path.

## Error Handling Patterns

### Strategy Parsing
Strategy creation returns `Result<Self, String>`. Errors are descriptive and include the parse failure reason:
```rust
RulesBasedStrategy::from_json(json)
    .map_err(|e| format!("Failed to parse strategy JSON: {}", e))?;
```

### Command Handlers
Command handlers validate inputs early and return `Result<T, String>`. Instrument format, date format, and JSON parsing are validated before any OANDA API calls. Walk-forward commands track job state and call `fail_job()` on any error path:
```rust
if !is_valid_instrument(&instrument) {
    let err = format!("Invalid instrument format: {}", instrument);
    fail_job(&job_id, &err);
    return Err(err);
}
```

### Optimizer Failures
When a parameter combination fails to create a valid strategy, the optimizer returns a dummy run with `score: f64::NEG_INFINITY` rather than propagating the error. This prevents one bad combination from aborting the entire grid search.

### Walk-Forward Cancellation
Cancellation uses an `Arc<AtomicBool>` token checked at the start of each window iteration. When set, the function returns `Err("Walk-forward analysis cancelled")`. The command handler distinguishes cancellation from actual errors when reporting job status.

## Testing Patterns

### Test Helper Functions
Tests use shared helper functions defined at the top of test modules:
- `create_test_candle(price, time_offset)` -- creates a Candle with predictable OHLC from a close price
- `create_simple_strategy()` -- returns a minimal `StrategyDefinition` with SMA crossover entry and R:R exit
- `create_trending_candles()` -- generates 50 candles with uptrend then downtrend
- `create_threshold_trigger(value, threshold, above)` -- creates a Threshold trigger for condition evaluation tests

### Test Structure
Tests are organized by concern:
- `engine.rs` tests: basic backtest execution, metric calculation, SL/TP enforcement, SL-over-TP priority
- `rules_engine_tests.rs`: engine creation, balance tracking, signal conversion, disabled conditions, grouped AND/OR evaluation
- `rules_strategy.rs` tests: JSON parsing, PriceSource format validation, StopLossSource parsing
- `optimizer.rs` tests: parameter range generation, combination counting, cartesian product
- `walk_forward.rs` tests: month arithmetic, window generation (rolling and anchored), drawdown calculation

### What to Test
- Every public function that performs calculation or evaluation
- Edge cases: zero division guards, empty inputs, boundary conditions
- Parsing: both valid and invalid JSON formats (especially DataSource format gotchas)
- Signal correctness: SL/TP enforcement, deferred execution timing

### Expected Coverage
- `engine.rs`: SL/TP enforcement, spread simulation, position sizing, metric calculations
- `rules_engine.rs` + `rules_triggers.rs`: every trigger type, grouped AND/OR logic, negation, disabled conditions
- `optimizer.rs`: grid generation, combination counting, parameter extraction
- `walk_forward.rs`: window boundary calculation, anchored vs rolling modes

## Code Organization Patterns

### File Split Strategy
The rules engine is split across three files:
- `rules_engine.rs` -- RulesEngine struct, core methods (on_candle, open_position, stop loss calculation, pivot updates, capture/trailing logic)
- `rules_triggers.rs` -- V2 evaluation methods (entry/exit rule evaluation, trigger chain grouping, individual trigger evaluators, data source resolution)
- `rules_types.rs` -- re-exports from shared crate plus runtime-only types (`CapturedValue`, `PositionState`)

This split happened because the rules engine exceeded the 500-600 line threshold. The trigger evaluation methods are in a separate file but are `impl RulesEngine` blocks -- they are conceptually part of the same struct.

### pub(crate) Visibility
Methods used by `rules_triggers.rs` (which is a sibling module) but not intended for external callers use `pub(crate)` visibility:
- `resolve_data_source_v2`
- `resolve_variable_expression`
- `calculate_sl_tp_for_signal`
- `open_position`
- `get_atr_value`
- `evaluate_entry_rules_v2`
- `evaluate_entry_rules_v2_with_position`

### Command Handler Pattern
Each Tauri command follows this structure:
1. Input validation (instrument format, date format)
2. JSON parsing (strategy, parameters, S/R zones, pivot config)
3. Candle fetching from OANDA (with date range pagination)
4. Strategy creation and configuration (pip value, zones, pivots)
5. Run backtest/optimization/walk-forward
6. Result transformation for frontend (Decimal to String, camelCase renaming)

## Anti-Patterns

### DO NOT use f64 for prices, P&L, or balance calculations
All financial arithmetic must use `rust_decimal::Decimal`. The only acceptable f64 usage is in `ParameterDefinition` (min/max/step/default) and in `WalkForwardResult` aggregate metrics that are already serialized as strings. If you need to do math on a value that will affect trade P&L, it must be Decimal.

### DO NOT execute signals on the current candle's close
The engine defers all signals to the next candle's open. If you add a new signal type or execution path, it MUST go through the `pending_signal` mechanism. Executing at close is look-ahead bias.

### DO NOT use `{"type": "price", ...}` in DataSource
DataSource is an UNTAGGED serde enum. Price sources use `{"source": "price", "value": "close"}`, not `{"type": "price", ...}`. The `type` key would match the wrong variant. This is a common mistake for MCP clients and is tested explicitly in `rules_strategy.rs`.

### DO NOT confuse DataSource (untagged) with StopLossSource (tagged)
`StopLossSource` IS a tagged enum and DOES use `{"type": "indicator", "indicator": "...", "output": "..."}`. `DataSource` in triggers is untagged. Mixing these formats up is a parsing error that produces confusing serde messages.

### DO NOT run CPU-intensive synchronous work directly in async command handlers
Tauri command handlers are `async fn` that run on the tokio runtime. Running tight loops with `Decimal` arithmetic (backtest engine, optimization grid search) directly in an `async fn` blocks the tokio worker thread, starving other concurrent async tasks (price streaming, Zero sync, auth). Always wrap CPU-intensive work in `tokio::task::spawn_blocking()`. The pattern:
```rust
let result = tokio::task::spawn_blocking(move || {
    // CPU-intensive work here
    run_backtest_with_strategy(&mut strategy, &candles, ...)
})
.await
.map_err(|e| format!("Task failed: {}", e))?;
```

### DO NOT send unbounded chart data to the frontend
Equity curves, candlestick series, and other chart data should be downsampled before sending over IPC. Use `downsample_equity_curve()` (LTTB algorithm) with `MAX_EQUITY_CURVE_POINTS = 500`. For multi-year H1 backtests (~6,500 candles), the raw equity curve is 13x larger than needed for visual accuracy.

### DO NOT add database migrations to Rust code
All schema migrations go in `queries-service/src/migrate.ts`. The Rust backend only runs queries. This is enforced by CI.

### DO NOT modify position state in evaluation methods
Evaluation methods (`evaluate_*`) should be `&self` (immutable borrow). Only `on_candle`, `on_candle_live`, `open_position`, and `reset` should mutate engine state. The entry evaluation has two variants: `evaluate_entry_rules_v2` (pure evaluation, `&self`) and `evaluate_entry_rules_v2_with_position` (opens position, `&mut self`). Use the correct one.

### DO NOT trust indicator-sourced stop losses without side validation
When the stop loss comes from an `Indicator` or `Variable` source, the returned price level is absolute — it may be on either side of entry depending on market conditions. Always validate that `SL < entry` for longs and `SL > entry` for shorts before opening a position. `FixedPips` and `Percent` sources are safe by construction because they offset from the close price. This validation is enforced in `calculate_sl_tp_for_signal()` (returns `None` for invalid SL) and `open_position()` (returns without creating position).

### DO NOT skip the `prepare()` call before backtesting
`Strategy::prepare()` must be called with the full candle array before the simulation loop. Without it, `RegimeDetector` patterns are not detected and all price-action regime triggers return false. The engine calls this automatically in `run()`.

### DO NOT assume pip value is always 0.0001
JPY pairs use 0.01, gold (XAU) uses 0.01, silver (XAG) uses 0.001. Always call `set_pip_value_for_instrument()` or use `shared::get_pip_value()` before running any backtest. Missing this call produces wildly wrong position sizes and SL distances for non-standard instruments.

### DO NOT use the broken listen() cleanup pattern in useEffect
Tauri's `listen()` returns a Promise. The following pattern ORPHANS listeners:
```typescript
// WRONG - unlisten is still null when cleanup runs
useEffect(() => {
  let unlisten = null;
  const setup = async () => { unlisten = await listen(...); };
  setup();
  return () => { if (unlisten) unlisten(); };
}, [deps]);
```
Use the cancelled-flag pattern instead:
```typescript
// RIGHT - cancelled flag ensures cleanup across async boundary
useEffect(() => {
  let cancelled = false;
  let unlistenFn = null;
  listen(...).then((fn) => {
    if (cancelled) { fn(); } else { unlistenFn = fn; }
  });
  return () => {
    cancelled = true;
    if (unlistenFn) unlistenFn();
  };
}, [deps]);
```
This was the root cause of BUG-044 (UI stutter after backtest). Orphaned listeners accumulated and continued processing events, causing progressive CPU consumption.
