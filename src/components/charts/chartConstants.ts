import { OVERLAY_INDICATOR_TYPES } from '../../types/strategy';

// Colors for indicator lines
export const INDICATOR_COLORS: Record<string, string> = {
  // Moving averages
  sma: '#3b82f6', // blue
  ema: '#8b5cf6', // purple
  // Bollinger bands
  'bollinger.upper': '#f59e0b',
  'bollinger.middle': '#f59e0b',
  'bollinger.lower': '#f59e0b',
  // Ichimoku - these are the actual rendering defaults
  'ichimoku.tenkan': '#d4a439', // gold/mustard
  'ichimoku.kijun': '#4a90c2', // blue
  'ichimoku.senkou_a': '#76a87e', // muted green
  'ichimoku.senkou_b': '#c27878', // muted red
  'ichimoku.chikou': '#ffffff', // white
  // Chandelier Exit - trailing stop levels (darker colors, solid lines)
  'chandelier.exit_long': '#15803d',   // Dark green - long trailing stop
  'chandelier.exit_short': '#d97706', // Dark orange/amber - short trailing stop
  // MACD
  'macd.macd': '#3b82f6',
  'macd.signal': '#f59e0b',
  // Stochastic
  'stochastic.k': '#3b82f6',
  'stochastic.d': '#f59e0b',
  // MA Histogram
  'ma_histogram.fast_ma': '#3b82f6',  // blue
  'ma_histogram.slow_ma': '#ef4444',  // red
  // MA Bands
  'ma_bands.upper': '#ec4899',  // pink
  'ma_bands.middle': '#ec4899',
  'ma_bands.lower': '#ec4899',
  // DSS
  'dss.dss': '#3b82f6',    // blue
  'dss.signal': '#ec4899', // pink
  // ADR
  adr: '#f59e0b', // amber
  // ADX
  adx: '#22c55e',           // green - trend strength
  'adx.plus_di': '#3b82f6', // blue - +DI
  'adx.minus_di': '#ef4444', // red - -DI
  // MFI
  mfi: '#10b981',           // emerald - volume-weighted oscillator
  // Donchian Channel
  'donchian.upper': '#06b6d4',   // cyan - upper channel
  'donchian.middle': '#06b6d4',  // cyan - middle line
  'donchian.lower': '#06b6d4',   // cyan - lower channel
  // VWAP
  vwap: '#E040FB',               // purple-pink - volume weighted average
  // Parabolic SAR
  parabolic_sar: '#FF6D00',      // deep orange - SAR dots
  'parabolic_sar.trend': '#FFA726', // lighter orange - trend direction
  // SuperTrend
  super_trend: '#00E676',        // green - supertrend line
  'super_trend.trend': '#69F0AE', // lighter green - trend direction
  // Default
  default: '#9ca3af',
};

// Indicators that should be drawn as overlays on price chart
// Derived from INDICATOR_METADATA.isOverlay in strategy.ts
export const OVERLAY_INDICATORS: string[] = OVERLAY_INDICATOR_TYPES;

