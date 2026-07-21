/**
 * summarizeAccounts — the aggregate behind the dashboard's hero figure.
 *
 * This is the single number the whole window leads with, so its edges matter:
 * mixed currencies must refuse to total, errored accounts must not be counted
 * as zeros, and a null win rate must stay null.
 */
import { describe, expect, it } from 'vitest';
import { summarizeAccounts } from './accountsSummary';
import type { AccountGlance } from '../../hooks/useAccountsGlance';

const account = (over: Partial<AccountGlance>): AccountGlance => ({
  account: 'x',
  names: ['x'],
  account_id: 'id-x',
  currency: 'USD',
  nav: '100000',
  balance: '100000',
  unrealized_pl: '0',
  open_trade_count: 0,
  realized: '0',
  trades: 0,
  wins: 0,
  losses: 0,
  win_rate: null,
  error: null,
  ...over,
});

describe('summarizeAccounts', () => {
  it('totals realized P&L and trade counts across accounts', () => {
    const s = summarizeAccounts([
      account({ realized: '47.20', trades: 6, wins: 4, losses: 2 }),
      account({ realized: '-15.70', trades: 47, wins: 5, losses: 42 }),
      account({ realized: '-0.79', trades: 6, wins: 3, losses: 3 }),
    ]);

    expect(s.realized).toBeCloseTo(30.71, 2);
    expect(s.trades).toBe(59);
    expect(s.wins).toBe(12);
    expect(s.losses).toBe(47);
    expect(s.winRate).toBeCloseTo(12 / 59, 4);
    expect(s.currency).toBe('USD');
    expect(s.mixedCurrency).toBe(false);
  });

  it('refuses to total across mixed currencies', () => {
    // Adding USD to JPY would produce a confident, meaningless number.
    const s = summarizeAccounts([
      account({ currency: 'USD', realized: '100' }),
      account({ currency: 'JPY', realized: '5000' }),
    ]);

    expect(s.mixedCurrency).toBe(true);
    expect(s.realized).toBeNull();
    expect(s.openPl).toBeNull();
    expect(s.currency).toBeNull();
    // Non-monetary counts are still valid across currencies.
    expect(s.accounts).toBe(2);
  });

  it('excludes errored accounts from totals and reports them separately', () => {
    // An unreachable account is unknown, not zero — folding it in as 0 would
    // present a partial total as complete.
    const s = summarizeAccounts([
      account({ realized: '10', trades: 2, wins: 2 }),
      account({ error: '401 Unauthorized', realized: null, trades: null }),
    ]);

    expect(s.realized).toBeCloseTo(10, 2);
    expect(s.trades).toBe(2);
    expect(s.errored).toBe(1);
    expect(s.accounts).toBe(2);
  });

  it('reports a null win rate when nothing was decided', () => {
    const s = summarizeAccounts([account({}), account({})]);

    expect(s.winRate).toBeNull();
    expect(s.realized).toBe(0);
  });

  it('sums open positions and their P&L', () => {
    const s = summarizeAccounts([
      account({ unrealized_pl: '12.40', open_trade_count: 1 }),
      account({ unrealized_pl: '-1.28', open_trade_count: 3 }),
    ]);

    expect(s.openPl).toBeCloseTo(11.12, 2);
    expect(s.openTrades).toBe(4);
  });

  it('treats an unparseable amount as zero rather than NaN-poisoning the total', () => {
    // One malformed row must not turn the hero figure into "—".
    const s = summarizeAccounts([
      account({ realized: '25' }),
      account({ realized: 'not-a-number' }),
    ]);

    expect(s.realized).toBeCloseTo(25, 2);
  });

  it('handles an empty account list', () => {
    const s = summarizeAccounts([]);

    expect(s.realized).toBe(0);
    expect(s.accounts).toBe(0);
    expect(s.winRate).toBeNull();
    expect(s.mixedCurrency).toBe(false);
  });
});
