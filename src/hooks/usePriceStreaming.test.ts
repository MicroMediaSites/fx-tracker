import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import { usePriceStreaming, alignedCandleStart } from './usePriceStreaming';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { createTimeMapState } from '../components/charts/chartTimeUtils';

// Get mocked functions
const mockInvoke = vi.mocked(invoke);
const mockListen = vi.mocked(listen);

describe('usePriceStreaming', () => {
  // Track event listeners so we can simulate events
  let priceListeners: Array<(event: { payload: unknown }) => void> = [];
  let errorListeners: Array<(event: { payload: unknown }) => void> = [];
  let unlistenCalls: Array<() => void> = [];

  beforeEach(() => {
    vi.clearAllMocks();
    priceListeners = [];
    errorListeners = [];
    unlistenCalls = [];

    // Mock listen to capture event handlers
    mockListen.mockImplementation((eventName, handler) => {
      const unlisten = vi.fn();
      unlistenCalls.push(unlisten);

      if (eventName === 'price-update') {
        priceListeners.push(handler as (event: { payload: unknown }) => void);
      } else if (eventName === 'stream-error') {
        errorListeners.push(handler as (event: { payload: unknown }) => void);
      }

      return Promise.resolve(unlisten);
    });

    // Mock invoke to succeed by default
    mockInvoke.mockResolvedValue(undefined);
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  const createMockRefs = () => ({
    timeMapStateRef: { current: createTimeMapState() },
    candleSeriesRef: {
      current: {
        update: vi.fn(),
        // Mock data() to return existing candles - required for update guard
        data: vi.fn(() => [{ time: 100, open: 1, high: 1, low: 1, close: 1 }]),
      }
    },
  });

  describe('subscription lifecycle', () => {
    it('subscribes to prices on mount', async () => {
      const refs = createMockRefs();

      renderHook(() =>
        usePriceStreaming({
          instrument: 'EUR_USD',
          granularity: 'H1',
          isHistoricalView: false,
          ...refs,
        })
      );

      await waitFor(() => {
        expect(mockInvoke).toHaveBeenCalledWith('subscribe_to_prices', { instrument: 'EUR_USD' });
      });
    });

    it('unsubscribes when unmounting', async () => {
      const refs = createMockRefs();

      const { unmount } = renderHook(() =>
        usePriceStreaming({
          instrument: 'EUR_USD',
          granularity: 'H1',
          isHistoricalView: false,
          ...refs,
        })
      );

      await waitFor(() => {
        expect(mockInvoke).toHaveBeenCalledWith('subscribe_to_prices', { instrument: 'EUR_USD' });
      });

      unmount();

      expect(mockInvoke).toHaveBeenCalledWith('unsubscribe_from_prices', { instrument: 'EUR_USD' });
    });

    it('unsubscribes from old instrument and subscribes to new one when instrument changes', async () => {
      const refs = createMockRefs();

      const { rerender } = renderHook(
        ({ instrument }) =>
          usePriceStreaming({
            instrument,
            granularity: 'H1',
            isHistoricalView: false,
            ...refs,
          }),
        { initialProps: { instrument: 'EUR_USD' } }
      );

      await waitFor(() => {
        expect(mockInvoke).toHaveBeenCalledWith('subscribe_to_prices', { instrument: 'EUR_USD' });
      });

      // Change instrument
      rerender({ instrument: 'GBP_USD' });

      await waitFor(() => {
        // Should unsubscribe from old
        expect(mockInvoke).toHaveBeenCalledWith('unsubscribe_from_prices', { instrument: 'EUR_USD' });
        // Should subscribe to new
        expect(mockInvoke).toHaveBeenCalledWith('subscribe_to_prices', { instrument: 'GBP_USD' });
      });
    });

    it('does not subscribe when in historical view mode', async () => {
      const refs = createMockRefs();

      const { result } = renderHook(() =>
        usePriceStreaming({
          instrument: 'EUR_USD',
          granularity: 'H1',
          isHistoricalView: true,
          ...refs,
        })
      );

      // Give it time to potentially subscribe
      await new Promise((r) => setTimeout(r, 50));

      expect(mockInvoke).not.toHaveBeenCalledWith('subscribe_to_prices', expect.anything());
      expect(result.current.streaming).toBe(false);
    });
  });

  describe('price updates', () => {
    it('updates currentPrice when receiving price-update events for subscribed instrument', async () => {
      const refs = createMockRefs();

      const { result } = renderHook(() =>
        usePriceStreaming({
          instrument: 'EUR_USD',
          granularity: 'H1',
          isHistoricalView: false,
          ...refs,
        })
      );

      await waitFor(() => {
        expect(priceListeners.length).toBeGreaterThan(0);
      });

      // Simulate price update
      act(() => {
        priceListeners[0]({
          payload: {
            instrument: 'EUR_USD',
            bid: '1.0850',
            ask: '1.0852',
            time: new Date().toISOString(),
          },
        });
      });

      expect(result.current.currentPrice).toEqual({
        instrument: 'EUR_USD',
        bid: '1.0850',
        ask: '1.0852',
        time: expect.any(String),
      });
    });

    it('ignores price updates for different instruments', async () => {
      const refs = createMockRefs();

      const { result } = renderHook(() =>
        usePriceStreaming({
          instrument: 'EUR_USD',
          granularity: 'H1',
          isHistoricalView: false,
          ...refs,
        })
      );

      await waitFor(() => {
        expect(priceListeners.length).toBeGreaterThan(0);
      });

      // Simulate price update for different instrument
      act(() => {
        priceListeners[0]({
          payload: {
            instrument: 'GBP_USD',
            bid: '1.2500',
            ask: '1.2502',
            time: new Date().toISOString(),
          },
        });
      });

      expect(result.current.currentPrice).toBeNull();
    });

    it('resets currentPrice to null when instrument changes (BUG-037 scenario)', async () => {
      const refs = createMockRefs();

      const { result, rerender } = renderHook(
        ({ instrument }) =>
          usePriceStreaming({
            instrument,
            granularity: 'H1',
            isHistoricalView: false,
            ...refs,
          }),
        { initialProps: { instrument: 'EUR_USD' } }
      );

      await waitFor(() => {
        expect(priceListeners.length).toBeGreaterThan(0);
      });

      // Receive a price for EUR_USD
      act(() => {
        priceListeners[0]({
          payload: {
            instrument: 'EUR_USD',
            bid: '1.0850',
            ask: '1.0852',
            time: new Date().toISOString(),
          },
        });
      });

      expect(result.current.currentPrice).not.toBeNull();

      // Change instrument - price should reset
      rerender({ instrument: 'GBP_USD' });

      // Price should be null after instrument change
      expect(result.current.currentPrice).toBeNull();
    });

    it('clears time map state when instrument changes to prevent stale data', async () => {
      const refs = createMockRefs();

      // Pre-populate time map with some data
      refs.timeMapStateRef.current.timeMap.set(1000, 1);
      refs.timeMapStateRef.current.reverseTimeMap.set(1, 1000);
      refs.timeMapStateRef.current.lastBusinessTime = 1;

      const { rerender } = renderHook(
        ({ instrument }) =>
          usePriceStreaming({
            instrument,
            granularity: 'H1',
            isHistoricalView: false,
            ...refs,
          }),
        { initialProps: { instrument: 'EUR_USD' } }
      );

      await waitFor(() => {
        expect(mockInvoke).toHaveBeenCalledWith('subscribe_to_prices', { instrument: 'EUR_USD' });
      });

      // Change instrument
      rerender({ instrument: 'GBP_USD' });

      // Time map should be cleared
      expect(refs.timeMapStateRef.current.timeMap.size).toBe(0);
      expect(refs.timeMapStateRef.current.reverseTimeMap.size).toBe(0);
      expect(refs.timeMapStateRef.current.lastBusinessTime).toBe(0);
    });
  });

  describe('streaming state', () => {
    it('sets streaming to true after successful subscription', async () => {
      const refs = createMockRefs();

      const { result } = renderHook(() =>
        usePriceStreaming({
          instrument: 'EUR_USD',
          granularity: 'H1',
          isHistoricalView: false,
          ...refs,
        })
      );

      expect(result.current.streaming).toBe(false);

      await waitFor(() => {
        expect(result.current.streaming).toBe(true);
      });
    });

    it('sets streaming to false when switching to historical view', async () => {
      const refs = createMockRefs();

      const { result, rerender } = renderHook(
        ({ isHistoricalView }) =>
          usePriceStreaming({
            instrument: 'EUR_USD',
            granularity: 'H1',
            isHistoricalView,
            ...refs,
          }),
        { initialProps: { isHistoricalView: false } }
      );

      await waitFor(() => {
        expect(result.current.streaming).toBe(true);
      });

      // Switch to historical view
      rerender({ isHistoricalView: true });

      expect(result.current.streaming).toBe(false);
    });
  });

  describe('candle updates', () => {
    it('updates candle series with streaming price data', async () => {
      const refs = createMockRefs();
      const candleStartTime = Math.floor(Date.now() / 1000 / 3600) * 3600; // Current hour start

      renderHook(() =>
        usePriceStreaming({
          instrument: 'EUR_USD',
          granularity: 'H1',
          isHistoricalView: false,
          ...refs,
        })
      );

      await waitFor(() => {
        expect(priceListeners.length).toBeGreaterThan(0);
      });

      // Set up time map AFTER hook mounts (the instrument-change effect clears
      // the time map on mount, so populating before render would be wiped out)
      refs.timeMapStateRef.current.timeMap.set(candleStartTime, 100);
      refs.timeMapStateRef.current.reverseTimeMap.set(100, candleStartTime);
      refs.timeMapStateRef.current.lastBusinessTime = 100;
      refs.timeMapStateRef.current.typicalInterval = 1;

      // Simulate price update within current candle
      const priceTime = new Date(candleStartTime * 1000 + 1000); // 1 second into candle
      act(() => {
        priceListeners[0]({
          payload: {
            instrument: 'EUR_USD',
            bid: '1.0850',
            ask: '1.0852',
            time: priceTime.toISOString(),
          },
        });
      });

      expect(refs.candleSeriesRef.current.update).toHaveBeenCalled();
    });

    it('skips update when series has no data (BUG-064 fix - prevents "Value is null" error)', async () => {
      const refs = createMockRefs();

      // Mock empty series data - simulates granularity switch in progress
      refs.candleSeriesRef.current.data = vi.fn(() => []);

      // Set up time map with existing candle
      const candleStartTime = Math.floor(Date.now() / 1000 / 3600) * 3600;
      refs.timeMapStateRef.current.timeMap.set(candleStartTime, 100);
      refs.timeMapStateRef.current.reverseTimeMap.set(100, candleStartTime);
      refs.timeMapStateRef.current.lastBusinessTime = 100;
      refs.timeMapStateRef.current.typicalInterval = 1;

      renderHook(() =>
        usePriceStreaming({
          instrument: 'EUR_USD',
          granularity: 'M1',
          isHistoricalView: false,
          ...refs,
        })
      );

      await waitFor(() => {
        expect(priceListeners.length).toBeGreaterThan(0);
      });

      // Simulate price update while series is empty
      const priceTime = new Date(candleStartTime * 1000 + 1000);
      act(() => {
        priceListeners[0]({
          payload: {
            instrument: 'EUR_USD',
            bid: '1.0850',
            ask: '1.0852',
            time: priceTime.toISOString(),
          },
        });
      });

      // Should NOT call update - series has no data yet
      expect(refs.candleSeriesRef.current.update).not.toHaveBeenCalled();
    });

    it('handles series.data() throwing error gracefully (during chart transitions)', async () => {
      const refs = createMockRefs();

      // Mock data() throwing - simulates transitional chart state
      refs.candleSeriesRef.current.data = vi.fn(() => {
        throw new Error('Chart is being destroyed');
      });

      const candleStartTime = Math.floor(Date.now() / 1000 / 3600) * 3600;
      refs.timeMapStateRef.current.timeMap.set(candleStartTime, 100);
      refs.timeMapStateRef.current.reverseTimeMap.set(100, candleStartTime);
      refs.timeMapStateRef.current.lastBusinessTime = 100;
      refs.timeMapStateRef.current.typicalInterval = 1;

      renderHook(() =>
        usePriceStreaming({
          instrument: 'EUR_USD',
          granularity: 'M1',
          isHistoricalView: false,
          ...refs,
        })
      );

      await waitFor(() => {
        expect(priceListeners.length).toBeGreaterThan(0);
      });

      // Should not throw - error is caught
      const priceTime = new Date(candleStartTime * 1000 + 1000);
      expect(() => {
        act(() => {
          priceListeners[0]({
            payload: {
              instrument: 'EUR_USD',
              bid: '1.0850',
              ask: '1.0852',
              time: priceTime.toISOString(),
            },
          });
        });
      }).not.toThrow();

      // Should NOT call update
      expect(refs.candleSeriesRef.current.update).not.toHaveBeenCalled();
    });
  });

  // Issue #7: candle series silently stops refreshing while header quote stays
  // live. Ticks that persistently fail to apply must trigger a full candle
  // reload (onResyncNeeded) instead of being dropped forever.
  describe('candle wedge self-heal (issue #7)', () => {
    const M1 = 60;

    const setupTimeMap = (refs: ReturnType<typeof createMockRefs>, candleStartTime: number) => {
      refs.timeMapStateRef.current.timeMap.set(candleStartTime, 100);
      refs.timeMapStateRef.current.reverseTimeMap.set(100, candleStartTime);
      refs.timeMapStateRef.current.lastBusinessTime = 100;
      refs.timeMapStateRef.current.typicalInterval = 1;
    };

    const renderStreamingHook = async (
      refs: ReturnType<typeof createMockRefs>,
      onResyncNeeded: () => void
    ) => {
      renderHook(() =>
        usePriceStreaming({
          instrument: 'EUR_USD',
          granularity: 'M1',
          isHistoricalView: false,
          onResyncNeeded,
          ...refs,
        })
      );
      await waitFor(() => {
        expect(priceListeners.length).toBeGreaterThan(0);
      });
    };

    const emitTick = (time: string) => {
      act(() => {
        priceListeners[0]({
          payload: { instrument: 'EUR_USD', bid: '1.0850', ask: '1.0852', time },
        });
      });
    };

    it('drops ticks with unparseable time without poisoning the time map', async () => {
      const refs = createMockRefs();
      const onResyncNeeded = vi.fn();
      const candleStartTime = Math.floor(Date.now() / 1000 / M1) * M1;

      await renderStreamingHook(refs, onResyncNeeded);
      setupTimeMap(refs, candleStartTime);

      emitTick('not-a-timestamp');

      expect(refs.candleSeriesRef.current.update).not.toHaveBeenCalled();
      // NaN must not have been inserted as a candle-start key
      expect([...refs.timeMapStateRef.current.timeMap.keys()].some(Number.isNaN)).toBe(false);

      // A subsequent valid tick still applies normally
      emitTick(new Date(candleStartTime * 1000 + 1000).toISOString());
      expect(refs.candleSeriesRef.current.update).toHaveBeenCalledTimes(1);
    });

    it('requests a resync after persistent series.update() failures, throttled to once', async () => {
      const refs = createMockRefs();
      refs.candleSeriesRef.current.update = vi.fn(() => {
        throw new Error('Cannot update oldest data');
      });
      const onResyncNeeded = vi.fn();
      const candleStartTime = Math.floor(Date.now() / 1000 / M1) * M1;

      await renderStreamingHook(refs, onResyncNeeded);
      setupTimeMap(refs, candleStartTime);

      // 9 consecutive failures: below the threshold, no resync yet
      for (let i = 0; i < 9; i++) {
        emitTick(new Date(candleStartTime * 1000 + 1000 + i).toISOString());
      }
      expect(onResyncNeeded).not.toHaveBeenCalled();

      // 10th failure crosses the threshold
      emitTick(new Date(candleStartTime * 1000 + 2000).toISOString());
      expect(onResyncNeeded).toHaveBeenCalledTimes(1);

      // Further failures inside the throttle window must not re-trigger
      for (let i = 0; i < 15; i++) {
        emitTick(new Date(candleStartTime * 1000 + 3000 + i).toISOString());
      }
      expect(onResyncNeeded).toHaveBeenCalledTimes(1);
    });

    it('a successful update resets the consecutive-failure count', async () => {
      const refs = createMockRefs();
      let failing = true;
      refs.candleSeriesRef.current.update = vi.fn(() => {
        if (failing) throw new Error('transient');
      });
      const onResyncNeeded = vi.fn();
      const candleStartTime = Math.floor(Date.now() / 1000 / M1) * M1;

      await renderStreamingHook(refs, onResyncNeeded);
      setupTimeMap(refs, candleStartTime);

      for (let i = 0; i < 9; i++) {
        emitTick(new Date(candleStartTime * 1000 + 1000 + i).toISOString());
      }
      // One success in between clears the streak
      failing = false;
      emitTick(new Date(candleStartTime * 1000 + 2000).toISOString());
      failing = true;
      for (let i = 0; i < 9; i++) {
        emitTick(new Date(candleStartTime * 1000 + 3000 + i).toISOString());
      }

      expect(onResyncNeeded).not.toHaveBeenCalled();
    });

    it('detects a time map whose business time stops advancing across candle boundaries', async () => {
      const refs = createMockRefs();
      const onResyncNeeded = vi.fn();
      const minute1 = Math.floor(Date.now() / 1000 / M1) * M1;
      const minute2 = minute1 + M1;

      await renderStreamingHook(refs, onResyncNeeded);
      // Corrupted map: two different candle starts share one business time
      // (what a zero typicalInterval used to produce)
      setupTimeMap(refs, minute1);
      refs.timeMapStateRef.current.timeMap.set(minute2, 100);

      // Establish the forming candle in minute1
      emitTick(new Date(minute1 * 1000 + 1000).toISOString());
      expect(refs.candleSeriesRef.current.update).toHaveBeenCalledTimes(1);

      // Ticks in minute2 resolve to the SAME business time — each must be
      // dropped (not silently overwrite the last bar) and counted as a failure
      for (let i = 0; i < 10; i++) {
        emitTick(new Date(minute2 * 1000 + 1000 + i).toISOString());
      }

      expect(refs.candleSeriesRef.current.update).toHaveBeenCalledTimes(1);
      expect(onResyncNeeded).toHaveBeenCalledTimes(1);
    });
  });
});

// BUG-079: alignedCandleStart must match OANDA's dailyAlignment=3 UTC
describe('alignedCandleStart', () => {
  // Helper: create epoch seconds from a UTC time string
  const utc = (iso: string) => new Date(iso).getTime() / 1000;

  describe('H4 — boundaries at 02, 06, 10, 14, 18, 22 UTC', () => {
    it.each([
      ['2026-03-16T02:00:00Z', '2026-03-16T02:00:00Z'],
      ['2026-03-16T05:59:59Z', '2026-03-16T02:00:00Z'],
      ['2026-03-16T06:00:00Z', '2026-03-16T06:00:00Z'],
      ['2026-03-16T09:30:00Z', '2026-03-16T06:00:00Z'],
      ['2026-03-16T10:00:00Z', '2026-03-16T10:00:00Z'],
      ['2026-03-16T14:00:00Z', '2026-03-16T14:00:00Z'],
      ['2026-03-16T17:48:00Z', '2026-03-16T14:00:00Z'],
      ['2026-03-16T18:00:00Z', '2026-03-16T18:00:00Z'],
      ['2026-03-16T22:00:00Z', '2026-03-16T22:00:00Z'],
      ['2026-03-16T23:59:59Z', '2026-03-16T22:00:00Z'],
    ])('tick at %s → candle start %s', (tickTime, expectedStart) => {
      expect(alignedCandleStart(utc(tickTime), 'H4')).toBe(utc(expectedStart));
    });

    it('hours 0-1 UTC wrap to previous day 22:00', () => {
      expect(alignedCandleStart(utc('2026-03-16T00:30:00Z'), 'H4')).toBe(utc('2026-03-15T22:00:00Z'));
      expect(alignedCandleStart(utc('2026-03-16T01:59:59Z'), 'H4')).toBe(utc('2026-03-15T22:00:00Z'));
    });
  });

  describe('H6 — boundaries at 02, 08, 14, 20 UTC', () => {
    it.each([
      ['2026-03-16T02:00:00Z', '2026-03-16T02:00:00Z'],
      ['2026-03-16T07:59:59Z', '2026-03-16T02:00:00Z'],
      ['2026-03-16T08:00:00Z', '2026-03-16T08:00:00Z'],
      ['2026-03-16T14:00:00Z', '2026-03-16T14:00:00Z'],
      ['2026-03-16T20:00:00Z', '2026-03-16T20:00:00Z'],
    ])('tick at %s → candle start %s', (tickTime, expectedStart) => {
      expect(alignedCandleStart(utc(tickTime), 'H6')).toBe(utc(expectedStart));
    });

    it('hours 0-1 UTC wrap to previous day 20:00', () => {
      expect(alignedCandleStart(utc('2026-03-16T01:00:00Z'), 'H6')).toBe(utc('2026-03-15T20:00:00Z'));
    });
  });

  describe('H8 — boundaries at 02, 10, 18 UTC', () => {
    it.each([
      ['2026-03-16T02:00:00Z', '2026-03-16T02:00:00Z'],
      ['2026-03-16T09:59:59Z', '2026-03-16T02:00:00Z'],
      ['2026-03-16T10:00:00Z', '2026-03-16T10:00:00Z'],
      ['2026-03-16T18:00:00Z', '2026-03-16T18:00:00Z'],
    ])('tick at %s → candle start %s', (tickTime, expectedStart) => {
      expect(alignedCandleStart(utc(tickTime), 'H8')).toBe(utc(expectedStart));
    });

    it('hours 0-1 UTC wrap to previous day 18:00', () => {
      expect(alignedCandleStart(utc('2026-03-16T01:00:00Z'), 'H8')).toBe(utc('2026-03-15T18:00:00Z'));
    });
  });

  describe('H12 — boundaries at 02, 14 UTC', () => {
    it.each([
      ['2026-03-16T02:00:00Z', '2026-03-16T02:00:00Z'],
      ['2026-03-16T13:59:59Z', '2026-03-16T02:00:00Z'],
      ['2026-03-16T14:00:00Z', '2026-03-16T14:00:00Z'],
      ['2026-03-16T23:59:59Z', '2026-03-16T14:00:00Z'],
    ])('tick at %s → candle start %s', (tickTime, expectedStart) => {
      expect(alignedCandleStart(utc(tickTime), 'H12')).toBe(utc(expectedStart));
    });

    it('hours 0-1 UTC wrap to previous day 14:00', () => {
      expect(alignedCandleStart(utc('2026-03-16T01:00:00Z'), 'H12')).toBe(utc('2026-03-15T14:00:00Z'));
    });
  });

  describe('sub-hourly granularities use simple epoch alignment', () => {
    it('M15 aligns to 15-minute boundaries', () => {
      expect(alignedCandleStart(utc('2026-03-16T10:37:00Z'), 'M15')).toBe(utc('2026-03-16T10:30:00Z'));
    });

    it('H1 aligns to hour boundaries', () => {
      expect(alignedCandleStart(utc('2026-03-16T10:37:00Z'), 'H1')).toBe(utc('2026-03-16T10:00:00Z'));
    });
  });
});
