/**
 * useAccountHistory — the D8 window→invoke-args mapping (AGT-1133).
 *
 * `account_history` only takes `since`/`to` (no `days`, no `since_baseline`),
 * so the section's `GlanceWindow` has to be translated on this side. These
 * tests pin that mapping directly (`windowToHistoryArgs`) and via the hook's
 * actual `invoke` call, for all four window kinds.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const { useAccountHistory, windowToHistoryArgs, daysAgoIso } = await import('./useAccountHistory');
const { localMidnightIso } = await import('./useAccountsGlance');
type GlanceWindow = import('./useAccountsGlance').GlanceWindow;
type AccountHistory = import('./useAccountHistory').AccountHistory;

const HISTORY: AccountHistory = {
  account: 'tf-m1',
  account_id: '101-001-26151603-002',
  environment: 'practice',
  baseline: null,
  since: null,
  count: 0,
  realized: '0',
  blended_exits: 0,
  truncated: false,
  trades: [],
};

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(HISTORY);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('daysAgoIso', () => {
  it('subtracts exact wall-clock days from `now`, matching the CLI\'s own arithmetic', () => {
    const now = new Date('2026-08-24T12:00:00.000Z');
    expect(daysAgoIso(7, now)).toBe('2026-08-17T12:00:00.000Z');
    expect(daysAgoIso(30, now)).toBe('2026-07-25T12:00:00.000Z');
  });

  it('defaults `now` to the current time', () => {
    const before = Date.now();
    const iso = daysAgoIso(1);
    const after = Date.now();
    const got = Date.parse(iso) + 24 * 60 * 60 * 1000;
    expect(got).toBeGreaterThanOrEqual(before);
    expect(got).toBeLessThanOrEqual(after);
  });
});

describe('windowToHistoryArgs (D8)', () => {
  const now = new Date('2026-08-24T12:00:00.000Z');

  it('baseline → neither since nor to', () => {
    expect(windowToHistoryArgs({ kind: 'baseline' }, now)).toEqual({ since: null, to: null });
  });

  it('range → since/to both set to the range instants exactly', () => {
    const from = '2026-08-01T00:00:00.000Z';
    const to = '2026-08-25T00:00:00.000Z';
    expect(windowToHistoryArgs({ kind: 'range', from, to }, now)).toEqual({ since: from, to });
  });

  it('today → since is local midnight, no to', () => {
    // Compared against localMidnightIso itself, not a hardcoded instant — the
    // viewer's local timezone (not necessarily UTC) decides where midnight
    // falls, same reasoning as useAccountsGlance's own `today` test.
    expect(windowToHistoryArgs({ kind: 'today' }, now)).toEqual({
      since: localMidnightIso(now),
      to: null,
    });
  });

  it('days → since is N days back from now, no to', () => {
    expect(windowToHistoryArgs({ kind: 'days', days: 7 }, now)).toEqual({
      since: daysAgoIso(7, now),
      to: null,
    });
  });
});

describe('useAccountHistory — invoke args per window kind (AC1/AC3)', () => {
  const args = () => invokeMock.mock.calls[0][1] as Record<string, unknown>;

  it('always invokes account_history with the account name', async () => {
    renderHook(() => useAccountHistory('tf-m1', { kind: 'baseline' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(invokeMock.mock.calls[0][0]).toBe('account_history');
    expect(args().account).toBe('tf-m1');
  });

  it('baseline: neither since nor to (history already defaults to since-baseline)', async () => {
    renderHook(() => useAccountHistory('tf-m1', { kind: 'baseline' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(args()).toMatchObject({ since: null, to: null });
  });

  it('range: both since and to, verbatim', async () => {
    const from = '2026-08-01T00:00:00.000Z';
    const to = '2026-08-25T00:00:00.000Z';
    renderHook(() => useAccountHistory('tf-m1', { kind: 'range', from, to }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(args()).toMatchObject({ since: from, to });
  });

  it('today: since only, no to', async () => {
    renderHook(() => useAccountHistory('tf-m1', { kind: 'today' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    const a = args();
    expect(a.to).toBeNull();
    expect(typeof a.since).toBe('string');
    expect(Number.isNaN(Date.parse(a.since as string))).toBe(false);
  });

  it('days: since only, no to', async () => {
    renderHook(() => useAccountHistory('tf-m1', { kind: 'days', days: 30 }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    const a = args();
    expect(a.to).toBeNull();
    expect(typeof a.since).toBe('string');
    expect(Number.isNaN(Date.parse(a.since as string))).toBe(false);
  });

  it('issues no fetch when account is null', () => {
    renderHook(() => useAccountHistory(null, { kind: 'baseline' }));
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('refetches when the window kind changes while the same account is open', async () => {
    const { rerender } = renderHook(
      ({ w }: { w: GlanceWindow }) => useAccountHistory('tf-m1', w),
      { initialProps: { w: { kind: 'baseline' } as GlanceWindow } }
    );
    await waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(1));
    expect(args()).toMatchObject({ since: null, to: null });

    rerender({ w: { kind: 'days', days: 7 } });
    await waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(2));
    const secondArgs = invokeMock.mock.calls[1][1] as Record<string, unknown>;
    expect(secondArgs.since).not.toBeNull();
  });
});
