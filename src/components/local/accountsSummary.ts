/**
 * Aggregate across accounts for the dashboard's hero figure.
 *
 * Pure, so the one number the whole window leads with is unit-tested rather
 * than assembled inline in JSX.
 *
 * Money arrives as exact decimal strings and is summed as JS numbers here.
 * That is acceptable *only* because this total is display-only — it is never
 * written back, compared for equality, or used to reconcile against the
 * broker. Anything that needs exactness reads the per-account strings, which
 * cross the boundary untouched.
 */
import type { AccountGlance } from '../../hooks/useAccountsGlance';

export interface AccountsSummary {
  /** Summed realized P&L over the window, or null if it cannot be summed. */
  realized: number | null;
  /** Summed unrealized P&L on open trades right now. */
  openPl: number | null;
  openTrades: number;
  trades: number;
  wins: number;
  losses: number;
  /** wins / decided, or null when nothing was decided. */
  winRate: number | null;
  /** The shared currency, or null when accounts disagree (see `mixedCurrency`). */
  currency: string | null;
  /**
   * True when accounts report different currencies. Their P&L is then NOT
   * summable — adding USD to JPY would produce a confident, meaningless
   * number — so `realized`/`openPl` come back null and the UI must say so
   * instead of showing a total.
   */
  mixedCurrency: boolean;
  /** Accounts whose fetch failed; their numbers are absent from the totals. */
  errored: number;
  /**
   * Healthy accounts the window simply does not measure — under
   * `--since-baseline`, an account with no baseline recorded (D3). Their
   * `realized` is `null`, which means UNMEASURED, not zero, so they are
   * absent from the totals and reported separately here.
   */
  unmeasured: number;
  /**
   * Accounts actually behind `realized` — i.e. neither errored nor
   * unmeasured. This is the number the hero's count line must quote:
   * "realized across N accounts" has to name the accounts that contributed.
   */
  measured: number;
  accounts: number;
}

const num = (v: string | null): number | null => {
  if (v === null) return null;
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
};

export const summarizeAccounts = (accounts: AccountGlance[]): AccountsSummary => {
  const healthy = accounts.filter((a) => !a.error);
  const errored = accounts.length - healthy.length;

  // D3: a healthy row with `realized: null` is UNMEASURED — the window has no
  // figure for it (no baseline recorded), which is not the same as having
  // traded flat. Folding it in as 0 would quietly widen the denominator of
  // "realized across N accounts" with an account that contributed nothing
  // knowable. Note the strict null check: an unparseable *string* is a
  // malformed measurement, still counted (as 0) rather than treated as absent.
  const measured = healthy.filter((a) => a.realized !== null);
  const unmeasured = healthy.length - measured.length;

  const currencies = new Set(healthy.map((a) => a.currency).filter((c): c is string => !!c));
  const mixedCurrency = currencies.size > 1;
  const currency = currencies.size === 1 ? [...currencies][0] : null;

  let realized = 0;
  let trades = 0;
  let wins = 0;
  let losses = 0;

  for (const a of measured) {
    realized += num(a.realized) ?? 0;
    trades += a.trades ?? 0;
    wins += a.wins ?? 0;
    losses += a.losses ?? 0;
  }

  // Open P&L is as-of-now and account-level, never window-derived — so an
  // unmeasured account still contributes it (D3: its account-level facts
  // still render). Only the window figures above are withheld.
  let openPl = 0;
  let openTrades = 0;

  for (const a of healthy) {
    openPl += num(a.unrealized_pl) ?? 0;
    openTrades += a.open_trade_count ?? 0;
  }

  const decided = wins + losses;

  return {
    realized: mixedCurrency ? null : realized,
    openPl: mixedCurrency ? null : openPl,
    openTrades,
    trades,
    wins,
    losses,
    winRate: decided > 0 ? wins / decided : null,
    currency,
    mixedCurrency,
    errored,
    unmeasured,
    measured: measured.length,
    accounts: accounts.length,
  };
};
