# Charting Conventions

## Component Organization

### File Layout

The `src/components/charts/` directory follows a flat structure with clear naming:

- **`index.ts`** -- Barrel file that re-exports all public components, types, helpers, and constants. Every consumer imports through this barrel. When adding a new export, add it here.
- **`chart*.ts`** -- Shared infrastructure: `chartTypes.ts` (interfaces), `chartConstants.ts` (config values and lookup tables), `chartTimeUtils.ts` (time mapping logic)
- **`indicator*.ts`** -- Indicator rendering: `indicatorRenderer.ts` (render/clear logic), `indicatorHelpers.ts` (ID generation, label formatting, category maps)
- **`*Plugin.ts`** -- Lightweight Charts primitives (canvas-level drawing): `TradeOverlayPlugin.ts`, `IchimokuCloudPlugin.ts`
- **`*.tsx`** -- React components: `ChartHeader`, `LivePriceDisplay`, `IndicatorMenu`, `IndicatorConfigModal`, `SRZoneOverlay`, `SRZoneEditor`, `SRToolsMenu`, `TradeLegend`, `IndicatorLegend`, `EquityCurveChart`

### Hooks

Chart-specific hooks live in `src/hooks/` (not in the charts directory):
- `useChartParams.ts` -- URL parameter parsing, run once on mount
- `useSRZones.ts` -- All S/R zone state, CRUD operations, and editing workflow
- `usePriceFlash.ts` -- Price change direction detection with timed reset
- `useTradeExecution.ts` -- Trade placement, strategy linking, window auto-close
- `usePriceStreaming.ts` -- Real-time price subscription (shared with other domains)

### Store

`src/stores/chartIndicatorStore.ts` -- Zustand store for user-selected indicators, persisted to `localStorage` under key `candlesight-chart-indicators`.

## Lightweight Charts Plugin Pattern

All custom canvas drawing follows the three-class pattern from the Lightweight Charts `ISeriesPrimitive` API:

```
Plugin (implements ISeriesPrimitive<Time>)
  |-- attached(param) -- receives series + chart references, stores requestUpdate
  |-- detached() -- cleans up references
  |-- updateAllViews() -- delegates to PaneView.update()
  |-- paneViews() -- returns [paneView]
  |
  +-- PaneView (implements IPrimitivePaneView)
       |-- setSeriesAndChart() -- stores references for coordinate conversion
       |-- update() -- converts data model to view coordinates using series.priceToCoordinate() and timeScale.timeToCoordinate()
       |-- renderer() -- returns the Renderer instance
       |
       +-- Renderer (implements IPrimitivePaneRenderer)
            |-- update(viewData) -- stores pre-computed view coordinates
            |-- draw(target: CanvasRenderingTarget2D) -- performs actual canvas drawing in bitmap coordinate space
```

When creating a new plugin:
1. Always use `target.useBitmapCoordinateSpace()` for drawing to handle DPR scaling
2. Scale all coordinates by `horizontalPixelRatio` and `verticalPixelRatio`
3. Clear plugin data in `detached()` to prevent ghost renders
4. The Renderer should only draw; all coordinate math belongs in the PaneView

## State Management Patterns

### Refs for Imperative Objects

Chart API objects, series references, and accumulated data that should never trigger React re-renders:

```typescript
const chartRef = useRef<IChartApi | null>(null);
const candleSeriesRef = useRef<ISeriesApi<any> | null>(null);
const indicatorSeriesRef = useRef<Map<string, ISeriesApi<any>>>(new Map());
const candlesDataRef = useRef<CandleData[]>([]);
const timeMapStateRef = useRef<TimeMapState>(createTimeMapState());
```

**Why**: A single streaming price tick calls `candleSeriesRef.current.update()`. If `candleSeries` were state, this would re-render the entire ChartApp component 2-4 times per second, cascading through all children.

### Ref Mirrors for Event Handlers

When an event handler (crosshair callback, mouse handler) needs the latest value of state or a Zustand store, keep a ref mirror:

