# Backtest Core Domain

Strategy backtesting simulation engine, walk-forward analysis, parameter optimization, and rules-based signal evaluation.

## Owned Files

```
src-tauri/src/backtest/engine.rs
src-tauri/src/backtest/walk_forward.rs
src-tauri/src/backtest/optimizer.rs
src-tauri/src/backtest/rules_engine.rs
src-tauri/src/backtest/rules_types.rs
src-tauri/src/backtest/rules_triggers.rs
src-tauri/src/backtest/rules_strategy.rs
src-tauri/src/backtest/strategy.rs
src-tauri/src/backtest/strategies.rs
src-tauri/src/backtest/rules_engine_tests.rs
src-tauri/src/backtest/mod.rs
src-tauri/src/commands/backtest.rs
```

## Shared Files (Coordinate with Other Domains)

| File | Other Domain | Coordination Notes |
|------|-------------|-------------------|
| `shared/src/lib.rs` | `shared` | All strategy type definitions (`StrategyDefinition`, `Trigger`, `DataSource`, `RiskSettings`, `ParameterDefinition`, etc.) live here. Changes to these types require coordinating with the MCP server and frontend. This domain is a consumer; never add backtest-runtime types here. |
| `src-tauri/src/backtest/indicators.rs` | `indicators` | Indicator calculation implementations. This domain calls indicators through `IndicatorEngine` but does not own the calculation logic. |
| `src-tauri/src/backtest/indicator_engine.rs` | `indicators` | Orchestrates indicator lifecycle (create, update, query). This domain depends on `IndicatorEngine::from_config_with_params()`, `on_candle()`, `get_output()`, `get_latest()`, and `get_snapshot()`. |
| `src-tauri/src/backtest/pivots.rs` | `indicators` | Pivot point calculations. This domain consumes pivot results but the computation logic is owned by `indicators`. |
| `src-tauri/src/backtest/regime_detector.rs` | `indicators` | Market regime detection. This domain consumes regime results but the detection logic is owned by `indicators`. |
| `src-tauri/src/models/mod.rs` | `models` | `Candle`, `Ohlc` types used throughout this domain. |
| `src-tauri/src/oanda/endpoints.rs` | `oanda` | Candle fetching functions (`get_candles`, `get_candles_paginated`) called by command handlers. |

## Primary Languages and Frameworks

- **Rust** (all core logic)
- **Tauri 2** (command handler interface in `commands/backtest.rs`)
- `rust_decimal` for all financial arithmetic
- `rayon` for parallel optimization
- `serde` / `serde_json` for strategy definition serialization
- `chrono` for time handling in walk-forward windows

## Key Dependencies

### External Crates
- `rust_decimal` / `rust_decimal_macros` -- mandatory for all price/P&L/metric values
- `rayon` -- parallel grid search in optimizer
- `serde` / `serde_json` -- strategy JSON parsing
- `chrono` -- DateTime operations in walk-forward windowing
- `tracing` -- structured logging throughout

### Internal Modules
- `shared` crate -- all strategy schema types (consumed, not owned)
- `crate::models::Candle` / `Ohlc` -- market data primitives
- `super::indicator_engine::IndicatorEngine` -- indicator computation layer
- `super::pivots` -- pivot point calculations (owned by `indicators` domain)
- `super::regime_detector` -- market regime detection (owned by `indicators` domain)
- `crate::AppState` -- Tauri application state (wf_cancel_token, OANDA client)
- `candlesight_lib::notifications` -- system notifications for job completion
- `candlesight_lib::QueriesServiceClient` -- job tracking via queries-service
