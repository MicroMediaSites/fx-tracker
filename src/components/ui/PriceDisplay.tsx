/**
 * PriceDisplay - Components for FX price display
 *
 * PriceWindow subscribes to its own price updates for render isolation.
 * Only the PriceWindow re-renders when prices change, not parent components.
 *
 * @example
 * ```tsx
 * <PriceWindow instrument="EUR_USD" />
 * ```
 */
import { useEffect, useState } from 'react';
import { usePriceStore } from '../../stores/priceStore';
import { useSpreadStats } from '../../stores/spreadStatsStore';
import { usePriceFlash } from '../../hooks/usePriceFlash';
import type { PriceDirection } from '../../hooks/usePriceFlash';
import { formatPriceParts, getPipMultiplier } from '../../lib/priceCalculations';

/** A displayed price older than this is visibly flagged as stale. */
const STALE_AFTER_MS = 60_000;

/** Coarse clock that re-renders subscribers every `intervalMs` so the
 *  staleness badge appears even when no events arrive at all (the freeze
 *  case is exactly the one with no new renders). */
function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs]);
  return now;
}

function staleAgeLabel(ms: number): string {
  const mins = Math.floor(ms / 60_000);
  if (mins < 1) return '<1m';
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  return `${hours}h ${mins % 60}m`;
}

// Re-export for convenience
export { formatPriceParts } from '../../lib/priceCalculations';
export type { PriceParts } from '../../lib/priceCalculations';

export interface PriceWindowProps {
  /** Instrument name (e.g., "EUR_USD") - subscribes to price internally */
  instrument: string;
}

/**
 * Default spread ranges by instrument type (in price units, not pips).
 * Used as fallback when no historical data is available yet.
 * These are rough estimates for typical market conditions.
 */
const DEFAULT_SPREAD_RANGES: Record<string, { min: number; max: number }> = {
  // Major pairs - tight spreads
  EUR_USD: { min: 0.00006, max: 0.00025 },
  GBP_USD: { min: 0.00008, max: 0.00035 },
  USD_JPY: { min: 0.006, max: 0.025 },
  USD_CHF: { min: 0.00008, max: 0.00030 },
  AUD_USD: { min: 0.00006, max: 0.00025 },
  USD_CAD: { min: 0.00008, max: 0.00030 },
  NZD_USD: { min: 0.00008, max: 0.00030 },
  // Cross pairs - slightly wider
  EUR_GBP: { min: 0.00008, max: 0.00035 },
  EUR_JPY: { min: 0.008, max: 0.035 },
  GBP_JPY: { min: 0.012, max: 0.050 },
  EUR_CHF: { min: 0.00010, max: 0.00040 },
  EUR_AUD: { min: 0.00012, max: 0.00050 },
  GBP_AUD: { min: 0.00015, max: 0.00060 },
  AUD_JPY: { min: 0.008, max: 0.035 },
  CAD_JPY: { min: 0.008, max: 0.035 },
  CHF_JPY: { min: 0.010, max: 0.040 },
  NZD_JPY: { min: 0.010, max: 0.040 },
  AUD_NZD: { min: 0.00012, max: 0.00050 },
  EUR_CAD: { min: 0.00012, max: 0.00050 },
  GBP_CAD: { min: 0.00015, max: 0.00060 },
  GBP_CHF: { min: 0.00012, max: 0.00050 },
  EUR_NZD: { min: 0.00015, max: 0.00060 },
  GBP_NZD: { min: 0.00020, max: 0.00080 },
  AUD_CAD: { min: 0.00012, max: 0.00050 },
  AUD_CHF: { min: 0.00012, max: 0.00050 },
  CAD_CHF: { min: 0.00012, max: 0.00050 },
  NZD_CAD: { min: 0.00012, max: 0.00050 },
  NZD_CHF: { min: 0.00012, max: 0.00050 },
};

/**
 * Get default spread range for an instrument.
 * Falls back to a generic range if instrument not in the lookup.
 */
function getDefaultSpreadRange(instrument: string): { min: number; max: number } {
  if (DEFAULT_SPREAD_RANGES[instrument]) {
    return DEFAULT_SPREAD_RANGES[instrument];
  }
  // Generic fallback - check if JPY pair
  if (instrument.includes('JPY')) {
    return { min: 0.010, max: 0.050 };
  }
  return { min: 0.00010, max: 0.00050 };
}

/**
 * Calculate spread bar color based on historical statistics.
 * Returns HSL color: green (low spread) -> yellow (average) -> red (high spread)
 * Returns purple/pink when no historical data is available (using defaults)
 *
 * @param currentSpread - Current spread value
 * @param minSpread - Historical minimum spread (decayed)
 * @param maxSpread - Historical maximum spread (decayed)
 * @param instrument - Instrument name for fallback ranges
 * @returns HSL color string
 */
