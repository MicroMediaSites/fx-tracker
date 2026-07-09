/**
 * Strategy Rules Type Definitions
 *
 * See docs/strategy-rules-schema.md for full documentation.
 */

// ============================================================================
// Parameters (for optimization & walk-forward testing)
// ============================================================================

export type ParameterType = 'number' | 'integer' | 'select' | 'boolean';

export interface ParameterOption {
  value: number;
  label: string;
}

export type ParameterGroup = 'indicator' | 'entry' | 'exit' | 'risk';

export interface ParameterDefinition {
  id: string;                         // "rsi_overbought" — referenced in triggers
  name: string;                       // "RSI Overbought Level" — display name
  description?: string;               // "Threshold for overbought condition"
  type: ParameterType;
  default: number;

  // For optimization
  min?: number;                       // Minimum value for optimization range
  max?: number;                       // Maximum value for optimization range
  step?: number;                      // Step size for grid search

  // For select type
  options?: ParameterOption[];

  // Grouping (UI organization)
  group?: ParameterGroup;
}

/**
 * Reference to a strategy parameter.
 * Use { $param: "param_id" } to reference a parameter value.
 */
export interface ParameterReference {
  $param: string;                     // References ParameterDefinition.id
}

/**
 * A value that can be either a fixed number or a parameter reference.
 * Used for fields that users may want to optimize (thresholds, ratios, etc.)
 */
export type ParameterizedValue = number | ParameterReference;

// Type guards for ParameterizedValue
export const isParameterReference = (value: ParameterizedValue): value is ParameterReference => {
  return typeof value === 'object' && value !== null && '$param' in value;
};

/**
 * Resolve a parameterized value to a concrete number.
 * Used during backtest execution with resolved parameter values.
 */
export const resolveParameterizedValue = (
  value: ParameterizedValue,
  params: Record<string, number>
): number | undefined => {
  if (typeof value === 'number') {
    return value;
  }
  return params[value.$param];
};

/**
 * Get the display value for a ParameterizedValue.
 * - If it's a number, return the number
 * - If it's a parameter reference, look up the default value from the parameter definition
 * - If the parameter is not found, return undefined
 */
export const getParameterizedDisplayValue = (
  value: ParameterizedValue,
  parameters: ParameterDefinition[]
): number | undefined => {
  if (typeof value === 'number') {
    return value;
  }
  const param = parameters.find(p => p.id === value.$param);
  return param?.default;
};

/**
 * Get the numeric value for UI input fields.
 * For parameter references, returns the default value.
 * Falls back to 0 if undefined.
 */
export const getParameterizedNumber = (
  value: ParameterizedValue | undefined,
  parameters: ParameterDefinition[] = []
): number => {
  if (value === undefined) return 0;
  if (typeof value === 'number') return value;
  const param = parameters.find(p => p.id === value.$param);
  return param?.default ?? 0;
};

/**
 * Check if a ParameterizedValue references a specific parameter.
 */
export const referencesParameter = (value: ParameterizedValue, paramId: string): boolean => {
  return isParameterReference(value) && value.$param === paramId;
};

/**
 * Sanitize a ParameterizedValue to ensure it is either a plain number or a valid {$param: string}.
 *
 * BUG-045: Number parameter values were sometimes serialized as objects (e.g., ParameterDefinition
 * objects or Number wrapper objects) instead of plain numbers or {$param: string} references.
 * This caused Zod validation to reject the mutation with "expected number, received object".
 *
 * This function normalizes any value to a valid ParameterizedValue:
 * - Plain numbers pass through unchanged
 * - Valid {$param: string} references pass through unchanged
 * - ParameterDefinition objects are converted to their .default value
 * - Number wrapper objects are unwrapped to primitives
 * - Other values are coerced to a number with a fallback
 */
export const sanitizeParameterizedValue = (
  value: unknown,
  fallback: number = 0
): ParameterizedValue => {
  // Null/undefined → fallback (e.g., close_percent defaults to 100 = close full position)
  if (value === undefined || value === null) {
    return fallback;
  }

  // Plain number - most common case
  if (typeof value === 'number' && isFinite(value)) {
    return value;
  }

  // Number object wrapper - unwrap to primitive
  if (value instanceof Number) {
    const num = value.valueOf();
    return isFinite(num) ? num : fallback;
  }

  if (typeof value === 'object') {
    // Valid parameter reference - {$param: string}
    if (
      '$param' in value &&
      typeof (value as ParameterReference).$param === 'string' &&
      (value as ParameterReference).$param.length > 0
    ) {
      // Return a clean object with only the $param key
      return { $param: (value as ParameterReference).$param };
    }

    // ParameterDefinition object accidentally used as value - extract default
    // Check for 'id' to confirm it's actually a ParameterDefinition, not some other object
    if (
      'id' in value &&
      'default' in value &&
      typeof (value as ParameterDefinition).default === 'number'
    ) {
      return (value as ParameterDefinition).default;
    }
  }

  // Try to coerce to number
  const num = Number(value);
  if (isFinite(num)) {
    return num;
  }

  // Ultimate fallback
  return fallback;
};

// ============================================================================
// Indicators
// ============================================================================

export type IndicatorType =
  | 'sma'           // Simple Moving Average
  | 'ema'           // Exponential Moving Average
  | 'rsi'           // Relative Strength Index
  | 'atr'           // Average True Range
  | 'adx'           // Average Directional Index (trend strength)
  | 'ichimoku'      // Ichimoku Cloud
  | 'chandelier'    // Chandelier Exit
  | 'bollinger'     // Bollinger Bands
  | 'macd'          // MACD
  | 'stochastic'    // Stochastic Oscillator
  | 'ma_histogram'  // MA Histogram (fast MA - slow MA)
  | 'ma_bands'      // MA Bands (upper/lower around MA)
  | 'dss'           // Double Smoothed Stochastic
  | 'adr'           // Average Daily Range
  | 'daily'         // Current Day's Stats (high, low, range, open)
  | 'swing'         // Swing High/Low Detection
  | 'mfi'           // Money Flow Index
  | 'donchian'      // Donchian Channel (N-bar high/low)
  | 'vwap'          // Volume Weighted Average Price
  | 'parabolic_sar' // Parabolic SAR
  | 'super_trend';  // SuperTrend

export interface IndicatorDefinition {
  id: string;                      // Unique ID within strategy
  type: IndicatorType;
  params: Record<string, ParameterizedValue>;  // Parameters can be fixed or linked to testing params
  symbol?: string;                 // Optional: for cross-symbol indicators
  timeframe?: string;              // Optional: for multi-timeframe indicators (e.g., "D", "H4", "W")
}

// ============================================================================
// Indicator Metadata - Single Source of Truth
// ============================================================================

/**
 * Contexts where indicator lists are loaded.
 * Add new contexts here when creating new places that show indicator lists.
 * If you forget to add a context, no problem - you just can't exclude indicators from that list.
 */
export type IndicatorContext =
  | 'overlay'        // Chart overlay (renders on price chart vs separate pane)
  | 'divergence'     // Divergence indicator selector in Givens
  | 'entryTriggers'  // Entry rule indicator selectors
  | 'exitTriggers'   // Exit rule indicator selectors
  | 'variables'      // Variable expression sources
  | 'dataSource';    // DataSourcePicker

/**
 * Metadata for each indicator type - the single source of truth.
 * When adding a new indicator, define all its properties here.
 */
export interface IndicatorMetadata {
  /** Short display name (RSI, MACD, etc.) */
  label: string;
  /** Full display name for detailed views (Relative Strength Index, etc.) */
  fullName?: string;
  /** Whether this indicator renders on the price chart (true) or separate pane (false) */
  isOverlay?: boolean;
  /**
   * Contexts to exclude this indicator from.
   * By default, indicators appear everywhere. Add contexts here to restrict.
   * Example: excludeFrom: ['divergence', 'entryTriggers']
   */
  excludeFrom?: IndicatorContext[];
}

