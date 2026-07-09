# Charting Architecture

## Chart Window Lifecycle

The chart runs as a standalone Tauri window. Its entry point is `ChartApp.tsx`, loaded when the Tauri backend opens a new window routed to the chart path.

### Initialization Sequence

1. **URL parameter parsing** (`useChartParams`) -- On mount, reads `window.location.search` and `localStorage` to extract instrument, granularity, date range, trades, strategy ID, signal info, and parameter overrides. The hook is memoized so parsing happens exactly once.

2. **Chart object creation** -- A `useEffect([], ...)` creates the Lightweight Charts instance, attaches it to a container div, configures styling (hex colors matching CSS custom properties), sets up the crosshair subscriber, and starts a `ResizeObserver`. The chart object and candle series are stored in refs, not state, because they are imperative objects that should never trigger re-renders.

3. **Candle loading** (`loadCandles`) -- Invokes the Tauri `get_candles` command, converts raw candle data through the time mapping system, and calls `candleSeriesRef.current.setData()`. If a strategy is loaded, also fetches indicator data and renders overlays.

4. **Price streaming** (`usePriceStreaming`) -- Subscribes to real-time `price-update` events for the current instrument. Updates the current candle in-place via the Lightweight Charts `update()` API. Handles new candle creation when the streaming price crosses a candle boundary.

5. **S/R zone loading** (`useSRZones`) -- Queries Zero for `sr_zone` records matching the current instrument. Zones render in a separate canvas overlay that redraws on every chart scroll/zoom.

### Why Refs Over State for Chart Objects

The chart API is entirely imperative. Storing `IChartApi`, `ISeriesApi`, `Map<string, ISeriesApi>`, and `CandleData[]` in React state would trigger re-renders on every price tick, candle load, and indicator update. Instead, these live in refs:

- `chartRef` -- The Lightweight Charts instance
- `candleSeriesRef` -- The primary candlestick series
- `indicatorSeriesRef` -- Map of indicator ID to their line/histogram series
- `ichimokuCloudRef` -- The Ichimoku cloud plugin (attached as a primitive)
- `tradeOverlayRef` -- The trade marker plugin
- `candlesDataRef` -- Raw candle data for context building and reference
- `timeMapStateRef` -- The business time mapping tables
- `chartIndicatorsRef` -- Mirror of Zustand indicator state, kept in sync for use in the crosshair handler without closure staleness

Only user-facing display values use React state: `hoveredCandle`, `selectedIndicator`, `error`, `loading`.

## Time Mapping System

### The Problem: Weekend Gaps

FX markets close Friday evening and reopen Sunday evening. Lightweight Charts renders time as a linear axis, so weekends create large empty gaps that waste screen real estate and distort visual analysis.

### The Solution: Business Time

`chartTimeUtils.ts` implements a bijective mapping between "actual time" (UTC timestamps) and "business time" (sequential timestamps with constant intervals):

1. **On candle load** -- `convertCandles()` iterates through candles, calculates the median interval between consecutive candles (ignoring gaps > 3 days), and assigns each candle a sequential business time starting from the first candle's actual time and incrementing by the median interval.

2. **Two maps are maintained** in `TimeMapState`:
   - `timeMap: Map<actualTime, businessTime>` -- for converting indicator/trade timestamps
   - `reverseTimeMap: Map<businessTime, actualTime>` -- for the time axis formatter to display real dates

3. **On streaming** -- When a new candle arrives that is not in the time map, the streaming hook extends the map by adding `lastBusinessTime + typicalInterval`.

4. **Time axis display** -- The chart's `localization.timeFormatter` intercepts business time values, looks up the actual time in `reverseTimeMap`, and formats it as a human-readable date.

### Invariant

Every timestamp rendered on the chart MUST go through the time mapping system. Actual Unix timestamps must never be passed directly to Lightweight Charts series data. Trade overlay times, indicator times, and S/R zone drawing all use `toBusinessTime()` for coordinate conversion.

