# Charting Domain

Standalone chart window for candlestick rendering, real-time price streaming, technical indicator overlays, support/resistance zone management, trade markers, and one-click trade execution from signals. This domain is flagged as **fragile** due to heavy imperative canvas manipulation, mutable ref-based state, and tight coupling to the Lightweight Charts API.

## Owned Files

```
src/ChartApp.tsx
src/components/charts/ChartHeader.tsx
src/components/charts/EquityCurveChart.tsx
src/components/charts/IchimokuCloudPlugin.ts
src/components/charts/IndicatorConfigModal.tsx
src/components/charts/IndicatorLegend.tsx
src/components/charts/IndicatorMenu.tsx
src/components/charts/LivePriceDisplay.tsx
src/components/charts/LivePriceDisplay.test.tsx
src/components/charts/SRToolsMenu.tsx
src/components/charts/SRZoneEditor.tsx
src/components/charts/SRZoneOverlay.tsx
src/components/charts/TradeLegend.tsx
src/components/charts/TradeOverlayPlugin.ts
src/components/charts/chartConstants.ts
src/components/charts/chartTimeUtils.ts
src/components/charts/chartTypes.ts
src/components/charts/index.ts
src/components/charts/indicatorHelpers.ts
src/components/charts/indicatorRenderer.ts
src/hooks/useChartParams.ts
src/hooks/useSRZones.ts
src/hooks/usePriceFlash.ts
src/hooks/useTradeExecution.ts
src/stores/chartIndicatorStore.ts
```

## Shared Files (Coordinate with Other Domains)

| File | Other Domain | Coordination Notes |
|------|-------------|-------------------|
| `src/hooks/usePriceStreaming.ts` | `oanda-trading`, `strategy-monitor` | Real-time price subscription via Tauri events. Chart consumes `price-update` events and calls `subscribe_to_prices`/`unsubscribe_from_prices`. Multiple charts share one stream via the backend's PriceStreamManager. |
| `src/types/strategy.ts` | `backtest-core`, `strategy-monitor` | `IndicatorType`, `INDICATOR_METADATA`, `INDICATOR_DEFAULTS`, `OVERLAY_INDICATOR_TYPES`, `OUTPUT_LABELS` are all consumed from here. This domain never modifies strategy type definitions. |
| `src/queries.ts` | `data-infrastructure` | Uses `myStrategyById` and `mySRZonesByInstrument` Zero queries. These queries are defined in the shared queries file. |
| `src/constants.ts` | `desktop-shell` | `GRANULARITIES` constant is consumed for timeframe picker options. |
| `src/contexts/DesktopZeroContext.tsx` | `data-infrastructure`, `auth-security` | Uses `useDesktopZero()` and `useDesktopAuthStatus()` for Zero mutations (S/R zones) and user identity. |
| `src/contexts/DesktopAuthContext.tsx` | `auth-security` | Uses `useDesktopAuth()` for user identity in trade execution and zone CRUD. |
| `src/hooks/useEntitlements.ts` | `membership-payments` | `IndicatorMenu` checks `canAccess('chart-indicators')` to gate the indicator feature behind a paid tier. |
| `src/contexts/PricingModalContext.tsx` | `membership-payments` | `IndicatorMenu` calls `openPricingModal()` when a non-premium user tries to add indicators. |
| `src/lib/chatContextBuilder.ts` | `ai-analysis` | `buildChartingContext()` constructs the AI terminal context from chart state (instrument, candles, indicators). |
| `src/lib/terminalWelcome.ts` | `ai-analysis` | `getTerminalWelcome('charting', ...)` generates welcome text for the AI terminal in the chart window. |
| `src/components/ui/WindowHeader.tsx` | `desktop-shell` | Shared window header with settings, AI terminal, and navigation controls. |
| `src/components/ui/SymbolPicker.tsx` | `desktop-shell` | Shared instrument picker used in `ChartHeader`. |
| `src/components/ui/Combobox.tsx` | `desktop-shell` | Shared combobox used for granularity selection in `ChartHeader`. |
| `src/stores/settingsStore.ts` | `desktop-shell` | `ChartHeader` reads `mySymbols` from the shared settings store for the symbol picker. |

## Primary Languages and Frameworks

- **React 19** (all UI components)
- **TypeScript** (strict mode)
- **Lightweight Charts v4** (`lightweight-charts` + `fancy-canvas`) for candlestick/line/histogram rendering and custom primitives
- **Tailwind CSS v4** with CSS custom properties for theming
- **Zustand** (persisted store for indicator configurations)
- **Zero** (`@rocicorp/zero`) for S/R zone and strategy data sync

## Key Dependencies

### NPM Packages
- `lightweight-charts` -- TradingView's charting library; provides `createChart`, `CandlestickSeries`, `LineSeries`, `HistogramSeries`, and the `ISeriesPrimitive` plugin interface
- `fancy-canvas` -- Canvas rendering helper used by Lightweight Charts plugins (`CanvasRenderingTarget2D`)
- `@tauri-apps/api/core` -- `invoke()` for Tauri command calls (`get_candles`, `get_indicator_data`, `subscribe_to_prices`, `place_order`, etc.)
- `@tauri-apps/api/event` -- `listen()` for `price-update` and `stream-error` events
- `@tauri-apps/api/window` -- `getCurrentWindow()` for window title and close operations
- `@rocicorp/zero/react` -- `useQuery()` for reactive S/R zone and strategy queries
- `zustand` + `zustand/middleware` -- Persisted indicator store with `localStorage` backend

### Tauri Backend Commands (consumed, not owned)
- `get_candles` -- Fetch OHLC candle data for instrument/granularity/date range
- `get_indicator_data` -- Compute indicator series from candle data
- `subscribe_to_prices` / `unsubscribe_from_prices` -- Start/stop real-time price streaming
- `place_order` -- Execute a trade via OANDA
- `broadcast_match_executed` -- Notify other windows that a signal was traded
- `calculate_pivot_points` -- Compute daily/weekly pivot support/resistance levels
