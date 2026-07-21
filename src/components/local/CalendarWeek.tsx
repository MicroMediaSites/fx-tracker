/**
 * CalendarWeek — the dashboard's news block: what's next, and the week ahead.
 *
 * Deliberately not the day-grouped list that `EconomicCalendarSection` renders
 * (that one still exists below, for digging in). Two questions get answered by
 * shape rather than by reading:
 *
 *  - "What's the next big thing?" → one prominent row with a live countdown.
 *  - "What's coming this week?"   → seven day-columns side by side, so the
 *    shape of the week (which days are loaded, which are quiet) is visible in
 *    one glance instead of by scrolling a list.
 *
 * Read-only and offline: the wickd CLI's `calendar sync` launchd job owns
 * freshness; this only invokes the `get_economic_calendar` reader.
 *
 * All day bucketing is in the VIEWER'S LOCAL timezone — the store keeps UTC,
 * but a 14:00 UTC Friday release belongs in Thursday-evening-you's *tomorrow*
 * column, not today's.
 */
import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { EconomicCalendarEvent } from './EconomicCalendarSection';
import { localDateKey } from './EconomicCalendarSection';

const REFRESH_INTERVAL_MS = 5 * 60 * 1000;
const COUNTDOWN_TICK_MS = 30 * 1000;
/** Days of forward coverage rendered as columns. */
const DAYS = 7;

/** "in 1h 41m" / "in 12m" / "now" — the countdown on the next-up row. */
export const countdown = (seconds: number): string => {
  if (seconds <= 0) return 'now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `in ${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const rem = minutes % 60;
  if (hours < 24) return rem > 0 ? `in ${hours}h ${rem}m` : `in ${hours}h`;
  const days = Math.floor(hours / 24);
  return `in ${days}d`;
};

/**
 * The next `DAYS` local-day buckets starting today, each with its events in
 * time order. Days with nothing are kept (an empty column is information: the
 * week is quiet, or coverage has run out). Pure.
 */
export const weekBuckets = (
  events: EconomicCalendarEvent[],
  nowUnix: number,
  days = DAYS
): { key: string; events: EconomicCalendarEvent[] }[] => {
  const buckets: { key: string; events: EconomicCalendarEvent[] }[] = [];
  for (let i = 0; i < days; i += 1) {
    buckets.push({ key: localDateKey(nowUnix + i * 86400), events: [] });
  }
  const index = new Map(buckets.map((b, i) => [b.key, i]));
  for (const ev of events) {
    const slot = index.get(localDateKey(ev.time_unix));
    if (slot !== undefined) buckets[slot].events.push(ev);
  }
  for (const b of buckets) b.events.sort((a, z) => a.time_unix - z.time_unix);
  return buckets;
};

const dayHeading = (key: string, nowUnix: number): string => {
  if (key === localDateKey(nowUnix)) return 'Today';
  if (key === localDateKey(nowUnix + 86400)) return 'Tomorrow';
  const [y, m, d] = key.split('-').map(Number);
  return new Date(y, m - 1, d).toLocaleDateString(undefined, {
    weekday: 'short',
    day: 'numeric',
  });
};

const localTime = (timeUnix: number): string =>
  new Date(timeUnix * 1000).toLocaleTimeString(undefined, {
    hour: 'numeric',
    minute: '2-digit',
  });

/**
 * One event inside a day column. High impact carries color; medium recedes.
 *
 * Two lines, not one: at seven columns a day is ~140px wide, and a single row
 * of time + currency + name truncated the name to "USD Cor…", which is not
 * information. Meta on top, name wrapped to two lines below it.
 */
const DayEvent = ({ ev, past }: { ev: EconomicCalendarEvent; past: boolean }) => {
  const high = ev.impact === 'high';
  return (
    <div
      data-testid="calendar-week-event"
      className={`min-w-0 ${past ? 'opacity-40' : ''}`}
      title={`${localTime(ev.time_unix)} · ${ev.currency} · ${ev.event}`}
    >
      <div className="flex items-center gap-1 min-w-0">
        <span
          aria-hidden="true"
          className={`shrink-0 w-1 h-1 rounded-full ${
            high ? 'bg-[var(--color-sell)]' : 'bg-[var(--color-text-faint)]'
          }`}
        />
        <span className="text-[10px] font-mono text-[var(--color-text-faint)] tabular-nums">
          {localTime(ev.time_unix)}
        </span>
        <span
          className={`text-[10px] font-mono ${
            high ? 'text-[var(--color-text-secondary)]' : 'text-[var(--color-text-muted)]'
          }`}
        >
          {ev.currency}
        </span>
      </div>
      <div
        className={`pl-2 text-[11px] leading-snug line-clamp-2 ${
          high ? 'text-[var(--color-text-primary)]' : 'text-[var(--color-text-muted)]'
        }`}
      >
        {ev.event}
      </div>
    </div>
  );
};

