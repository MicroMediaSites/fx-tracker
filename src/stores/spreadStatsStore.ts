/**
 * Historical spread statistics for the spread-bar coloring.
 *
 * The wickd CLI samples every quote's spread into `~/.wickd/spreads.db`
 * (per-instrument decayed min/max + slow EMA — see
 * `crates/wickd/src/spread_stats.rs`). The app reads that store via the
 * read-only `get_spread_stats` Tauri command and grades the live spread
 * against it: green = historically low, yellow = average, red = high,
 * purple = no history yet.
 *
 * Stats move slowly (the EMA has a ~1-day half-life), so a coarse poll is
 * plenty. Polling runs only while at least one component subscribes via
 * `useSpreadStats`, and any fetch failure is swallowed — spread history is
 * display-only and must never break a price surface.
 */
import { useEffect } from 'react';
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';

/** One instrument's stats row (decimal fields are strings, as stored). */
export interface SpreadStats {
  instrument: string;
  sample_count: number;
  min_spread: string;
  max_spread: string;
  ema_spread: string;
}

interface SpreadStatsState {
  /** Stats by instrument (empty until the first successful fetch). */
  stats: Record<string, SpreadStats>;
  setStats: (rows: SpreadStats[]) => void;
}

export const useSpreadStatsStore = create<SpreadStatsState>((set) => ({
  stats: {},
  setStats: (rows) =>
    set({ stats: Object.fromEntries(rows.map((r) => [r.instrument, r])) }),
}));

/** How often the (slow-moving) stats are re-read while subscribed. */
export const SPREAD_STATS_REFRESH_MS = 30_000;

/** Fetch once; failures leave the last known stats in place. */
export async function refreshSpreadStats(): Promise<void> {
  try {
    const rows = await invoke<SpreadStats[]>('get_spread_stats');
    useSpreadStatsStore.getState().setStats(rows ?? []);
  } catch {
    // Display-only data: a missing DB or transient read error just means the
    // bar keeps its current (or fallback) coloring.
  }
}

// Reference-counted poller shared by all subscribed components: one timer no
// matter how many PriceWindows are mounted, stopped when the last unmounts.
let subscriberCount = 0;
let pollTimer: ReturnType<typeof setInterval> | null = null;

/**
 * Subscribe to one instrument's historical spread stats. Starts the shared
 * poller on first mount; returns `undefined` until stats exist (the UI's
 * "no history" purple fallback).
 */
export function useSpreadStats(instrument: string): SpreadStats | undefined {
  const stats = useSpreadStatsStore((state) => state.stats[instrument]);

  useEffect(() => {
    subscriberCount += 1;
    if (subscriberCount === 1) {
      void refreshSpreadStats();
      pollTimer = setInterval(() => void refreshSpreadStats(), SPREAD_STATS_REFRESH_MS);
    }
    return () => {
      subscriberCount -= 1;
      if (subscriberCount === 0 && pollTimer !== null) {
        clearInterval(pollTimer);
        pollTimer = null;
      }
    };
  }, []);

  return stats;
}