export const INDICATOR_METADATA: Record<IndicatorType, IndicatorMetadata> = {
  // Trend-following indicators (overlays)
  sma: {
    label: 'SMA',
    fullName: 'Simple Moving Average',
    isOverlay: true,
    excludeFrom: ['divergence'],
  },
  ema: {
    label: 'EMA',
    fullName: 'Exponential Moving Average',
    isOverlay: true,
    excludeFrom: ['divergence'],
  },
  ichimoku: {
    label: 'Ichimoku',
    fullName: 'Ichimoku Cloud',
    isOverlay: true,
    excludeFrom: ['divergence'],
  },
  bollinger: {
    label: 'Bollinger',
    fullName: 'Bollinger Bands',
    isOverlay: true,
    excludeFrom: ['divergence'],
  },
  ma_bands: {
    label: 'MA Bands',
    fullName: 'Moving Average Bands',
    isOverlay: true,
    excludeFrom: ['divergence'],
  },
  chandelier: {
    label: 'Chandelier',
    fullName: 'Chandelier Exit',
    isOverlay: true,
    excludeFrom: ['divergence', 'entryTriggers', 'variables'],
  },

  // Oscillators (separate pane, support divergence)
  rsi: {
    label: 'RSI',
    fullName: 'Relative Strength Index',
    // No excludeFrom - available everywhere
  },
  macd: {
    label: 'MACD',
    fullName: 'Moving Average Convergence Divergence',
    // No excludeFrom - available everywhere
  },
  stochastic: {
    label: 'Stochastic',
    fullName: 'Stochastic Oscillator',
    // No excludeFrom - available everywhere
  },
  dss: {
    label: 'DSS',
    fullName: 'Double Smoothed Stochastic',
    // No excludeFrom - available everywhere
  },
  ma_histogram: {
    label: 'MA Histogram',
    fullName: 'Moving Average Histogram',
    // No excludeFrom - available everywhere
  },
  adx: {
    label: 'ADX',
    fullName: 'Average Directional Index',
    excludeFrom: ['divergence'],  // Measures trend strength, not oscillating
  },

  // Volatility indicators
  atr: {
    label: 'ATR',
    fullName: 'Average True Range',
    excludeFrom: ['divergence'],
  },
  adr: {
    label: 'ADR',
    fullName: 'Average Daily Range',
    excludeFrom: ['divergence'],
  },

  // Price level indicators
  daily: {
    label: 'Daily',
    fullName: 'Daily Stats',
    excludeFrom: ['divergence'],
  },
  swing: {
    label: 'Swing',
    fullName: 'Swing Points',
    excludeFrom: ['divergence'],
  },
  mfi: {
    label: 'MFI',
    fullName: 'Money Flow Index',
    // No excludeFrom - available everywhere (oscillator, supports divergence)
  },
  donchian: {
    label: 'Donchian',
    fullName: 'Donchian Channel',
    isOverlay: true,
    excludeFrom: ['divergence'],  // Range indicator, not oscillating
  },
  vwap: {
    label: 'VWAP',
    fullName: 'Volume Weighted Average Price',
    isOverlay: true,
    excludeFrom: ['divergence'],
  },
  parabolic_sar: {
    label: 'PSAR',
    fullName: 'Parabolic SAR',
    isOverlay: true,
    excludeFrom: ['divergence'],
  },
  super_trend: {
    label: 'SuperTrend',
    fullName: 'SuperTrend',
    isOverlay: true,
  },
};

// ============================================================================
// Derived Constants (auto-generated from INDICATOR_METADATA)
// ============================================================================

/** All indicator types as array */
export const ALL_INDICATOR_TYPES: IndicatorType[] = Object.keys(INDICATOR_METADATA) as IndicatorType[];

/** Short display labels for all indicator types */
export const INDICATOR_TYPE_LABELS: Record<IndicatorType, string> = (
  Object.entries(INDICATOR_METADATA) as [IndicatorType, IndicatorMetadata][]
).reduce((acc, [type, meta]) => {
  acc[type] = meta.label;
  return acc;
}, {} as Record<IndicatorType, string>);

/** Full display names for all indicator types */
export const INDICATOR_FULL_NAMES: Record<IndicatorType, string> = (
  Object.entries(INDICATOR_METADATA) as [IndicatorType, IndicatorMetadata][]
).reduce((acc, [type, meta]) => {
  acc[type] = meta.fullName ?? meta.label;
  return acc;
}, {} as Record<IndicatorType, string>);

/** Indicator types that render as chart overlays */
export const OVERLAY_INDICATOR_TYPES: IndicatorType[] = (
  Object.entries(INDICATOR_METADATA) as [IndicatorType, IndicatorMetadata][]
).filter(([, meta]) => meta.isOverlay).map(([type]) => type);

/**
 * Get indicator types available for a specific context.
 * Returns all indicators except those that have the context in their excludeFrom list.
 */
export const getIndicatorsFor = (context: IndicatorContext): IndicatorType[] =>
  ALL_INDICATOR_TYPES.filter(type =>
    !INDICATOR_METADATA[type].excludeFrom?.includes(context)
  );

// Legacy: kept for backward compatibility, now derived from getIndicatorsFor
export const DIVERGENCE_INDICATOR_TYPES: IndicatorType[] = getIndicatorsFor('divergence');

// Indicator outputs by type (for reference in UI)
export const INDICATOR_OUTPUTS: Record<IndicatorType, string[]> = {
  sma: ['value'],
  ema: ['value'],
  rsi: ['value'],
  atr: ['value'],
  adx: ['value', 'plus_di', 'minus_di'],
  ichimoku: ['tenkan', 'kijun', 'senkou_a', 'senkou_b', 'chikou', 'cloud_top', 'cloud_bottom'],
  chandelier: ['exit_long', 'exit_short'],
  bollinger: ['upper', 'middle', 'lower'],
  macd: ['macd', 'signal', 'histogram'],
  stochastic: ['k', 'd'],
  ma_histogram: ['histogram', 'fast_ma', 'slow_ma'],
  ma_bands: ['upper', 'middle', 'lower'],
  dss: ['dss', 'signal'],
  adr: ['value', 'ratio'],
  daily: ['high', 'low', 'range', 'open'],
  swing: ['recent_high', 'recent_high_bars', 'recent_low', 'recent_low_bars', 'prev_high', 'prev_high_bars', 'prev_low', 'prev_low_bars'],
  mfi: ['value'],
  donchian: ['upper', 'middle', 'lower'],
  vwap: ['vwap'],
  parabolic_sar: ['sar', 'trend'],
  super_trend: ['supertrend', 'trend'],
};

// Friendly display names for indicator outputs
export const OUTPUT_LABELS: Record<string, string> = {
  // Ichimoku
  tenkan: 'Conversion (tenkan)',
  kijun: 'Base (kijun)',
  senkou_a: 'Leading Span A (senkou_a)',
  senkou_b: 'Leading Span B (senkou_b)',
  chikou: 'Lagging (chikou)',
  cloud_top: 'Cloud Top',
  cloud_bottom: 'Cloud Bottom',
  // Chandelier
  exit_long: 'Exit Long',
  exit_short: 'Exit Short',
  // Bollinger / MA Bands
  upper: 'Upper Band',
  middle: 'Middle Band',
  lower: 'Lower Band',
  // MACD / MA Histogram
  macd: 'MACD Line',
  signal: 'Signal Line',
  histogram: 'Histogram',
  fast_ma: 'Fast MA',
  slow_ma: 'Slow MA',
  // Stochastic
  k: '%K (fast)',
  d: '%D (slow)',
  // DSS
  dss: 'DSS Line',
  // ADX
  plus_di: '+DI (Plus Directional)',
  minus_di: '-DI (Minus Directional)',
  // ADR
  ratio: 'Range Ratio (%)',
  // Daily
  range: 'Range',
  // Swing
  recent_high: 'Recent Swing High',
  recent_high_bars: 'Bars Since Recent High',
  recent_low: 'Recent Swing Low',
  recent_low_bars: 'Bars Since Recent Low',
  prev_high: 'Previous Swing High',
  prev_high_bars: 'Bars Since Previous High',
  prev_low: 'Previous Swing Low',
  prev_low_bars: 'Bars Since Previous Low',
  // VWAP
  vwap: 'VWAP',
  // Parabolic SAR
  sar: 'SAR',
  trend: 'Trend',
  // SuperTrend
  supertrend: 'SuperTrend',
  // Generic
  value: 'Value',
};

// Default parameters by indicator type
export const INDICATOR_DEFAULTS: Record<IndicatorType, Record<string, number>> = {
  sma: { period: 20 },
  ema: { period: 20 },
  rsi: { period: 14 },
  atr: { period: 14 },
  adx: { period: 14 },
  ichimoku: { tenkan_period: 9, kijun_period: 26, senkou_b_period: 52, displacement: 26 },
  chandelier: { period: 22, multiplier: 3 },
  bollinger: { period: 20, std_dev: 2 },
  macd: { fast_period: 12, slow_period: 26, signal_period: 9 },
  stochastic: { k_period: 14, d_period: 3 },
  ma_histogram: { fast_period: 5, slow_period: 13 },
  ma_bands: { period: 20, distance: 20 },
  dss: { stoch_period: 13, ema_period: 8, signal_period: 8 },
  adr: { period: 14 },
  daily: {},  // No parameters
  swing: { strength: 5 },  // Bars on each side to confirm swing
  mfi: { period: 14 },    // Same default as RSI
  donchian: { period: 20 },  // N-bar high/low channel
  vwap: {},
  parabolic_sar: { af_start: 0.02, af_increment: 0.02, af_max: 0.2 },
  super_trend: { period: 10, multiplier: 3 },
};

// ============================================================================
// Data Sources
// ============================================================================

/**
 * When to capture/evaluate a data source value.
 * - 'each_candle': Evaluate fresh on each candle (default, dynamic)
 * - 'at_entry': Capture value when trade opens, use as fixed reference
 */
export type CaptureMode = 'each_candle' | 'at_entry';

