import {
  INDICATOR_METADATA,
  INDICATOR_DEFAULTS,
  type IndicatorType,
  type IndicatorDefinition,
  isParameterReference,
} from '../../types/strategy';
import type { ChartIndicatorConfig } from './chartTypes';

/**
 * Convert strategy IndicatorDefinition[] to ChartIndicatorConfig[].
 * Resolves $param references using the strategy's parameter definitions
 * (uses min when min===max for promoted strategies, otherwise default).
 * Falls back to global INDICATOR_DEFAULTS if no parameter definitions provided.
 */
export function strategyIndicatorsToChartConfigs(
  indicators: IndicatorDefinition[],
  strategyParams?: Array<{ id: string; default: number; min?: number; max?: number }>,
): ChartIndicatorConfig[] {
  // Build a map of param ID → resolved value from strategy parameters
  const paramValues = new Map<string, number>();
  if (Array.isArray(strategyParams)) {
    for (const p of strategyParams) {
      // Use min when min===max (resolved/locked value), otherwise default
      const resolved = (p.min !== undefined && p.max !== undefined && p.min === p.max)
        ? p.min
        : p.default;
      paramValues.set(p.id, resolved);
    }
  }

  return indicators.map((ind) => {
    const resolvedParams: Record<string, number> = {};
    const defaults = INDICATOR_DEFAULTS[ind.type] ?? {};
    for (const [key, value] of Object.entries(ind.params)) {
      if (isParameterReference(value)) {
        // Resolve from strategy params first, then indicator defaults
        resolvedParams[key] = paramValues.get(value.$param) ?? defaults[key] ?? 0;
      } else {
        resolvedParams[key] = value;
      }
    }
    return {
      id: ind.id,
      type: ind.type,
      params: resolvedParams,
    };
  });
}

/**
 * Generate a unique ID for a new indicator instance.
 * IDs follow the pattern: type_N (e.g., sma_1, sma_2, rsi_1)
 */
export function generateIndicatorId(
  type: IndicatorType,
  existing: ChartIndicatorConfig[]
): string {
  const sameType = existing.filter((ind) => ind.type === type);
  const maxSuffix = sameType.reduce((max, ind) => {
    const match = ind.id.match(/_(\d+)$/);
    return match ? Math.max(max, parseInt(match[1], 10)) : max;
  }, 0);
  return `${type}_${maxSuffix + 1}`;
}

/**
 * Format an indicator config for display in legend/menu.
 * Examples: "SMA(20)", "Bollinger(20,2)", "MACD(12,26,9)", "RSI(14)"
 */
export function formatIndicatorLabel(config: ChartIndicatorConfig): string {
  const type = config.type as IndicatorType;
  const meta = INDICATOR_METADATA[type];
  const label = meta?.label ?? type.toUpperCase();
  // Use stored params, or fall back to defaults if empty
  const storedParams = config.params;
  const params = Object.keys(storedParams).length > 0
    ? storedParams
    : (INDICATOR_DEFAULTS[type] ?? {});

  const paramValues = Object.values(params);
  if (paramValues.length === 0) return label;
  if (paramValues.length === 1) return `${label}(${paramValues[0]})`;

  // Multi-param: show key params in a logical order
  switch (type) {
    case 'bollinger':
      return `${label}(${params.period},${params.std_dev})`;
    case 'macd':
      return `${label}(${params.fast_period},${params.slow_period},${params.signal_period})`;
    case 'stochastic':
      return `${label}(${params.k_period},${params.d_period})`;
    case 'ma_histogram':
      return `${label}(${params.fast_period},${params.slow_period})`;
    case 'ma_bands':
      return `${label}(${params.period},${params.distance})`;
    case 'dss':
      return `${label}(${params.stoch_period},${params.ema_period})`;
    case 'ichimoku': {
      const t = params.tenkan_period ?? 9;
      const k = params.kijun_period ?? 26;
      const s = params.senkou_b_period ?? 52;
      const d = params.displacement ?? 26;
      return `${label}(${t},${k},${s},${d})`;
    }
    case 'chandelier':
      return `${label}(${params.period},${params.multiplier})`;
    default:
      return `${label}(${paramValues.join(',')})`;
  }
}

/**
 * Indicator categories for menu grouping.
 */
export const INDICATOR_CATEGORIES = ['Trend', 'Momentum', 'Volatility', 'Advanced'] as const;
export type IndicatorCategory = (typeof INDICATOR_CATEGORIES)[number];

/**
 * Map indicator types to their categories.
 */
export const INDICATOR_TYPES_BY_CATEGORY: Record<IndicatorCategory, IndicatorType[]> = {
  Trend: ['sma', 'ema', 'adx'],
  Momentum: ['rsi', 'mfi', 'macd', 'stochastic', 'dss', 'ma_histogram'],
  Volatility: ['bollinger', 'atr', 'adr', 'ma_bands', 'donchian'],
  Advanced: ['ichimoku', 'chandelier'],
};

/**
 * Get the category for an indicator type.
 */
export function getIndicatorCategory(type: IndicatorType): IndicatorCategory {
  for (const [category, types] of Object.entries(INDICATOR_TYPES_BY_CATEGORY)) {
    if (types.includes(type)) {
      return category as IndicatorCategory;
    }
  }
  return 'Advanced'; // Default fallback
}
