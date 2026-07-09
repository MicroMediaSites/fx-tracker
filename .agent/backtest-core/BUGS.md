# Backtest Core -- Bug Log

All bug fixes in this domain must be logged here with full detail.

## Template

```
## [DATE] Short description
- **Symptom**: What was observed
- **Root Cause**: Why it happened
- **Fix**: What was changed
- **Prevention**: Pattern to follow going forward to avoid this class of bug
```

## [2026-02-15] Missing SL in backtest results and missing Ichimoku displacement

- **Symptom**: Backtest trades were showing no stop loss values, and Ichimoku cloud was not displaced correctly.
- **Root Cause**: The stop loss from the strategy was not being propagated through the `ExtendedSignal` into `SimulatedTrade`, and the Ichimoku indicator was not applying the displacement parameter.
- **Fix**: Commit `b46423b` fixed both issues -- SL is now captured in the extended signal path and the Ichimoku displacement is applied during indicator calculation.
- **Prevention**: When adding new fields to `ExtendedSignal` or `SimulatedTrade`, always verify the field flows through the entire chain: `RulesEngine` -> `RulesSignal` -> `ExtendedSignal` -> `SimulatedTrade`. Add a test that asserts the value appears in the final trade output.

## [2026-02-15] Missing risk settings in trading

- **Symptom**: Live trading was not using the strategy's risk settings for position sizing.
- **Root Cause**: The risk settings from the `StrategyDefinition` were not being passed through to the order execution path.
- **Fix**: Commit `588ac72` ensured risk settings are used in the trading command path.
- **Prevention**: Any time a new field is added to `RiskSettings`, verify it is consumed in both the backtest engine path AND the live trading path. These are separate code paths that can diverge.

## [Pre-2026] JSON round-trip import fails without id/user_id

- **Symptom**: Pasting exported strategy JSON back into the Strategy Builder failed validation.
- **Root Cause**: JSON export strips database metadata fields (`id`, `user_id`, `version`, `is_active`), but `StrategyDefinition` deserialization requires them.
- **Fix**: `validate_strategy_json()` in `commands/backtest.rs` now injects placeholder values for missing metadata fields before parsing (BUG-042).
- **Prevention**: When adding required fields to `StrategyDefinition`, always update `validate_strategy_json()` to provide defaults for import. Better yet, make new fields `Option<T>` with `#[serde(default)]` so they are never required for parsing.

## [2026-02-16] BUG-069: Walk-forward ignores parameterized risk settings (verified fixed)

- **Symptom**: Risk parameters (e.g., position size, stop loss) defined as `ParameterizedValue::Reference` in strategy risk settings were reportedly not applied during walk-forward simulation.
- **Root Cause**: Originally, SL was not propagated through the `ExtendedSignal` chain (fixed in b46423b) and risk settings were missing from the trading path (fixed in 588ac72).
- **Fix**: Prior commits resolved this. Verified 2026-02-16 by tracing the full pipeline: `RulesEngine::with_params()` populates `resolved_params` from all strategy parameters, and every risk setting accessor (`get_rr_ratio`, `calculate_stop_loss`, `get_risk_value`, `get_spread_buffer_pips`) calls `resolve_parameterized()` against that map. Each optimizer iteration and OOS test creates a fresh strategy instance with its own resolved params. No unresolved code paths remain.
- **Prevention**: The `resolve_parameterized()` pattern is the single gateway for all parameterized risk values. If adding a new parameterized field to `RiskSettings`, always access it via `self.resolve_parameterized(&field)` — never read the `ParameterizedValue` directly.

## [2026-03-01] BUG-044/BUG-067: UI stutter and high CPU after running backtest

- **Symptom**: After running a backtest or walk-forward analysis, the UI became sluggish with typing delays that worsened over time. CPU usage remained elevated even after the backtest completed. Restarting the app was the only way to restore normal performance.
- **Root Cause**: Race condition in async Tauri event listener cleanup. Every `listen()` call in the frontend uses Tauri's async IPC, which returns a Promise resolving to an unlisten function. The cleanup pattern used in multiple backtest hooks was:
  ```typescript
  useEffect(() => {
    let unlisten = null;
    const setup = async () => { unlisten = await listen(...); };
    setup();
    return () => { if (unlisten) unlisten(); };  // BUG: unlisten is still null
  }, [deps]);
  ```
  When the effect re-fired (due to dependency changes like `strategyId` or `callbacks`), the cleanup function ran synchronously before the async `listen()` resolved. Since `unlisten` was still `null`, the old listener was never removed. Each backtest run or strategy switch accumulated orphaned event listeners that continued processing `job-heartbeat`, `job-completed`, `walk-forward-progress`, `parameter-sweep-progress`, and `optimization-progress` events, causing progressive CPU consumption and UI stutter.