/**
 * Trailing configuration for captured values.
 * Only applies when capture='at_entry'.
 */
export interface TrailConfig {
  enabled: boolean;
  /** Trail by this percentage in the favorable direction */
  percent?: number;
}

export interface IndicatorSource {
  indicator: string;               // ID of indicator in strategy
  output: string;                  // Which output to use
  offset?: ParameterizedValue;     // Bars back (0 = current, 1 = previous)
  symbol?: string;                 // For cross-symbol (defaults to config instrument)
  /** Timeframe for this data source (e.g., 'D' for daily). Defaults to strategy's timeframe. */
  timeframe?: string;
  /** When to capture this value (default: 'each_candle') */
  capture?: CaptureMode;
  /** Trailing behavior (only applies when capture='at_entry') */
  trail?: TrailConfig;
}

export interface PriceSource {
  source: 'price';
  value: 'open' | 'high' | 'low' | 'close';
  offset?: ParameterizedValue;
  symbol?: string;
  /** Timeframe for this data source (e.g., 'D' for daily). Defaults to strategy's timeframe. */
  timeframe?: string;
  /** When to capture this value (default: 'each_candle') */
  capture?: CaptureMode;
  /** Trailing behavior (only applies when capture='at_entry') */
  trail?: TrailConfig;
}

export interface FixedSource {
  fixed: number;
}

/**
 * Reference to a strategy parameter as a data source.
 * Used when a DataSource field should come from a parameter.
 */
export interface ParameterSource {
  $param: string;                     // References ParameterDefinition.id
}

// ============================================================================
// Candlestick Patterns
// ============================================================================

export type CandlestickPattern =
  | 'bullish_engulfing'
  | 'bearish_engulfing'
  | 'hammer'
  | 'inverted_hammer'
  | 'doji'
  | 'pin_bar'
  | 'morning_star'
  | 'evening_star'
  | 'bullish_harami'
  | 'bearish_harami';

export interface PatternSource {
  source: 'pattern';
  pattern: CandlestickPattern;
  offset?: number;
}

export const CANDLESTICK_PATTERN_LABELS: Record<CandlestickPattern, string> = {
  bullish_engulfing: 'Bullish Engulfing',
  bearish_engulfing: 'Bearish Engulfing',
  hammer: 'Hammer',
  inverted_hammer: 'Inverted Hammer',
  doji: 'Doji',
  pin_bar: 'Pin Bar',
  morning_star: 'Morning Star',
  evening_star: 'Evening Star',
  bullish_harami: 'Bullish Harami',
  bearish_harami: 'Bearish Harami',
};

export type DataSource = IndicatorSource | PriceSource | FixedSource | ParameterSource | PatternSource | number;

// Type guards

export const isIndicatorSource = (source: DataSource): source is IndicatorSource => {
  return typeof source === 'object' && source !== null && 'indicator' in source;
};

export const isPriceSource = (source: DataSource): source is PriceSource => {
  return typeof source === 'object' && source !== null && 'source' in source && source.source === 'price';
};

export const isFixedSource = (source: DataSource): source is FixedSource => {
  return typeof source === 'object' && source !== null && 'fixed' in source;
};

export const isParameterSource = (source: DataSource): source is ParameterSource => {
  return typeof source === 'object' && source !== null && '$param' in source;
};

export const isPatternSource = (source: DataSource): source is PatternSource => {
  return typeof source === 'object' && source !== null && 'source' in source && source.source === 'pattern';
};

// ============================================================================
// Triggers
// ============================================================================

export interface CrossTrigger {
  type: 'cross';
  left: DataSource;
  right: DataSource;
  direction: 'above' | 'below';
  /** Number of candles to look back for the condition (default: 1 = current candle only) */
  lookback?: ParameterizedValue;
}

export interface CompareTrigger {
  type: 'compare';
  left: DataSource;
  operator: 'above' | 'below' | 'equals' | 'gte' | 'lte';
  right: DataSource;
  /** Number of candles to look back for the condition (default: 1 = current candle only) */
  lookback?: ParameterizedValue;
}

export interface ThresholdTrigger {
  type: 'threshold';
  source: DataSource;
  operator: 'above' | 'below' | 'crosses_above' | 'crosses_below';
  value: ParameterizedValue;
  /** Number of candles to look back for the condition (default: 1 = current candle only) */
  lookback?: ParameterizedValue;
}

export interface PriceCrossesTrigger {
  type: 'price_crosses';
  source: { source: 'price'; value: 'high' | 'low' | 'close' };
  level: DataSource | { fixed: number };
  direction: 'above' | 'below';
  /** Number of candles to look back for the condition (default: 1 = current candle only) */
  lookback?: ParameterizedValue;
}

export interface RiskRewardTrigger {
  type: 'risk_reward_reached';
  ratio: ParameterizedValue;
}

export interface PercentOfTpTrigger {
  type: 'percent_of_tp_reached';
  percent: ParameterizedValue;
}

export interface TimeTrigger {
  type: 'time';
  condition: 'bar_count' | 'minutes' | 'hours';
  value: ParameterizedValue;       // bars, minutes, or hours since entry
}

export interface TimeInRangeTrigger {
  type: 'time_in_range';
  start_hour: number;    // 0-23 UTC
  start_minute: number;  // 0-59
  end_hour: number;      // 0-23 UTC
  end_minute: number;    // 0-59
}

export interface DayOfWeekTrigger {
  type: 'day_of_week';
  days: number[];     // 0=Sun, 1=Mon, ..., 6=Sat
  exclude: boolean;   // true = don't trade on these days
}

export interface RiskToleranceTrigger {
  type: 'risk_tolerance';
  condition:
    | 'daily_loss_percent'         // % of daily loss limit used
    | 'account_drawdown_percent'   // % drawdown from account high
    | 'trade_loss_percent'         // % loss on current trade
    | 'trade_loss_atr';            // Loss exceeds N x ATR
  threshold: ParameterizedValue;   // Percentage or ATR multiplier
  atr_period?: number;             // ATR period (for trade_loss_atr)
}

export const RISK_TOLERANCE_LABELS: Record<RiskToleranceTrigger['condition'], string> = {
  daily_loss_percent: 'Daily Loss Limit %',
  account_drawdown_percent: 'Account Drawdown %',
  trade_loss_percent: 'Trade Loss %',
  trade_loss_atr: 'Trade Loss (ATR)',
};

export const RISK_TOLERANCE_DESCRIPTIONS: Record<RiskToleranceTrigger['condition'], string> = {
  daily_loss_percent: 'Exit when daily loss reaches X% of daily limit',
  account_drawdown_percent: 'Exit when account drops X% from peak balance',
  trade_loss_percent: 'Exit when this trade loses X% of entry value',
  trade_loss_atr: 'Exit when loss exceeds X times the ATR',
};

export interface CompositeTrigger {
  type: 'composite';
  logic: 'and' | 'or';
  triggers: Trigger[];
}

export interface SRZoneDistance {
  type: 'pips' | 'atr';
  value: ParameterizedValue;
  atr_period?: number;  // Required if type='atr'
}

export interface SRZoneTrigger {
  type: 'sr_zone';
  condition: 'near' | 'enters' | 'breaks';
  target: 'upper' | 'lower' | 'either';
  direction?: 'above' | 'below';  // Required for 'breaks' condition
  distance: SRZoneDistance;
  zone_id?: string;  // Specific zone ID, or undefined for "any zone"
  /** Number of candles to look back for the condition (default: 1 = current candle only) */
  lookback?: ParameterizedValue;
}

export const SR_ZONE_CONDITION_LABELS: Record<SRZoneTrigger['condition'], string> = {
  near: 'Price Near Zone',
  enters: 'Price Enters Zone',
  breaks: 'Price Breaks Zone',
};

export const SR_ZONE_CONDITION_DESCRIPTIONS: Record<SRZoneTrigger['condition'], string> = {
  near: 'Triggers when price is within distance of zone boundary',
  enters: 'Triggers when price moves into the zone area',
  breaks: 'Triggers when price closes beyond zone boundary',
};

export const SR_ZONE_TARGET_LABELS: Record<SRZoneTrigger['target'], string> = {
  upper: 'Upper Boundary',
  lower: 'Lower Boundary',
  either: 'Either Boundary',
};

// ============================================================================
// Pivot Point Types
// ============================================================================

export type PivotLevel = 'pp' | 'r1' | 'r2' | 'r3' | 's1' | 's2' | 's3';
export type PivotPeriod = 'daily' | 'weekly';

export interface PivotConfig {
  enabled: boolean;
  period: PivotPeriod;
}

export interface PivotTrigger {
  type: 'pivot';
  level: PivotLevel;
  condition: 'near' | 'crosses' | 'breaks';
  direction?: 'above' | 'below';  // Required for 'crosses' and 'breaks' conditions
  distance: SRZoneDistance;        // Reuse distance config from S/R zones
  /** Number of candles to look back for the condition (default: 1 = current candle only) */
  lookback?: ParameterizedValue;
}

