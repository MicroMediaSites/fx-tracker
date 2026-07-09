import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import type { CandlestickData, Time } from 'lightweight-charts';
import type { PriceUpdate, UpdateCandleCallback } from '../components/charts/chartTypes';
import type { TimeMapState } from '../components/charts/chartTimeUtils';
import { getGranularitySeconds } from '../components/charts/chartConstants';

const DAILY_ALIGNMENT = 2; // OANDA dailyAlignment=2 UTC (H4 at 02, 06, 10, 14, 18, 22)

/**
 * Calculate the candle start time for a given timestamp and granularity,
 * respecting OANDA's dailyAlignment=2 UTC setting.
 *
 * Must match the Rust implementation in strategy/candle_boundary.rs.
 */
export function alignedCandleStart(epochSeconds: number, granularity: string): number {
  const candleSeconds = getGranularitySeconds(granularity);

  // For sub-hourly granularities, simple epoch modulo is correct
  if (candleSeconds < 3600) {
    return Math.floor(epochSeconds / candleSeconds) * candleSeconds;
  }

  // For H1, simple hour alignment is correct
  if (granularity === 'H1') {
    return Math.floor(epochSeconds / 3600) * 3600;
  }

  // For multi-hour granularities, align to dailyAlignment=2 UTC
  const date = new Date(epochSeconds * 1000);
  const hour = date.getUTCHours();
  const hours = candleSeconds / 3600;

  // Calculate aligned hour using dailyAlignment offset
  // Candle boundaries: 3, 3+hours, 3+2*hours, ... wrapping at 24
  // Shift hour so alignment base is 0, then divide, then shift back
  let shifted = hour - DAILY_ALIGNMENT;
  if (shifted < 0) shifted += 24;
  const alignedShifted = Math.floor(shifted / hours) * hours;
  let alignedHour = (alignedShifted + DAILY_ALIGNMENT) % 24;

  // Build the aligned timestamp
  const result = new Date(date);
  result.setUTCHours(alignedHour, 0, 0, 0);

  // Handle wraparound: if aligned hour is > current hour, it's previous day
  if (alignedHour > hour) {
    result.setUTCDate(result.getUTCDate() - 1);
  }

  return Math.floor(result.getTime() / 1000);
}

interface UsePriceStreamingOptions {
  instrument: string;
  granularity: string;
  isHistoricalView: boolean;
  timeMapStateRef: React.RefObject<TimeMapState>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  candleSeriesRef: React.RefObject<any>;
  /** Called when streaming crosses a new candle boundary (not on every tick) */
  onNewCandle?: () => void;
}

interface UsePriceStreamingResult {
  streaming: boolean;
  currentPrice: PriceUpdate | null;
  currentCandleRef: React.RefObject<CandlestickData | null>;
  updateCurrentCandleRef: React.RefObject<UpdateCandleCallback | null>;
}

