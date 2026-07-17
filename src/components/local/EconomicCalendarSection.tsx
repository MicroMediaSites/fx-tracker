/**
 * Economic Calendar — upcoming releases from the local calendar store
 * (`~/.wickd/calendar/`), rendered on the HOME window.
 *
 * Read-only and offline: the wickd CLI's `calendar sync` (periodic via the
 * `com.openthink.wickd-calendar` launchd job) owns freshness; this section
 * only invokes the `get_economic_calendar` reader. Informational, like the
 * Signals feed — the Live Monitor stays reserved for actionable state.
 */
import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { CollapsibleSection } from '../ui/CollapsibleSection';

const REFRESH_INTERVAL_MS = 5 * 60 * 1000; // store changes at most every sync
const COUNTDOWN_TICK_MS = 30 * 1000;

/** Currencies selected by default: the legs of the pairs Matt's watchers trade. */
const DEFAULT_CURRENCIES = ['USD', 'EUR', 'GBP', 'JPY', 'CHF', 'AUD', 'CAD'];
const CURRENCY_STORAGE_KEY = 'candlesight_calendar_currencies';
const IMPACT_STORAGE_KEY = 'candlesight_calendar_high_only';

export interface EconomicCalendarEvent {
  date: string;
  time: string;
  time_unix: number;
  currency: string;
  event: string;
  impact: string;
  actual: string;
  forecast: string;
  previous: string;
}

const impactDot = (impact: string): string => {
  switch (impact) {
    case 'high':
      return 'bg-[var(--color-sell)]';
    case 'medium':
      return 'bg-[var(--color-info)]';
    default:
      return 'bg-[var(--color-text-muted)]';
  }
};

const dayLabel = (date: string, nowUnix: number): string => {
  const today = new Date(nowUnix * 1000).toISOString().slice(0, 10);
  const tomorrow = new Date((nowUnix + 86400) * 1000).toISOString().slice(0, 10);
  if (date === today) return 'Today';
  if (date === tomorrow) return 'Tomorrow';
  return new Date(`${date}T00:00:00Z`).toLocaleDateString(undefined, {
    weekday: 'long',
    month: 'short',
    day: 'numeric',
    timeZone: 'UTC',
  });
};

const localTime = (timeUnix: number): string =>
  new Date(timeUnix * 1000).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });

export const countdown = (seconds: number): string => {
  if (seconds < 60) return 'now';
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (h >= 48) return `in ${Math.floor(h / 24)}d`;
  if (h > 0) return `in ${h}h ${m}m`;
  return `in ${m}m`;
};

export const EventRow = ({ ev }: { ev: EconomicCalendarEvent }) => (
  <div
    data-testid="calendar-event-row"
    className="flex flex-wrap items-center gap-3 px-3 py-1.5 rounded border border-[var(--color-border)] bg-[var(--color-bg-elevated)]"
  >
    <span className="text-xs text-[var(--color-text-muted)] whitespace-nowrap w-16">{localTime(ev.time_unix)}</span>
    <span
      data-testid="calendar-impact"
      className={`w-2 h-2 rounded-full shrink-0 ${impactDot(ev.impact)}`}
      title={`${ev.impact} impact`}
    />
    <span className="text-xs font-semibold w-9">{ev.currency}</span>
    <span className="flex-1 min-w-0 text-sm text-[var(--color-text-secondary)] truncate" title={ev.event}>
      {ev.event}
    </span>
    {ev.actual && (
      <span className="text-xs whitespace-nowrap" title="Actual">
        {ev.actual}
      </span>
    )}
    {ev.forecast && (
      <span className="text-xs text-[var(--color-text-muted)] whitespace-nowrap" title="Forecast">
        f {ev.forecast}
      </span>
    )}
    {ev.previous && (
      <span className="text-xs text-[var(--color-text-muted)] whitespace-nowrap" title="Previous">
        p {ev.previous}
      </span>
    )}
  </div>
);