export const PIVOT_LEVEL_LABELS: Record<PivotLevel, string> = {
  r3: 'PP+3',
  r2: 'PP+2',
  r1: 'PP+1',
  pp: 'PP',
  s1: 'PP-1',
  s2: 'PP-2',
  s3: 'PP-3',
};

export const PIVOT_CONDITION_LABELS: Record<PivotTrigger['condition'], string> = {
  near: 'Price Near Level',
  crosses: 'Price Crosses Level',
  breaks: 'Price Breaks Level',
};

export const PIVOT_CONDITION_DESCRIPTIONS: Record<PivotTrigger['condition'], string> = {
  near: 'Triggers when price is within distance of pivot level',
  crosses: 'Triggers when price crosses through the pivot level',
  breaks: 'Triggers when price closes beyond the pivot level',
};

export const PIVOT_PERIOD_LABELS: Record<PivotPeriod, string> = {
  daily: 'Daily Pivots',
  weekly: 'Weekly Pivots',
};

export type Trigger =
  | CrossTrigger
  | CompareTrigger
  | ThresholdTrigger
  | PriceCrossesTrigger
  | RiskRewardTrigger
  | PercentOfTpTrigger
  | TimeTrigger
  | TimeInRangeTrigger
  | DayOfWeekTrigger
  | RiskToleranceTrigger
  | CompositeTrigger
  | SRZoneTrigger
  | PivotTrigger;

// ============================================================================
// Entry Rules
// ============================================================================

export interface EntryRule {
  id: string;
  name?: string;                   // Human-readable name
  direction: 'long' | 'short' | 'both';
  trigger: Trigger;
  weight?: number;                 // For weighted scoring (default: 1)
  required?: boolean;              // Must be true for entry (default: false)
}

export interface EntryLogic {
  mode: 'all' | 'any' | 'weighted';
  min_score?: number;              // For weighted mode
}

// ============================================================================
// Exit Rules
// ============================================================================

export type ExitRuleType =
  | 'stop_loss'
  | 'take_profit'
  | 'take_profit_partial'
  | 'trailing_stop'
  | 'signal'
  | 'time_based'
  | 'risk_tolerance';        // Exit based on account/daily risk limits

export interface ExitAction {
  close_percent: number;           // 1-100
  move_stop_to?: 'breakeven' | 'partial_level';
}

export interface ExitRule {
  id: string;
  name?: string;
  type: ExitRuleType;
  direction: 'long' | 'short' | 'both';  // Which position direction this rule applies to
  trigger: Trigger;
  action: ExitAction;
  priority?: number;               // Higher = evaluated first (default: 0)
}

// ============================================================================
// Risk Settings
// ============================================================================

export type RiskMethod = 'percent' | 'fixed';

export const RISK_METHOD_LABELS: Record<RiskMethod, string> = {
  percent: '% of Balance',
  fixed: 'Fixed Dollar ($)',
};

export const RISK_METHOD_DESCRIPTIONS: Record<RiskMethod, string> = {
  percent: 'Risk a percentage of your account balance per trade',
  fixed: 'Risk a specific dollar amount per trade',
};

// ============================================================================
// Stop Loss Source (Custom Stop for R:R and Position Sizing)
// ============================================================================

export type StopLossSourceType = 'indicator' | 'fixed_pips' | 'percent' | 'variable';

export const STOP_LOSS_SOURCE_LABELS: Record<StopLossSourceType, string> = {
  indicator: 'Indicator Value',
  fixed_pips: 'Fixed Pips',
  percent: 'Percent of Account',
  variable: 'Custom Variable',
};

export const STOP_LOSS_SOURCE_DESCRIPTIONS: Record<StopLossSourceType, string> = {
  indicator: 'Use an indicator value as the stop level (e.g., Kijun, EMA)',
  fixed_pips: 'Set stop at a fixed pip distance from entry',
  percent: 'Set stop at a percentage of account value from entry',
  variable: 'Use a computed variable as the stop level',
};

/**
 * Evaluation mode for variable-based stop loss.
 */
export type StopLossEvaluationMode = 'at_open' | 'trailing';

export const STOP_LOSS_EVALUATION_LABELS: Record<StopLossEvaluationMode, string> = {
  at_open: 'Fixed at Entry',
  trailing: 'Trailing (re-evaluate each candle)',
};

/**
 * Stop loss source configuration.
 * Determines how the conceptual stop level is calculated for:
 * - Position sizing (risk per trade)
 * - R:R calculations (risk_reward_reached trigger)
 * - Take profit calculations
 */
export type StopLossSource =
  | { type: 'indicator'; indicator: string; output: string; capture?: CaptureMode }
  | { type: 'fixed_pips'; pips: ParameterizedValue }
  | { type: 'percent'; percent: ParameterizedValue }  // Percent of account value
  | { type: 'variable'; variable: string; evaluation: StopLossEvaluationMode };

export interface RiskSettings {
  risk_method: RiskMethod;                // How to calculate risk
  risk_value: ParameterizedValue;         // 1 = 1% or $1 depending on method
  rr_ratio: ParameterizedValue;           // Risk:Reward ratio for TP calculation
  spread_buffer_pips: ParameterizedValue; // Spread/slippage buffer
  stop_loss_source?: StopLossSource;      // Custom stop loss source (defaults to 'auto')
  // Short trade overrides (when use_same_for_shorts is false / these are set)
  risk_method_short?: RiskMethod;
  risk_value_short?: ParameterizedValue;
  rr_ratio_short?: ParameterizedValue;
  spread_buffer_pips_short?: ParameterizedValue;
  stop_loss_source_short?: StopLossSource;
}

// ============================================================================
// Chat Messages (for AI planning conversations)
// ============================================================================

export interface ChatMessage {
  role: 'user' | 'assistant';
  content: string;
}

// ============================================================================
// Strategy (V2 only - V1 has been removed)
// ============================================================================

/**
 * Strategy type alias for StrategyV2.
 * @deprecated Use StrategyV2 directly. This alias exists for backward compatibility.
 */
export type Strategy = StrategyV2;

// ============================================================================
// Strategy Configuration
// ============================================================================

export interface StrategyConfig {
  id: string;
  strategy_id: string;
  user_id: string;
  name: string;                    // e.g., "Ichimoku EUR/USD H1"
  instrument: string;              // e.g., "EUR_USD"
  timeframe: string;               // e.g., "H1"
  indicator_params: Record<string, Record<string, number>>; // Override params
  risk_overrides?: Partial<RiskSettings>;
  is_live: boolean;                // Whether to show live signals
  created_at: number;
  updated_at: number;
}

// ============================================================================
// Helpers for creating strategies
// ============================================================================

export const createEmptyStrategy = (userId: string): Omit<Strategy, 'id' | 'created_at' | 'updated_at'> => ({
  user_id: userId,
  name: '',
  description: '',
  parameters: [],
  indicators: [],
  variables: [],
  entry_rules: [],
  entry_logic: { mode: 'all' },
  exit_rules: [],
  risk_settings: {
    risk_method: 'percent',
    risk_value: 1,
    rr_ratio: 2.0,
    spread_buffer_pips: 1,
  },
  version: 1,
  is_active: true,
  is_promoted: false,
  is_locked: false,
  is_archived: false,
});

export const createIndicator = (
  id: string,
  type: IndicatorType,
  params?: Record<string, number>
): IndicatorDefinition => ({
  id,
  type,
  params: params ?? { ...INDICATOR_DEFAULTS[type] },
});

// ============================================================================
// Optimization Types
// ============================================================================

export type OptimizationObjective =
  | 'sharpe_ratio'
  | 'profit_factor'
  | 'total_return'
  | 'win_rate'
  | 'min_drawdown'
  | 'trade_count';

export interface OptimizationMetrics {
  total_pnl: string;
  total_return_pct: string;
  winning_trades: number;
  losing_trades: number;
  win_rate: string;
  profit_factor: string;
  max_drawdown_pct: string;
  sharpe_ratio: string;
  total_trades: number;
  final_balance: string;
}

export interface OptimizationRun {
  params: Record<string, number>;
  metrics: OptimizationMetrics;
  score: number;
}

export interface OptimizationResult {
  total_combinations: number;
  valid_results: number;
  runs: OptimizationRun[];
  best_params: Record<string, number> | null;
  objective: OptimizationObjective;
}

export const OPTIMIZATION_OBJECTIVE_LABELS: Record<OptimizationObjective, string> = {
  sharpe_ratio: 'Sharpe Ratio',
  profit_factor: 'Profit Factor',
  total_return: 'Total Return %',
  win_rate: 'Win Rate',
  min_drawdown: 'Min Drawdown',
  trade_count: 'Trade Count',
};

// ============================================================================
// Backtest Methodology Types
// ============================================================================

export type BacktestMethodology =
  | 'simple'
  | 'train_test'
  | 'walk_forward'
  | 'anchored_walk_forward'
  | 'monte_carlo_sequence'
  | 'monte_carlo_parameter'
  | 'regime_based'
  | 'bootstrap';