export const usePriceStreaming = ({
  instrument,
  granularity,
  isHistoricalView,
  timeMapStateRef,
  candleSeriesRef,
  onNewCandle,
}: UsePriceStreamingOptions): UsePriceStreamingResult => {
  const [streaming, setStreaming] = useState(false);
  const [currentPrice, setCurrentPrice] = useState<PriceUpdate | null>(null);
  const currentCandleRef = useRef<CandlestickData | null>(null);
  const currentInstrumentRef = useRef<string>(instrument);
  const updateCurrentCandleRef = useRef<UpdateCandleCallback | null>(null);
  const onNewCandleRef = useRef(onNewCandle);
  onNewCandleRef.current = onNewCandle;

  // Update the current candle with streaming price
  const updateCurrentCandle = useCallback((price: PriceUpdate) => {
    // Guard: verify price is for the current instrument (defense in depth)
    if (price.instrument !== currentInstrumentRef.current) {
      return;
    }

    if (!candleSeriesRef.current) {
      return;
    }

    // Guard: check if series has data - update() throws "Value is null" on empty/transitional series
    // This can happen during granularity switches when setData() is in progress
    try {
      const seriesData = candleSeriesRef.current.data();
      if (!seriesData || seriesData.length === 0) {
        return;
      }
    } catch {
      // Series may be in transitional state during granularity change
      return;
    }

    const timeMapState = timeMapStateRef.current;
    if (!timeMapState) return;

    // Guard: if time map is empty, candles haven't loaded yet for this instrument.
    // Skip the update to prevent orphan candles at wrong positions (Bug #6).
    if (timeMapState.timeMap.size === 0) return;

    const priceTime = new Date(price.time).getTime() / 1000;

    // Calculate current candle's start time (aligned to OANDA's dailyAlignment=3)
    const candleStartTime = alignedCandleStart(priceTime, granularity);

    // Check if this candle time is already in our time map
    let currentBusinessTime = timeMapState.timeMap.get(candleStartTime);

    if (currentBusinessTime === undefined) {
      // Not in our time map yet - this is a new candle.
      // NOTE: This branch is safe because the timeMap.size === 0 guard above
      // ensures we only reach here when the time map is populated (has valid
      // lastBusinessTime and typicalInterval). Do not remove that guard without
      // re-adding the size check here.
      const newBusinessTime = timeMapState.lastBusinessTime + timeMapState.typicalInterval;
      timeMapState.timeMap.set(candleStartTime, newBusinessTime);
      timeMapState.reverseTimeMap.set(newBusinessTime, candleStartTime);
      timeMapState.lastBusinessTime = newBusinessTime;
      currentBusinessTime = newBusinessTime;
    }

    const midPrice = (parseFloat(price.bid) + parseFloat(price.ask)) / 2;

    try {
      if (currentCandleRef.current && currentCandleRef.current.time === currentBusinessTime) {
        // Update existing candle
        const candle = currentCandleRef.current;
        candle.high = Math.max(candle.high as number, midPrice);
        candle.low = Math.min(candle.low as number, midPrice);
        candle.close = midPrice;
        candleSeriesRef.current.update(candle);
      } else {
        // First tick for this candle period — check if historical data exists
        const hadPreviousCandle = currentCandleRef.current !== null;

        // Try to read the existing candle from the series (loaded by loadCandles).
        // This preserves the historical OHLC instead of starting from the tick price.
        let newCandle: CandlestickData;
        const freshCandle = (): CandlestickData => ({
          time: currentBusinessTime as Time,
          open: midPrice, high: midPrice, low: midPrice, close: midPrice,
        });

        try {
          const seriesData = candleSeriesRef.current.data();
          const lastBar = seriesData.length > 0 ? seriesData[seriesData.length - 1] as CandlestickData : null;
          if (lastBar && lastBar.time === currentBusinessTime) {
            // Historical candle exists — extend it with the new tick
            const prevHigh = lastBar.high ?? midPrice;
            const prevLow = lastBar.low ?? midPrice;
            newCandle = {
              time: currentBusinessTime as Time,
              open: lastBar.open ?? midPrice,
              high: Math.max(prevHigh as number, midPrice),
              low: Math.min(prevLow as number, midPrice),
              close: midPrice,
            };
          } else {
            newCandle = freshCandle();
          }
        } catch (err) {
          console.warn('[usePriceStreaming] Failed to read series data for current candle:', err);
          newCandle = freshCandle();
        }

        currentCandleRef.current = newCandle;
        candleSeriesRef.current.update(newCandle);

        // Notify that a new candle boundary was crossed (for indicator refresh)
        // Only fire when transitioning from an existing candle (not on the first
        // candle boundary after load, since indicators were just fetched by loadCandles)
        if (hadPreviousCandle) {
          onNewCandleRef.current?.();
        }
      }
    } catch {
      // Ignore errors during chart data transitions (e.g., granularity switch)
      // The next successful update will sync the candle state
    }
  }, [granularity, candleSeriesRef, timeMapStateRef]);

  // Keep ref in sync with the latest callback
  updateCurrentCandleRef.current = updateCurrentCandle;

  // Reset streaming state when instrument changes
  useEffect(() => {
    currentInstrumentRef.current = instrument;
    setCurrentPrice(null);
    currentCandleRef.current = null;
    // Clear time maps to prevent stale data from contaminating new instrument's candles
    const timeMapState = timeMapStateRef.current;
    if (timeMapState) {
      timeMapState.timeMap.clear();
      timeMapState.reverseTimeMap.clear();
      timeMapState.lastBusinessTime = 0;
    }
  }, [instrument, timeMapStateRef]);

  // Subscribe to price streaming for current instrument
  // Uses centralized PriceStreamManager - multiple charts share one stream
  // Skip streaming when viewing historical data (e.g., from Trade Analysis)
  useEffect(() => {
    // Don't stream prices when viewing historical data
    if (isHistoricalView) {
      setStreaming(false);
      return;
    }

    let cancelled = false;
    let priceUnlisten: UnlistenFn | null = null;
    let errorUnlisten: UnlistenFn | null = null;

    const subscribe = async () => {
      try {
        // Set up listener FIRST to avoid race condition
        const priceFn = await listen<PriceUpdate>('price-update', (event) => {
          // Skip if effect was cancelled (instrument changed)
          if (cancelled) return;
          if (event.payload.instrument === instrument) {
            setCurrentPrice(event.payload);
            // Use ref to call the latest version of updateCurrentCandle
            updateCurrentCandleRef.current?.(event.payload);
          }
        });

        // Check if cancelled while awaiting listener setup
        if (cancelled) {
          priceFn();
          return;
        }
        priceUnlisten = priceFn;

        // Also listen for stream errors
        const errorFn = await listen<{ errorType: string; message: string }>('stream-error', (event) => {
          if (cancelled) return;
          console.error('[Chart] Stream error:', event.payload);
        });

        if (cancelled) {
          errorFn();
          return;
        }
        errorUnlisten = errorFn;

        // Subscribe to prices for this instrument
        await invoke('subscribe_to_prices', { instrument });

        // Check if cancelled while awaiting subscription
        if (cancelled) {
          await invoke('unsubscribe_from_prices', { instrument });
          return;
        }
        setStreaming(true);
      } catch (err) {
        if (!cancelled) {
          console.error('[Chart] Failed to subscribe to prices:', err);
        }
      }
    };

    subscribe();

    return () => {
      cancelled = true;
      priceUnlisten?.();
      errorUnlisten?.();

      // Always attempt to unsubscribe - backend handles gracefully if not subscribed
      // This catches the case where subscribe() is still in progress
      invoke('unsubscribe_from_prices', { instrument }).catch((err) => {
        console.error('[Chart] Failed to unsubscribe:', err);
      });
    };
  }, [instrument, isHistoricalView]);

  return {
    streaming,
    currentPrice,
    currentCandleRef,
    updateCurrentCandleRef,
  };
};