export const EconomicCalendarSection = () => {
  const [events, setEvents] = useState<EconomicCalendarEvent[]>([]);
  const [nowUnix, setNowUnix] = useState(() => Math.floor(Date.now() / 1000));
  const [highOnly, setHighOnly] = useState(() => localStorage.getItem(IMPACT_STORAGE_KEY) === 'true');
  const [currencies, setCurrencies] = useState<string[]>(() => {
    const stored = localStorage.getItem(CURRENCY_STORAGE_KEY);
    if (stored) {
      try {
        const parsed: unknown = JSON.parse(stored);
        if (Array.isArray(parsed)) return parsed.filter((c): c is string => typeof c === 'string');
      } catch {
        // fall through to default
      }
    }
    return DEFAULT_CURRENCIES;
  });

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const rows = await invoke<EconomicCalendarEvent[]>('get_economic_calendar', {
          daysBack: 0,
          daysAhead: 7,
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

  useEffect(() => {
    const tick = setInterval(() => setNowUnix(Math.floor(Date.now() / 1000)), COUNTDOWN_TICK_MS);
    return () => clearInterval(tick);
  }, []);

  const allCurrencies = useMemo(
    () => Array.from(new Set(events.map((e) => e.currency))).sort(),
    [events]
  );

  const visible = useMemo(
    () =>
      events.filter(
        (e) =>
          e.time_unix >= nowUnix &&
          (highOnly ? e.impact === 'high' : e.impact === 'high' || e.impact === 'medium') &&
          currencies.includes(e.currency)
      ),
    [events, nowUnix, highOnly, currencies]
  );

  const nextHigh = useMemo(
    () => visible.find((e) => e.impact === 'high'),
    [visible]
  );

  const byDay = useMemo(() => {
    const groups = new Map<string, EconomicCalendarEvent[]>();
    for (const ev of visible) {
      const list = groups.get(ev.date) ?? [];
      list.push(ev);
      groups.set(ev.date, list);
    }
    return Array.from(groups.entries());
  }, [visible]);

  const toggleCurrency = (c: string) => {
    setCurrencies((prev) => {
      const next = prev.includes(c) ? prev.filter((x) => x !== c) : [...prev, c];
      localStorage.setItem(CURRENCY_STORAGE_KEY, JSON.stringify(next));
      return next;
    });
  };

  const toggleHighOnly = () => {
    setHighOnly((prev) => {
      localStorage.setItem(IMPACT_STORAGE_KEY, String(!prev));
      return !prev;
    });
  };

  return (
    <CollapsibleSection
      id="home_economic_calendar"
      title="Economic Calendar"
      badge={
        nextHigh ? (
          <span data-testid="calendar-next-high" className="text-xs text-[var(--color-text-muted)]">
            next high: {nextHigh.currency} {nextHigh.event} {countdown(nextHigh.time_unix - nowUnix)}
          </span>
        ) : undefined
      }
    >
      <div className="flex flex-wrap items-center gap-1.5 mb-3 px-1">
        <button
          onClick={toggleHighOnly}
          data-testid="calendar-impact-toggle"
          className={`px-2 py-0.5 text-xs rounded border transition-colors ${
            highOnly
              ? 'border-[var(--color-sell)]/40 text-[var(--color-sell)]'
              : 'border-[var(--color-border)] text-[var(--color-text-secondary)]'
          }`}
        >
          {highOnly ? 'high only' : 'med + high'}
        </button>
        {allCurrencies.map((c) => (
          <button
            key={c}
            onClick={() => toggleCurrency(c)}
            data-testid={`calendar-currency-${c}`}
            className={`px-2 py-0.5 text-xs rounded border transition-colors ${
              currencies.includes(c)
                ? 'border-[var(--color-info)]/40 text-[var(--color-text-primary)]'
                : 'border-[var(--color-border)] text-[var(--color-text-muted)] opacity-50'
            }`}
          >
            {c}
          </button>
        ))}
      </div>

      {visible.length === 0 ? (
        <p data-testid="calendar-empty" className="text-sm text-[var(--color-text-muted)] px-1">
          No upcoming events in the store. The `com.openthink.wickd-calendar` job syncs the
          ForexFactory week on a 6-hour interval (`wickd calendar sync` runs one manually).
        </p>
      ) : (
        <div className="space-y-3">
          {byDay.map(([date, dayEvents]) => (
            <div key={date}>
              <p className="text-xs uppercase tracking-wide text-[var(--color-text-muted)] px-1 mb-1.5">
                {dayLabel(date, nowUnix)}
              </p>
              <div className="space-y-1.5">
                {dayEvents.map((ev) => (
                  <EventRow key={`${ev.date}-${ev.time}-${ev.currency}-${ev.event}`} ev={ev} />
                ))}
              </div>
            </div>
          ))}
        </div>
      )}
    </CollapsibleSection>
  );
};
