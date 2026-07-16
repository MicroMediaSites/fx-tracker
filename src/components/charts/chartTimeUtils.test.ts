import { describe, it, expect } from 'vitest';
import { convertCandles, createTimeMapState, type CandleData } from './chartTimeUtils';

const candle = (time: string): CandleData => ({
  time,
  open: '1.1000',
  high: '1.1010',
  low: '1.0990',
  close: '1.1005',
});

describe('convertCandles', () => {
  it('computes typicalInterval from candle spacing', () => {
    const state = createTimeMapState();
    const candles = [
      candle('2026-07-16T22:00:00Z'),
      candle('2026-07-16T22:01:00Z'),
      candle('2026-07-16T22:02:00Z'),
    ];

    convertCandles(candles, state);

    expect(state.typicalInterval).toBe(60);
  });

  it('never produces a zero typicalInterval from duplicate timestamps (issue #7)', () => {
    const state = createTimeMapState();
    // Majority-duplicate timestamps used to drive the median interval to 0,
    // which mapped every streamed candle to the same business time and froze
    // the chart's candle series.
    const candles = [
      candle('2026-07-16T22:00:00Z'),
      candle('2026-07-16T22:00:00Z'),
      candle('2026-07-16T22:00:00Z'),
      candle('2026-07-16T22:01:00Z'),
    ];

    convertCandles(candles, state);

    expect(state.typicalInterval).toBeGreaterThan(0);
  });

  it('business time advances monotonically for every candle', () => {
    const state = createTimeMapState();
    const candles = [
      candle('2026-07-16T22:00:00Z'),
      candle('2026-07-16T22:01:00Z'),
      // weekend-style gap
      candle('2026-07-19T22:00:00Z'),
      candle('2026-07-19T22:01:00Z'),
    ];

    const chartData = convertCandles(candles, state);

    for (let i = 1; i < chartData.length; i++) {
      expect(Number(chartData[i].time)).toBeGreaterThan(Number(chartData[i - 1].time));
    }
    expect(state.lastBusinessTime).toBe(Number(chartData[chartData.length - 1].time));
  });
});
