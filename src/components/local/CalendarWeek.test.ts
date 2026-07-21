/**
 * weekBuckets / countdown — the shape of the dashboard's news block.
 *
 * The bucketing is the part worth pinning: it must keep empty days (an empty
 * column is information) and must bucket by the VIEWER'S local day, not UTC.
 */
import { describe, expect, it } from 'vitest';
import { countdown, weekBuckets } from './CalendarWeek';
import type { EconomicCalendarEvent } from './EconomicCalendarSection';

const at = (unix: number, over: Partial<EconomicCalendarEvent> = {}): EconomicCalendarEvent => ({
  date: '2026-07-20',
  time: '12:00',
  time_unix: unix,
  currency: 'USD',
  event: 'Some Release',
  impact: 'high',
  actual: '',
  forecast: '',
  previous: '',
  ...over,
});

/** Midday local, so ±hours stay inside the same local day in any timezone. */
const noon = Math.floor(new Date(2026, 6, 20, 12, 0, 0).getTime() / 1000);

describe('weekBuckets', () => {
  it('returns one bucket per day including empty ones', () => {
    // An empty column tells you the week is quiet there; dropping it would
    // silently reshape the grid.
    const buckets = weekBuckets([at(noon)], noon);

    expect(buckets).toHaveLength(7);
    expect(buckets[0].events).toHaveLength(1);
    expect(buckets.slice(1).every((b) => b.events.length === 0)).toBe(true);
  });

  it('buckets events into their local day', () => {
    const tomorrowNoon = noon + 86400;
    const buckets = weekBuckets([at(tomorrowNoon), at(noon)], noon);

    expect(buckets[0].events).toHaveLength(1);
    expect(buckets[1].events).toHaveLength(1);
    expect(buckets[1].events[0].time_unix).toBe(tomorrowNoon);
  });

  it('orders events within a day by time', () => {
    const buckets = weekBuckets(
      [at(noon + 3600, { event: 'later' }), at(noon, { event: 'earlier' })],
      noon
    );

    expect(buckets[0].events.map((e) => e.event)).toEqual(['earlier', 'later']);
  });

  it('drops events outside the seven-day horizon', () => {
    const buckets = weekBuckets([at(noon + 30 * 86400), at(noon)], noon);

    expect(buckets.flatMap((b) => b.events)).toHaveLength(1);
  });

  it('honours a custom day count', () => {
    expect(weekBuckets([], noon, 3)).toHaveLength(3);
  });
});

describe('countdown', () => {
  it('renders minutes under an hour', () => {
    expect(countdown(12 * 60)).toBe('in 12m');
  });

  it('renders hours and minutes', () => {
    expect(countdown(101 * 60)).toBe('in 1h 41m');
  });

  it('omits the minutes on a whole hour', () => {
    expect(countdown(2 * 3600)).toBe('in 2h');
  });

  it('renders days past 24 hours', () => {
    expect(countdown(50 * 3600)).toBe('in 2d');
  });

  it('collapses a due or past event to "now" rather than a negative', () => {
    expect(countdown(0)).toBe('now');
    expect(countdown(-500)).toBe('now');
  });
});
