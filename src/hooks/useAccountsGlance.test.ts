/**
 * useAccountsGlance — the window model, its pure helpers, and the argv it
 * hands to `accounts_glance`.
 *
 * The hook itself just plumbs `GlanceWindow` into invoke args and polls; the
 * helpers here (label, storage migration, the D6 default) are what the
 * picker UI (AGT-1132) and drill-down (AGT-1133) will build against, so they
 * are pinned independently of any component.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const {
  useAccountsGlance,
  localMidnightIso,
  localDateRangeToInstants,
  windowLabel,
  isGlanceWindow,
  serializeWindow,
  parseStoredWindow,
  readStoredWindow,
  persistWindow,
  hasAnyBaseline,
  defaultWindow,
  WINDOW_STORAGE_KEY,
} = await import('./useAccountsGlance');
type AccountGlance = import('./useAccountsGlance').AccountGlance;
type AccountsGlance = import('./useAccountsGlance').AccountsGlance;
type GlanceWindow = import('./useAccountsGlance').GlanceWindow;

const GLANCE: AccountsGlance = {
  environment: 'practice',
  days: 7,
  since: null,
  to: null,
  generated_at: '2026-08-24T12:00:00Z',
  accounts: [],
};

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

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(GLANCE);
});

describe('localMidnightIso', () => {
  it('returns the start of the local day, not 24 hours ago', () => {
    const now = new Date(2026, 6, 20, 15, 30, 0); // 20 Jul 2026, 15:30 local
    const midnight = new Date(localMidnightIso(now));

    expect(midnight.getFullYear()).toBe(2026);
    expect(midnight.getMonth()).toBe(6);
    expect(midnight.getDate()).toBe(20);
    expect(midnight.getHours()).toBe(0);
    expect(midnight.getMinutes()).toBe(0);
    expect(midnight.getSeconds()).toBe(0);
    expect(midnight.getMilliseconds()).toBe(0);
  });

  it('is a shorter window than 24h when the day is young', () => {
    // The distinction that motivates the whole `--since` path: at 00:30, the
    // last 24 hours is mostly *yesterday*, which is not what "today" means.
    const now = new Date(2026, 6, 20, 0, 30, 0);
    const midnight = new Date(localMidnightIso(now)).getTime();
    const dayAgo = now.getTime() - 24 * 60 * 60 * 1000;

    expect(midnight).toBeGreaterThan(dayAgo);
    expect(now.getTime() - midnight).toBe(30 * 60 * 1000);
  });

  it('does not mutate the date it is given', () => {
    // It sets hours on a Date; doing that in place would corrupt a caller's
    // clock value.
    const now = new Date(2026, 6, 20, 15, 30, 0);
    const before = now.getTime();
    localMidnightIso(now);

    expect(now.getTime()).toBe(before);
  });

  it('emits a parseable RFC3339 instant', () => {
    const iso = localMidnightIso(new Date(2026, 6, 20, 15, 30, 0));

    expect(Number.isNaN(Date.parse(iso))).toBe(false);
    expect(iso).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/);
  });
});

describe('localDateRangeToInstants', () => {
  it('starts at local midnight of the start date', () => {
    const { from } = localDateRangeToInstants(
      new Date(2026, 7, 1, 9, 0, 0),
      new Date(2026, 7, 24, 9, 0, 0)
    );
    const start = new Date(from);
    expect(start.getDate()).toBe(1);
    expect(start.getHours()).toBe(0);
  });

  it('closes at local midnight of the day AFTER the end date (inclusive-end rule)', () => {
    // The end date must be inclusive for the human but exclusive for the
    // machine — this is the whole point of the helper (D4).
    const { to } = localDateRangeToInstants(
      new Date(2026, 7, 1, 9, 0, 0),
      new Date(2026, 7, 24, 23, 59, 0)
    );
    const end = new Date(to);
    expect(end.getDate()).toBe(25);
    expect(end.getHours()).toBe(0);
    expect(end.getMinutes()).toBe(0);
  });

  it('produces a single-day range when start and end are the same date', () => {
    const { from, to } = localDateRangeToInstants(
      new Date(2026, 7, 1, 3, 0, 0),
      new Date(2026, 7, 1, 22, 0, 0)
    );
    expect(new Date(to).getTime() - new Date(from).getTime()).toBe(24 * 60 * 60 * 1000);
  });

  it('does not mutate its arguments', () => {
    const start = new Date(2026, 7, 1, 9, 0, 0);
    const end = new Date(2026, 7, 24, 9, 0, 0);
    const startBefore = start.getTime();
    const endBefore = end.getTime();
    localDateRangeToInstants(start, end);
    expect(start.getTime()).toBe(startBefore);
    expect(end.getTime()).toBe(endBefore);
  });
});

describe('windowLabel', () => {
  it('labels every preset (D7)', () => {
    expect(windowLabel({ kind: 'baseline' })).toBe('Since baseline');
    expect(windowLabel({ kind: 'today' })).toBe('Today');
    expect(windowLabel({ kind: 'days', days: 7 })).toBe('Last 7d');
    expect(windowLabel({ kind: 'days', days: 30 })).toBe('Last 30d');
  });

  it('labels a custom range as a local date span, end-inclusive', () => {
    const { from, to } = localDateRangeToInstants(
      new Date(2026, 7, 1, 0, 0, 0),
      new Date(2026, 7, 24, 0, 0, 0)
    );
    expect(windowLabel({ kind: 'range', from, to })).toBe('Aug 1 – Aug 24');
  });
});

describe('isGlanceWindow', () => {
  it('accepts every valid variant', () => {
    expect(isGlanceWindow({ kind: 'today' })).toBe(true);
    expect(isGlanceWindow({ kind: 'baseline' })).toBe(true);
    expect(isGlanceWindow({ kind: 'days', days: 7 })).toBe(true);
    expect(
      isGlanceWindow({ kind: 'range', from: '2026-08-01T00:00:00Z', to: '2026-08-25T00:00:00Z' })
    ).toBe(true);
  });

  it('rejects garbage shapes', () => {
    expect(isGlanceWindow(null)).toBe(false);
    expect(isGlanceWindow(42)).toBe(false);
    expect(isGlanceWindow('today')).toBe(false);
    expect(isGlanceWindow({ kind: 'nonsense' })).toBe(false);
    expect(isGlanceWindow({ kind: 'days', days: 'seven' })).toBe(false);
    expect(isGlanceWindow({ kind: 'days', days: -1 })).toBe(false);
    expect(isGlanceWindow({ kind: 'range', from: 'not-a-date', to: '2026-08-25T00:00:00Z' })).toBe(
      false
    );
    expect(isGlanceWindow({ kind: 'range', from: '2026-08-01T00:00:00Z' })).toBe(false);
  });
});

describe('serializeWindow / parseStoredWindow (D7 persistence + migration)', () => {
  it('round-trips every current variant', () => {
    const windows: GlanceWindow[] = [
      { kind: 'today' },
      { kind: 'baseline' },
      { kind: 'days', days: 30 },
      { kind: 'range', from: '2026-08-01T00:00:00Z', to: '2026-08-25T00:00:00Z' },
    ];
    for (const w of windows) {
      expect(parseStoredWindow(serializeWindow(w))).toEqual(w);
    }
  });

  it('migrates each legacy string id', () => {
    expect(parseStoredWindow('today')).toEqual({ kind: 'today' });
    expect(parseStoredWindow('7d')).toEqual({ kind: 'days', days: 7 });
    expect(parseStoredWindow('30d')).toEqual({ kind: 'days', days: 30 });
  });

  it('reads null when nothing is stored', () => {
    expect(parseStoredWindow(null)).toBeNull();
  });

  it('falls back cleanly on unparseable/garbage values', () => {
    expect(parseStoredWindow('not json at all')).toBeNull();
    expect(parseStoredWindow('{"kind":"nonsense"}')).toBeNull();
    expect(parseStoredWindow('{"kind":"days"}')).toBeNull();
    expect(parseStoredWindow('null')).toBeNull();
    expect(parseStoredWindow('42')).toBeNull();
  });
});

describe('readStoredWindow / persistWindow', () => {
  afterEach(() => {
    localStorage.clear();
  });

  it('persists under the existing wickd_accounts_window key and reads it back', () => {
    persistWindow({ kind: 'baseline' });
    expect(localStorage.getItem(WINDOW_STORAGE_KEY)).toBe(serializeWindow({ kind: 'baseline' }));
    expect(readStoredWindow()).toEqual({ kind: 'baseline' });
  });

  it('migrates a legacy value already sitting under the key', () => {
    localStorage.setItem(WINDOW_STORAGE_KEY, '30d');
    expect(readStoredWindow()).toEqual({ kind: 'days', days: 30 });
  });

  it('reads a thrown localStorage access as null rather than throwing', () => {
    const spy = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('storage disabled');
    });
    expect(readStoredWindow()).toBeNull();
    spy.mockRestore();
  });

  it('swallows a thrown localStorage write', () => {
    const spy = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('quota exceeded');
    });
    expect(() => persistWindow({ kind: 'today' })).not.toThrow();
    spy.mockRestore();
  });
});

describe('hasAnyBaseline / defaultWindow (D6)', () => {
  it('defaults to baseline when any account has one recorded', () => {
    const accounts = [
      account({ account: 'a', window_source: 'days' }),
      account({ account: 'b', window_source: 'baseline', window_start: '2026-08-01T00:00:00Z' }),
    ];
    expect(hasAnyBaseline(accounts)).toBe(true);
    expect(defaultWindow(accounts)).toEqual({ kind: 'baseline' });
  });

  it('defaults to today when no account has a baseline', () => {
    const accounts = [
      account({ account: 'a', window_source: 'days' }),
      account({ account: 'b', window_source: null }),
    ];
    expect(hasAnyBaseline(accounts)).toBe(false);
    expect(defaultWindow(accounts)).toEqual({ kind: 'today' });
  });

  it('defaults to today for an empty account list (unchanged behaviour pre-baselines)', () => {
    expect(hasAnyBaseline([])).toBe(false);
    expect(defaultWindow([])).toEqual({ kind: 'today' });
  });
});

describe('useAccountsGlance — variant to invoke args (AC1)', () => {
  it('maps `today` to a computed local-midnight `since`', async () => {
    renderHook(() => useAccountsGlance({ kind: 'today' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    const args = invokeMock.mock.calls[0][1] as Record<string, unknown>;
    expect(args.days).toBeNull();
    expect(args.sinceBaseline).toBeNull();
    expect(args.to).toBeNull();
    expect(typeof args.since).toBe('string');
    expect(Number.isNaN(Date.parse(args.since as string))).toBe(false);
  });

  it('maps `days` to the days param, no since/to/baseline', async () => {
    renderHook(() => useAccountsGlance({ kind: 'days', days: 30 }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    const args = invokeMock.mock.calls[0][1] as Record<string, unknown>;
    expect(args).toMatchObject({ days: 30, since: null, to: null, sinceBaseline: null });
  });

  it('maps `baseline` to sinceBaseline: true, with days/since/to null', async () => {
    renderHook(() => useAccountsGlance({ kind: 'baseline' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    const args = invokeMock.mock.calls[0][1] as Record<string, unknown>;
    expect(args).toMatchObject({ days: null, since: null, to: null, sinceBaseline: true });
  });

  it('maps `range` to its from/to instants verbatim, no days/baseline', async () => {
    const from = '2026-08-01T00:00:00.000Z';
    const to = '2026-08-25T00:00:00.000Z';
    renderHook(() => useAccountsGlance({ kind: 'range', from, to }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    const args = invokeMock.mock.calls[0][1] as Record<string, unknown>;
    expect(args).toMatchObject({ days: null, since: from, to, sinceBaseline: null });
  });

  it('always passes the command name accounts_glance', async () => {
    renderHook(() => useAccountsGlance({ kind: 'today' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(invokeMock.mock.calls[0][0]).toBe('accounts_glance');
  });
});
