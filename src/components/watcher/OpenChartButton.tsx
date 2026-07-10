import { invoke } from '@tauri-apps/api/core';
import { GRANULARITIES, DEFAULT_GRANULARITY } from '../../constants';

const CHARTABLE_GRANULARITIES = new Set<string>(GRANULARITIES.map((g) => g.value));

/**
 * Map a watcher granularity onto one the chart can render. Watchers can run
 * timeframes the chart selector doesn't offer (H2/H6/H12) — fall back to H4
 * for hour-based ones, else the default.
 */
export const chartableGranularity = (granularity: string | null | undefined): string => {
  if (!granularity) return DEFAULT_GRANULARITY;
  if (CHARTABLE_GRANULARITIES.has(granularity)) return granularity;
  return granularity.startsWith('H') ? 'H4' : DEFAULT_GRANULARITY;
};

/** Opens the instrument in a chart window (contextual multi-chart label). */
export const OpenChartButton = ({
  instrument,
  granularity,
  indicatorSeed,
}: {
  instrument: string;
  granularity?: string | null;
  /** Optional ChartApp indicator-seed envelope (strategy @indicators). */
  indicatorSeed?: string;
}) => {
  const chartGranularity = chartableGranularity(granularity);
  return (
    <button
      data-testid="open-chart-button"
      onClick={() => {
        void invoke('open_chart_window', {
          instrument,
          granularity: chartGranularity,
          indicators: indicatorSeed,
        }).catch((err) => console.error('[LiveMonitor] Failed to open chart:', err));
      }}
      className="p-1 rounded text-[var(--color-text-muted)] hover:text-[var(--color-info)] transition-colors"
      title={`Open ${instrument.replace('_', '/')} chart (${chartGranularity})`}
      aria-label={`Open ${instrument.replace('_', '/')} chart`}
    >
      <svg className="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" d="M4 20h16" />
        <path strokeLinecap="round" d="M7 16v-5M12 16V6M17 16v-8" />
      </svg>
    </button>
  );
};
