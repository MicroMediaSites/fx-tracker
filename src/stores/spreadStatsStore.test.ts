import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import {
  useSpreadStats,
  useSpreadStatsStore,
  refreshSpreadStats,
  SPREAD_STATS_REFRESH_MS,
  type SpreadStats,
} from './spreadStatsStore';

const EUR: SpreadStats = {
  instrument: 'EUR_USD',
  sample_count: 17871,
  min_spread: '0.00014',
  max_spread: '0.00026',
  ema_spread: '0.000158',
};

describe('spreadStatsStore', () => {
  beforeEach(() => {
    useSpreadStatsStore.setState({ stats: {} });
    vi.mocked(invoke).mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('refreshSpreadStats indexes rows by instrument', async () => {
    vi.mocked(invoke).mockResolvedValue([EUR]);
    await refreshSpreadStats();
    expect(invoke).toHaveBeenCalledWith('get_spread_stats');
    expect(useSpreadStatsStore.getState().stats['EUR_USD']).toEqual(EUR);
  });

  it('refreshSpreadStats swallows failures and keeps last stats', async () => {
    useSpreadStatsStore.setState({ stats: { EUR_USD: EUR } });
    vi.mocked(invoke).mockRejectedValue(new Error('no db'));
    await expect(refreshSpreadStats()).resolves.toBeUndefined();
    expect(useSpreadStatsStore.getState().stats['EUR_USD']).toEqual(EUR);
  });

  it('useSpreadStats fetches on mount and returns the instrument row', async () => {
    vi.mocked(invoke).mockResolvedValue([EUR]);
    const { result, unmount } = renderHook(() => useSpreadStats('EUR_USD'));
    await waitFor(() => expect(result.current).toEqual(EUR));
    unmount();
  });

  it('useSpreadStats returns undefined for instruments with no history', async () => {
    vi.mocked(invoke).mockResolvedValue([EUR]);
    const { result, unmount } = renderHook(() => useSpreadStats('GBP_USD'));
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(result.current).toBeUndefined();
    unmount();
  });

  it('shares one poller across mounts and stops it after the last unmount', async () => {
    vi.useFakeTimers();
    vi.mocked(invoke).mockResolvedValue([EUR]);

    const a = renderHook(() => useSpreadStats('EUR_USD'));
    const b = renderHook(() => useSpreadStats('GBP_USD'));
    // One initial fetch despite two subscribers.
    expect(invoke).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(SPREAD_STATS_REFRESH_MS);
    });
    expect(invoke).toHaveBeenCalledTimes(2);

    a.unmount();
    b.unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(SPREAD_STATS_REFRESH_MS * 3);
    });
    // No polls after the last subscriber unmounted.
    expect(invoke).toHaveBeenCalledTimes(2);
  });
});