export const METHODOLOGY_LABELS: Record<BacktestMethodology, string> = {
  simple: 'Simple Historical',
  train_test: 'Train/Test Split',
  walk_forward: 'Walk-Forward',
  anchored_walk_forward: 'Anchored Walk-Forward',
  monte_carlo_sequence: 'Monte Carlo (Sequence)',
  monte_carlo_parameter: 'Monte Carlo (Parameter)',
  regime_based: 'Regime-Based Testing',
  bootstrap: 'Bootstrap Analysis',
};

// ============================================================================
// Walk-Forward Analysis Types
// ============================================================================

export interface WalkForwardConfig {
  /** Training window duration in months */
  train_months: number;
  /** Test window duration in months */
  test_months: number;
  /** Step size for window advancement in months */
  step_months: number;
  /** Optimization objective for training periods */
  objective: OptimizationObjective;
  /** Minimum trades required per window */
  min_trades_per_window?: number;
  /** Whether to use anchored mode (expanding training window) */
  anchored: boolean;
}

export interface WalkForwardWindow {
  /** Window number (1-indexed) */
  window_num: number;
  /** Training period start date (RFC3339) */
  train_start: string;
  /** Training period end date (RFC3339) */
  train_end: string;
  /** Test period start date (RFC3339) */
  test_start: string;
  /** Test period end date (RFC3339) */
  test_end: string;
}

/** A simulated trade from a backtest (matches Rust SimulatedTrade with camelCase) */
export interface SimulatedTrade {
  /** Entry time (RFC3339) */
  entryTime: string;
  /** Exit time (RFC3339), null if still open */
  exitTime: string | null;
  /** Entry price (Decimal serialized as string) */
  entryPrice: string;
  /** Exit price, null if still open (Decimal serialized as string) */
  exitPrice: string | null;
  /** Position size in units (Decimal serialized as string) */
  units: string;
  /** Realized P&L (Decimal serialized as string) */
  pnl: string;
  /** True if long position, false if short */
  isLong: boolean;
  /** ID of the entry rule that triggered this trade */
  entryRuleId?: string;
  /** Name of the entry rule */
  entryRuleName?: string;
  /** Reason for exit (from exit rule) */
  exitReason?: string;
  /** Stop loss price at entry (Decimal serialized as string) */
  stopLoss?: string;
  /** Take profit price at entry (Decimal serialized as string) */
  takeProfit?: string;
  /** Indicator values at entry time (indicator_id or indicator_id.output -> value) */
  entryIndicators?: Record<string, string>;
}

export interface WalkForwardPeriod {
  /** Window timing information */
  window: WalkForwardWindow;
  /** Best parameters found during in-sample optimization */
  optimized_params: Record<string, number>;
  /** In-sample (training) performance metrics */
  in_sample_metrics: OptimizationMetrics;
  /** In-sample Sharpe ratio */
  in_sample_sharpe: number;
  /** Out-of-sample (test) performance metrics */
  out_of_sample_metrics: OptimizationMetrics;
  /** Out-of-sample Sharpe ratio */
  out_of_sample_sharpe: number;
  /** Number of trades in OOS period */
  oos_trade_count: number;
  /** Whether this period was profitable OOS */
  oos_profitable: boolean;
  /** Individual trades from the OOS backtest (for drill-down analysis) */
  oos_trades: SimulatedTrade[];
}

export interface ParameterStabilityInfo {
  /** Parameter ID */
  param_id: string;
  /** Parameter display name */
  param_name: string;
  /** Most frequently selected value across windows */
  mode_value: number;
  /** How many windows selected this value */
  mode_count: number;
  /** Total number of windows */
  total_windows: number;
  /** Stability percentage (mode_count / total_windows * 100) */
  stability_pct: number;
}

export interface WalkForwardResult {
  /** Configuration used for this analysis */
  config: WalkForwardConfig;
  /** All period results */
  periods: WalkForwardPeriod[];
  /** Number of periods generated */
  total_periods: number;
  /** Number of valid periods */
  valid_periods: number;
  /** Number of profitable OOS periods */
  profitable_periods: number;

  // Aggregated OOS Metrics
  /** Total OOS P&L across all test periods */
  oos_total_pnl: string;
  /** Total OOS return percentage */
  oos_total_return_pct: string;
  /** Average OOS Sharpe ratio */
  oos_avg_sharpe: number;
  /** OOS win rate */
  oos_win_rate: string;
  /** OOS max drawdown */
  oos_max_drawdown_pct: string;
  /** Total OOS trades */
  oos_total_trades: number;

  // Efficiency Metrics
  /** Walk-forward efficiency based on Sharpe ratio (OOS Sharpe / IS Sharpe) */
  sharpe_efficiency: number;
  /** Walk-forward efficiency based on returns (OOS Return / IS Return) */
  return_efficiency: number;
  /** Robustness score (0-100) */
  robustness_score: number;

  // Parameter Stability
  /** Stability analysis for each optimized parameter */
  parameter_stability: ParameterStabilityInfo[];

  // Stitched OOS Equity Curve
  oos_equity_curve: string[];
}

export type WalkForwardPhase = 'optimization' | 'testing';

export interface WalkForwardProgress {
  phase: WalkForwardPhase;
  windowNum: number;
  totalWindows: number;
  optimizationCurrent?: number;
  optimizationTotal?: number;
  percent: number;
  /** Training period start date (RFC3339) */
  trainStart?: string;
  /** Training period end date (RFC3339) */
  trainEnd?: string;
  /** Test period start date (RFC3339) */
  testStart?: string;
  /** Test period end date (RFC3339) */
  testEnd?: string;
  /** Strategy ID this progress belongs to (for filtering concurrent backtests) */
  strategyId?: string;
}

// ============================================================================
// Parameter Sweep Types
// ============================================================================

export interface SweepValueResult {
  value: number;
  oosTotalPnl: string;
  oosTotalReturnPct: string;
  oosAvgSharpe: number;
  oosTotalTrades: number;
  oosMaxDrawdownPct: string;
  oosWinRate: string;
}

export interface ParameterSweepResult {
  paramId: string;
  paramName: string;
  defaultValue: number;
  results: SweepValueResult[];
}

export interface ParameterSweepProgress {
  currentIndex: number;
  totalValues: number;
  currentValue: number;
  paramId: string;
}

// ============================================================================
// V2 Strategy Rule Builder Types
// ============================================================================
// These types support the redesigned 3-tab rule builder with:
// - Inline indicators (no separate Indicators tab)
// - AND/OR trigger chaining
// - Market regime "givens"
// - "is within" distance operator

/** Schema version for backward compatibility */
export type StrategySchemaVersion = 1 | 2;

// ============================================================================
// V2 Market Regimes (Givens)
// ============================================================================

/**
 * Predefined market regime conditions with hardcoded backend detection.
 * These are evaluated on the fly by the backtest engine.
 *
 * Regimes are grouped into categories:
 * 1. Trend/Volatility regimes - based on ADX, ATR, Bollinger Bands
 * 2. S/R regimes - based on user-defined zones
 * 3. Price action regimes - programmatically detected patterns (gaps, order blocks, etc.)
 */
export type MarketRegime =
  // Trend/Volatility Regimes
  | 'trending_up'        // ADX > 25, price > SMA20 > SMA50
  | 'trending_down'      // ADX > 25, price < SMA20 < SMA50
  | 'ranging'            // ADX < 20, BB width contracted
  | 'high_volatility'    // ATR > 1.5x rolling average ATR
  | 'low_volatility'     // ATR < 0.5x rolling average ATR
  // Custom Zone Regimes
  | 'sr_tested'          // Price within X pips of user's custom zone
  // Price Action Regimes - Gaps
  | 'at_bullish_gap'     // Price at/near unfilled bullish gap
  | 'at_bearish_gap'     // Price at/near unfilled bearish gap
  // Price Action Regimes - Supply/Demand Zones
  | 'at_demand_zone'     // Price at/near detected demand zone (DBR/RBR)
  | 'at_supply_zone'     // Price at/near detected supply zone (RBD/DBD)
  // Price Action Regimes - Order Blocks
  | 'at_bullish_ob'      // Price at/near bullish order block
  | 'at_bearish_ob'      // Price at/near bearish order block
  // Price Action Regimes - Structure
  | 'retesting_support'  // Retest of broken high (resistance that was broken now acting as support)
  | 'retesting_resistance' // Retest of broken low (support that was broken now acting as resistance)
  // Pivot Point Regimes
  | 'at_pivot'           // Price at/near a pivot point level (PP, R1-R3, S1-S3)
  // Trading Sessions (UTC)
  | 'london_session'     // 08:00-17:00 UTC
  | 'us_session'         // 13:00-22:00 UTC
  | 'asian_session'      // 00:00-09:00 UTC
  // Analysis Patterns
  | 'divergence';        // Price vs indicator divergence (requires config)

export type MarketRegimeCategory = 'trend_volatility' | 'sr_zones' | 'price_action' | 'sessions' | 'analysis';

