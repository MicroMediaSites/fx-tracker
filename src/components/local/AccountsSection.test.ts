/**
 * orderedAccounts — active accounts above idle ones.
 *
 * With a six-account ladder, four are usually flat. Sorting the ones that
 * actually traded to the top is what makes "was today profitable" answerable
 * at a glance instead of by scanning.
 */
import { describe, expect, it } from 'vitest';
import {
  baselineHint,
  initialWindow,
  isUnmeasured,
  orderedAccounts,
  pairLabel,
  parseDateInput,
  presetId,
  rangeFromInputs,
  rangeToInputs,
  sinceLabel,
  unitsLabel,
} from './AccountsSection';
import { summarizeAccounts } from './accountsSummary';
import { defaultWindow } from '../../hooks/useAccountsGlance';
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
  window_start: null,
  window_source: null,
  note: null,
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

  it('does not bury an errored account among the idle ones', () => {
    // A broken account is something to look at; ranking it idle would hide it.
    const ordered = orderedAccounts([
      account({ account: 'idle' }),
      account({ account: 'broken', error: '401 Unauthorized' }),
    ]);

    expect(ordered[0].account).toBe('broken');
  });

  it('ranks errored and active together, preserving their input order', () => {
    // Both are "not idle" and share rank 0 — errored rows are NOT promoted
    // above accounts that traded, they simply are not demoted. Pinned because
    // the name of the test above could be read as claiming more than that.
    const ordered = orderedAccounts([
      account({ account: 'active', trades: 3 }),
      account({ account: 'broken', error: '401 Unauthorized' }),
    ]);

    expect(ordered.map((a) => a.account)).toEqual(['active', 'broken']);
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

describe('pairLabel / unitsLabel', () => {
  it('writes pairs the way the OANDA dashboard does', () => {
    expect(pairLabel('USD_JPY')).toBe('USD/JPY');
  });

  it('compacts round thousands and signs the direction', () => {
    expect(unitsLabel('2000')).toBe('+2k');
    expect(unitsLabel('-2000')).toBe('−2k');
    expect(unitsLabel('-1500')).toBe('−1.5k');
    expect(unitsLabel('250')).toBe('+250');
    expect(unitsLabel('0')).toBe('');
    expect(unitsLabel('garbage')).toBe('');
  });
});

/**
 * The window picker + tile states (AGT-1132).
 *
 * These pin the pure decisions behind the section — which window it opens on,
 * which accounts the hero total is allowed to include, and which of the two
 * muted tile states a row lands in. The rendered result is covered by the
 * Playwright specs; what matters here is that the hero and the tile decide
 * "unmeasured" from the SAME predicate, so they can never disagree about
 * which accounts count.
 */
describe('initialWindow (D6, cold boot)', () => {
  it('opens on baseline when nothing is persisted', () => {
    // Not a guess at the answer: whether any account has a baseline is only
    // knowable from a since-baseline response, so this opening window is also
    // the probe `defaultWindow` then decides from.
    expect(initialWindow(null)).toEqual({ kind: 'baseline' });
  });

  it('lets a persisted choice win over the probe', () => {
    expect(initialWindow({ kind: 'days', days: 30 })).toEqual({ kind: 'days', days: 30 });
    expect(initialWindow({ kind: 'today' })).toEqual({ kind: 'today' });
  });

  it('resolves to baseline only when a row actually reports one', () => {
    expect(defaultWindow([account({}), account({})])).toEqual({ kind: 'today' });
    expect(
      defaultWindow([
        account({}),
        account({ window_source: 'baseline', window_start: '2026-08-01T00:00:00Z' }),
      ])
    ).toEqual({ kind: 'baseline' });
  });

  it('falls back to today when no account is configured at all', () => {
    expect(defaultWindow([])).toEqual({ kind: 'today' });
  });
});

describe('presetId', () => {
  it('maps each window onto the button that reads as pressed', () => {
    expect(presetId({ kind: 'baseline' })).toBe('baseline');
    expect(presetId({ kind: 'today' })).toBe('today');
    expect(presetId({ kind: 'days', days: 7 })).toBe('7d');
    expect(presetId({ kind: 'days', days: 30 })).toBe('30d');
    expect(presetId({ kind: 'range', from: '2026-08-01T00:00:00Z', to: '2026-08-25T00:00:00Z' })).toBe(
      'custom'
    );
  });
});

describe('isUnmeasured (D3) and the hero exclusion', () => {
  it('is true only for a healthy row with no window figure', () => {
    expect(isUnmeasured(account({ realized: null, trades: null, note: 'no baseline recorded' }))).toBe(
      true
    );
    // Traded flat is measured — $0.00 is a real answer.
    expect(isUnmeasured(account({ realized: '0', trades: 0 }))).toBe(false);
    // An errored row is its own tile state, not this one.
    expect(isUnmeasured(account({ realized: null, error: '401 Unauthorized' }))).toBe(false);
  });

  it('keeps an unmeasured account out of the hero total and its count', () => {
    const s = summarizeAccounts([
      account({ realized: '47.20', trades: 6, wins: 4, losses: 2 }),
      account({ realized: null, trades: null, wins: null, losses: null, note: 'no baseline recorded' }),
    ]);

    expect(s.realized).toBeCloseTo(47.2, 2);
    expect(s.trades).toBe(6);
    // "realized across N accounts" must name only the accounts behind it.
    expect(s.measured).toBe(1);
    expect(s.unmeasured).toBe(1);
    expect(s.errored).toBe(0);
  });

  it('reports nothing to total when every account is unmeasured', () => {
    // The section renders "—" off this: a $0.00 hero across zero contributing
    // accounts would read as a flat day rather than as nothing to add up.
    const s = summarizeAccounts([account({ realized: null }), account({ realized: null })]);

    expect(s.measured).toBe(0);
    expect(s.unmeasured).toBe(2);
  });

  it("still counts an unmeasured account's open positions, which are as-of-now", () => {
    // D3: only the WINDOW-derived numbers are absent; the account-level facts
    // still render, and open P&L was never window-derived.
    const s = summarizeAccounts([
      account({ realized: '10', trades: 1, wins: 1 }),
      account({ realized: null, unrealized_pl: '12.40', open_trade_count: 1 }),
    ]);

    expect(s.realized).toBeCloseTo(10, 2);
    expect(s.openPl).toBeCloseTo(12.4, 2);
    expect(s.openTrades).toBe(1);
  });
});

describe('tile footer and hint', () => {
  it("writes a tile's own window start as 'since <Mon D>'", () => {
    expect(sinceLabel(new Date(2026, 7, 25, 9, 30).toISOString())).toBe('since Aug 25');
  });

  it('omits the footer rather than guessing when there is no start', () => {
    expect(sinceLabel(null)).toBeNull();
    expect(sinceLabel(undefined)).toBeNull();
    expect(sinceLabel('not-an-instant')).toBeNull();
  });

  it('names the account in the one-line fix a no-baseline tile shows', () => {
    expect(baselineHint('tf-m1')).toBe('wickd trade baseline set --account tf-m1');
  });
});

describe('custom range inputs (D4)', () => {
  it('reads a date input as LOCAL midnight, not UTC', () => {
    // `new Date('2026-08-01')` is specified to parse as UTC, which lands on
    // July 31 for everyone west of Greenwich — the off-by-one D4 exists to
    // avoid.
    const d = parseDateInput('2026-08-01');
    expect(d?.getFullYear()).toBe(2026);
    expect(d?.getMonth()).toBe(7);
    expect(d?.getDate()).toBe(1);
    expect(d?.getHours()).toBe(0);
  });

  it('rejects anything that is not a real calendar date', () => {
    expect(parseDateInput('')).toBeNull();
    expect(parseDateInput('2026-8-1')).toBeNull();
    expect(parseDateInput('not a date')).toBeNull();
    // `new Date` would happily roll this over to March 3.
    expect(parseDateInput('2026-02-31')).toBeNull();
  });

  it('round-trips a range back to the two dates that produced it', () => {
    const w = rangeFromInputs('2026-08-01', '2026-08-24');
    expect(w).not.toBeNull();
    expect(rangeToInputs(w!)).toEqual({ start: '2026-08-01', end: '2026-08-24' });
  });

  it('keeps the end date inclusive for the human', () => {
    const w = rangeFromInputs('2026-08-24', '2026-08-24') as {
      kind: 'range';
      from: string;
      to: string;
    };
    const to = new Date(w.to);
    expect(new Date(w.from).getDate()).toBe(24);
    expect(to.getDate()).toBe(25);
    expect(to.getHours()).toBe(0);
  });

  it('refuses a backwards or unparseable range instead of inverting it', () => {
    expect(rangeFromInputs('2026-08-24', '2026-08-01')).toBeNull();
    expect(rangeFromInputs('', '2026-08-01')).toBeNull();
    expect(rangeFromInputs('2026-08-01', 'garbage')).toBeNull();
  });

  it('has no date inputs to offer for a non-range window', () => {
    expect(rangeToInputs({ kind: 'today' })).toBeNull();
    expect(rangeToInputs({ kind: 'baseline' })).toBeNull();
  });
});
