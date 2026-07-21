/**
 * localMidnightIso — the boundary behind the accounts panel's "today" window.
 *
 * This is the one piece of "was today profitable" that cannot be expressed as
 * a day count: the CLI has no idea what timezone the reader is in, so the app
 * states the instant. Worth pinning, because getting it wrong silently reports
 * the wrong period's P&L rather than failing.
 */
import { describe, expect, it } from 'vitest';
import { localMidnightIso } from './useAccountsGlance';

describe('localMidnightIso', () => {
  it('returns the start of the local day, not 24 hours ago', () => {
    const now = new Date(2026, 6, 20, 15, 30, 0); // 20 Jul 2026, 15:30 local
    const midnight = new Date(localMidnightIso(now));

    expect(midnight.getFullYear()).toBe(2026);
    expect(midnight.getMonth()).toBe(6);
    expect(midnight.getDate()).toBe(20);
    expect(midnight.getHours()).toBe(0);
    expect(midnight.getMinutes()).toBe(0);
    expect(midnight.getSeconds()).toBe(0);
    expect(midnight.getMilliseconds()).toBe(0);
  });

  it('is a shorter window than 24h when the day is young', () => {
    // The distinction that motivates the whole `--since` path: at 00:30, the
    // last 24 hours is mostly *yesterday*, which is not what "today" means.
    const now = new Date(2026, 6, 20, 0, 30, 0);
    const midnight = new Date(localMidnightIso(now)).getTime();
    const dayAgo = now.getTime() - 24 * 60 * 60 * 1000;

    expect(midnight).toBeGreaterThan(dayAgo);
    expect(now.getTime() - midnight).toBe(30 * 60 * 1000);
  });

  it('does not mutate the date it is given', () => {
    // It sets hours on a Date; doing that in place would corrupt a caller's
    // clock value.
    const now = new Date(2026, 6, 20, 15, 30, 0);
    const before = now.getTime();
    localMidnightIso(now);

    expect(now.getTime()).toBe(before);
  });

  it('emits a parseable RFC3339 instant', () => {
    const iso = localMidnightIso(new Date(2026, 6, 20, 15, 30, 0));

    expect(Number.isNaN(Date.parse(iso))).toBe(false);
    expect(iso).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/);
  });
});
