# Charting Interfaces

This domain is primarily a **consumer** of data from other domains. It exposes very little to the rest of the application beyond its barrel exports in `src/components/charts/index.ts`, which are used by the backtest domain (`EquityCurveChart`, time utilities, chart types).

## Tauri Commands Consumed

These commands are invoked via `invoke()` from `@tauri-apps/api/core`. The chart domain does not own their implementation.

| Command | Called From | Parameters | Response Type | Owner Domain |
|---------|-----------|------------|---------------|--------------|
| `get_candles` | `ChartApp.loadCandles()` | `{ instrument, granularity, count, from?, to? }` | `CandleData[]` | `oanda-trading` |
| `get_indicator_data` | `ChartApp` (2 call sites: strategy load + user indicators) | `{ instrument, granularity, count, from?, to?, indicatorsJson }` | `IndicatorSeries[]` | `indicators` |
| `subscribe_to_prices` | `usePriceStreaming` | `{ instrument }` | `void` | `oanda-trading` |
| `unsubscribe_from_prices` | `usePriceStreaming` (cleanup) | `{ instrument }` | `void` | `oanda-trading` |
| `place_order` | `useTradeExecution` | `{ instrument, units, stopLoss?, takeProfit? }` | `{ filled, price?, units, instrument, realized_pl?, trade_id?, error? }` | `oanda-trading` |
| `broadcast_match_executed` | `useTradeExecution` | `{ matchId }` | `void` | `strategy-monitor` |
| `calculate_pivot_points` | `useSRZones.handleImportPivots()` | `{ instrument, timeframe }` | `PivotLevel[]` | `backtest-core` |

## Tauri Events Consumed

Events are received via `listen()` from `@tauri-apps/api/event`.

| Event | Consumed By | Payload Type | Emitter Domain |
|-------|-----------|--------------|----------------|
| `price-update` | `usePriceStreaming` | `PriceUpdate` (`{ instrument, bid, ask, spread, time, tradeable }`) | `oanda-trading` |
| `stream-error` | `usePriceStreaming` | `{ errorType: string, message: string }` | `oanda-trading` |

## Zero Queries Consumed

Reactive queries via `useQuery()` from `@rocicorp/zero/react`.

| Query | Called From | Parameters | Returns | Notes |
|-------|-----------|------------|---------|-------|
| `myStrategyById` | `ChartApp` | `(userID, strategyId)` | `strategy[]` (0 or 1) | Used to load strategy indicator configs when chart opens with `?strategyId=...` |
| `mySRZonesByInstrument` | `useSRZones` | `(userID, instrument)` | `sr_zone[]` | Reactive -- zones update in real-time as they are created, edited, or deleted |

## Zero Mutations Produced

Mutations via `zero.mutate` from the desktop Zero context.

| Entity | Operation | Called From | Notes |
|--------|-----------|-----------|-------|
| `sr_zone` | `insert` | `useSRZones.handleSaveZone()`, `useSRZones.handleImportPivots()` | Creates new zones (user-drawn or pivot-imported) |
| `sr_zone` | `update` | `useSRZones.handleUpdateZone()` | Updates zone label, color, or boundary prices |
| `sr_zone` | `delete` | `useSRZones.handleDeleteSRZone()`, `useSRZones.handleClearAllZones()` | Deletes individual zones or all zones for an instrument |
| `strategy_trade` | `insert` | `useTradeExecution.executeTrade()` | Links an OANDA trade to a strategy after execution |

## Data Types Consumed from Other Domains

### From `types/strategy.ts` (strategy-monitor / backtest-core)

```typescript
IndicatorType          // Union type of all indicator IDs ('sma' | 'ema' | 'rsi' | ...)
INDICATOR_METADATA     // Metadata per indicator (label, fullName, isOverlay, params)
INDICATOR_DEFAULTS     // Default parameter values per indicator type
OVERLAY_INDICATOR_TYPES // Array of indicator types that render as chart overlays
OUTPUT_LABELS          // Human-readable labels for indicator output names
```

### From `contexts/` (auth-security)

```typescript
useDesktopAuthStatus() -> { userID }     // Used in ChartApp for Zero query context
useDesktopAuth()       -> { user }       // Used in useSRZones and useTradeExecution for user_id
useDesktopZero()       -> Zero instance  // Used for mutations
```

### From `hooks/useEntitlements.ts` (membership-payments)

```typescript
canAccess('chart-indicators') -> boolean  // Gates indicator feature in IndicatorMenu
```

## Exports Consumed by Other Domains

### By backtest-core / trade-analysis

The barrel export at `src/components/charts/index.ts` exposes:

```typescript
// Components
EquityCurveChart         // Used by backtest result display
TradeLegend              // Could be used by trade analytics
IndicatorLegend          // Used when showing strategy indicators

// Types (widely consumed)
PriceUpdate              // Shared price update payload type
IndicatorSeries          // Backend indicator result format
IndicatorConfig          // Frontend-to-backend indicator request format
ChartIndicatorConfig     // User-configurable indicator with custom colors
OHLCData                 // Open/High/Low/Close display values
PivotLevel               // Pivot point calculation result
CandleData               // Raw candle from backend
TimeMapState             // Time mapping state structure

// Utilities
getGranularitySeconds    // Convert granularity string to seconds
getInstrumentPrecision   // Get decimal places for an instrument
formatIndicatorLabel     // Format indicator config as "SMA(20)" style label
generateIndicatorId      // Generate unique indicator instance IDs
createTimeMapState       // Factory for time mapping state
convertCandles           // Convert raw candles to business time
toBusinessTime           // Single timestamp conversion

// Constants
INDICATOR_COLORS         // Default colors per indicator output
OVERLAY_INDICATORS       // Which indicator types are overlays
AVAILABLE_INDICATORS     // Pre-configured indicator definitions
INITIAL_VISIBLE_CANDLES  // How many candles to show on load (60)
FUTURE_CANDLE_SLOTS      // Empty right-side slots (30)
```

## URL Parameters (Chart Window Input)

The chart window receives its configuration via URL parameters and localStorage. These are parsed once on mount by `useChartParams`.

| Parameter | Type | Source | Description |
|-----------|------|--------|-------------|
| `instrument` | string | URL | FX pair (e.g., `EUR_USD`). Defaults to `EUR_USD` |
| `granularity` | string | URL | Timeframe (e.g., `H1`, `H4`, `D`). Defaults to `H1` |
| `count` | number | URL | Number of candles to fetch. Defaults to `5000` |
| `from` | string | URL | ISO date for historical range start |
| `to` | string | URL | ISO date for historical range end |
| `strategyId` | string | URL | Strategy ID to load indicators from |
| `signalDirection` | `'long'` / `'short'` | URL | Trade signal direction (shows Execute button) |
| `signalId` | string | URL | Pattern match ID (for broadcasting execution) |
| `stopLoss` | string | URL | Stop loss price for trade execution |
| `takeProfit` | string | URL | Take profit price for trade execution |
| `entryPrice` | string | URL | Entry price to draw as a horizontal line |
| `positionSize` | string | URL | Units for trade execution |
| `indicators` | JSON string | URL | Pre-selected indicator types (from AI analysis) |
| `trades` | JSON string | URL | Trade data for overlay (from Trade Analysis single trade) |
| `chart_trades` | JSON string | localStorage | Bulk trade data for overlay (from BacktestApp, avoids URL length limits) |
| `chart_parameter_overrides` | JSON string | localStorage | Walk-forward parameter overrides (shown in purple bar) |

localStorage entries are consumed-and-deleted (read once, then removed to prevent stale data on future chart opens).
