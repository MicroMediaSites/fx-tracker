# Charting Domain - Bug Tracker

## Active Bugs

_No active bugs tracked._

## Resolved Bugs

| ID | Description | Resolution | Date |
|----|-------------|------------|------|
| BUG-066 | Indicator lines render behind histogram bars in oscillator panes | Reordered oscillator output rendering: histogram series added before line series | 2026-03-01 |
| BUG-CHART-072 | Chart overlays don't refresh on new candle | Added onNewCandle callback from streaming to re-fetch indicator data | 2026-02-15 |

### BUG-066: Indicator lines render behind histogram bars in oscillator panes

**Severity**: Low
**Reported**: 2026-02-04
**Status**: Resolved (2026-03-01)

**Symptoms**: When displaying an oscillator indicator that has both line and histogram outputs (e.g., MACD with macd/signal lines and histogram bars), the histogram bars render on top of the lines, obscuring them. Most visible when MACD histogram bars cover the signal and MACD lines at their intersection points.

**Root Cause**: In `indicatorRenderer.ts`, the oscillator rendering loop iterated through `indSeries.outputs` in the order returned by the Rust backend (e.g., MACD returns `["macd", "signal", "histogram"]`). Lightweight Charts renders series in the order they are added to the chart -- later series draw on top of earlier ones. For MACD, this meant:
1. `macd` line added first (bottom layer)
2. `signal` line added second
3. `histogram` added last (top layer, obscuring lines)

**Reproduction Steps**:
1. Open a chart window
2. Add a MACD indicator (or any oscillator with both line and histogram outputs)
3. Observe the MACD/signal lines are partially hidden behind histogram bars
4. Expected: lines should be visible above histogram bars

**Fix**: Partitioned the outputs into histogram outputs and line outputs before iterating. Histogram outputs are added to the chart first (rendering as the bottom layer), then line outputs are added after (rendering on top). The reordering uses a simple filter-and-concat: `[...histogramOutputs, ...lineOutputs]`.

Files changed:
- `src/components/charts/indicatorRenderer.ts` -- Reordered oscillator output rendering: histogram first, lines second
- `src/components/charts/indicatorRenderer.test.ts` -- New test file with 4 tests verifying rendering order

**Regression Test**: Unit test `indicatorRenderer.test.ts` verifies that `chart.addSeries()` is called with `HistogramSeries` before `LineSeries` for MACD and other mixed-output oscillators. Also verifies RSI (line-only) and pane index assignment are unaffected.

### BUG-CHART-072: Chart overlays don't refresh on each new candle

**Severity**: Medium
**Reported**: 2026-02-15
**Status**: Resolved

**Symptoms**: Indicator overlays (Ichimoku confirmed, likely all others too) remain frozen at their initial load state and never update as new candles form during live streaming. The candle series updates correctly but indicators stay static.

**Root Cause**: The streaming path in `usePriceStreaming` only calls `candleSeriesRef.current.update()` to push new candle data to the chart. Indicator series are populated by `get_indicator_data` (a Tauri backend command) during initial `loadCandles()` or when the user changes indicator selections. When a new candle boundary is crossed during streaming, no code existed to re-fetch and re-render indicator data.

**Reproduction Steps**:
1. Open a live chart with any instrument
2. Add an indicator (e.g., SMA, Ichimoku)
3. Wait for a new candle to form (or use a small granularity like M1)
4. Expected: indicator lines extend with the new candle
5. Actual: indicator lines remain frozen at their state from initial load

**Fix**:
- Added an optional `onNewCandle` callback to `usePriceStreaming` (stored via ref to avoid stale closures)
- The callback fires when a new candle boundary is crossed (not on every tick, and not on the first tick after load)
- In `ChartApp.tsx`, the callback re-invokes `get_indicator_data` and does a full clear+render cycle for indicator series
- Handles both user-selected indicators (Zustand store) and strategy-loaded indicators

Files changed:
- `src/hooks/usePriceStreaming.ts` -- Added `onNewCandle` option and `onNewCandleRef` for stale-closure safety
- `src/ChartApp.tsx` -- Added `refreshIndicatorsOnNewCandle` callback, passed as `onNewCandle` to the streaming hook

**Performance Notes**: The current implementation re-fetches full indicator history (`get_indicator_data` with full `count`) and does a complete clear+render cycle on every candle boundary. On M1 with multiple expensive indicators (e.g., Ichimoku), this may cause brief lag once per minute. A future optimization would add backend support for incremental indicator calculation (compute only the latest bar) and use series `.update()` instead of full teardown/rebuild.

**Regression Test**: Manual test: open chart with indicators on a low timeframe (M1 or M5), verify indicator lines extend when new candles form. No automated test added (imperative canvas API makes chart rendering untestable in unit tests).

## Template

When adding a bug, use this format:

### BUG-CHART-NNN: Short description

**Severity**: Critical / High / Medium / Low
**Reported**: YYYY-MM-DD
**Status**: Open / In Progress / Resolved

**Symptoms**: What the user sees or experiences.

**Root Cause**: Technical explanation of why it happens.

**Reproduction Steps**:
1. Step one
2. Step two
3. Expected vs. actual behavior

**Fix**: Description of the fix, with relevant file paths.

**Regression Test**: What test was added or should be added to prevent recurrence.