export function calculateSpreadColor(
  currentSpread: number,
  minSpread: number | undefined,
  maxSpread: number | undefined,
  instrument: string
): string {
  // If no historical stats, return purple/pink to indicate "no data"
  if (minSpread === undefined || maxSpread === undefined || maxSpread <= minSpread) {
    // Use default ranges to calculate relative position, but show as purple
    const defaults = getDefaultSpreadRange(instrument);
    const percentile = Math.max(0, Math.min(1, (currentSpread - defaults.min) / (defaults.max - defaults.min)));
    // Purple gradient: 280 (violet) to 320 (pink/magenta) based on spread level
    const hue = 280 + (percentile * 40);
    return `hsl(${hue}, 60%, 50%)`;
  }

  // Calculate percentile: 0 = at minimum (green), 1 = at maximum (red)
  const percentile = Math.max(0, Math.min(1, (currentSpread - minSpread) / (maxSpread - minSpread)));

  // HSL color: 120 = green, 60 = yellow, 0 = red
  // We interpolate from green (low spread) to red (high spread)
  const hue = 120 - (percentile * 120);

  return `hsl(${hue}, 70%, 45%)`;
}

/**
 * Displays bid/ask prices with spread indicator bar.
 * Subscribes to price updates internally for render isolation.
 * Shows prices in the standard FX format with big pips emphasized.
 * Includes color flash animation when prices change.
 * Spread bar color indicates whether current spread is historically low (green),
 * average (yellow), or high (red).
 */
