import { useMemo } from 'react';
import type { TradeData } from '../components/charts/TradeOverlayPlugin';

export interface ChartParams {
  instrument: string;
  granularity: string;
  count: number;
  from: string | null;
  to: string | null;
  trades: TradeData[] | null;
  strategyId: string | null;
  signalDirection: 'long' | 'short' | null;
  signalId: string | null;
  stopLoss: string | null;
  takeProfit: string | null;
  entryPrice: string | null;
  positionSize: string | null;
  /** Raw indicator JSON from caller (strategy indicators passed via URL) */
  indicatorSeed: string | null;
  /** Parameter overrides from walk-forward optimization (param_id -> value) */
  parameterOverrides: Record<string, number> | null;
}

// localStorage key for parameter overrides (used to pass from backtest to chart)
export const CHART_PARAMS_OVERRIDES_KEY = 'chart_parameter_overrides';

// Parse URL params for initial state
export const getInitialParams = (): ChartParams => {
  const params = new URLSearchParams(window.location.search);

  // Parse parameter overrides from localStorage (set by walk-forward window detail)
  let parameterOverrides: Record<string, number> | null = null;
  const overridesJson = localStorage.getItem(CHART_PARAMS_OVERRIDES_KEY);
  if (overridesJson) {
    try {
      parameterOverrides = JSON.parse(overridesJson) as Record<string, number>;
      // Clear after reading so it doesn't persist to future chart opens
      localStorage.removeItem(CHART_PARAMS_OVERRIDES_KEY);
    } catch (err) {
      console.error('Failed to parse parameter overrides from localStorage:', err);
    }
  }

  // Parse trades - URL params take priority over localStorage
  // URL params: used by Trade Analysis "Open in Chart" (single trade)
  // localStorage: used by BacktestApp for bulk trades (avoids URL size limits)
  let trades: TradeData[] | null = null;

  // First check URL params (takes priority - explicit trade from Trade Analysis)
  const tradesParam = params.get('trades');
  if (tradesParam) {
    try {
      // Format from Trade Analysis: { id, instrument, units, open_price, close_price, open_time, close_time, realized_pl }
      // Format needed for overlay: { entryTime, exitTime, entryPrice, exitPrice, direction, pnl }
      const rawTrades = JSON.parse(tradesParam) as Array<{
        id: string;
        instrument: string;
        units: string;
        open_price: string;
        close_price?: string;
        open_time: number;
        close_time?: number;
        realized_pl?: string;
      }>;
      trades = rawTrades
        .filter(t => t.close_time && t.close_price) // Only closed trades
        .map(t => ({
          entryTime: Math.floor(t.open_time / 1000), // ms to seconds
          exitTime: Math.floor((t.close_time || t.open_time) / 1000),
          entryPrice: parseFloat(t.open_price),
          exitPrice: parseFloat(t.close_price || t.open_price),
          direction: (parseFloat(t.units) > 0 ? 'long' : 'short') as 'long' | 'short',
          pnl: parseFloat(t.realized_pl || '0'),
          }));
      } catch (err) {
        console.error('Failed to parse trades from URL:', err);
      }
    }

  // Fallback to localStorage (used by BacktestApp for bulk trades to avoid URL size limits)
  if (!trades) {
    const tradesJson = localStorage.getItem('chart_trades');
    if (tradesJson) {
      try {
        trades = JSON.parse(tradesJson) as TradeData[];
        // Clear after reading so it doesn't persist to future chart opens
        localStorage.removeItem('chart_trades');
      } catch (err) {
        console.error('Failed to parse trades from localStorage:', err);
      }
    }
  }

  const strategyId = params.get('strategyId') || null;
  const signalDirection = params.get('signalDirection') as 'long' | 'short' | null;
  const signalId = params.get('signalId') || null;
  const stopLoss = params.get('stopLoss') || null;
  const takeProfit = params.get('takeProfit') || null;
  const entryPrice = params.get('entryPrice') || null;
  const positionSize = params.get('positionSize') || null;

  // Date range for historical view (e.g., viewing a specific trade)
  const from = params.get('from') || null;
  const to = params.get('to') || null;

  // Parse indicator seed from URL (strategy indicators passed by watcher)
  const indicatorSeed = params.get('indicators') || null;

  return {
    instrument: params.get('instrument') || 'EUR_USD',
    granularity: params.get('granularity') || 'H1',
    count: parseInt(params.get('count') || '5000'),
    from,
    to,
    trades,
    strategyId,
    signalDirection,
    signalId,
    stopLoss,
    takeProfit,
    entryPrice,
    positionSize,
    indicatorSeed,
    parameterOverrides,
  };
};

// Hook that memoizes the initial params (only parses once on mount)
export const useChartParams = (): ChartParams => {
  return useMemo(() => getInitialParams(), []);
};
