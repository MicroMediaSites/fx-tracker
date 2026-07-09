import type { CandlestickData, Time } from 'lightweight-charts';
import type { IndicatorSeries } from './chartTypes';

// CandleData interface matching the backend response
export interface CandleData {
  time: string;
  open: string;
  high: string;
  low: string;
  close: string;
}

// Time mapping state for eliminating weekend gaps
export interface TimeMapState {
  timeMap: Map<number, number>;
  reverseTimeMap: Map<number, number>;
  typicalInterval: number;
  lastBusinessTime: number;
}

// Create initial time map state
export const createTimeMapState = (): TimeMapState => ({
  timeMap: new Map(),
  reverseTimeMap: new Map(),
  typicalInterval: 3600,
  lastBusinessTime: 0,
});

// Convert candle data to chart format with business time (no weekend gaps)
// Returns the converted data and updates the time map state
export const convertCandles = (
  candles: CandleData[],
  state: TimeMapState
): CandlestickData[] => {
  if (candles.length === 0) return [];

  // Calculate the typical interval between candles
  const intervals: number[] = [];
  for (let i = 1; i < Math.min(candles.length, 20); i++) {
    const curr = new Date(candles[i].time).getTime() / 1000;
    const prev = new Date(candles[i - 1].time).getTime() / 1000;
    const diff = curr - prev;
    // Only count normal intervals (not weekend gaps)
    if (diff < 3 * 24 * 3600) { // Less than 3 days
      intervals.push(diff);
    }
  }
  const typicalInterval = intervals.length > 0
    ? intervals.sort((a, b) => a - b)[Math.floor(intervals.length / 2)]
    : 3600; // Default to 1 hour

  state.typicalInterval = typicalInterval;

  // Build time mapping - use sequential business time
  state.timeMap.clear();
  state.reverseTimeMap.clear();

  let businessTime = new Date(candles[0].time).getTime() / 1000;

  const chartData = candles.map((c, i) => {
    const actualTime = new Date(c.time).getTime() / 1000;

    if (i > 0) {
      // Always increment by the typical interval (eliminates gaps)
      businessTime += typicalInterval;
    }

    // Store mappings for indicator alignment
    state.timeMap.set(actualTime, businessTime);
    state.reverseTimeMap.set(businessTime, actualTime);

    return {
      time: businessTime as Time,
      open: parseFloat(c.open),
      high: parseFloat(c.high),
      low: parseFloat(c.low),
      close: parseFloat(c.close),
    };
  });

  state.lastBusinessTime = businessTime;

  return chartData;
};

// Convert actual timestamp to business time
export const toBusinessTime = (
  actualTime: number,
  timeMap: Map<number, number>
): number => {
  // Find exact match or closest earlier time
  const mapped = timeMap.get(actualTime);
  if (mapped !== undefined) return mapped;

  // Find closest match
  let closest = actualTime;
  let minDiff = Infinity;
  for (const [actual, business] of timeMap.entries()) {
    const diff = Math.abs(actual - actualTime);
    if (diff < minDiff) {
      minDiff = diff;
      closest = business;
    }
  }
  return closest;
};

/**
 * Ensures the time map has entries for new candle timestamps that arrived
 * after the initial candle load (e.g., a new candle completed between indicator
 * refreshes). Only appends timestamps beyond the current max actual time to
 * maintain monotonic correspondence between actual and business time ordering.
 *
 * Without this, toBusinessTime would fall back to "closest match" for unknown
 * timestamps, potentially mapping new indicator data points to incorrect
 * chart positions.
 */
const ensureTimeMapped = (
  actualTimes: number[],
  state: TimeMapState
): void => {
  if (state.typicalInterval <= 0) return;

  // Find the current max actual time in the map
  let maxActualTime = -Infinity;
  for (const actual of state.timeMap.keys()) {
    if (actual > maxActualTime) maxActualTime = actual;
  }

  // Only process timestamps beyond the current max (new candles at the end)
  const newTimes = actualTimes
    .filter((t) => t > maxActualTime && !state.timeMap.has(t))
    .sort((a, b) => a - b);

  for (const actualTime of newTimes) {
    const newBusinessTime = state.lastBusinessTime + state.typicalInterval;
    state.timeMap.set(actualTime, newBusinessTime);
    state.reverseTimeMap.set(newBusinessTime, actualTime);
    state.lastBusinessTime = newBusinessTime;
  }
};

/**
 * Extracts all unique timestamps from indicator results and ensures they
 * are present in the time map. Used after fetching indicator data to sync
 * any new candle timestamps that arrived since the last candle load.
 */
export const syncIndicatorTimestamps = (
  indicatorResults: IndicatorSeries[],
  state: TimeMapState
): void => {
  const timestamps = new Set<number>();
  for (const series of indicatorResults) {
    for (const point of series.data) {
      timestamps.add(new Date(point.time).getTime() / 1000);
    }
  }
  if (timestamps.size > 0) {
    ensureTimeMapped(Array.from(timestamps), state);
  }
};
