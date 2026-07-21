/**
 * useCalendarEvents — one poller for the whole window.
 *
 * The point of the module-scoped store is that N consumers produce ONE fetch
 * and share ONE snapshot. A per-consumer hook would have satisfied "share the
 * code" while leaving the actual defect (two IPC calls, two independently
 * timed snapshots that can disagree) in place — so the dedupe is what these
 * tests pin.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const EVENTS = [
  {
    date: '2026-07-21',
    time: '12:30',
    time_unix: 1784900000,
    currency: 'USD',
    event: 'Core CPI m/m',
    impact: 'high',
    actual: '',
    forecast: '0.2%',
    previous: '0.1%',
  },
];

// Imported after the mock is registered.
const { useCalendarEvents, __resetCalendarEventsForTest } = await import('./useCalendarEvents');

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(EVENTS);
  __resetCalendarEventsForTest();
});

afterEach(() => {
  __resetCalendarEventsForTest();
});

describe('useCalendarEvents', () => {
  it('fetches once and shares the result across consumers', async () => {
    const a = renderHook(() => useCalendarEvents());
    const b = renderHook(() => useCalendarEvents());

    await waitFor(() => expect(a.result.current).not.toBeNull());

    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(b.result.current).toEqual(EVENTS);
    // Identical reference, not merely equal — the two views cannot drift.
    expect(a.result.current).toBe(b.result.current);
  });

  it('asks for the window both consumers need', async () => {
    const { result } = renderHook(() => useCalendarEvents());
    await waitFor(() => expect(result.current).not.toBeNull());

    expect(invokeMock).toHaveBeenCalledWith('get_economic_calendar', {
      daysBack: 1,
      daysAhead: 14,
    });
  });

  it('starts null so loading is distinguishable from empty', () => {
    const { result } = renderHook(() => useCalendarEvents());

    expect(result.current).toBeNull();
  });

  it('degrades to an empty list when the store is unreadable', async () => {
    invokeMock.mockRejectedValue(new Error('store unreadable'));
    const { result } = renderHook(() => useCalendarEvents());

    await waitFor(() => expect(result.current).toEqual([]));
  });

  it('keeps the last good snapshot when a later refresh fails', async () => {
    // Must actually drive the refresh — asserting the value before the
    // re-fetch would pass no matter what the failure path did.
    vi.useFakeTimers();
    try {
      const { result } = renderHook(() => useCalendarEvents());
      await vi.advanceTimersByTimeAsync(0);
      expect(result.current).toEqual(EVENTS);

      invokeMock.mockRejectedValue(new Error('transient'));
      await vi.advanceTimersByTimeAsync(5 * 60 * 1000);

      // The refresh really ran, and really failed…
      expect(invokeMock).toHaveBeenCalledTimes(2);
      // …and a transient failure did not blank a populated dashboard.
      expect(result.current).toEqual(EVENTS);
    } finally {
      vi.useRealTimers();
    }
  });

  it('polls while mounted and stops once the last consumer unmounts', async () => {
    vi.useFakeTimers();
    try {
      const a = renderHook(() => useCalendarEvents());
      const b = renderHook(() => useCalendarEvents());

      // Initial load — one fetch for both consumers.
      await vi.advanceTimersByTimeAsync(0);
      expect(invokeMock).toHaveBeenCalledTimes(1);

      // The interval is live, and still shared: one fetch per tick, not two.
      await vi.advanceTimersByTimeAsync(5 * 60 * 1000);
      expect(invokeMock).toHaveBeenCalledTimes(2);

      a.unmount();
      b.unmount();

      // Nothing left running against an unmounted window.
      invokeMock.mockClear();
      await vi.advanceTimersByTimeAsync(15 * 60 * 1000);
      expect(invokeMock).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });
});
