# Indicators Domain

Technical indicator calculations, indicator orchestration engine, market regime detection, and pivot point calculations for the CandleSight trading application.

## Owned Files

```
src-tauri/src/backtest/indicators.rs
src-tauri/src/backtest/indicator_engine.rs
src-tauri/src/backtest/regime_detector.rs
src-tauri/src/backtest/pivots.rs
```

Note: `candle_utils.rs` does not exist as a standalone file. Candle utility functions (`is_bullish()`, `is_bearish()`, `range()`, `body_size()`) live on the `Candle` model in `src-tauri/src/models/candle.rs`, which is owned by the models domain.

Note: These files physically live under `src-tauri/src/backtest/` but are logically owned by the indicators domain. The `backtest-core` OWNERSHIP.md currently also lists `pivots.rs` and `regime_detector.rs` -- this is a known discrepancy that should be resolved in favor of indicators domain ownership, since the calculation logic is the concern of this domain while backtest-core is purely a consumer.

## Shared Files (Coordinate with Other Domains)

| File | Other Domain | Coordination Notes |
|------|-------------|-------------------|
| `shared/src/lib.rs` | shared | Owns `IndicatorType` enum, `IndicatorConfig`, `ParameterizedValue`, `MarketRegime`, `PivotLevel`, `PivotPeriod`, `DataSource` variants. This domain CONSUMES these types. Adding a new indicator requires adding to the enum here first. Coordinate with MCP server and frontend on changes. |
| `src-tauri/src/backtest/mod.rs` | backtest-core | Re-exports from this domain (`indicators::*`, `IndicatorEngine`, `OutputHistory`, `PivotLevels`, `RegimeDetector`, etc.). Any new public types must be added to the re-exports. |
| `src-tauri/src/backtest/rules_engine.rs` | backtest-core | Primary consumer of `IndicatorEngine`, `RegimeDetector`, `PivotPeriodTracker`, and `PivotLevels`. The `RulesEngine` struct holds instances of these types. |
| `src-tauri/src/backtest/rules_triggers.rs` | backtest-core | Evaluates triggers by calling `IndicatorEngine::get_output()`, `get_latest()`, `can_detect_cross()`, and `RegimeDetector::is_regime_active()`. |
| `src-tauri/src/analysis/enrichment.rs` | ai-analysis | Directly instantiates `RsiIndicator`, `AtrIndicator`, `SmaIndicator`, `EmaIndicator` to calculate market context at trade entry. |
| `src-tauri/src/analysis/trade_review.rs` | ai-analysis | Same direct indicator usage for single-trade review analysis. |
| `src/types/strategy.ts` | frontend | TypeScript mirror of `IndicatorType`, `INDICATOR_OUTPUTS`, `INDICATOR_DEFAULTS`, `INDICATOR_METADATA`. Must stay in sync with Rust enum. |
| `src/components/charts/chartConstants.ts` | charting | `INDICATOR_COLORS` and `AVAILABLE_INDICATORS` for chart rendering. |
| `docs/patterns/adding-indicators.md` | documentation | Step-by-step guide for adding new indicators across the full stack. |

## Consumer Domains

| Domain | What It Consumes | How |
|--------|-----------------|-----|
| **backtest-core** | `IndicatorEngine`, `RegimeDetector`, `PivotPeriodTracker`, `PivotLevels` | `RulesEngine` owns instances, calls `on_candle()`, `get_output()`, `get_latest()`, `is_regime_active()`, `detect_patterns()` |
| **strategy-monitor** | `IndicatorEngine` (via `RulesEngine`) | Live candle processing uses same indicator pipeline as backtesting |
| **charting** | Indicator type metadata, output names | Frontend renders indicator overlays/oscillators using `INDICATOR_METADATA`, `INDICATOR_OUTPUTS` |
| **ai-analysis** | Individual indicator structs directly | `enrichment.rs` and `trade_review.rs` instantiate `SmaIndicator`, `EmaIndicator`, `RsiIndicator`, `AtrIndicator` directly (bypassing `IndicatorEngine`) |
| **mcp-server** | `IndicatorType` enum, `IndicatorConfig` (via shared crate) | Strategy validation and creation through the MCP server uses the shared types |

## Primary Languages and Frameworks

- **Rust** (all calculation logic)
- `rust_decimal` / `rust_decimal_macros` for all financial arithmetic (never f64)
- `chrono` for timestamp handling (day boundaries in ADR/Daily, pivot period tracking)
- `serde` for serialization of `PivotLevels`, `PivotConfig`, `RegimeConfig`
- `shared` crate for type-safe enums (`IndicatorType`, `PivotLevel`, `PivotPeriod`, `MarketRegime`)

## Key Dependencies

### External Crates
- `rust_decimal` -- ALL price and indicator values are `Decimal`, never `f64`
- `rust_decimal_macros` -- `dec!()` macro for decimal literals
- `chrono` -- `DateTime<Utc>`, `Datelike` for day-of-year detection (ADR, Daily, pivots)
- `serde` / `serde_json` -- serialization for pivot levels and configs

### Internal Modules
- `crate::models::Candle` / `Ohlc` -- the primary input type for all indicator calculations
- `shared::IndicatorType` -- type-safe enum defining all supported indicators
- `shared::IndicatorConfig` -- configuration for indicator instantiation with parameterized values
- `shared::MarketRegime` -- enum of all detectable market conditions
- `shared::PivotLevel` / `shared::PivotPeriod` -- pivot point enums
- `super::rules_engine::SRZone` -- S/R zone type consumed by `RegimeDetector` (re-exported from shared)
