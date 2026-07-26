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

/** One real exit fill behind a blended average (AGT-782). */
export interface ExitFill {
  /** The OANDA `ORDER_FILL` transaction this fill came from. */
  transaction_id: string;
  time: string;
  /** The price this fill actually happened at — not the blended average. */
  price: string;
  /** Signed as OANDA signs it: opposite the trade's direction. */
  units: string;
  realized_pl: string;
}

export interface HistoryTrade {
  id: string;
  instrument: string;
  side: 'long' | 'short';
  units: string;
  strategy: string | null;
  entry: { time: string; price: string };
  exit: TradeExit;
  /**
   * The fills behind `exit.price`, oldest first — or `null` when the trade was
   * not decomposed (no transaction coverage, or fills that did not reconcile).
   *
   * `null` and `[]` mean different things: `null` is "we did not break this
   * down", while an empty array would claim the trade had no exits at all.
   * Optional because a history from an older CLI has no such field.
   */
  exits?: ExitFill[] | null;
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
  /**
   * How many of those the transaction feed could break back down into real
   * fills. Below `blended_exits` means some trade kept its average.
   */
  decomposed_exits?: number;
  /** Why the decomposition did not run, when it was wanted but failed. */
  decompose_error?: string | null;
  /**
   * True when the paged walk hit its page budget with history still unread —
   * the result does NOT reach back to `since`.
   */
  truncated: boolean;
  /** How many OANDA requests the paged walk took. */
  pages?: number;
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
