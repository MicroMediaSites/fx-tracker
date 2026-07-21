/**
 * orderedAccounts — active accounts above idle ones.
 *
 * With a six-account ladder, four are usually flat. Sorting the ones that
 * actually traded to the top is what makes "was today profitable" answerable
 * at a glance instead of by scanning.
 */
import { describe, expect, it } from 'vitest';
import { orderedAccounts } from './AccountsSection';
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

describe('orderedAccounts', () => {
  it('puts accounts that traded above idle ones', () => {
    const ordered = orderedAccounts([
      account({ account: 'tf-h1' }),
      account({ account: 'tf-m1', trades: 47, realized: '-15.70' }),
      account({ account: 'tf-m15' }),
      account({ account: 'tf-m5', trades: 6, realized: '-0.79' }),
    ]);

    expect(ordered.map((a) => a.account)).toEqual(['tf-m1', 'tf-m5', 'tf-h1', 'tf-m15']);
  });

  it('treats an open position as active even with no closed trades', () => {
    // Nothing closed yet today, but money is at risk right now — that is not
    // an idle account.
    const ordered = orderedAccounts([
      account({ account: 'idle' }),
      account({ account: 'holding', open_trade_count: 1, unrealized_pl: '12.40' }),
    ]);

    expect(ordered[0].account).toBe('holding');
  });

  it('keeps errored accounts at the top, not buried with the idle ones', () => {
    // A broken account is something to look at; ranking it idle would hide it.
    const ordered = orderedAccounts([
      account({ account: 'idle' }),
      account({ account: 'broken', error: '401 Unauthorized' }),
    ]);

    expect(ordered[0].account).toBe('broken');
  });

  it('preserves the relative order within each group', () => {
    const ordered = orderedAccounts([
      account({ account: 'a-idle' }),
      account({ account: 'b-active', trades: 2 }),
      account({ account: 'c-idle' }),
      account({ account: 'd-active', trades: 5 }),
    ]);

    expect(ordered.map((a) => a.account)).toEqual(['b-active', 'd-active', 'a-idle', 'c-idle']);
  });

  it('does not mutate the array it is given', () => {
    const input = [account({ account: 'idle' }), account({ account: 'active', trades: 1 })];
    orderedAccounts(input);

    expect(input.map((a) => a.account)).toEqual(['idle', 'active']);
  });
});