- **Fix**: Changed all affected listener patterns to use a `cancelled` flag that persists across the async boundary:
  ```typescript
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
  For the `useBacktestJob.ts` hook, also used refs to guarantee cleanup of prior listeners before creating new ones. Removed `strategyName` and `zero.mutate.backtest_job` from dependency arrays where they were causing unnecessary listener churn.
  Files changed: `useBacktestJob.ts`, `useWalkForwardState.ts`, `BacktestApp.tsx`, `OptimizationPanel.tsx`, `WalkForwardPanel.tsx`.
- **Prevention**: When using Tauri's `listen()` in a React `useEffect`, ALWAYS use the cancelled-flag pattern. The `listen()` call is async (returns a Promise), so the synchronous cleanup function cannot safely capture the unlisten function via a local variable. The cancelled-flag pattern ensures cleanup happens even when the Promise resolves after the effect has been torn down. Note: This same antipattern exists in non-backtest files (TradeAnalysisApp, TradingTicketApp, EnvironmentBadge, WindowHeader, useVault) which should be fixed as a follow-up.

## [2026-03-01] BUG-067: Persistent high CPU after simple historical backtests on macOS

- **Symptom**: After a simple historical backtest completes, macOS Activity Monitor shows persistent high CPU usage from the CandleSight process that does not subside. The bug report describes "background process or thread not being cleaned up properly."
- **Root Cause**: Two contributing factors:
  1. **Tokio runtime blocking**: All backtest commands (`run_backtest`, `run_custom_backtest`, `run_backtest_debug`, `optimize_strategy`) were `async fn` handlers that performed heavy synchronous CPU work (iterating thousands of candles with `rust_decimal` arithmetic) directly on the tokio async runtime's worker threads. This starved other concurrent async tasks (price streaming, Zero sync, auth token refresh) which then executed in bursts after the blocking work completed, causing sustained CPU pressure.
  2. **Unbounded equity curve**: The equity curve contained one `EquityPoint` per candle processed. For multi-year H1 backtests (~6,500+ candles), this large dataset was serialized over IPC and rendered by `lightweight-charts`, whose internal canvas rendering loop consumed significant CPU proportional to data size.
- **Fix**:
  1. Wrapped all synchronous backtest computation in `tokio::task::spawn_blocking()` to run on dedicated blocking threads, freeing the async runtime for other tasks.
  2. Added LTTB (Largest-Triangle-Three-Buckets) downsampling of equity curves to a maximum of 500 points, preserving visual shape while reducing rendering overhead by ~13x for typical backtests.
- **Prevention**: Any new Tauri command that performs CPU-intensive synchronous work (more than a few milliseconds) must use `tokio::task::spawn_blocking()`. Never run tight loops with `Decimal` arithmetic directly in an `async fn` handler. For chart data, always consider the maximum dataset size and downsample before sending to the frontend.

## [2026-03-14] Indicator-sourced stop loss on wrong side of entry produces guaranteed-profit trades

- **Symptom**: A strategy using swing low as the stop loss source for long trades on M1 charts showed 88% win rate, 0% max drawdown, 159/160 trades entering and exiting on the same candle, and all trades exiting at "Stop Loss" with ~$10 profit. The charted exit price visually extended above the candle's actual price range.
- **Root Cause**: When RSI dips below the threshold on M1 candles, price has typically already fallen below the recent swing low. The `calculate_stop_loss()` function in `rules_engine.rs` returned the indicator value (swing low) without validating whether it was on the correct side of the entry price. For longs, the SL must be below entry; for shorts, above. When the swing low was above entry, the backtester immediately hit the "stop loss" on the same candle — but since SL was above entry for a long, the exit was profitable. The `FixedPips` and `Percent` SL sources were unaffected because they compute the SL relative to entry by construction.
- **Fix**: Added validation in two places:
  1. `calculate_sl_tp_for_signal()` (rules_engine.rs) — now returns `Option<(Decimal, Decimal)>` instead of `(Decimal, Decimal)`. Returns `None` when SL is on the wrong side of entry, causing the signal to be suppressed (trade skipped).
  2. `open_position()` (rules_engine.rs) — validates SL side before creating `PositionState`. Returns early without setting `self.position` if invalid.
  3. Both callers in `rules_triggers.rs` (`evaluate_entry_rules_v2` and `evaluate_entry_rules_v2_with_position`) updated to handle the `Option` return and skip trades with invalid SL.
- **Prevention**: When adding a new `StopLossSource` variant that derives its value from external data (indicators, variables, S/R zones), always validate that the computed SL is on the correct side of the entry price: `SL < entry` for longs, `SL > entry` for shorts. The `FixedPips` and `Percent` variants are safe by construction because they offset from the close price in the correct direction. Indicator and Variable variants pull absolute price levels that may be on either side of entry depending on market conditions — these MUST be validated.

<!-- Add new entries above this line -->