```typescript
const selectedIndicatorRef = useRef<{ id: string; label: string } | null>(null);
selectedIndicatorRef.current = selectedIndicator; // sync on every render

const chartIndicatorsRef = useRef<ChartIndicatorConfig[]>([]);
chartIndicatorsRef.current = chartIndicators; // sync on every render
```

**Why**: The crosshair callback is registered once during chart initialization. Without the ref mirror, it would close over the initial value of `selectedIndicator` and `chartIndicators`, never seeing updates.

### React State for Display Values Only

Only values that need to appear in JSX use `useState`:

```typescript
const [hoveredCandle, setHoveredCandle] = useState<OHLCData | null>(null);
const [error, setError] = useState<string | null>(null);
const [loading, setLoading] = useState(false);
const [indicatorMenuOpen, setIndicatorMenuOpen] = useState(false);
```

### Direct DOM Manipulation for High-Frequency Updates

The indicator hover label updates via ref to avoid re-renders on every mouse move:

```typescript
const indicatorLabelRef = useRef<HTMLDivElement>(null);
// In crosshair handler:
indicatorLabelRef.current.textContent = displayIndicator.label;
indicatorLabelRef.current.style.display = 'block';
```

### Zustand Store for Cross-Session Persistence

Indicator selections persist via the Zustand store with `persist` middleware. The store is the single source of truth for which indicators are active. ChartApp reads from the store and triggers re-computation whenever it changes.

## Performance Patterns

### Avoid Re-renders During Streaming

The golden rule: a streaming price update should NEVER cause `ChartApp` to re-render. The data path is:
1. Tauri event -> `usePriceStreaming` -> `setCurrentPrice()` (this triggers a re-render of `LivePriceDisplay` only)
2. Tauri event -> `usePriceStreaming` -> `updateCurrentCandle()` -> `candleSeriesRef.current.update()` (imperative, no re-render)

### ResizeObserver Over Window Resize

The chart uses a `ResizeObserver` on the container div rather than a `window.resize` listener. This handles both window resizing and layout changes (e.g., when a sibling panel collapses) with better performance.

### requestAnimationFrame for Canvas Redraws

`SRZoneOverlay` batches redraws using `requestAnimationFrame` when the chart scrolls or zooms, canceling the previous frame request if a new one arrives before paint.

### Guard Against Empty Series

Streaming candle updates include multiple guards:
```typescript
// 1. Check series exists
if (!candleSeriesRef.current) return;
// 2. Check series has data (avoids "Value is null" error during transitions)
const seriesData = candleSeriesRef.current.data();
if (!seriesData || seriesData.length === 0) return;
// 3. Wrap update in try/catch for transitional states
try { candleSeriesRef.current.update(candle); } catch { /* ignore */ }
```

## UI Patterns Specific to Charting

### Color System

Chart colors are hardcoded hex values matching the design tokens from `styles.css`:
- Background: `#0e1117` (matches `--color-bg-page`)
- Grid lines: `#1a1f26` (matches `--color-bg-elevated`)
- Borders: `#2d333b` (matches `--color-border`)
- Text: `#9ca3af`
- Candle up: transparent body, `#22c55e` border/wick (hollow green candles)
- Candle down: `#ef4444` filled body and wick

**Why hex instead of CSS variables**: Lightweight Charts requires hex color strings at initialization. CSS custom properties cannot be resolved by the library's internal canvas renderer.

### Price Precision

`getInstrumentPrecision()` returns the correct decimal places per instrument:
- Standard FX pairs: 5 decimals (pipettes)
- JPY pairs: 3 decimals
- XAU (gold): 2 decimals
- XAG (silver): 3 decimals

This is used for candle series `priceFormat`, price line display, and zone boundary formatting.

### Indicator Colors

Defined in `INDICATOR_COLORS` in `chartConstants.ts`. Each indicator output has a named key (e.g., `'bollinger.upper'`, `'ichimoku.tenkan'`). Custom user colors override these defaults.

### Indicator Label Formatting

`formatIndicatorLabel()` produces compact human-readable labels:
- Single param: `SMA(20)`
- Multi-param: `MACD(12,26,9)`, `Bollinger(20,2)`, `Ichimoku(9,26,52,26)`