export const MARKET_REGIME_CATEGORIES: Record<MarketRegimeCategory, { label: string; regimes: MarketRegime[] }> = {
  trend_volatility: {
    label: 'Trend & Volatility',
    regimes: ['trending_up', 'trending_down', 'ranging', 'high_volatility', 'low_volatility'],
  },
  sessions: {
    label: 'Trading Sessions',
    regimes: ['london_session', 'us_session', 'asian_session'],
  },
  analysis: {
    label: 'Analysis Patterns',
    regimes: ['divergence'],
  },
  sr_zones: {
    label: 'Custom Zones',
    regimes: ['sr_tested'],
  },
  price_action: {
    label: 'Price Action Patterns',
    regimes: [
      'at_bullish_gap', 'at_bearish_gap',
      'at_demand_zone', 'at_supply_zone',
      'at_bullish_ob', 'at_bearish_ob',
      'retesting_support', 'retesting_resistance',
      'at_pivot',
    ],
  },
};

export const MARKET_REGIME_LABELS: Record<MarketRegime, string> = {
  // Trend/Volatility
  trending_up: 'Trending Up',
  trending_down: 'Trending Down',
  ranging: 'Ranging/Consolidating',
  high_volatility: 'High Volatility',
  low_volatility: 'Low Volatility',
  // Custom Zones
  sr_tested: 'Custom Zone',
  // Price Action - Gaps
  at_bullish_gap: 'Bullish Gap',
  at_bearish_gap: 'Bearish Gap',
  // Price Action - Supply/Demand
  at_demand_zone: 'Demand Zone',
  at_supply_zone: 'Supply Zone',
  // Price Action - Order Blocks
  at_bullish_ob: 'Bullish Order Block',
  at_bearish_ob: 'Bearish Order Block',
  // Price Action - Structure
  retesting_support: 'Retest of Broken High',
  retesting_resistance: 'Retest of Broken Low',
  // Pivot Points
  at_pivot: 'Pivot Point',
  // Trading Sessions
  london_session: 'London Session',
  us_session: 'US Session',
  asian_session: 'Asian Session',
  // Analysis Patterns
  divergence: 'Divergence',
};

export const MARKET_REGIME_DESCRIPTIONS: Record<MarketRegime, string> = {
  // Trend/Volatility
  trending_up: 'ADX > 25 with price above SMA20 above SMA50',
  trending_down: 'ADX > 25 with price below SMA20 below SMA50',
  ranging: 'ADX < 20 with contracted Bollinger Band width',
  high_volatility: 'ATR is 1.5x or more above its rolling average',
  low_volatility: 'ATR is 0.5x or less below its rolling average',
  // Custom Zones
  sr_tested: 'Price is within 20 pips of a user-defined custom zone boundary',
  // Price Action - Gaps
  at_bullish_gap: 'Price is at or near an unfilled bullish gap (previous resistance now support)',
  at_bearish_gap: 'Price is at or near an unfilled bearish gap (previous support now resistance)',
  // Price Action - Supply/Demand
  at_demand_zone: 'Price is at or near a detected demand zone (Drop-Base-Rally or Rally-Base-Rally formation)',
  at_supply_zone: 'Price is at or near a detected supply zone (Rally-Base-Drop or Drop-Base-Drop formation)',
  // Price Action - Order Blocks
  at_bullish_ob: 'Price is at or near a bullish order block (last bearish candle before bullish impulse)',
  at_bearish_ob: 'Price is at or near a bearish order block (last bullish candle before bearish impulse)',
  // Price Action - Structure
  retesting_support: 'Price is retesting a broken swing high (previous resistance now acting as support)',
  retesting_resistance: 'Price is retesting a broken swing low (previous support now acting as resistance)',
  // Pivot Points
  at_pivot: 'Price is at or near a pivot point level (select level and period below)',
  // Trading Sessions
  london_session: 'Current time is within London trading hours (08:00-17:00 UTC)',
  us_session: 'Current time is within US trading hours (13:00-22:00 UTC)',
  asian_session: 'Current time is within Asian trading hours (00:00-09:00 UTC)',
  // Analysis Patterns
  divergence: 'Detects divergence between price swing points and an indicator (configure below)',
};

// ============================================================================
// V2 Data Sources
// ============================================================================

export interface SRZoneSource {
  type: 'sr_zone';
  target: 'upper' | 'lower' | 'midpoint';
  zone_id?: string;  // Specific zone ID, or undefined for nearest
}

export interface PivotSource {
  type: 'pivot';
  level: PivotLevel;
  period: PivotPeriod;
}

// ============================================================================
// Strategy Variables (Named Computed Values)
// ============================================================================

/**
 * Variable expression types supported by the engine.
 */
export type VariableExpressionType = 'distance' | 'ratio' | 'change' | 'value'
  | 'abs' | 'negate' | 'min' | 'max'
  | 'highest' | 'lowest' | 'sum' | 'average' | 'conditional';

/**
 * Math operators for value expressions.
 */
export type MathOperator = '+' | '-' | '*' | '/' | '**' | '%';

/**
 * A single math operation in a value expression chain.
 * Evaluated left-to-right (no operator precedence).
 */
export interface MathOperation {
  operator: MathOperator;
  operand: DataSourceV2;
}

/**
 * Distance expression: left - right (or |left - right| if absolute)
 */
export interface DistanceExpression {
  type: 'distance';
  left: DataSourceV2;
  right: DataSourceV2;
  /** If true, returns |left - right|. Default: false (signed) */
  absolute?: boolean;
}

/**
 * Ratio expression: numerator / denominator
 * Useful for percentage comparisons
 */
export interface RatioExpression {
  type: 'ratio';
  numerator: DataSourceV2;
  denominator: DataSourceV2;
}

/**
 * Change expression: source[bars] - source[0]
 * Measures velocity/momentum - how much a value changed over N bars
 */
export interface ChangeExpression {
  type: 'change';
  source: DataSourceV2;
  /** How many bars back to compare (e.g., 3 = value[3] - value[0]) */
  bars: number;
}

/**
 * Value expression: a data source with optional math operations.
 * Evaluated left-to-right. For PEMDAS, chain multiple variables.
 *
 * Examples:
 * - Just capture: { type: 'value', source: chandelier.exit_long }
 * - With math: { type: 'value', source: atr, operations: [{ operator: '*', operand: { fixed: 2.5 } }] }
 */
export interface ValueExpression {
  type: 'value';
  source: DataSourceV2;
  /** Optional chain of math operations, evaluated left-to-right */
  operations?: MathOperation[];
}

/**
 * Abs expression: |source|
 */
export interface AbsExpression {
  type: 'abs';
  source: DataSourceV2;
}

/**
 * Negate expression: -source
 */
export interface NegateExpression {
  type: 'negate';
  source: DataSourceV2;
}

/**
 * Min expression: min(left, right) per bar
 */
export interface MinExpression {
  type: 'min';
  left: DataSourceV2;
  right: DataSourceV2;
}

/**
 * Max expression: max(left, right) per bar
 */
export interface MaxExpression {
  type: 'max';
  left: DataSourceV2;
  right: DataSourceV2;
}

/**
 * Highest expression: rolling max of source over N bars
 */
export interface HighestExpression {
  type: 'highest';
  source: DataSourceV2;
  period: ParameterizedValue;
}

/**
 * Lowest expression: rolling min of source over N bars
 */
export interface LowestExpression {
  type: 'lowest';
  source: DataSourceV2;
  period: ParameterizedValue;
}

/**
 * Sum expression: rolling sum of source over N bars
 */
export interface SumExpression {
  type: 'sum';
  source: DataSourceV2;
  period: ParameterizedValue;
}

/**
 * Average expression: rolling mean of source over N bars
 */
export interface AverageExpression {
  type: 'average';
  source: DataSourceV2;
  period: ParameterizedValue;
}

/**
 * Conditional expression: if condition_left op condition_right then true_value else false_value
 */
export interface ConditionalExpression {
  type: 'conditional';
  condition_left: DataSourceV2;
  operator: CompareOperatorV2;
  condition_right: DataSourceV2;
  true_value: DataSourceV2;
  false_value: DataSourceV2;
}

export type VariableExpression =
  | DistanceExpression | RatioExpression | ChangeExpression | ValueExpression
  | AbsExpression | NegateExpression | MinExpression | MaxExpression
  | HighestExpression | LowestExpression | SumExpression | AverageExpression
  | ConditionalExpression;

/**
 * A named variable definition.
 * Variables are computed values that can be referenced in triggers.
 */
export interface StrategyVariable {
  id: string;           // Unique ID, referenced in VariableSource
  name: string;         // Display name (e.g., "TK Gap")
  description?: string; // Optional explanation
  expression: VariableExpression;
}

/**
 * Reference to a variable as a data source.
 * Offset allows comparing the variable at different times.
 */
export interface VariableSource {
  type: 'variable';
  variable: string;     // ID of variable in strategy.variables
  offset?: number;      // Bars back (0 = current, default)
}

