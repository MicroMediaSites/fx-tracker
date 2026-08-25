/**
 * Closed-trade history for one account, via the `account_history` command.
 *
 * The drill-down behind an account tile. Like the glance it reaches OANDA
 * through the CLI, so it takes a few seconds and is fetched on demand (when the
 * modal opens), never on a timer.
 *
 * The fetch honours the SAME window the tile grid is showing (D8 of
 * `wickd-account-windows`) — the tile and its drill-down must agree or the
 * numbers can't be reconciled. See `windowToHistoryArgs` for the mapping.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { GlanceWindow, localMidnightIso } from './useAccountsGlance';

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

/**
 * `N` days back from `now` as an RFC3339 instant — the `account_history`
 * equivalent of `accounts_glance`'s `--days N`, which the CLI computes as
 * `now - N days` itself (`glance_window()` in `trade.rs`). `wickd trade
 * history` has no `--days` flag (only `--since`/`--to`), so for a `days`
 * window the frontend must supply the instant. Exact wall-clock subtraction
 * (not calendar/local-midnight math, unlike `localMidnightIso`) to match the
 * CLI's own arithmetic exactly.
 */
export const daysAgoIso = (days: number, now: Date = new Date()): string =>
  new Date(now.getTime() - days * 24 * 60 * 60 * 1000).toISOString();

/**
 * D8: map the section's active `GlanceWindow` to the `since`/`to` args
 * `account_history` takes.
 *
 * - `baseline` → neither: `trade history` already defaults to since-baseline
 *   per account when `since` is omitted, which is exactly this window.
 * - `range` → both, verbatim (closed window, `since` inclusive / `to`
 *   exclusive — same convention the CLI itself uses).
 * - `today` → `since` = the viewer's local midnight, recomputed per call so a
 *   long-open modal follows the date over, same as the glance hook.
 * - `days` → `since` = `daysAgoIso(days)`; no `to` (open-ended to now).
 *
 * Pure and exported so the mapping is tested independently of the fetch.
 */
export const windowToHistoryArgs = (
  w: GlanceWindow,
  now: Date = new Date()
): { since: string | null; to: string | null } => {
  switch (w.kind) {
    case 'baseline':
      return { since: null, to: null };
    case 'range':
      return { since: w.from, to: w.to };
    case 'today':
      return { since: localMidnightIso(now), to: null };
    case 'days':
      return { since: daysAgoIso(w.days, now), to: null };
  }
};

/**
 * `account === null` means the drill-down is closed — no fetch is issued.
 *
 * Param is `glanceWindow`, not `window` — shadowing the global would make any
 * later `window.*` access in this hook fail in a confusing way (same reason
 * `useAccountsGlance` avoids it).
 */
export const useAccountHistory = (
  account: string | null,
  glanceWindow: GlanceWindow
): UseAccountHistory => {
  const [data, setData] = useState<AccountHistory | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  // Depend on the window's primitive fields, not the object, so an inline
  // `{ kind: 'days', days: 7 }` literal at the call site doesn't rebuild
  // `load` (and refetch) on every render — same pattern as useAccountsGlance.
  const kind = glanceWindow.kind;
  const days = glanceWindow.kind === 'days' ? glanceWindow.days : null;
  const from = glanceWindow.kind === 'range' ? glanceWindow.from : null;
  const to = glanceWindow.kind === 'range' ? glanceWindow.to : null;

  // Recombined from the primitives above rather than used directly, so the
  // reference is stable across renders unless one of those fields actually
  // changes (see the note on `load`'s deps).
  const stableWindow = useMemo<GlanceWindow>(() => {
    switch (kind) {
      case 'days':
        return { kind, days: days as number };
      case 'range':
        return { kind, from: from as string, to: to as string };
      default:
        return { kind };
    }
  }, [kind, days, from, to]);

  const load = useCallback(async () => {
    if (account === null) return;
    setLoading(true);
    setError(null);
    try {
      const { since, to: toArg } = windowToHistoryArgs(stableWindow, new Date());
      const result = await invoke<AccountHistory>('account_history', {
        account,
        since,
        to: toArg,
      });
      setData(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [account, stableWindow]);

  useEffect(() => {
    // Clear the prior account's trades immediately so an open modal never shows
    // one account's history under another's name while the next fetch runs.
    setData(null);
    setError(null);
    if (account !== null) void load();
  }, [account, load]);

  return { data, error, loading, reload: load };
};
