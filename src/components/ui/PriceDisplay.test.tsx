import { describe, it, expect } from 'vitest';
import { calculateSpreadColor } from './PriceDisplay';

/** Extract the HSL hue from a `hsl(H, S%, L%)` string. */
function hue(color: string): number {
  const m = /^hsl\((-?[\d.]+),/.exec(color);
  if (!m) throw new Error(`not an hsl color: ${color}`);
  return parseFloat(m[1]);
}

describe('calculateSpreadColor', () => {
  const MIN = 0.00014;
  const MAX = 0.00026;

  it('grades a historically low spread green (hue 120)', () => {
    expect(hue(calculateSpreadColor(MIN, MIN, MAX, 'EUR_USD'))).toBe(120);
  });

  it('grades a mid-range spread yellow (hue 60)', () => {
    expect(hue(calculateSpreadColor((MIN + MAX) / 2, MIN, MAX, 'EUR_USD'))).toBeCloseTo(60);
  });

  it('grades a historically high spread red (hue 0)', () => {
    expect(hue(calculateSpreadColor(MAX, MIN, MAX, 'EUR_USD'))).toBe(0);
  });

  it('clamps spreads outside the historical range', () => {
    expect(hue(calculateSpreadColor(MIN / 2, MIN, MAX, 'EUR_USD'))).toBe(120);
    expect(hue(calculateSpreadColor(MAX * 3, MIN, MAX, 'EUR_USD'))).toBe(0);
  });

  it('falls back to the purple no-data gradient without stats', () => {
    const h = hue(calculateSpreadColor(0.00016, undefined, undefined, 'EUR_USD'));
    expect(h).toBeGreaterThanOrEqual(280);
    expect(h).toBeLessThanOrEqual(320);
  });

  it('falls back to purple when history is degenerate (min == max)', () => {
    const h = hue(calculateSpreadColor(0.00016, 0.00016, 0.00016, 'EUR_USD'));
    expect(h).toBeGreaterThanOrEqual(280);
    expect(h).toBeLessThanOrEqual(320);
  });
});
