/**
 * Economic-calendar events for one instrument's currency legs, from the
 * local store via `get_economic_calendar` (read-only, offline — the wickd
 * CLI's launchd sync job owns freshness).
 *
 * Medium+high impact only: chart markers and the header strip are glanceable
 * surfaces; low-impact noise belongs in the home window's calendar section
 * where it can be filtered deliberately.
 */
import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { EconomicCalendarEvent } from '../components/local/EconomicCalendarSection';

const REFRESH_INTERVAL_MS = 5 * 60 * 1000;
/** History window: enough to cover any realistic loaded candle range. */
const DAYS_BACK = 30;
const DAYS_AHEAD = 7;

/** `EUR_USD` → `['EUR', 'USD']`; unknown shapes → empty (no events). */
export const instrumentLegs = (instrument: string): string[] => {
  const parts = instrument.split('_');
  return parts.length === 2 ? parts : [];
};

export const useEconomicEvents = (instrument: string): EconomicCalendarEvent[] => {
  const [events, setEvents] = useState<EconomicCalendarEvent[]>([]);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const rows = await invoke<EconomicCalendarEvent[]>('get_economic_calendar', {
          daysBack: DAYS_BACK,
          daysAhead: DAYS_AHEAD,
        });
        if (!cancelled) setEvents(Array.isArray(rows) ? rows : []);
      } catch {
        // Store unreadable — keep the last known list
      }
    };
    void load();
    const interval = setInterval(() => void load(), REFRESH_INTERVAL_MS);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, []);

  return useMemo(() => {
    const legs = instrumentLegs(instrument);
    return events.filter(
      (e) => (e.impact === 'high' || e.impact === 'medium') && legs.includes(e.currency)
    );
  }, [events, instrument]);
};
