import { INDICATOR_COLORS, OVERLAY_INDICATORS } from './chartConstants';
import { OUTPUT_LABELS } from '../../types/strategy';
import type { IndicatorSeries, ChartIndicatorConfig } from './chartTypes';
import { formatIndicatorLabel } from './indicatorHelpers';

interface IndicatorLegendProps {
  indicatorData: IndicatorSeries[];
  /** Optional configs to show params in legend (for chart indicators from store) */
  indicatorConfigs?: ChartIndicatorConfig[];
  strategyId: string | null;
  strategyName?: string;
  signalDirection: 'long' | 'short' | null;
}

// Short display names for legend (more compact than OUTPUT_LABELS)
const LEGEND_OUTPUT_NAMES: Record<string, string> = {
  tenkan: 'Tenkan',
  kijun: 'Kijun',
  chikou: 'Chikou',
  senkou_a: 'Senkou A',
  senkou_b: 'Senkou B',
  exit_long: 'Exit Long',
  exit_short: 'Exit Short',
  upper: 'Upper',
  middle: 'Middle',
  lower: 'Lower',
  macd: 'MACD',
  signal: 'Signal',
  histogram: 'Histogram',
  k: '%K',
  d: '%D',
  dss: 'DSS',
  plus_di: '+DI',
  minus_di: '-DI',
  fast_ma: 'Fast MA',
  slow_ma: 'Slow MA',
  value: 'Value',
  ratio: 'Ratio',
};

export const IndicatorLegend = ({
  indicatorData,
  indicatorConfigs,
  strategyId,
  strategyName,
  signalDirection,
}: IndicatorLegendProps) => {
  // Get the line colors for an indicator output - always use INDICATOR_COLORS for consistency with chart
  const getLineColor = (indicator: IndicatorSeries, output: string) => {
    const colorKey = `${indicator.type}.${output}`;
    return INDICATOR_COLORS[colorKey] || INDICATOR_COLORS[indicator.type] || INDICATOR_COLORS.default;
  };

  // Get display name for output - prefer short legend names, fall back to OUTPUT_LABELS
  const getDisplayName = (output: string) => {
    return LEGEND_OUTPUT_NAMES[output] || OUTPUT_LABELS[output] || output;
  };

  // Get indicator label - use config if available, otherwise fall back to type name
  const getIndicatorLabel = (ind: IndicatorSeries) => {
    if (indicatorConfigs) {
      const config = indicatorConfigs.find((c) => c.id === ind.id);
      if (config) {
        return formatIndicatorLabel(config);
      }
    }
    return ind.type.toUpperCase();
  };

  const overlayIndicators = indicatorData.filter((ind) =>
    OVERLAY_INDICATORS.includes(ind.type)
  );

  return (
    <div className="flex-shrink-0 px-4 py-2 text-xs text-[var(--color-text-muted)] min-h-[32px]">
      <div className="flex flex-wrap gap-x-6 gap-y-1">
        {overlayIndicators.length === 0 && strategyId && (
          <span className="text-[var(--color-text-muted)]">Loading indicators...</span>
        )}
        {overlayIndicators.map((ind) => {
          // Filter out cloud_top/cloud_bottom and filter Chandelier by signal direction
          const visibleOutputs = ind.outputs.filter((o) => {
            if (o === 'cloud_top' || o === 'cloud_bottom') return false;
            // Filter Chandelier based on signal direction (same logic as chart rendering)
            if (ind.type === 'chandelier' && signalDirection) {
              if (signalDirection === 'long' && o !== 'exit_long') return false;
              if (signalDirection === 'short' && o !== 'exit_short') return false;
            }
            return true;
          });

          return (
            <div key={ind.id} className="flex items-center gap-2">
              <span className="text-[var(--color-text-muted)] font-medium">{getIndicatorLabel(ind)}:</span>
              {visibleOutputs.map((output) => (
                <span key={output} className="flex items-center gap-1">
                  <span
                    className="inline-block w-3 h-0.5 rounded"
                    style={{ backgroundColor: getLineColor(ind, output) }}
                  ></span>
                  <span style={{ color: getLineColor(ind, output) }}>{getDisplayName(output)}</span>
                </span>
              ))}
            </div>
          );
        })}
        {strategyName && (
          <span className="ml-auto text-[var(--color-text-muted)]">
            Strategy: {strategyName}
          </span>
        )}
      </div>
    </div>
  );
};
