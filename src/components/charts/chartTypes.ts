import type { IChartApi, ISeriesApi, CandlestickData } from 'lightweight-charts';
import type { TimeMapState } from './chartTimeUtils';
import type { TradeOverlayPlugin } from './TradeOverlayPlugin';
import type { IchimokuCloudPlugin } from './IchimokuCloudPlugin';
import type { SRZone } from './SRZoneOverlay';

// Price update from streaming
export interface PriceUpdate {
  instrument: string;
  bid: string;
  ask: string;
  spread: string;
  time: string;
  tradeable: boolean;
}

// Indicator data from backend
export interface IndicatorDataPoint {
  time: string;
  values: Record<string, string>;
}

export interface IndicatorSeries {
  id: string;
  type: string;
  outputs: string[];
  data: IndicatorDataPoint[];
}

// OHLC data for display in header
export interface OHLCData {
  open: string;
  high: string;
  low: string;
  close: string;
}

// Chart refs exposed to parent
export interface ChartRefs {
  chart: React.RefObject<IChartApi | null>;
  candleSeries: React.RefObject<ISeriesApi<'Candlestick'> | null>;
  tradeOverlay: React.RefObject<TradeOverlayPlugin | null>;
  ichimokuCloud: React.RefObject<IchimokuCloudPlugin | null>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  indicatorSeries: React.RefObject<Map<string, ISeriesApi<any>>>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  entryPriceLine: React.RefObject<any>;
  timeMapState: React.RefObject<TimeMapState>;
  currentCandle: React.RefObject<CandlestickData | null>;
}

// S/R zone editing state
export interface SRZoneEditingState {
  srEditingMode: boolean;
  pendingZoneBoundary: number | null;
  secondBoundary: number | null;
  selectedZoneId: string | null;
  editingZone: SRZone | null;
  previewZone: { upperY: number; lowerY: number } | null;
  srMenuOpen: boolean;
  confirmClearAll: boolean;
}

// S/R zone preview line refs
export interface SRZonePreviewRefs {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  previewLine: React.RefObject<any>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  firstBoundaryLine: React.RefObject<any>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  secondBoundaryLine: React.RefObject<any>;
}

// Execution state for trade signals
export interface ExecutionState {
  executing: boolean;
  executeError: string | null;
  executeSuccess: string | null;
}

// Streaming state
export interface StreamingState {
  streaming: boolean;
  currentPrice: PriceUpdate | null;
}

// Indicator config for loading (used by backend)
export interface IndicatorConfig {
  id: string;
  type: string;
  params: Record<string, number | string>;
}

// Chart indicator config (user-configurable indicators on chart)
export interface ChartIndicatorConfig {
  id: string;                     // Unique instance ID: 'sma_1', 'rsi_1'
  type: string;                   // Indicator type: 'sma', 'rsi', etc.
  params: Record<string, number>; // Configuration params: { period: 20 }
  colors?: Record<string, string>; // Custom colors per output: { value: '#ff0000' } or { upper: '#00ff00', lower: '#0000ff' }
}

// Pivot level from backend
export interface PivotLevel {
  label: string;
  price: number;
  level_type: string; // "pivot", "support", "resistance"
}

// Convert business time helper type
export type ToBusinessTimeFn = (actualTime: number) => number;

// Candle update callback type
export type UpdateCandleCallback = (price: PriceUpdate) => void;
