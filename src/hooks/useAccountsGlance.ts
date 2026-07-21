/**
 * Rolling-window performance for every account configured in the wickd CLI,
 * via the `accounts_glance` command.
 *
 * Unlike the calendar/feed readers this one is NOT offline — it reaches OANDA
 * through the CLI and takes ~5s for a full fan-out. So the hook deliberately
 * keeps the last good value on screen while a refresh runs (`loading` is for a
 * subtle indicator, never for blanking the panel), and an error never clears
 * already-rendered data.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

/** Backend TTL is 60s; polling faster just returns the same cached object. */
const REFRESH_INTERVAL_MS = 60 * 1000;

export interface AccountGlance {
  account: string;
  names: string[];
  account_id: string | null;
  currency: string | null;
  nav: string | null;
  balance: string | null;
  unrealized_pl: string | null;
  open_trade_count: number | null;
  realized: string | null;
  trades: number | null;
  wins: number | null;
  losses: number | null;
  /** Null when nothing was decided in the window — render "—", not 0%. */
  win_rate: number | null;
  error: string | null;
}

export interface AccountsGlance {
  environment: string;
  /** Null when an explicit `since` drove the window (i.e. "Today"). */
  days: number | null;
  since: string;
  generated_at: string;
  accounts: AccountGlance[];
}

/**
 * The window the panel is showing.
 *
 * "Today" is not `days: 1` — it is since the viewer's local midnight, which is
 * a different (and, mid-morning, much shorter) span than the last 24 hours.
 * "Was today profitable" is the question this panel exists to answer, so the
 * distinction is load-bearing rather than pedantic.
 */
export type GlanceWindow = { kind: 'today' } | { kind: 'days'; days: number };

/**
 * Start of the viewer's local day as an RFC3339 instant.
 *
 * Computed per fetch rather than once per mount: this app stays open for days
 * at a time, and a midnight captured at mount would silently keep reporting
 * yesterday's P&L as "today" after the date rolls.
 */
export const localMidnightIso = (now: Date = new Date()): string => {
  const midnight = new Date(now);
  midnight.setHours(0, 0, 0, 0);
  return midnight.toISOString();
};

export interface UseAccountsGlance {
  data: AccountsGlance | null;
  /** Set only when there is nothing to show; a refresh failure keeps stale data. */
  error: string | null;
  loading: boolean;
  refresh: () => void;
}

// Param is `glanceWindow`, not `window` — shadowing the global would make a
// later `window.localStorage` in this hook fail in a very confusing way.
export const useAccountsGlance = (glanceWindow: GlanceWindow): UseAccountsGlance => {
  const [data, setData] = useState<AccountsGlance | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  // Read inside load() so a manual refresh doesn't need `data` as a dependency
  // (which would tear down the poll interval on every successful fetch).
  const hasData = useRef(false);

  // Depend on the window's primitive fields, not the object: a caller passing
  // an inline `{ kind: 'days', days: 7 }` creates a new object every render,
  // which would rebuild `load` and tear down the poll interval each time.
  const kind = glanceWindow.kind;
  const days = glanceWindow.kind === 'days' ? glanceWindow.days : null;

  const load = useCallback(
    async (force: boolean) => {
      setLoading(true);
      try {
        const result = await invoke<AccountsGlance>('accounts_glance', {
          days: kind === 'days' ? days : null,
          // Recomputed per call so a long-lived window follows the date over.
          since: kind === 'today' ? localMidnightIso() : null,
          refresh: force,
        });
        setData(result);
        hasData.current = true;
        setError(null);
      } catch (e) {
        // Keep the last good render; only surface the error on an empty panel.
        if (!hasData.current) setError(String(e));
      } finally {
        setLoading(false);
      }
    },
    [kind, days]
  );

  useEffect(() => {
    // Deliberate: changing the window discards the previous result rather than
    // holding it on screen while the new one loads. Keeping it would render the
    // 7d numbers underneath a "30d" label — briefly, but wrongly. A momentary
    // "Loading accounts…" is the honest render, and the backend caches per
    // (env, days, since), so switching back is instant inside the TTL.
    hasData.current = false;
    void load(false);
    const interval = setInterval(() => void load(false), REFRESH_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [load]);

  const refresh = useCallback(() => void load(true), [load]);

  return { data, error, loading, refresh };
};
