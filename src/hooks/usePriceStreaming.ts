import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import type { CandlestickData, Time } from 'lightweight-charts';
import type { PriceUpdate, UpdateCandleCallback } from '../components/charts/chartTypes';
import type { TimeMapState } from '../components/charts/chartTimeUtils';
import { getGranularitySeconds, hollowCandleColors } from '../components/charts/chartConstants';
import { addDebugLog } from '../components/ui/DebugOverlay';

const DAILY_ALIGNMENT = 2; // OANDA dailyAlignment=2 UTC (H4 at 02, 06, 10, 14, 18, 22)

// Wedge detection (issue #7): ticks can keep arriving (header quote live)
// while every one of them silently fails to advance the candle series —
// e.g. a persistent series.update() throw, a disposed series, an emptied
// time map, or a time map that stopped advancing business time. After this
// many CONSECUTIVE failed ticks we force a full candle reload (the
// programmatic equivalent of the timeframe toggle that snaps the chart back).
const WEDGE_FAILURE_THRESHOLD = 10;
// Never force-reload more often than this, so a genuinely broken backend
// can't turn the recovery path into a fetch loop.
const RESYNC_THROTTLE_MS = 60_000;

// Streaming diagnostics for the in-app debug overlay (Ctrl+Shift+D).
// Per-message rate limiting: each distinct message logs on its 1st, 25th,
// 50th... occurrence, so per-tick messages can't flood the 100-entry buffer.
const streamDebugCounts = new Map<string, number>();
const streamDebug = (message: string) => {
  const count = (streamDebugCounts.get(message) ?? 0) + 1;
  streamDebugCounts.set(message, count);
  if (count === 1) {
    addDebugLog('STREAM', message);
  } else if (count % 25 === 0) {
    addDebugLog('STREAM', `${message} (x${count})`);
  }
};

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
  /**
   * Called when the candle series is wedged — ticks are arriving but
   * persistently failing to apply. The handler should do a full candle
   * reload (refetch + setData + time map rebuild). Throttled internally.
   */
  onResyncNeeded?: () => void;
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
  onResyncNeeded,
}: UsePriceStreamingOptions): UsePriceStreamingResult => {
  const [streaming, setStreaming] = useState(false);
  const [currentPrice, setCurrentPrice] = useState<PriceUpdate | null>(null);
  const currentCandleRef = useRef<CandlestickData | null>(null);
  // Close of the candle before the forming one — drives OANDA-style hollow
  // candle coloring (direction vs previous close). Lazily read from series
  // data on the first tick, then maintained on each boundary roll.
  const prevCloseRef = useRef<number | null>(null);
  const currentInstrumentRef = useRef<string>(instrument);
  const updateCurrentCandleRef = useRef<UpdateCandleCallback | null>(null);
  const onNewCandleRef = useRef(onNewCandle);
  onNewCandleRef.current = onNewCandle;
  const onResyncNeededRef = useRef(onResyncNeeded);
  onResyncNeededRef.current = onResyncNeeded;

  // Wedge detection state (issue #7): consecutive ticks that arrived for our
  // instrument but failed to apply to the series, plus the ACTUAL (epoch)
  // candle start of the forming candle so we can tell "business time stopped
  // advancing" apart from normal same-candle updates.
  const consecutiveTickFailuresRef = useRef(0);
  const lastResyncMsRef = useRef(0);
  const formingCandleStartRef = useRef<number | null>(null);

  // Record a tick that failed to apply; after WEDGE_FAILURE_THRESHOLD
  // consecutive failures, request a full candle reload (throttled).
  const noteTickFailure = useCallback((message: string) => {
    streamDebug(message);
    consecutiveTickFailuresRef.current += 1;
    if (consecutiveTickFailuresRef.current < WEDGE_FAILURE_THRESHOLD) return;

    const now = Date.now();
    if (now - lastResyncMsRef.current < RESYNC_THROTTLE_MS) return;
    lastResyncMsRef.current = now;
    consecutiveTickFailuresRef.current = 0;
    currentCandleRef.current = null;
    prevCloseRef.current = null;
    formingCandleStartRef.current = null;

    console.warn(`[usePriceStreaming] candle series wedged (${message}) — forcing full candle reload`);
    addDebugLog('STREAM', `wedge detected: ${message} — resyncing candles`);
    onResyncNeededRef.current?.();
  }, []);

  // Update the current candle with streaming price
  const updateCurrentCandle = useCallback((price: PriceUpdate) => {
    // Guard: verify price is for the current instrument (defense in depth)
    if (price.instrument !== currentInstrumentRef.current) {
      return;
    }

    if (!candleSeriesRef.current) {
      noteTickFailure('tick dropped: no candle series');
      return;
    }

    // Guard: check if series has data - update() throws "Value is null" on empty/transitional series
    // This can happen during granularity switches when setData() is in progress
    try {
      const seriesData = candleSeriesRef.current.data();
      if (!seriesData || seriesData.length === 0) {
        noteTickFailure('tick dropped: series empty');
        return;
      }
    } catch {
      // Series may be in transitional state during granularity change
      noteTickFailure('tick dropped: series transitional');
      return;
    }

    const timeMapState = timeMapStateRef.current;
    if (!timeMapState) return;

    // Guard: if time map is empty, candles haven't loaded yet for this instrument.
    // Skip the update to prevent orphan candles at wrong positions (Bug #6).
    if (timeMapState.timeMap.size === 0) {
      noteTickFailure('tick dropped: time map empty');
      return;
    }

    const priceTime = new Date(price.time).getTime() / 1000;

    // Guard: an unparseable tick time yields NaN, and NaN is a valid Map key —
    // one such tick would map a permanent "candle" the series can never
    // advance past (issue #7). Drop the tick instead.
    if (!Number.isFinite(priceTime)) {
      noteTickFailure(`tick dropped: unparseable time "${price.time}"`);
      return;
    }

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

    // Guard: real time crossed into a new candle but the time map handed back
    // the forming candle's business time (e.g. a zero typicalInterval mapped
    // every candle start to the same business time). Applying the tick would
    // silently overwrite the last bar forever, so treat it as a wedge instead.
    if (
      currentCandleRef.current &&
      currentCandleRef.current.time === currentBusinessTime &&
      formingCandleStartRef.current !== null &&
      candleStartTime > formingCandleStartRef.current
    ) {
      noteTickFailure('tick dropped: business time not advancing across candle boundary');
      return;
    }

    const midPrice = (parseFloat(price.bid) + parseFloat(price.ask)) / 2;

    try {
      if (currentCandleRef.current && currentCandleRef.current.time === currentBusinessTime) {
        // Update existing candle
        const candle = currentCandleRef.current;
        candle.high = Math.max(candle.high as number, midPrice);
        candle.low = Math.min(candle.low as number, midPrice);
        candle.close = midPrice;
        // Recolor per tick — direction (vs previous close) and hollowness
        // (vs open) can both flip while the candle forms
        if (prevCloseRef.current === null) {
          try {
            const bars = candleSeriesRef.current.data();
            if (bars.length > 1) {
              prevCloseRef.current = (bars[bars.length - 2] as CandlestickData).close as number;
            }
          } catch {
            // Series transitional — color falls back to close-vs-open
          }
        }
        Object.assign(
          candle,
          hollowCandleColors(candle.open as number, midPrice, prevCloseRef.current)
        );
        candleSeriesRef.current.update(candle);
        consecutiveTickFailuresRef.current = 0;
        formingCandleStartRef.current = candleStartTime;
        streamDebug('tick applied to forming candle');
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

          // Maintain the previous close for hollow-candle coloring: the candle
          // we just rolled off of, or the bar behind the one we're extending.
          if (currentCandleRef.current && currentCandleRef.current.time !== currentBusinessTime) {
            prevCloseRef.current = currentCandleRef.current.close as number;
          } else if (lastBar && lastBar.time === currentBusinessTime) {
            const beforeLast = seriesData.length > 1
              ? (seriesData[seriesData.length - 2] as CandlestickData)
              : null;
            if (beforeLast) prevCloseRef.current = beforeLast.close as number;
          } else if (lastBar) {
            prevCloseRef.current = lastBar.close as number;
          }

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

        Object.assign(
          newCandle,
          hollowCandleColors(newCandle.open as number, newCandle.close as number, prevCloseRef.current)
        );
        currentCandleRef.current = newCandle;
        candleSeriesRef.current.update(newCandle);
        consecutiveTickFailuresRef.current = 0;
        formingCandleStartRef.current = candleStartTime;
        streamDebug(`candle roll -> business time ${String(newCandle.time)}`);

        // Notify that a new candle boundary was crossed (for indicator refresh)
        // Only fire when transitioning from an existing candle (not on the first
        // candle boundary after load, since indicators were just fetched by loadCandles)
        if (hadPreviousCandle) {
          onNewCandleRef.current?.();
        }
      }
    } catch (err) {
      // Errors during chart data transitions (e.g., granularity switch) are
      // expected and self-heal on the next tick. A PERSISTENT failure here
      // means the chart looks live (header price ticks) while candles
      // silently never move (issue #7) — noteTickFailure escalates that to a
      // full candle reload once the consecutive-failure threshold is hit.
      noteTickFailure(`candle update failed: ${err instanceof Error ? err.message : String(err)}`);
    }
  }, [granularity, candleSeriesRef, timeMapStateRef, noteTickFailure]);

  // Keep ref in sync with the latest callback
  updateCurrentCandleRef.current = updateCurrentCandle;

  // Reset streaming state when instrument changes
  useEffect(() => {
    currentInstrumentRef.current = instrument;
    setCurrentPrice(null);
    currentCandleRef.current = null;
    prevCloseRef.current = null;
    consecutiveTickFailuresRef.current = 0;
    formingCandleStartRef.current = null;
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
            streamDebug(`price-update received for ${instrument}`);
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
