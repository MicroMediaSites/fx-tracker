import { useState, useEffect } from 'react';

/** dailyAlignment=2 UTC — must match endpoints.rs and candle_boundary.rs */
const DAILY_ALIGNMENT_HOURS = 2;
const DAILY_ALIGNMENT_SECS = DAILY_ALIGNMENT_HOURS * 3600;

/**
 * Maps granularity strings to their duration in seconds.
 */
function getGranularitySeconds(granularity: string): number {
  switch (granularity) {
    case 'M1': return 60;
    case 'M5': return 300;
    case 'M15': return 900;
    case 'M30': return 1800;
    case 'H1': return 3600;
    case 'H2': return 7200;
    case 'H3': return 10800;
    case 'H4': return 14400;
    case 'H6': return 21600;
    case 'H8': return 28800;
    case 'H12': return 43200;
    case 'D': return 86400;
    case 'W': return 604800;
    default: return 3600;
  }
}

/**
 * Calculate seconds until the next candle closes.
 *
 * For sub-hourly and H1, candles align to simple epoch intervals.
 * For multi-hour (H4+), candles align to dailyAlignment=2 UTC
 * (boundaries at 02:00, 06:00, 10:00, 14:00, 18:00, 22:00 UTC for H4).
 */
function getSecondsUntilNextCandle(granularity: string): number {
  const intervalSec = getGranularitySeconds(granularity);
  const nowSec = Math.floor(Date.now() / 1000);

  if (intervalSec <= 3600) {
    // Sub-hourly and H1: simple epoch modulo
    const elapsed = nowSec % intervalSec;
    return intervalSec - elapsed;
  }

  // Multi-hour: account for dailyAlignment offset
  // Shift time so alignment base is at 0, then modulo, then shift back
  const shifted = nowSec - DAILY_ALIGNMENT_SECS;
  const elapsed = ((shifted % intervalSec) + intervalSec) % intervalSec;
  return intervalSec - elapsed;
}

function formatCountdown(totalSeconds: number): string {
  if (totalSeconds <= 0) return '0s';
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;

  if (hours > 0) {
    return `${hours}h ${String(minutes).padStart(2, '0')}m ${String(seconds).padStart(2, '0')}s`;
  }
  if (minutes > 0) {
    return `${minutes}m ${String(seconds).padStart(2, '0')}s`;
  }
  return `${seconds}s`;
}

interface CandleCountdownProps {
  granularity: string;
}

export const CandleCountdown = ({ granularity }: CandleCountdownProps) => {
  const [remaining, setRemaining] = useState(() => getSecondsUntilNextCandle(granularity));

  useEffect(() => {
    setRemaining(getSecondsUntilNextCandle(granularity));

    const interval = setInterval(() => {
      setRemaining(getSecondsUntilNextCandle(granularity));
    }, 1000);

    return () => clearInterval(interval);
  }, [granularity]);

  // Don't show for daily/weekly — not useful
  if (granularity === 'D' || granularity === 'W') return null;

  return (
    <span
      className="text-[var(--color-text-muted)] text-xs font-mono tabular-nums"
      title="Time until next candle"
    >
      {formatCountdown(remaining)}
    </span>
  );
};
