import {
  LineSeries,
  HistogramSeries,
  type IChartApi,
  type ISeriesApi,
  type LineData,
  type HistogramData,
  type Time,
} from 'lightweight-charts';
import { IchimokuCloudPlugin, type CloudPoint } from './IchimokuCloudPlugin';
import {
  INDICATOR_COLORS,
  OVERLAY_INDICATORS,
  DEFAULT_ICHIMOKU_CONFIG,
} from './chartConstants';
import type { IndicatorSeries, IndicatorConfig, ToBusinessTimeFn } from './chartTypes';

interface RenderIndicatorsOptions {
  chart: IChartApi;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  candleSeries: ISeriesApi<any>;
  indicatorResults: IndicatorSeries[];
  indicatorConfigs: IndicatorConfig[];
  /** Custom colors per indicator (key = indicator id) */
  customColors?: Map<string, Record<string, string>>;
  candleSeconds: number;
  toBusinessTime: ToBusinessTimeFn;
  signalDirection?: 'long' | 'short' | null;
}

/** Helper to get color for an indicator output, checking custom colors first */
const getIndicatorColor = (
  indId: string,
  indType: string,
  output: string,
  customColors?: Map<string, Record<string, string>>
): string => {
  // Check for custom color first
  const indicatorColors = customColors?.get(indId);
  if (indicatorColors && indicatorColors[output]) {
    return indicatorColors[output];
  }
  // Fall back to default
  const colorKey = output === 'value' ? indType : `${indType}.${output}`;
  return INDICATOR_COLORS[colorKey] || INDICATOR_COLORS.default;
};

interface RenderIndicatorsResult {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  indicatorSeries: Map<string, ISeriesApi<any>>;
  ichimokuCloud: IchimokuCloudPlugin | null;
}

/**
 * Renders indicator series on the chart.
 * This is called after candles are loaded and whenever selected indicators change.
 */