export type DataSourceV2 =
  | IndicatorSource          // Reference to indicator in strategy.indicators
  | PriceSource              // OHLC price
  | FixedSource              // Fixed numeric value
  | ParameterSource          // Parameter reference
  | SRZoneSource             // S/R zone level
  | PivotSource              // Pivot point level
  | VariableSource           // Reference to a named variable
  | PatternSource            // Candlestick pattern detection
  | number;                  // Bare numeric (resolved $param)

/** Convert bare numeric DataSourceV2 to FixedSource. Use at component boundaries. */
export const normalizeDataSource = <T extends DataSource | DataSourceV2>(source: T): Exclude<T, number> => {
  if (typeof source === 'number') return { fixed: source } as Exclude<T, number>;
  return source as Exclude<T, number>;
};

// Type guards for V2 data sources
export const isIndicatorSourceV2 = (source: DataSourceV2): source is IndicatorSource => {
  return typeof source === 'object' && source !== null && 'indicator' in source && typeof source.indicator === 'string';
};

export const isSRZoneSource = (source: DataSourceV2): source is SRZoneSource => {
  return typeof source === 'object' && source !== null && 'type' in source && source.type === 'sr_zone';
};

export const isPivotSource = (source: DataSourceV2): source is PivotSource => {
  return typeof source === 'object' && source !== null && 'type' in source && source.type === 'pivot';
};

export const isVariableSource = (source: DataSourceV2): source is VariableSource => {
  return typeof source === 'object' && source !== null && 'type' in source && source.type === 'variable';
};

export const isFixedSourceV2 = (source: DataSourceV2): source is FixedSource => {
  return typeof source === 'object' && source !== null && 'fixed' in source;
};

export const isPriceSourceV2 = (source: DataSourceV2): source is PriceSource => {
  return typeof source === 'object' && source !== null && 'source' in source && source.source === 'price';
};

export const isParameterSourceV2 = (source: DataSourceV2): source is ParameterSource => {
  return typeof source === 'object' && source !== null && '$param' in source;
};

export const isPatternSourceV2 = (source: DataSourceV2): source is PatternSource => {
  return typeof source === 'object' && source !== null && 'source' in source && source.source === 'pattern';
};

// ============================================================================
// V2 Distance Config (for "is within" operator)
// ============================================================================

export type DistanceUnit = 'pips' | 'atr' | 'percent';

export interface DistanceConfig {
  value: ParameterizedValue;
  unit: DistanceUnit;
  atr_period?: number;  // Required if unit='atr'
}

export const DISTANCE_UNIT_LABELS: Record<DistanceUnit, string> = {
  pips: 'Pips',
  atr: 'ATR',
  percent: 'Percent',
};

// ============================================================================
// V2 Triggers
// ============================================================================

/**
 * Givens trigger - market regime condition.
 * No left/right operands - just a regime to check.
 */
export interface GivensTriggerV2 {
  type: 'givens';
  regime: MarketRegime;
  /** Pivot level - only used when regime is 'at_pivot' */
  pivot_level?: PivotLevel;
  /** Pivot period - only used when regime is 'at_pivot' */
  pivot_period?: PivotPeriod;
  /** Divergence type - only used when regime is 'divergence' */
  divergence_type?: DivergenceType;
  /** Indicator ID - only used when regime is 'divergence' */
  divergence_indicator?: string;
  /** Indicator output - only used when regime is 'divergence' */
  divergence_output?: string;
  /** Lookback bars - only used when regime is 'divergence' */
  divergence_lookback?: number;
  /** Swing strength - only used when regime is 'divergence' */
  divergence_swing_strength?: number;
}

/**
 * V2 Compare operators including "is_within" for distance checks.
 */
export type CompareOperatorV2 =
  | '>'
  | '<'
  | '>='
  | '<='
  | '=='
  | '!='
  | 'is_within';

/**
 * Cross trigger V2 - supports V2 data sources including inline indicators.
 */
export interface CrossTriggerV2 {
  type: 'cross';
  left: DataSourceV2;
  right: DataSourceV2;
  direction: 'above' | 'below';
  lookback?: ParameterizedValue;
}

/**
 * Compare trigger V2 - supports V2 operators and data sources.
 * When operator is 'is_within', the distance config specifies the threshold.
 */
export interface CompareTriggerV2 {
  type: 'compare';
  left: DataSourceV2;
  operator: CompareOperatorV2;
  right: DataSourceV2;
  distance?: DistanceConfig;  // Required when operator is 'is_within'
  lookback?: ParameterizedValue;
}

/**
 * Threshold trigger V2 - compares a data source against a fixed value.
 * Uses same operators as compare trigger but with simplified source/value structure.
 */
export interface ThresholdTriggerV2 {
  type: 'threshold';
  source: DataSourceV2;
  operator: '>' | '<' | '>=' | '<=' | '==' | '!=' | 'crosses_above' | 'crosses_below';
  value: ParameterizedValue;
  lookback?: ParameterizedValue;
}

/**
 * Divergence types for price vs indicator analysis.
 */
export type DivergenceType = 'bullish' | 'bearish' | 'hidden_bullish' | 'hidden_bearish';

export const DIVERGENCE_TYPE_LABELS: Record<DivergenceType, string> = {
  bullish: 'Bullish',
  bearish: 'Bearish',
  hidden_bullish: 'Hidden Bullish',
  hidden_bearish: 'Hidden Bearish',
};

export const DIVERGENCE_TYPE_DESCRIPTIONS: Record<DivergenceType, string> = {
  bullish: 'Price makes lower low, indicator makes higher low (signals reversal up)',
  bearish: 'Price makes higher high, indicator makes lower high (signals reversal down)',
  hidden_bullish: 'Price makes higher low, indicator makes lower low (signals continuation up)',
  hidden_bearish: 'Price makes lower high, indicator makes higher high (signals continuation down)',
};

export type TriggerV2 =
  | GivensTriggerV2
  | CrossTriggerV2
  | CompareTriggerV2
  | ThresholdTriggerV2
  | TimeTrigger
  | TimeInRangeTrigger
  | DayOfWeekTrigger
  | RiskRewardTrigger
  | PercentOfTpTrigger;

// Type guards for V2 triggers
export const isGivensTrigger = (trigger: TriggerV2): trigger is GivensTriggerV2 => {
  return trigger.type === 'givens';
};

export const isCrossTriggerV2 = (trigger: TriggerV2): trigger is CrossTriggerV2 => {
  return trigger.type === 'cross';
};

export const isCompareTriggerV2 = (trigger: TriggerV2): trigger is CompareTriggerV2 => {
  return trigger.type === 'compare';
};

export const isThresholdTriggerV2 = (trigger: TriggerV2): trigger is ThresholdTriggerV2 => {
  return trigger.type === 'threshold';
};

export const isTimeTrigger = (trigger: TriggerV2): trigger is TimeTrigger => {
  return trigger.type === 'time';
};

export const isTimeInRangeTrigger = (trigger: TriggerV2): trigger is TimeInRangeTrigger => {
  return trigger.type === 'time_in_range';
};

export const isDayOfWeekTrigger = (trigger: TriggerV2): trigger is DayOfWeekTrigger => {
  return trigger.type === 'day_of_week';
};

export const isRiskRewardTrigger = (trigger: TriggerV2): trigger is RiskRewardTrigger => {
  return trigger.type === 'risk_reward_reached';
};

export const isPercentOfTpTrigger = (trigger: TriggerV2): trigger is PercentOfTpTrigger => {
  return trigger.type === 'percent_of_tp_reached';
};

// ============================================================================
// V2 Conditions (AND/OR chaining with NOT support)
// ============================================================================

export type ChainOperator = 'and' | 'or';

/**
 * A trigger with an optional NOT flag.
 * When negated=true, the trigger must evaluate to FALSE for the condition to pass.
 */
export interface TriggerWithNot {
  trigger: TriggerV2;
  negated: boolean;
}

/**
 * A chained trigger with AND/OR operator and NOT flag.
 */
export interface ChainedTriggerWithNot {
  operator: ChainOperator;
  trigger: TriggerWithNot;
}

/**
 * A Condition is a group of triggers with AND/OR logic.
 * - Primary trigger is always required
 * - Chain contains additional triggers with AND/OR operators
 * - Each trigger can be negated (NOT) to require it to be FALSE
 *
 * Example: [NOT] A AND [NOT] B OR [NOT] C
 */
export interface Condition {
  /** Optional name for the condition (for display purposes only) */
  name?: string;
  primary: TriggerWithNot;
  chain: ChainedTriggerWithNot[];
  /** Skip this condition during evaluation. Can be parameterized for optimization sweeps. */
  disabled?: ParameterizedValue;
}

// ============================================================================
// V2 Entry Rules
// ============================================================================

// ============================================================================
// Entry Order Types (for pending/limit/stop orders)
// ============================================================================

