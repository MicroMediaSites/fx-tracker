/**
 * Economic Calendar — upcoming releases from the local calendar store
 * (`~/.wickd/calendar/`), rendered on the HOME window.
 *
 * Read-only and offline: the wickd CLI's `calendar sync` (periodic via the
 * `com.openthink.wickd-calendar` launchd job) owns freshness; this section
 * only invokes the `get_economic_calendar` /
 * `get_economic_event_history` readers. Informational, like the Signals
 * feed — the Live Monitor stays reserved for actionable state.
 *
 * All day grouping is in the VIEWER'S LOCAL timezone: the store keeps UTC,
 * rows show local clock times, so "Today"/"Tomorrow" must be local too — a
 * 14:00 UTC Friday release is Thursday-evening-you's *tomorrow at 8am*,
 * not "today" (the UTC date has already rolled by a Mountain-time evening).
 */
import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { CollapsibleSection } from '../ui/CollapsibleSection';

const REFRESH_INTERVAL_MS = 5 * 60 * 1000; // store changes at most every sync
const COUNTDOWN_TICK_MS = 30 * 1000;
const HISTORY_LIMIT = 8;

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

/** `YYYY-MM-DD` of an instant in the viewer's local timezone. */
export const localDateKey = (unix: number): string => {
  const d = new Date(unix * 1000);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
};

const dayLabel = (key: string, nowUnix: number): string => {
  if (key === localDateKey(nowUnix)) return 'Today';
  if (key === localDateKey(nowUnix + 86400)) return 'Tomorrow';
  const [y, m, d] = key.split('-').map(Number);
  return new Date(y, m - 1, d).toLocaleDateString(undefined, {
    weekday: 'long',
    month: 'short',
    day: 'numeric',
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

export const ImpactBadge = ({ impact }: { impact: string }) => (
  <span
    data-testid="calendar-impact"
    className={`px-1.5 py-0.5 text-[10px] font-semibold rounded uppercase shrink-0 ${
      impact === 'high'
        ? 'bg-[var(--color-sell)]/15 text-[var(--color-sell)]'
        : impact === 'medium'
          ? 'bg-[var(--color-info)]/15 text-[var(--color-info)]'
          : 'bg-white/10 text-[var(--color-text-muted)]'
    }`}
  >
    {impact === 'medium' ? 'med' : impact}
  </span>
);

/** Release history for one series, lazily loaded when a row expands. */
const EventHistory = ({ ev }: { ev: EconomicCalendarEvent }) => {
  const [rows, setRows] = useState<EconomicCalendarEvent[] | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    invoke<EconomicCalendarEvent[]>('get_economic_event_history', {
      currency: ev.currency,
      event: ev.event,
      limit: HISTORY_LIMIT,
    })
      .then((r) => {
        if (!cancelled) setRows(Array.isArray(r) ? r : []);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [ev.currency, ev.event]);

  if (failed) return <p className="text-xs text-[var(--color-text-muted)]">History unavailable.</p>;
  if (rows === null) return <p className="text-xs text-[var(--color-text-muted)]">Loading history…</p>;
  if (rows.length === 0)
    return <p className="text-xs text-[var(--color-text-muted)]">No prior releases of this series in the store.</p>;

  return (
    <table data-testid="calendar-history" className="text-xs w-full max-w-sm">
      <thead>
        <tr className="text-[var(--color-text-muted)] text-left">
          <th className="font-normal pb-1">Release</th>
          <th className="font-normal pb-1 text-right">Actual</th>
          <th className="font-normal pb-1 text-right">Forecast</th>
          <th className="font-normal pb-1 text-right">Previous</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={`${r.date}-${r.time}`} className="text-[var(--color-text-secondary)]">
            <td className="py-0.5">{r.date}</td>
            <td className="py-0.5 text-right">{r.actual || '—'}</td>
            <td className="py-0.5 text-right">{r.forecast || '—'}</td>
            <td className="py-0.5 text-right">{r.previous || '—'}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
};

export const EventRow = ({ ev }: { ev: EconomicCalendarEvent }) => {
  const [expanded, setExpanded] = useState(false);

  return (
    <div
      data-testid="calendar-event-row"
      className="rounded border border-[var(--color-border)] bg-[var(--color-bg-elevated)]"
    >
      <button
        onClick={() => setExpanded((v) => !v)}
        data-testid="calendar-event-toggle"
        className="w-full flex flex-wrap items-center gap-3 px-3 py-1.5 text-left"
        aria-expanded={expanded}
      >
        <span className="text-xs text-[var(--color-text-muted)] whitespace-nowrap w-16">
          {localTime(ev.time_unix)}
        </span>
        <ImpactBadge impact={ev.impact} />
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
      </button>
      {expanded && (
        <div
          data-testid="calendar-event-detail"
          className="px-3 pb-2.5 pt-1 border-t border-[var(--color-border)] space-y-2"
        >
          <div className="flex flex-wrap gap-x-6 gap-y-1 text-xs text-[var(--color-text-secondary)]">
            <span>
              <span className="text-[var(--color-text-muted)]">Releases </span>
              {localTime(ev.time_unix)} local ({ev.time} UTC)
            </span>
            <span>
              <span className="text-[var(--color-text-muted)]">Actual </span>
              {ev.actual || 'pending'}
            </span>
            <span>
              <span className="text-[var(--color-text-muted)]">Forecast </span>
              {ev.forecast || '—'}
            </span>
            <span>
              <span className="text-[var(--color-text-muted)]">Previous </span>
              {ev.previous || '—'}
            </span>
          </div>
          <EventHistory ev={ev} />
        </div>
      )}
    </div>
  );
};

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
          daysAhead: 14,
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

  // Chips: the default set is always offered (an empty store must not make
  // currencies silently vanish, per Matt's 2026-07-16 feedback); event data
  // can only add to it.
  const allCurrencies = useMemo(
    () => Array.from(new Set([...DEFAULT_CURRENCIES, ...events.map((e) => e.currency)])).sort(),
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

  const nextHigh = useMemo(() => visible.find((e) => e.impact === 'high'), [visible]);

  const byDay = useMemo(() => {
    const groups = new Map<string, EconomicCalendarEvent[]>();
    for (const ev of visible) {
      const key = localDateKey(ev.time_unix);
      const list = groups.get(key) ?? [];
      list.push(ev);
      groups.set(key, list);
    }
    return Array.from(groups.entries());
  }, [visible]);

  // Store coverage horizon (from ALL fetched events, before filtering): the
  // free FF feed publishes one week at a time, so near the weekend the
  // forward window is honestly thin — say so instead of looking broken.
  const coverageEndsUnix = useMemo(
    () => (events.length ? Math.max(...events.map((e) => e.time_unix)) : null),
    [events]
  );
  const coverageThin = coverageEndsUnix !== null && coverageEndsUnix - nowUnix < 5 * 86400;

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
          {byDay.map(([key, dayEvents]) => (
            <div key={key}>
              <p className="text-xs uppercase tracking-wide text-[var(--color-text-muted)] px-1 mb-1.5">
                {dayLabel(key, nowUnix)}
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

      {coverageThin && coverageEndsUnix !== null && (
        <p data-testid="calendar-coverage-note" className="text-xs text-[var(--color-text-muted)] px-1 mt-3">
          Store coverage ends {dayLabel(localDateKey(coverageEndsUnix), nowUnix).toLowerCase()} — the free
          ForexFactory feed publishes one week at a time and rolls to the new week on Sunday (ET). The sync
          job picks it up automatically.
        </p>
      )}
    </CollapsibleSection>
  );
};