export const CalendarWeek = () => {
  const [events, setEvents] = useState<EconomicCalendarEvent[] | null>(null);
  const [nowUnix, setNowUnix] = useState(() => Math.floor(Date.now() / 1000));

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const rows = await invoke<EconomicCalendarEvent[]>('get_economic_calendar', {
          daysBack: 1,
          daysAhead: 14,
        });
        if (!cancelled) setEvents(Array.isArray(rows) ? rows : []);
      } catch {
        if (!cancelled) setEvents([]);
      }
    };
    void load();
    const interval = setInterval(() => void load(), REFRESH_INTERVAL_MS);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, []);

  // Countdown ticks independently of the (much slower) store poll.
  useEffect(() => {
    const t = setInterval(() => setNowUnix(Math.floor(Date.now() / 1000)), COUNTDOWN_TICK_MS);
    return () => clearInterval(t);
  }, []);

  const upcoming = useMemo(
    () => (events ?? []).filter((e) => e.impact === 'high' || e.impact === 'medium'),
    [events]
  );
  const nextHigh = useMemo(
    () => upcoming.find((e) => e.impact === 'high' && e.time_unix > nowUnix),
    [upcoming, nowUnix]
  );
  const buckets = useMemo(() => weekBuckets(upcoming, nowUnix), [upcoming, nowUnix]);

  return (
    <section data-testid="calendar-week">
      <div className="flex items-baseline justify-between gap-3">
        <h2 className="text-[11px] font-medium uppercase tracking-wider text-[var(--color-text-muted)]">
          Next up
        </h2>
        <span className="text-[11px] text-[var(--color-text-faint)]">next 7 days</span>
      </div>

      {/* ── The one event worth knowing about right now ───────────────────── */}
      {nextHigh ? (
        <div
          data-testid="calendar-week-next-high"
          className="mt-1 flex items-baseline gap-2.5 flex-wrap"
        >
          <span
            aria-hidden="true"
            className="w-1.5 h-1.5 rounded-full bg-[var(--color-sell)] shrink-0"
          />
          <span className="text-sm font-mono text-[var(--color-text-secondary)]">
            {nextHigh.currency}
          </span>
          <span className="text-xl font-medium text-[var(--color-text-primary)] min-w-0 truncate">
            {nextHigh.event}
          </span>
          <span className="text-xl font-mono tabular-nums text-[var(--color-warning)]">
            {countdown(nextHigh.time_unix - nowUnix)}
          </span>
          <span className="text-xs text-[var(--color-text-faint)]">
            {localTime(nextHigh.time_unix)}
          </span>
        </div>
      ) : (
        <div className="mt-1 text-sm text-[var(--color-text-muted)]">
          {events === null ? 'Loading calendar…' : 'No high-impact releases in the store window.'}
        </div>
      )}

      {/* ── The week, as columns ──────────────────────────────────────────── */}
      <div
        data-testid="calendar-week-grid"
        className="mt-3 grid gap-x-3 gap-y-3 grid-cols-2 sm:grid-cols-4 lg:grid-cols-7"
      >
        {buckets.map((b) => {
          const isToday = b.key === localDateKey(nowUnix);
          return (
            <div key={b.key} data-testid="calendar-week-day" className="min-w-0">
              <div
                className={`text-[10px] uppercase tracking-wide pb-1 mb-1.5 border-b ${
                  isToday
                    ? 'text-[var(--color-text-secondary)] border-[var(--color-text-muted)]/40'
                    : 'text-[var(--color-text-faint)] border-[var(--color-border)]'
                }`}
              >
                {dayHeading(b.key, nowUnix)}
              </div>
              {b.events.length === 0 ? (
                <div className="text-[11px] text-[var(--color-text-faint)]">—</div>
              ) : (
                <div className="space-y-1">
                  {b.events.map((ev) => (
                    <DayEvent
                      key={`${ev.date}-${ev.time}-${ev.currency}-${ev.event}`}
                      ev={ev}
                      past={ev.time_unix <= nowUnix}
                    />
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </section>
  );
};