/**
 * Type of entry order.
 * - market: Execute immediately at next candle open (current/default behavior)
 * - buy_stop: Buy when price rises to level (breakout long)
 * - sell_stop: Sell when price falls to level (breakout short)
 * - buy_limit: Buy when price falls to level (pullback long)
 * - sell_limit: Sell when price rises to level (pullback short)
 */
export type EntryOrderType = 'market' | 'buy_stop' | 'sell_stop' | 'buy_limit' | 'sell_limit';

/**
 * Configuration for a pending entry order (stop or limit).
 * Attached to an EntryRule to specify that matched signals should create
 * pending orders instead of immediate market entries.
 */
export interface PendingOrderConfig {
  order_type: EntryOrderType;
  price: DataSourceV2;        // The trigger price level (resolved from a DataSource)
  expiry_bars?: number;       // Cancel after N bars if not filled
}

/**
 * Resolved pending order info at runtime (concrete price from DataSource).
 */
export interface PendingOrderInfo {
  order_type: EntryOrderType;
  price: number;              // The resolved trigger price
  expiry_bars?: number;       // Cancel after N bars if not filled
}

/**
 * V2 Entry Rule with multiple conditions.
 * - All conditions are ANDed together (all must be true)
 * - Multiple rules for same direction are ORed (any can trigger)
 */
export interface EntryRuleV2 {
  id: string;
  name?: string;
  direction: 'long' | 'short' | 'both';
  conditions: Condition[];  // All conditions ANDed together
  pending_order?: PendingOrderConfig;  // Optional pending order configuration
}

// ============================================================================
// V2 Exit Rules
// ============================================================================

/**
 * V2 Exit Rule with multiple conditions.
 * - All conditions are ANDed together (all must be true)
 */
export interface ExitRuleV2 {
  id: string;
  name?: string;
  direction: 'long' | 'short' | 'both';
  conditions: Condition[];  // All conditions ANDed together
  close_percent: ParameterizedValue;    // 1-100, how much of position to close (can be linked to parameter)
  priority: number;         // Higher = evaluated first
}

// ============================================================================
// Legacy types (kept for backwards compatibility)
// ============================================================================

/** @deprecated Use ChainedTriggerWithNot instead */
export interface ChainedTrigger {
  operator: ChainOperator;
  trigger: TriggerV2;
}

/** @deprecated Use Condition instead */
export interface TriggerChain {
  primary: TriggerV2;
  chain: ChainedTrigger[];
}

// ============================================================================
// V2 Strategy Definition
// ============================================================================

/**
 * V2 Strategy uses the new rule builder structure.
 * - indicators array defines indicators, referenced by ID in triggers
 * - parameters array for optimizable values (used in WFT)
 * - entry_rules use TriggerChain instead of single Trigger
 * - exit_rules simplified (no exit types, just triggers + close_percent)
 */
export type StrategyType = 'rules' | 'scripted';

export interface StrategyV2 {
  id: string;
  user_id: string;
  name: string;
  description: string;
  schema_version?: number;  // Always 2 for V2 strategies, optional for backwards compat

  // Strategy type: 'rules' (default, visual builder) or 'scripted' (Rhai script)
  strategy_type?: StrategyType;

  // Rhai script source code (only set when strategy_type === 'scripted')
  script_content?: string;

  // Indicator definitions, referenced by ID in triggers
  indicators: IndicatorDefinition[];

  // Optimizable parameters for walk-forward testing
  parameters: ParameterDefinition[];

  // Named computed values (variables)
  variables?: StrategyVariable[];

  // V2 rules
  entry_rules: EntryRuleV2[];
  exit_rules: ExitRuleV2[];
  risk_settings: RiskSettings;

  // Entry logic (kept for backwards compatibility with saved data)
  entry_logic?: EntryLogic;

  // Optional configs
  pivot_config?: PivotConfig;
  planning_conversation?: ChatMessage[];
  auto_note_indicators?: string[];  // Deprecated but kept for backwards compat

  // Metadata
  version: number;
  is_active: boolean;
  is_promoted: boolean;
  is_locked: boolean;
  is_archived: boolean;
  created_at: number;
  updated_at: number;
}

// ============================================================================
// Helpers for V2 types
// ============================================================================

/**
 * Creates an empty condition with a default compare trigger.
 */
export const createEmptyCondition = (): Condition => {
  return {
    primary: {
      trigger: {
        type: 'compare',
        left: { source: 'price', value: 'close' },
        operator: '>',
        right: { fixed: 0 },
      },
      negated: false,
    },
    chain: [],
  };
};

export const createEmptyEntryRuleV2 = (id: string): EntryRuleV2 => ({
  id,
  direction: 'both',
  conditions: [createEmptyCondition()],
});

export const createEmptyExitRuleV2 = (id: string): ExitRuleV2 => ({
  id,
  direction: 'both',
  conditions: [createEmptyCondition()],
  close_percent: 100,
  priority: 0,
});

/** @deprecated Use createEmptyCondition instead */
export const createEmptyTriggerChain = (): TriggerChain => {
  return {
    primary: {
      type: 'compare',
      left: { source: 'price', value: 'close' },
      operator: '>',
      right: { fixed: 0 },
    },
    chain: [],
  };
};

/**
 * Extract parameter definitions from V2 rules.
 * In V2, parameters are inline with indicator configurations.
 * This extracts them for display in the UI.
 */
export const extractParametersFromV2Rules = (
  entryRules: EntryRuleV2[],
  exitRules: ExitRuleV2[]
): ParameterDefinition[] => {
  const params: Map<string, ParameterDefinition> = new Map();

  const extractFromValue = (value: ParameterizedValue, _label: string) => {
    if (typeof value === 'object' && value !== null && '$param' in value) {
      const paramRef = value as { $param: string };
      if (!params.has(paramRef.$param)) {
        params.set(paramRef.$param, {
          id: paramRef.$param,
          name: paramRef.$param,
          type: 'number',
          default: 0, // Default value unknown from reference
        });
      }
    }
  };

  const extractFromDataSource = (source: DataSourceV2) => {
    const s = normalizeDataSource(source);
    // Fixed sources can have parameterized values
    if ('fixed' in s && typeof s.fixed === 'object' && s.fixed !== null && '$param' in s.fixed) {
      extractFromValue(s.fixed as ParameterizedValue, 'fixed');
    }
  };

  const extractFromDistance = (distance?: DistanceConfig) => {
    if (distance) {
      extractFromValue(distance.value, 'distance');
    }
  };

  const extractFromTrigger = (trigger: TriggerV2) => {
    if (trigger.type === 'cross') {
      extractFromDataSource(trigger.left);
      extractFromDataSource(trigger.right);
    } else if (trigger.type === 'compare') {
      extractFromDataSource(trigger.left);
      extractFromDataSource(trigger.right);
      extractFromDistance(trigger.distance);
    }
  };

  const extractFromCondition = (condition: Condition) => {
    extractFromTrigger(condition.primary.trigger);
    // Defensive: handle missing chain (AI sometimes omits it)
    (condition.chain ?? []).forEach(c => extractFromTrigger(c.trigger.trigger));
  };

  entryRules.forEach(rule => rule.conditions.forEach(extractFromCondition));
  exitRules.forEach(rule => {
    rule.conditions.forEach(extractFromCondition);
    // Extract from close_percent if it's parameterized
    extractFromValue(rule.close_percent, 'close_percent');
  });

  return Array.from(params.values());
};

// ============================================================================
// Schema Normalization
// ============================================================================

/**
 * Normalize conditions to ensure `chain` is always present.
 * AI-generated strategies sometimes omit the chain property.
 */
const normalizeCondition = (condition: Condition): Condition => ({
  ...condition,
  chain: condition.chain ?? [],
});

/**
 * Normalize entry/exit rules to ensure all conditions have chain arrays.
 */
export const normalizeRules = <T extends { conditions: Condition[] }>(rules: T[]): T[] =>
  rules.map((rule) => ({
    ...rule,
    conditions: rule.conditions.map(normalizeCondition),
  }));

/**
 * Normalize a strategy to fix common issues from AI generation.
 * - Ensures all conditions have `chain` arrays (even if empty)
 */
export const normalizeStrategy = (strategy: StrategyV2): StrategyV2 => ({
  ...strategy,
  entry_rules: normalizeRules(strategy.entry_rules),
  exit_rules: normalizeRules(strategy.exit_rules),
});

// ============================================================================
// Schema Versioning & Migration
// ============================================================================

/**
 * Union type for strategies - now only V2 is supported.
 * @deprecated All strategies are now V2 format
 */
export type AnyStrategy = StrategyV2;

// ============================================================================
// Strategy Conversion (SP6)
// ============================================================================

/**
 * Supported source languages for strategy conversion.
 */
export type SourceLanguage = 'pine_script' | 'mql4' | 'mql5' | 'natural_language';

/**
 * Result of converting a script to wickd strategy JSON.
 */
export interface ConversionResult {
  /** The converted strategy definition */
  strategy: StrategyV2;
  /** Optional warnings about unsupported features or approximations */
  warnings?: string[];
}