export const renderIndicators = ({
  chart,
  candleSeries,
  indicatorResults,
  indicatorConfigs,
  customColors,
  candleSeconds,
  toBusinessTime,
  signalDirection,
}: RenderIndicatorsOptions): RenderIndicatorsResult => {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const indicatorSeries = new Map<string, ISeriesApi<any>>();
  let ichimokuCloud: IchimokuCloudPlugin | null = null;

  // Track which pane each oscillator type goes into
  let nextPaneIndex = 1; // Pane 0 is the main price chart
  const oscillatorPaneMap: Record<string, number> = {};

  for (const indSeries of indicatorResults) {
    const isOverlay = OVERLAY_INDICATORS.includes(indSeries.type);

    // Handle oscillators (non-overlay indicators) in separate panes
    if (!isOverlay) {
      // Assign a pane for this oscillator type (group same types together)
      if (!(indSeries.type in oscillatorPaneMap)) {
        oscillatorPaneMap[indSeries.type] = nextPaneIndex++;
      }
      const paneIndex = oscillatorPaneMap[indSeries.type];

      // Render oscillator outputs — histogram first, then lines.
      // In Lightweight Charts, series added later render on top. By adding
      // histogram bars before line series, the lines draw above the bars
      // and remain visible (fixes BUG-066).
      const histogramOutputs = indSeries.outputs.filter(o => o === 'histogram');
      const lineOutputs = indSeries.outputs.filter(o => o !== 'histogram');
      const orderedOutputs = [...histogramOutputs, ...lineOutputs];

      for (const output of orderedOutputs) {
        // Skip fast_ma/slow_ma for MA Histogram - only show the histogram
        // (The MAs are at price level, histogram is near zero - can't share a scale)
        if (indSeries.type === 'ma_histogram' && (output === 'fast_ma' || output === 'slow_ma')) {
          continue;
        }

        const seriesKey = `${indSeries.id}.${output}`;
        const color = getIndicatorColor(indSeries.id, indSeries.type, output, customColors);

        const lineData: LineData[] = indSeries.data
          .filter((d) => d.values[output] !== undefined)
          .map((d) => {
            const actualTime = new Date(d.time).getTime() / 1000;
            const businessTime = toBusinessTime(actualTime);
            return {
              time: businessTime as Time,
              value: parseFloat(d.values[output]),
            };
          });

        if (lineData.length > 0) {
          const isHistogram = output === 'histogram';

          if (isHistogram) {
            const histSeries = chart.addSeries(HistogramSeries, {
              priceLineVisible: false,
              lastValueVisible: true,
              priceFormat: { type: 'price', precision: 5, minMove: 0.00001 },
            }, paneIndex);
            const coloredData: HistogramData[] = lineData.map(d => ({
              time: d.time,
              value: d.value,
              color: d.value >= 0 ? '#26a69a' : '#ef5350',
            }));
            histSeries.setData(coloredData);
            indicatorSeries.set(seriesKey, histSeries);
          } else {
            const lineSeries = chart.addSeries(LineSeries, {
              color,
              lineWidth: output === 'signal' ? 1 : 2,
              priceLineVisible: false,
              lastValueVisible: true,
              priceFormat: { type: 'price', precision: 2, minMove: 0.01 },
            }, paneIndex);
            lineSeries.setData(lineData);
            indicatorSeries.set(seriesKey, lineSeries);
          }
        }
      }

      continue;
    }

    // Special handling for Ichimoku
    if (indSeries.type === 'ichimoku') {
      const config = indicatorConfigs.find((c) => c.id === indSeries.id);
      const displacement = (config?.params as { displacement?: number })?.displacement ?? DEFAULT_ICHIMOKU_CONFIG.displacement;

      // Get custom colors if provided, otherwise use INDICATOR_COLORS
      const indicatorCustomColors = customColors?.get(indSeries.id);
      const getIchimokuColor = (output: string): string => {
        // Custom color takes priority
        if (indicatorCustomColors?.[output]) {
          return indicatorCustomColors[output];
        }
        // Fall back to INDICATOR_COLORS (single source of truth)
        return INDICATOR_COLORS[`ichimoku.${output}`] || INDICATOR_COLORS.default;
      };

      // Build cloud data for the plugin using raw (undisplaced) senkou values
      // Backend outputs senkou_a_raw/senkou_b_raw = freshly computed at each candle
      // Frontend shifts them forward by displacement for correct charting position
      const cloudPoints: CloudPoint[] = indSeries.data
        .filter((d) => d.values.senkou_a_raw !== undefined && d.values.senkou_b_raw !== undefined)
        .map((d) => {
          const actualTime = new Date(d.time).getTime() / 1000;
          const businessTime = toBusinessTime(actualTime);
          return {
            time: businessTime + (displacement * candleSeconds),
            senkou_a: parseFloat(d.values.senkou_a_raw),
            senkou_b: parseFloat(d.values.senkou_b_raw),
          };
        });

      // Create and attach cloud plugin
      if (cloudPoints.length > 0) {
        ichimokuCloud = new IchimokuCloudPlugin(cloudPoints);
        candleSeries.attachPrimitive(ichimokuCloud);
      }

      // Render Ichimoku lines
      // Use raw outputs for displaced lines (senkou_a/b shifted forward, chikou shifted back)
      // Backend's displaced outputs are for backtesting; raw outputs are for charting
      for (const output of indSeries.outputs) {
        if (output === 'cloud_top' || output === 'cloud_bottom') continue;
        // Skip raw outputs in the loop — we use them explicitly below
        if (output === 'senkou_a_raw' || output === 'senkou_b_raw' || output === 'chikou_raw') continue;

        const seriesKey = `${indSeries.id}.${output}`;
        const color = getIchimokuColor(output);

        // Map displaced outputs to their raw counterparts for charting
        let dataKey = output;
        let periodOffset = 0;
        if (output === 'senkou_a') { dataKey = 'senkou_a_raw'; periodOffset = displacement; }
        else if (output === 'senkou_b') { dataKey = 'senkou_b_raw'; periodOffset = displacement; }
        else if (output === 'chikou') { dataKey = 'chikou_raw'; periodOffset = -displacement; }

        const lineData: LineData[] = indSeries.data
          .filter((d) => d.values[dataKey] !== undefined)
          .map((d) => {
            const actualTime = new Date(d.time).getTime() / 1000;
            const businessTime = toBusinessTime(actualTime);
            return {
              time: (businessTime + (periodOffset * candleSeconds)) as Time,
              value: parseFloat(d.values[dataKey]),
            };
          });

        if (lineData.length > 0) {
          const lineWidth = (output === 'chikou') ? 2 : (output === 'senkou_a' || output === 'senkou_b') ? 1 : 2;
          const lineSeries = chart.addSeries(LineSeries, {
            color,
            lineWidth,
            priceLineVisible: false,
            lastValueVisible: false,
          });
          lineSeries.setData(lineData);
          indicatorSeries.set(seriesKey, lineSeries);
        }
      }
    } else {
      // Standard overlay indicator rendering (SMA, EMA, Bollinger, Chandelier)
      for (const output of indSeries.outputs) {
        // For Chandelier, filter based on signal direction
        if (indSeries.type === 'chandelier' && signalDirection) {
          if (signalDirection === 'long' && output !== 'exit_long') continue;
          if (signalDirection === 'short' && output !== 'exit_short') continue;
        }

        const seriesKey = `${indSeries.id}.${output}`;
        const color = getIndicatorColor(indSeries.id, indSeries.type, output, customColors);

        const lineData: LineData[] = indSeries.data
          .filter((d) => d.values[output] !== undefined)
          .map((d) => {
            const actualTime = new Date(d.time).getTime() / 1000;
            const businessTime = toBusinessTime(actualTime);
            return {
              time: businessTime as Time,
              value: parseFloat(d.values[output]),
            };
          });

        if (lineData.length > 0) {
          // Line style: dashed for Bollinger/MA Bands/Chandelier, solid for moving averages
          const isChandelier = indSeries.type === 'chandelier';
          const isBands = indSeries.type === 'bollinger' || indSeries.type === 'ma_bands';
          const lineStyle = (isBands || isChandelier) ? 2 : 0;
          const lineSeries = chart.addSeries(LineSeries, {
            color,
            lineWidth: isChandelier ? 2 : 1,
            lineStyle,
            priceLineVisible: false,
            lastValueVisible: false,
          });
          lineSeries.setData(lineData);
          indicatorSeries.set(seriesKey, lineSeries);
        }
      }
    }
  }

  return { indicatorSeries, ichimokuCloud };
};

/**
 * Clears all indicator series from the chart.
 */
export const clearIndicators = (
  chart: IChartApi,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  indicatorSeries: Map<string, ISeriesApi<any>>,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  candleSeries: ISeriesApi<any> | null,
  ichimokuCloud: IchimokuCloudPlugin | null,
): void => {
  indicatorSeries.forEach((series) => {
    chart.removeSeries(series);
  });
  indicatorSeries.clear();

  if (ichimokuCloud && candleSeries) {
    candleSeries.detachPrimitive(ichimokuCloud);
    // Force chart to repaint after detaching cloud primitive
    // Without this, the canvas retains the old cloud drawing
    chart.timeScale().applyOptions({});
  }
};