// Available indicator configurations for the popover menu
// Keys match the indicator type IDs passed from AI analysis conflicting_indicators
export const AVAILABLE_INDICATORS = {
  // Trend indicators
  sma_20: { id: 'sma_20', type: 'sma', params: { period: 20 }, label: 'SMA (20)', category: 'Trend' },
  sma_50: { id: 'sma_50', type: 'sma', params: { period: 50 }, label: 'SMA (50)', category: 'Trend' },
  ema_20: { id: 'ema_20', type: 'ema', params: { period: 20 }, label: 'EMA (20)', category: 'Trend' },
  ema_50: { id: 'ema_50', type: 'ema', params: { period: 50 }, label: 'EMA (50)', category: 'Trend' },
  macd: { id: 'macd', type: 'macd', params: { fast_period: 12, slow_period: 26, signal_period: 9 }, label: 'MACD', category: 'Trend' },
  ma_histogram: { id: 'ma_histogram', type: 'ma_histogram', params: { fast_period: 5, slow_period: 13 }, label: 'MA Histogram', category: 'Trend' },
  // Momentum indicators
  rsi_14: { id: 'rsi_14', type: 'rsi', params: { period: 14 }, label: 'RSI (14)', category: 'Momentum' },
  mfi_14: { id: 'mfi_14', type: 'mfi', params: { period: 14 }, label: 'MFI (14)', category: 'Momentum' },
  stochastic: { id: 'stochastic', type: 'stochastic', params: { k_period: 14, d_period: 3, slowing: 3 }, label: 'Stochastic', category: 'Momentum' },
  dss: { id: 'dss', type: 'dss', params: { stoch_period: 13, ema_period: 8, signal_period: 8 }, label: 'DSS', category: 'Momentum' },
  // Volatility indicators
  bollinger: { id: 'bollinger', type: 'bollinger', params: { period: 20, std_dev: 2.0 }, label: 'Bollinger Bands', category: 'Volatility' },
  atr_14: { id: 'atr_14', type: 'atr', params: { period: 14 }, label: 'ATR (14)', category: 'Volatility' },
  adr: { id: 'adr', type: 'adr', params: { period: 14 }, label: 'ADR (14)', category: 'Volatility' },
  ma_bands: { id: 'ma_bands', type: 'ma_bands', params: { period: 20, distance: 20 }, label: 'MA Bands', category: 'Volatility' },
  // Trend strength
  adx: { id: 'adx', type: 'adx', params: { period: 14 }, label: 'ADX (14)', category: 'Trend' },
  // Advanced indicators
  ichimoku: { id: 'ichimoku', type: 'ichimoku', params: { tenkan_period: 9, kijun_period: 26, senkou_b_period: 52, displacement: 26 }, label: 'Ichimoku Cloud', category: 'Advanced' },
  chandelier: { id: 'chandelier', type: 'chandelier', params: { period: 22, multiplier: 3.0 }, label: 'Chandelier', category: 'Advanced' },
  donchian: { id: 'donchian', type: 'donchian', params: { period: 20 }, label: 'Donchian Channel', category: 'Volatility' },
  // VWAP / Parabolic SAR / SuperTrend
  vwap: { id: 'vwap', type: 'vwap', params: {}, label: 'VWAP', category: 'Trend' },
  parabolic_sar: { id: 'parabolic_sar', type: 'parabolic_sar', params: { af_start: 0.02, af_increment: 0.02, af_max: 0.2 }, label: 'Parabolic SAR', category: 'Trend' },
  super_trend: { id: 'super_trend', type: 'super_trend', params: { period: 10, multiplier: 3 }, label: 'SuperTrend (10, 3)', category: 'Trend' },
} as const;

export type IndicatorKey = keyof typeof AVAILABLE_INDICATORS;

// Get candle duration in seconds for a given granularity
export const getGranularitySeconds = (granularity: string): number => {
  const durations: Record<string, number> = {
    S5: 5, S10: 10, S15: 15, S30: 30,
    M1: 60, M2: 120, M4: 240, M5: 300, M10: 600, M15: 900, M30: 1800,
    H1: 3600, H2: 7200, H3: 10800, H4: 14400, H6: 21600, H8: 28800, H12: 43200,
    D: 86400, W: 604800, M: 2592000,
  };
  return durations[granularity] || 3600; // Default to H1
};

// Get the standard decimal precision for a forex instrument
// JPY pairs use 3 decimals, most others use 5
export const getInstrumentPrecision = (instrument: string): number => {
  if (instrument.includes('JPY')) {
    return 3;
  }
  // XAU (gold), XAG (silver) use 2-3 decimals
  if (instrument.startsWith('XAU')) {
    return 2;
  }
  if (instrument.startsWith('XAG')) {
    return 3;
  }
  // Most forex pairs use 5 decimals (pipettes)
  return 5;
};

// Ichimoku-specific rendering configuration
export interface IchimokuConfig {
  displacement: number; // Default 26 periods
}

export const DEFAULT_ICHIMOKU_CONFIG: IchimokuConfig = {
  displacement: 26,
};

// Chart display constants
export const INITIAL_VISIBLE_CANDLES = 60; // How many candles to show initially
export const FUTURE_CANDLE_SLOTS = 30; // Empty slots on the right for future price movement
