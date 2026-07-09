import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { renderHook } from '@testing-library/react';
import { useChartParams, getInitialParams, CHART_PARAMS_OVERRIDES_KEY } from './useChartParams';

describe('useChartParams', () => {
  const originalLocation = window.location;

  // Mock localStorage
  let mockStorage: Record<string, string> = {};

  beforeEach(() => {
    mockStorage = {};

    // Mock localStorage
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation((key: string) => {
      return mockStorage[key] || null;
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation((key: string, value: string) => {
      mockStorage[key] = value;
    });
    vi.spyOn(Storage.prototype, 'removeItem').mockImplementation((key: string) => {
      delete mockStorage[key];
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    // Restore location
    Object.defineProperty(window, 'location', {
      value: originalLocation,
      writable: true,
    });
  });

  const setUrlParams = (params: string) => {
    Object.defineProperty(window, 'location', {
      value: {
        ...originalLocation,
        search: params,
      },
      writable: true,
    });
  };

  describe('default values', () => {
    it('returns default values when no URL params or localStorage', () => {
      setUrlParams('');

      const params = getInitialParams();

      expect(params.instrument).toBe('EUR_USD');
      expect(params.granularity).toBe('H1');
      expect(params.count).toBe(5000);
      expect(params.from).toBeNull();
      expect(params.to).toBeNull();
      expect(params.trades).toBeNull();
      expect(params.strategyId).toBeNull();
      expect(params.signalDirection).toBeNull();
    });
  });

  describe('URL parameter parsing', () => {
    it('parses instrument and granularity from URL', () => {
      setUrlParams('?instrument=GBP_USD&granularity=H4');

      const params = getInitialParams();

      expect(params.instrument).toBe('GBP_USD');
      expect(params.granularity).toBe('H4');
    });

    it('parses date range for historical view', () => {
      setUrlParams('?from=2024-01-01&to=2024-01-31');

      const params = getInitialParams();

      expect(params.from).toBe('2024-01-01');
      expect(params.to).toBe('2024-01-31');
    });

    it('parses strategy-related params', () => {
      setUrlParams('?strategyId=strat-123&signalDirection=long&signalId=sig-456');

      const params = getInitialParams();

      expect(params.strategyId).toBe('strat-123');
      expect(params.signalDirection).toBe('long');
      expect(params.signalId).toBe('sig-456');
    });

    it('parses trade execution params', () => {
      setUrlParams('?stopLoss=1.0800&takeProfit=1.1000&entryPrice=1.0900');

      const params = getInitialParams();

      expect(params.stopLoss).toBe('1.0800');
      expect(params.takeProfit).toBe('1.1000');
      expect(params.entryPrice).toBe('1.0900');
    });
  });

  describe('trade marker parsing (BUG-038 scenario)', () => {
    it('parses trades from URL params when opening from Trade Analysis', () => {
      const urlTrades = [
        {
          id: 'trade-1',
          instrument: 'EUR_USD',
          units: '1000',
          open_price: '1.0900',
          close_price: '1.0950',
          open_time: 1704067200000, // 2024-01-01 00:00:00 UTC in ms
          close_time: 1704153600000, // 2024-01-02 00:00:00 UTC in ms
          realized_pl: '50.00',
        },
      ];

      setUrlParams(`?trades=${encodeURIComponent(JSON.stringify(urlTrades))}`);

      const params = getInitialParams();

      expect(params.trades).not.toBeNull();
      expect(params.trades).toHaveLength(1);
      expect(params.trades![0]).toEqual({
        entryTime: 1704067200, // seconds
        exitTime: 1704153600,
        entryPrice: 1.09,
        exitPrice: 1.095,
        direction: 'long',
        pnl: 50,
      });
    });

    it('correctly determines direction from positive units (long)', () => {
      const urlTrades = [
        {
          id: 'trade-1',
          instrument: 'EUR_USD',
          units: '1000', // positive = long
          open_price: '1.0900',
          close_price: '1.0950',
          open_time: 1704067200000,
          close_time: 1704153600000,
          realized_pl: '50.00',
        },
      ];

      setUrlParams(`?trades=${encodeURIComponent(JSON.stringify(urlTrades))}`);

      const params = getInitialParams();

      expect(params.trades![0].direction).toBe('long');
    });

    it('correctly determines direction from negative units (short)', () => {
      const urlTrades = [
        {
          id: 'trade-1',
          instrument: 'EUR_USD',
          units: '-1000', // negative = short
          open_price: '1.0900',
          close_price: '1.0850',
          open_time: 1704067200000,
          close_time: 1704153600000,
          realized_pl: '50.00',
        },
      ];

      setUrlParams(`?trades=${encodeURIComponent(JSON.stringify(urlTrades))}`);

      const params = getInitialParams();

      expect(params.trades![0].direction).toBe('short');
    });

    it('filters out open trades (no close_time)', () => {
      const urlTrades = [
        {
          id: 'trade-1',
          instrument: 'EUR_USD',
          units: '1000',
          open_price: '1.0900',
          open_time: 1704067200000,
          // No close_time - open trade
        },
        {
          id: 'trade-2',
          instrument: 'EUR_USD',
          units: '1000',
          open_price: '1.0900',
          close_price: '1.0950',
          open_time: 1704067200000,
          close_time: 1704153600000,
          realized_pl: '50.00',
        },
      ];

      setUrlParams(`?trades=${encodeURIComponent(JSON.stringify(urlTrades))}`);

      const params = getInitialParams();

      expect(params.trades).toHaveLength(1);
      expect(params.trades![0].exitTime).toBe(1704153600);
    });
  });

  describe('localStorage trade parsing (backtest bulk trades)', () => {
    it('reads trades from localStorage when not in URL', () => {
      setUrlParams('');

      const localTrades = [
        {
          entryTime: 1704067200,
          exitTime: 1704153600,
          entryPrice: 1.09,
          exitPrice: 1.095,
          direction: 'long' as const,
          pnl: 50,
        },
      ];
      mockStorage['chart_trades'] = JSON.stringify(localTrades);

      const params = getInitialParams();

      expect(params.trades).toEqual(localTrades);
    });

    it('clears localStorage after reading trades (one-time use)', () => {
      setUrlParams('');

      mockStorage['chart_trades'] = JSON.stringify([{ entryTime: 1000, exitTime: 2000 }]);

      getInitialParams();

      expect(mockStorage['chart_trades']).toBeUndefined();
    });

    it('prefers URL trades over localStorage trades', () => {
      const urlTrades = [
        {
          id: 'url-trade',
          instrument: 'EUR_USD',
          units: '1000',
          open_price: '1.1000',
          close_price: '1.1050',
          open_time: 1704067200000,
          close_time: 1704153600000,
          realized_pl: '50.00',
        },
      ];

      const localTrades = [
        {
          entryTime: 1000000000,
          exitTime: 1000086400,
          entryPrice: 1.05,
          exitPrice: 1.06,
          direction: 'short' as const,
          pnl: 100,
        },
      ];

      setUrlParams(`?trades=${encodeURIComponent(JSON.stringify(urlTrades))}`);
      mockStorage['chart_trades'] = JSON.stringify(localTrades);

      const params = getInitialParams();

      // Should use URL trades, not localStorage
      expect(params.trades![0].entryPrice).toBe(1.1);
      expect(params.trades![0].direction).toBe('long');
    });
  });

  describe('parameter overrides (walk-forward)', () => {
    it('reads parameter overrides from localStorage', () => {
      setUrlParams('');

      const overrides = { sma_period: 20, rsi_threshold: 70 };
      mockStorage[CHART_PARAMS_OVERRIDES_KEY] = JSON.stringify(overrides);

      const params = getInitialParams();

      expect(params.parameterOverrides).toEqual(overrides);
    });

    it('clears parameter overrides after reading', () => {
      setUrlParams('');

      mockStorage[CHART_PARAMS_OVERRIDES_KEY] = JSON.stringify({ sma_period: 20 });

      getInitialParams();

      expect(mockStorage[CHART_PARAMS_OVERRIDES_KEY]).toBeUndefined();
    });

    it('handles invalid JSON in parameter overrides gracefully', () => {
      setUrlParams('');

      mockStorage[CHART_PARAMS_OVERRIDES_KEY] = 'not valid json';

      // Should not throw
      const params = getInitialParams();

      expect(params.parameterOverrides).toBeNull();
    });
  });

  // Indicator URL param parsing removed — indicators now passed via localStorage seed

  describe('hook memoization', () => {
    it('only parses params once on mount', () => {
      setUrlParams('?instrument=EUR_USD');

      const { result, rerender } = renderHook(() => useChartParams());

      const firstResult = result.current;

      // Change URL (simulating navigation, though in reality this would remount)
      setUrlParams('?instrument=GBP_USD');

      rerender();

      // Should still return the original memoized value
      expect(result.current).toBe(firstResult);
      expect(result.current.instrument).toBe('EUR_USD');
    });
  });

  describe('error handling', () => {
    it('handles malformed trades JSON gracefully', () => {
      setUrlParams('?trades=not-valid-json');

      // Should not throw
      const params = getInitialParams();

      expect(params.trades).toBeNull();
    });

    it('handles malformed localStorage trades gracefully', () => {
      setUrlParams('');
      mockStorage['chart_trades'] = 'not valid json';

      // Should not throw
      const params = getInitialParams();

      expect(params.trades).toBeNull();
    });
  });
});