export function PriceWindow({ instrument }: PriceWindowProps) {
  // Subscribe to just this instrument's price for render isolation
  const price = usePriceStore((state) => state.prices[instrument]);
  const streamHealth = usePriceStore((state) => state.streamHealth);
  const lastTickAtMs = usePriceStore((state) => state.lastTickAtMs);

  // Staleness: either the backend says the stream is unhealthy, or no price
  // has flushed in STALE_AFTER_MS (covers a silent hub with no health
  // events). The 15s clock exists because a frozen stream produces no
  // renders of its own.
  const now = useNow(15_000);
  const sinceTick = lastTickAtMs === null ? null : now - lastTickAtMs;
  const isStale =
    (streamHealth !== null && !streamHealth.healthy) ||
    (sinceTick !== null && sinceTick > STALE_AFTER_MS);

  // Historical spread stats sampled by the wickd CLI into
  // ~/.wickd/spreads.db (read via the get_spread_stats command). Undefined
  // until history exists, which renders the purple "no data" fallback.
  const stats = useSpreadStats(instrument);

  // Price flash hooks
  const bidFlash = usePriceFlash(price?.bid);
  const askFlash = usePriceFlash(price?.ask);

  const isJpy = instrument.includes('JPY');
  const [baseCurrency] = instrument.split('_');

  // Loading state
  if (!price) {
    return (
      <div className="h-24 flex items-center justify-center text-[var(--color-text-muted)] text-sm">
        Loading...
      </div>
    );
  }

  const bid = parseFloat(price.bid);
  const ask = parseFloat(price.ask);
  const spread = parseFloat(price.spread);

  const bidParts = formatPriceParts(bid, isJpy);
  const askParts = formatPriceParts(ask, isJpy);
  const spreadPips = (spread * getPipMultiplier(isJpy)).toFixed(1);

  // Bar width: logarithmic scale - handles both tight and wide spreads gracefully
  // 0 pips = 0%, ~1 pip = 40%, ~5 pips = 70%, ~20 pips = 90%, 50+ pips = 100%
  const spreadNum = parseFloat(spreadPips);
  const barWidthPercent =
    spreadNum <= 0 ? 0 : Math.min(100, (Math.log10(spreadNum + 1) / Math.log10(51)) * 100);

  // Calculate spread bar color based on historical stats
  // Green = historically low spread, Yellow = average, Red = high
  // Falls back to default ranges per instrument when no historical data yet
  const spreadColor = calculateSpreadColor(
    spread,
    stats?.min_spread ? parseFloat(stats.min_spread) : undefined,
    stats?.max_spread ? parseFloat(stats.max_spread) : undefined,
    instrument
  );

  const getFlashColor = (flash: PriceDirection | undefined | null) => {
    if (flash === 'up') return 'text-[var(--color-buy-text)]';
    if (flash === 'down') return 'text-[var(--color-sell-text)]';
    return 'text-[var(--color-text-primary)]';
  };

  return (
    <div className={`flex items-stretch relative ${isStale ? 'opacity-60' : ''}`}>
      {isStale && (
        <div
          className="absolute top-0.5 right-0.5 z-10 text-[9px] font-mono px-1.5 py-0.5 rounded border border-amber-600/50 text-amber-600 bg-[var(--color-bg,transparent)]"
          title={`No live data${sinceTick !== null ? ` for ${staleAgeLabel(sinceTick)}` : ''} — showing last received price`}
        >
          STALE{sinceTick !== null ? ` ${staleAgeLabel(sinceTick)}` : ''}
        </div>
      )}
      {/* Sell side */}
      <div className="flex-1 flex flex-col">
        <div className="pt-1 pb-4 px-3 rounded-tl border border-[var(--color-border)] border-b-0 flex-1">
          <div className="text-[10px] text-[var(--color-text-muted)] mb-1 text-left">
            Sell {baseCurrency}
          </div>
          <div className="flex flex-col items-center">
            <span className="text-xs text-[var(--color-text-muted)] font-mono -translate-x-6">
              {bidParts.top}
            </span>
            <div className="flex items-baseline">
              <span
                className={`text-3xl font-mono font-semibold transition-colors duration-300 ${getFlashColor(bidFlash)}`}
              >
                {bidParts.big}
              </span>
              <span className="text-lg text-[var(--color-text-secondary)] font-mono self-start">
                {bidParts.small}
              </span>
            </div>
          </div>
        </div>
        {/* Spread bar - color indicates historical spread level (green=low, yellow=avg, red=high) */}
        <div className="h-1.5 border border-[var(--color-border)] border-t-0 rounded-bl -mt-px flex justify-end">
          <div
            className="h-full transition-all duration-300"
            style={{
              width: `${barWidthPercent}%`,
              backgroundColor: spreadColor,
            }}
          />
        </div>
      </div>

      {/* Notch triangle - stacked for border effect */}
      <div className="absolute bottom-0 left-1/2 -translate-x-1/2 z-[1]">
        {/* Border triangle (larger, behind) */}
        <div
          style={{
            width: 0,
            height: 0,
            borderLeft: '25px solid transparent',
            borderRight: '25px solid transparent',
            borderBottom: '21px solid var(--color-border)',
          }}
        />
        {/* Fill triangle (smaller, on top) */}
        <div
          className="absolute bottom-0 left-1/2 -translate-x-1/2"
          style={{
            width: 0,
            height: 0,
            borderLeft: '24px solid transparent',
            borderRight: '24px solid transparent',
            borderBottom: '20px solid var(--color-bg-page)',
          }}
        />
        {/* Spread value */}
        <span
          className="absolute left-1/2 -translate-x-1/2 text-[9px] text-[var(--color-text-muted)] font-mono"
          style={{ bottom: '-2px' }}
        >
          {spreadPips}
        </span>
      </div>

      {/* Buy side */}
      <div className="flex-1 flex flex-col">
        <div className="pt-1 pb-4 px-3 rounded-tr border border-[var(--color-border)] border-l-0 border-b-0 flex-1">
          <div className="text-[10px] text-[var(--color-text-muted)] mb-1 text-right">
            Buy {baseCurrency}
          </div>
          <div className="flex flex-col items-center">
            <span className="text-xs text-[var(--color-text-muted)] font-mono -translate-x-6">
              {askParts.top}
            </span>
            <div className="flex items-baseline">
              <span
                className={`text-3xl font-mono font-semibold transition-colors duration-300 ${getFlashColor(askFlash)}`}
              >
                {askParts.big}
              </span>
              <span className="text-lg text-[var(--color-text-secondary)] font-mono self-start">
                {askParts.small}
              </span>
            </div>
          </div>
        </div>
        {/* Spread bar - color indicates historical spread level (green=low, yellow=avg, red=high) */}
        <div className="h-1.5 border border-[var(--color-border)] border-t-0 border-l-0 rounded-br -mt-px flex justify-start">
          <div
            className="h-full transition-all duration-300"
            style={{
              width: `${barWidthPercent}%`,
              backgroundColor: spreadColor,
            }}
          />
        </div>
      </div>
    </div>
  );
}

export default PriceWindow;

/**
 * MidPrice - Compact mid price display with render isolation
 *
 * Subscribes to its own price updates so only this component re-renders
 * when prices change, not the parent card.
 */
export interface MidPriceProps {
  instrument: string;
}

export function MidPrice({ instrument }: MidPriceProps) {
  const price = usePriceStore((state) => state.prices[instrument]);

  const isJpy = instrument.includes('JPY');

  if (!price) {
    return <span className="text-xs text-[var(--color-text-muted)] font-mono">—</span>;
  }

  const bid = parseFloat(price.bid);
  const ask = parseFloat(price.ask);
  const mid = (bid + ask) / 2;

  const parts = formatPriceParts(mid, isJpy);

  return (
    <span className="inline-flex items-baseline gap-0.5 font-mono text-[var(--color-text-secondary)]">
      <span className="text-[10px] text-[var(--color-text-muted)]">{parts.top}</span>
      <span className="text-sm font-semibold">{parts.big}</span>
      <span className="text-[10px] text-[var(--color-text-muted)] -translate-y-1">{parts.small}</span>
    </span>
  );
}