## Indicator Rendering Pipeline

### Data Flow

1. **User adds indicator** via `IndicatorMenu` -> Zustand store (`chartIndicatorStore`) adds a `ChartIndicatorConfig` entry
2. **Store change triggers effect** in `ChartApp.tsx` -> converts configs to `IndicatorConfig[]` -> invokes `get_indicator_data` Tauri command
3. **Rust backend** computes indicator values from candle data, returns `IndicatorSeries[]` with time-stamped output values
4. **`renderIndicators()`** in `indicatorRenderer.ts`:
   - Clears all existing indicator series from the chart
   - For each indicator result, determines if it is an overlay (drawn on price pane) or oscillator (drawn in a separate pane)
   - Creates `LineSeries` or `HistogramSeries` with appropriate colors, line styles, and pane indices
   - Special handling for Ichimoku: creates a `IchimokuCloudPlugin` primitive for the cloud fill area, plus individual line series for each component with time offsets for displacement
   - Special handling for Chandelier: filters outputs based on signal direction (only shows relevant trailing stop)

### Overlay vs. Oscillator

The distinction is driven by `OVERLAY_INDICATOR_TYPES` from `types/strategy.ts`:
- **Overlay** indicators (SMA, EMA, Bollinger, Ichimoku, Chandelier, Donchian, MA Bands) render on pane 0 (the price chart)
- **Oscillator** indicators (RSI, MACD, Stochastic, ATR, ADR, MFI, ADX, DSS, MA Histogram) render in separate panes below the price chart, auto-assigned incrementing pane indices

### Indicator Persistence

The Zustand store uses `zustand/middleware/persist` with `localStorage` key `candlesight-chart-indicators`. Indicator configs survive browser refreshes and window reopens. The store tracks `_hasHydrated` for safe async rehydration.

### Custom Colors

Each indicator config can have a `colors` map (output name -> hex color). The `renderIndicators` function checks custom colors first, then falls back to `INDICATOR_COLORS` defaults in `chartConstants.ts`.

## Custom Plugins (Lightweight Charts Primitives)

Two plugins implement the `ISeriesPrimitive<Time>` interface for canvas-level drawing:

### TradeOverlayPlugin (`TradeOverlayPlugin.ts`)

Draws trade entry/exit markers on the chart:
- Entry: colored circle (green for long, red for short) with white border
- Exit: colored square with white border
- Connecting line from entry to exit
- Filled rectangle showing trade duration and price range
- P/L label near the exit marker
- Colors are profit/loss based (green/red), not direction-based

Architecture: `TradeOverlayPlugin` -> `TradeOverlayPaneView` -> `TradeOverlayRenderer`. The renderer operates in bitmap coordinate space (scaled by device pixel ratio).

### IchimokuCloudPlugin (`IchimokuCloudPlugin.ts`)

Draws the Ichimoku cloud fill between Senkou Span A and Senkou Span B:
- Segments the cloud at crossover points where the bullish/bearish direction changes
- Bullish segments: muted green fill (`rgba(118, 168, 126, 0.4)`)
- Bearish segments: muted red fill (`rgba(194, 120, 120, 0.4)`)
- Has a `clear()` method called on detachment to prevent stale renders

Both plugins follow the same three-class pattern: Plugin (manages lifecycle) -> PaneView (transforms data to coordinates) -> Renderer (draws to canvas).

## S/R Zone System

### Data Layer

Zones are stored in PostgreSQL via Zero sync. The `useSRZones` hook queries `mySRZonesByInstrument`, filters by user and instrument, and returns typed `SRZone[]` objects with numeric prices (parsed from string storage).

### Drawing Modes

1. **Zone creation** -- User clicks "Zones > Custom" to enter editing mode. First click sets one boundary (shown as a price line), second click sets the other boundary. Shift+click creates a single-line zone (both boundaries equal). A preview zone shows between clicks.

