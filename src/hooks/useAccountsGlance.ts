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
  days: number;
  since: string;
  generated_at: string;
  accounts: AccountGlance[];
}

export interface UseAccountsGlance {
  data: AccountsGlance | null;
  /** Set only when there is nothing to show; a refresh failure keeps stale data. */
  error: string | null;
  loading: boolean;
  refresh: () => void;
}

export const useAccountsGlance = (days: number): UseAccountsGlance => {
  const [data, setData] = useState<AccountsGlance | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  // Read inside load() so a manual refresh doesn't need `data` as a dependency
  // (which would tear down the poll interval on every successful fetch).
  const hasData = useRef(false);

  const load = useCallback(
    async (force: boolean) => {
      setLoading(true);
      try {
        const result = await invoke<AccountsGlance>('accounts_glance', {
          days,
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
    [days]
  );

  useEffect(() => {
    // Deliberate: changing the window discards the previous result rather than
    // holding it on screen while the new one loads. Keeping it would render the
    // 7d numbers underneath a "30d" label — briefly, but wrongly. A momentary
    // "Loading accounts…" is the honest render, and the backend caches per
    // (env, days), so switching back is instant inside the TTL.
    hasData.current = false;
    void load(false);
    const interval = setInterval(() => void load(false), REFRESH_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [load]);

  const refresh = useCallback(() => void load(true), [load]);

  return { data, error, loading, refresh };
};
