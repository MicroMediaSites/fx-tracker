/**
 * BUG-066: Indicator lines render behind histogram bars in chart.
 *
 * Root cause: In the oscillator rendering loop, outputs were iterated in their
 * natural order (e.g., MACD returns ["macd", "signal", "histogram"]). Since
 * Lightweight Charts renders series in the order they are added (later = on top),
 * the histogram was drawn over the lines when it appeared last in the outputs array.
 *
 * Fix: Partition outputs into histogram-first, then lines, so line series are
 * always added after histogram series and render on top.
 */
import { describe, it, expect, vi } from 'vitest';
import { renderIndicators } from './indicatorRenderer';


// Track addSeries calls in order
type AddSeriesCall = { seriesType: unknown; paneIndex?: number };

function createMockChart() {
  const addSeriesCalls: AddSeriesCall[] = [];

  const mockSeries = {
    setData: vi.fn(),
    applyOptions: vi.fn(),
    priceToCoordinate: vi.fn(),
  };

  const chart = {
    addSeries: vi.fn((_type: unknown, _opts: unknown, paneIndex?: number) => {
      addSeriesCalls.push({ seriesType: _type, paneIndex });
      return { ...mockSeries };
    }),
    removeSeries: vi.fn(),
    timeScale: vi.fn(() => ({ applyOptions: vi.fn() })),
  };

  return { chart, addSeriesCalls };
}

function createMockCandleSeries() {
  return {
    attachPrimitive: vi.fn(),
    detachPrimitive: vi.fn(),
  };
}

// Helper to get the type name from a series definition
// LineSeries has { type: "Line" }, HistogramSeries has { type: "Histogram" }
function getSeriesTypeName(seriesType: unknown): string {
  return (seriesType as { type: string }).type;
}

// Helper: build a minimal IndicatorSeries with given outputs and some data points
function buildIndicatorSeries(
  id: string,
  type: string,
  outputs: string[],
  dataPoints = 3,
) {
  const data = Array.from({ length: dataPoints }, (_, i) => {
    const time = new Date(2025, 0, 1 + i).toISOString();
    const values: Record<string, string> = {};
    for (const o of outputs) {
      values[o] = String(1 + i * 0.1);
    }
    return { time, values };
  });
  return { id, type, outputs, data };
}

describe('indicatorRenderer — BUG-066 z-order fix', () => {
  const toBusinessTime = (t: number) => t; // identity for testing

  it('adds histogram series before line series for MACD', () => {
    const { chart, addSeriesCalls } = createMockChart();
    const candleSeries = createMockCandleSeries();

    // MACD backend returns outputs in order: ["macd", "signal", "histogram"]
    const macdSeries = buildIndicatorSeries('macd-1', 'macd', [
      'macd',
      'signal',
      'histogram',
    ]);

    renderIndicators({
      chart: chart as any,
      candleSeries: candleSeries as any,
      indicatorResults: [macdSeries],
      indicatorConfigs: [{ id: 'macd-1', type: 'macd', params: {} }],
      candleSeconds: 3600,
      toBusinessTime,
    });

    // Should have 3 series added (histogram, macd line, signal line)
    expect(addSeriesCalls.length).toBe(3);

    // First call should be HistogramSeries (histogram output added first)
    expect(getSeriesTypeName(addSeriesCalls[0].seriesType)).toBe('Histogram');

    // Second and third calls should be LineSeries (macd and signal lines on top)
    expect(getSeriesTypeName(addSeriesCalls[1].seriesType)).toBe('Line');
    expect(getSeriesTypeName(addSeriesCalls[2].seriesType)).toBe('Line');
  });

  it('adds histogram series before line series for any oscillator with mixed outputs', () => {
    const { chart, addSeriesCalls } = createMockChart();
    const candleSeries = createMockCandleSeries();

    // Simulate an indicator with outputs where histogram is in the middle
    const indicatorSeries = buildIndicatorSeries('test-1', 'custom_osc', [
      'line_a',
      'histogram',
      'line_b',
    ]);

    renderIndicators({
      chart: chart as any,
      candleSeries: candleSeries as any,
      indicatorResults: [indicatorSeries],
      indicatorConfigs: [{ id: 'test-1', type: 'custom_osc', params: {} }],
      candleSeconds: 3600,
      toBusinessTime,
    });

    expect(addSeriesCalls.length).toBe(3);

    // Histogram must be first (rendered as bottom layer)
    expect(getSeriesTypeName(addSeriesCalls[0].seriesType)).toBe('Histogram');

    // Lines must come after (rendered on top)
    expect(getSeriesTypeName(addSeriesCalls[1].seriesType)).toBe('Line');
    expect(getSeriesTypeName(addSeriesCalls[2].seriesType)).toBe('Line');
  });

  it('handles oscillators with no histogram output unchanged', () => {
    const { chart, addSeriesCalls } = createMockChart();
    const candleSeries = createMockCandleSeries();

    // RSI only has "value" output — no histogram
    const rsiSeries = buildIndicatorSeries('rsi-1', 'rsi', ['value']);

    renderIndicators({
      chart: chart as any,
      candleSeries: candleSeries as any,
      indicatorResults: [rsiSeries],
      indicatorConfigs: [{ id: 'rsi-1', type: 'rsi', params: {} }],
      candleSeconds: 3600,
      toBusinessTime,
    });

    expect(addSeriesCalls.length).toBe(1);
    expect(getSeriesTypeName(addSeriesCalls[0].seriesType)).toBe('Line');
  });

  it('assigns correct pane indices to oscillator series', () => {
    const { chart, addSeriesCalls } = createMockChart();
    const candleSeries = createMockCandleSeries();

    const rsiSeries = buildIndicatorSeries('rsi-1', 'rsi', ['value']);
    const macdSeries = buildIndicatorSeries('macd-1', 'macd', [
      'macd',
      'signal',
      'histogram',
    ]);

    renderIndicators({
      chart: chart as any,
      candleSeries: candleSeries as any,
      indicatorResults: [rsiSeries, macdSeries],
      indicatorConfigs: [
        { id: 'rsi-1', type: 'rsi', params: {} },
        { id: 'macd-1', type: 'macd', params: {} },
      ],
      candleSeconds: 3600,
      toBusinessTime,
    });

    // RSI gets pane 1 (1 line series), MACD gets pane 2 (histogram + 2 lines)
    expect(addSeriesCalls.length).toBe(4);

    // RSI: 1 line series in pane 1
    expect(addSeriesCalls[0].paneIndex).toBe(1);

    // MACD: histogram in pane 2 first, then 2 lines in pane 2
    expect(addSeriesCalls[1].paneIndex).toBe(2);
    expect(getSeriesTypeName(addSeriesCalls[1].seriesType)).toBe('Histogram');
    expect(addSeriesCalls[2].paneIndex).toBe(2);
    expect(addSeriesCalls[3].paneIndex).toBe(2);
  });
});
