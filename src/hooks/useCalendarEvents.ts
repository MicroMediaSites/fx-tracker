/**
 * Economic-calendar events, fetched ONCE for the whole window.
 *
 * Two components render this data on the home window — `CalendarWeek` (the
 * dashboard's next-up row and week grid) and `EconomicCalendarSection` (the
 * detailed "All releases" list). They previously each ran their own
 * `get_economic_calendar` poller, which meant two IPC round-trips for the same
 * local store and, worse, two independently-timed snapshots: the hero's
 * countdown and the "All releases" badge could disagree by up to a refresh
 * interval, which reads as a bug.
 *
 * A plain custom hook would NOT have fixed that — each consumer would still
 * own an interval. So the poller lives in module scope with a subscriber
 * count: the first mount starts it, the last unmount stops it, and every
 * consumer reads the identical array via `useSyncExternalStore`.
 *
 * Read-only and offline: the wickd CLI's `calendar sync` launchd job owns
 * freshness; this only invokes the reader.
 */
import { useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { EconomicCalendarEvent } from '../components/local/EconomicCalendarSection';

/** The store changes at most once per sync job run (6h); 5m is ample. */
const REFRESH_INTERVAL_MS = 5 * 60 * 1000;

/**
 * One day back so the dashboard can show today's already-released events
 * (dimmed) rather than an empty "Today" column; the detailed list filters to
 * future events itself, so the extra day is inert there.
 */
const DAYS_BACK = 1;
const DAYS_AHEAD = 14;

/** `null` until the first fetch resolves — distinguishes loading from empty. */
let snapshot: EconomicCalendarEvent[] | null = null;
const listeners = new Set<() => void>();
let timer: ReturnType<typeof setInterval> | null = null;
let subscribers = 0;

const emit = () => {
  for (const l of listeners) l();
};

const load = async () => {
  try {
    const rows = await invoke<EconomicCalendarEvent[]>('get_economic_calendar', {
      daysBack: DAYS_BACK,
      daysAhead: DAYS_AHEAD,
    });
    const next = Array.isArray(rows) ? rows : [];
    snapshot = next;
    emit();
  } catch {
    // Store unreadable — keep the last known list rather than blanking both
    // consumers. On a first-load failure this leaves `snapshot` null and the
    // consumers show their loading state, which is honest: we don't know yet.
    if (snapshot === null) {
      snapshot = [];
      emit();
    }
  }
};

const subscribe = (onChange: () => void): (() => void) => {
  listeners.add(onChange);
  subscribers += 1;
  if (subscribers === 1) {
    void load();
    timer = setInterval(() => void load(), REFRESH_INTERVAL_MS);
  }
  return () => {
    listeners.delete(onChange);
    subscribers -= 1;
    if (subscribers === 0 && timer !== null) {
      clearInterval(timer);
      timer = null;
    }
  };
};

/**
 * Stable reference between loads — required by `useSyncExternalStore`, which
 * would loop forever if this returned a fresh array each call.
 */
const getSnapshot = (): EconomicCalendarEvent[] | null => snapshot;

export const useCalendarEvents = (): EconomicCalendarEvent[] | null =>
  useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

/** Test-only: drop cached state so specs don't leak into each other. */
export const __resetCalendarEventsForTest = () => {
  snapshot = null;
  listeners.clear();
  subscribers = 0;
  if (timer !== null) {
    clearInterval(timer);
    timer = null;
  }
};
