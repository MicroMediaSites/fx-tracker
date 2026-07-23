/**
 * Closed-trade history for one account, via the `account_history` command.
 *
 * The drill-down behind an account tile. Like the glance it reaches OANDA
 * through the CLI, so it takes a few seconds and is fetched on demand (when the
 * modal opens), never on a timer.
 */
import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export interface TradeExit {
  time: string | null;
  price: string | null;
  count: number;
  /** True when `price` is an average across several fills, not a single exit. */
  blended: boolean;
}

export interface HistoryTrade {
  id: string;
  instrument: string;
  side: 'long' | 'short';
  units: string;
  strategy: string | null;
  entry: { time: string; price: string };
  exit: TradeExit;
  realized_pl: string;
  duration_secs: number | null;
}

export interface AccountHistory {
  account: string;
  account_id: string;
  environment: string;
  baseline: { balance: string; date: string } | null;
  since: string | null;
  count: number;
  realized: string;
  /** How many trades in this window had a blended (multi-fill) exit. */
  blended_exits: number;
  /** True when OANDA's fetch cap was hit and older trades may exist. */
  truncated: boolean;
  trades: HistoryTrade[];
}

export interface UseAccountHistory {
  data: AccountHistory | null;
  error: string | null;
  loading: boolean;
  reload: () => void;
}

/** `account === null` means the drill-down is closed — no fetch is issued. */
export const useAccountHistory = (account: string | null): UseAccountHistory => {
  const [data, setData] = useState<AccountHistory | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    if (account === null) return;
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<AccountHistory>('account_history', { account });
      setData(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [account]);

  useEffect(() => {
    // Clear the prior account's trades immediately so an open modal never shows
    // one account's history under another's name while the next fetch runs.
    setData(null);
    setError(null);
    if (account !== null) void load();
  }, [account, load]);

  return { data, error, loading, reload: load };
};