2. **Zone selection** -- Clicking inside an existing zone selects it, showing edit (pencil) and delete (X) buttons rendered on the canvas.

3. **Edge resizing** -- Hovering near a zone edge (within 6px) shows a `cursor-ns-resize`. Dragging resizes the zone edge in real-time with a preview, committing on mouse up.

4. **Pivot import** -- "Daily Pivots" / "Weekly Pivots" calculates pivot points via the `calculate_pivot_points` Tauri command and inserts them as thin zones with colored labels (yellow for pivot, red for resistance, green for support).

### Rendering Architecture

`SRZoneOverlay` is a separate `<canvas>` element positioned absolutely over the chart container with `pointerEvents: 'none'`. This allows chart pan/zoom to work normally while the zones are drawn on top. The canvas redraws on:
- Data changes (zones added/removed/updated)
- Chart scroll/zoom (subscribed to `visibleTimeRangeChange` and `visibleLogicalRangeChange`)
- Uses `requestAnimationFrame` batching during scroll to prevent excessive redraws

The overlay handles its own DPR scaling for crisp rendering on high-DPI displays.

### Oscillator Pane Protection

Both the zone drawing preview and click handlers validate that the mouse Y position corresponds to a price within the visible candle range (with 20% buffer). This prevents accidental zone creation when the user clicks on an oscillator pane below the main chart.

## Trade Execution Flow

When the chart opens with signal parameters (`signalDirection`, `strategyId`, `signalId`, `stopLoss`, `takeProfit`, `positionSize`), an "Execute" button appears in the header. The `useTradeExecution` hook handles:

1. Calls `place_order` Tauri command with instrument, units (direction-adjusted), SL, and TP
2. On success, creates a `strategy_trade` record via Zero to link the OANDA trade to the strategy
3. Broadcasts `broadcast_match_executed` to notify the strategy monitor window
4. Shows success toast, then auto-closes the chart window after 1.5 seconds

## Crosshair and Indicator Hover

The crosshair subscriber (set up in chart initialization) does two things:

1. **OHLC display** -- Extracts candle data from `param.seriesData.get(candleSeries)` and sets `hoveredCandle` state for the `LivePriceDisplay` component.

2. **Indicator hover detection** -- Iterates through `indicatorSeriesRef.current`, checks each series for data at the crosshair time, converts the value to a Y coordinate via `priceToCoordinate()`, and finds the closest indicator line within a 10px threshold. Updates the indicator label DOM node directly via ref (not state) to avoid re-renders on every mouse move.

Clicking on a hovered indicator "selects" it (label becomes sticky). Clicking the label opens `IndicatorConfigModal` for editing.

## Known Technical Debt

1. **ChartApp.tsx is ~1070 lines** -- The main component handles chart initialization, candle loading, indicator management, S/R zone drawing, trade execution, and crosshair handling. The hooks (`useSRZones`, `usePriceStreaming`, `useTradeExecution`) extracted some logic, but the mouse event handlers and chart setup remain monolithic.

2. **eslint-disable for exhaustive-deps** -- Several `useEffect` and `useCallback` hooks suppress the exhaustive-deps rule because they intentionally use refs instead of state dependencies. This is a deliberate performance choice, not a bug, but it makes the dependency graph harder to reason about.

3. **Type safety gaps** -- Multiple `ISeriesApi<any>` refs and `any`-typed price line refs exist because the Lightweight Charts API's generic types are difficult to thread through the component hierarchy.

4. **EquityCurveChart inconsistency** -- Uses different styling (gray-700 backgrounds) than the main chart window. Lives in this domain but is consumed by the backtest domain.

5. **Canvas-based SRZoneOverlay hit detection** -- Edit/delete button clicks are detected by manual distance calculations in `handleChartClick`, duplicating the position logic from the canvas renderer. These two sources of truth can drift.
