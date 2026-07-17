/**
 * Next economic release for the chart's instrument legs — a glanceable
 * countdown in the chart header, fed by the same store as the chart's
 * event markers. Renders nothing when the store holds no upcoming
 * medium/high event for the legs.
 */
import { useEffect, useState } from 'react';
import type { EconomicCalendarEvent } from '../local/EconomicCalendarSection';
import { countdown } from '../local/EconomicCalendarSection';

const TICK_MS = 30 * 1000;

export const EventStrip = ({ events }: { events: EconomicCalendarEvent[] }) => {
  const [nowUnix, setNowUnix] = useState(() => Math.floor(Date.now() / 1000));

  useEffect(() => {
    const tick = setInterval(() => setNowUnix(Math.floor(Date.now() / 1000)), TICK_MS);
    return () => clearInterval(tick);
  }, []);

  const next = events.find((e) => e.time_unix >= nowUnix);
  if (!next) return null;

  return (
    <span
      data-testid="chart-event-strip"
      className="flex items-center gap-1.5 text-xs text-[var(--color-text-muted)] whitespace-nowrap"
      title={`${next.currency} ${next.event} (${next.impact} impact)`}
    >
      <span
        className={`w-1.5 h-1.5 rounded-full shrink-0 ${
          next.impact === 'high' ? 'bg-[var(--color-sell)]' : 'bg-[var(--color-info)]'
        }`}
      />
      {next.currency} {next.event} {countdown(next.time_unix - nowUnix)}
    </span>
  );
};