### Live Price Display Priority

`LivePriceDisplay` follows a strict display priority:
1. **Hovered candle OHLC** (highest -- always wins if mouse is over a candle)
2. **Historical View label** (when `from`/`to` date range is set)
3. **Streaming bid/ask with spread** (normal live mode)
4. **Loading skeleton** (price not yet available)

### Price Flash

`usePriceFlash` compares string prices (not floats, to catch sub-pip changes) and returns `'up'`, `'down'`, or `'neutral'`. The direction resets to `neutral` after a configurable duration (default 500ms). `getPriceColorClass()` maps direction to Tailwind color classes.

## Testing Expectations

The charting domain has one test file: `LivePriceDisplay.test.tsx`. Tests cover:
- Display mode priority (OHLC > historical > streaming > loading)
- Streaming indicator dot color
- Instrument-specific precision (EUR_USD = 5 decimals, USD_JPY = 3 decimals)
- Transition from OHLC back to streaming (regression test for BUG-037)

Plugin and renderer code is currently untested due to the imperative canvas API. New UI components added to this domain should have basic rendering tests following the `LivePriceDisplay.test.tsx` pattern.

## Anti-Patterns

### Never Store Chart API Objects in React State

```typescript
// WRONG -- will cause re-renders on every chart interaction
const [chart, setChart] = useState<IChartApi | null>(null);

// RIGHT -- imperative object stays in a ref
const chartRef = useRef<IChartApi | null>(null);
```

### Never Pass Actual Timestamps to Lightweight Charts

```typescript
// WRONG -- will create weekend gaps
candleSeries.setData(candles.map(c => ({
  time: new Date(c.time).getTime() / 1000 as Time,
  ...
})));

// RIGHT -- convert through time mapping
const chartData = convertCandles(candles, timeMapStateRef.current);
candleSeries.setData(chartData);
```

### Lightweight Charts Series Z-Order Is Determined by Insertion Order

Series added later to the chart render on top of series added earlier. There is no explicit `zOrder` property for series within the same pane. When an oscillator indicator has both histogram and line outputs (e.g., MACD), always add the histogram series BEFORE line series so lines remain visible above the bars.

```typescript
// WRONG — histogram added last, renders on top of lines
for (const output of indSeries.outputs) { // ["macd", "signal", "histogram"]
  if (output === 'histogram') chart.addSeries(HistogramSeries, ...);
  else chart.addSeries(LineSeries, ...);
}

// RIGHT — partition and add histograms first
const histogramOutputs = outputs.filter(o => o === 'histogram');
const lineOutputs = outputs.filter(o => o !== 'histogram');
for (const output of [...histogramOutputs, ...lineOutputs]) { ... }
```

### Never Create S/R Zones Without Oscillator Pane Protection

When handling clicks or mouse events for zone creation, always validate that the Y position is within the main price chart range. Without this check, clicking on an RSI or MACD pane would create zones at nonsensical prices.

### Never Add Indicator Types Without Updating Multiple Files

Adding a new indicator requires changes in:
1. `types/strategy.ts` -- `INDICATOR_METADATA`, `INDICATOR_DEFAULTS`, `IndicatorType`
2. `chartConstants.ts` -- `INDICATOR_COLORS`, `AVAILABLE_INDICATORS`
3. `indicatorHelpers.ts` -- `INDICATOR_TYPES_BY_CATEGORY`, possibly `formatIndicatorLabel` switch case
4. `IndicatorConfigModal.tsx` -- `COLORABLE_OUTPUTS` (if the indicator has multi-output lines)
5. `indicatorRenderer.ts` -- any special rendering logic (like Ichimoku displacement or Chandelier direction filtering)

### Never Mutate TimeMapState From Multiple Hooks

`timeMapStateRef` is owned by `ChartApp` and passed to `usePriceStreaming`. Only the streaming hook extends the map for new candles. The `convertCandles` utility rebuilds it from scratch on candle load. There must be exactly one writer at a time.
