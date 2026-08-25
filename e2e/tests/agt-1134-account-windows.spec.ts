/**
 * AGT-1134 — since-baseline and custom date-range windows for the accounts
 * dashboard (`wickd-account-windows`, story S7 of 8), against mocked IPC.
 *
 * Covers, from `projects/wickd-account-windows/README.md`:
 *  - D3: an account with no baseline is a distinct, honest "no baseline"
 *    state — never `$0.00` — and is excluded from the hero total.
 *  - D4: a custom range is a closed window (`since` inclusive, `to`
 *    exclusive) — the end date picked is inclusive for the human, so `to`
 *    is local midnight of the day *after* the end date.
 *  - D6: cold boot opens on `baseline`; a mocked response with no
 *    `window_source: 'baseline'` row makes the section fall back to `today`.
 *  - D7: preset labels and persistence of the selected window.
 *  - D8: the trade-history drill-down repeats the section's window label and
 *    honours that window in its own `account_history` call.
 *
 * `accounts_glance` and `account_history` stay on the mocked-IPC path — the
 * offline-boot specs' zero-non-localhost-request contract is untouched.
 *
 * `mockTauriCommand` can't assert on invoke *args* — its override is carried
 * through `addInitScript`'s serialized arg channel, which can hold data but
 * not a function (see the note in local-mode-offline-boot.spec.ts). The
 * `mock*Recording` helpers below install the override function directly in
 * page scope instead, and record every call's args on `window` for the test
 * to read back.
 */
import { test, expect } from '../helpers/app-fixture';
import type { Page } from '@playwright/test';

/** Matches `AccountsSection.tsx`'s own `dayMonth` formatter, so the expected
 *  tile-footer / hero-label text is derived the same way the app derives it
 *  rather than hardcoded against one timezone's rendering. */
const dayMonth = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' });
const sinceLabelFor = (iso: string) => `since ${dayMonth.format(new Date(iso))}`;

/** Matches `useAccountsGlance.ts`'s `localMidnightIso` — local midnight of
 *  the given date, as an RFC3339 instant. */
const localMidnight = (d: Date): string => {
  const midnight = new Date(d);
  midnight.setHours(0, 0, 0, 0);
  return midnight.toISOString();
};

/**
 * Persist a window selection before boot so the D6 baseline probe never
 * fires — the section reads it via `readStoredWindow` and opens directly on
 * this window, giving the test a single deterministic `accounts_glance` call
 * instead of the cold-boot probe-then-fallback pair.
 */
const seedWindow = async (page: Page, glanceWindow: unknown) => {
  await page.addInitScript((serialized) => {
    localStorage.setItem('wickd_accounts_window', serialized);
  }, JSON.stringify(glanceWindow));
};

/**
 * Install a recording `accounts_glance` override: every call's args land in
 * `window.__E2E_GLANCE_CALLS__`, and the response is `baseline` when the call
 * carried `sinceBaseline: true`, `today` otherwise. That single branch is
 * enough to drive both the D6 cold-boot probe (whichever fires first) and an
 * explicit "since baseline" selection through one mock.
 */
const mockGlanceRecording = async (
  page: Page,
  responses: { today: unknown; baseline: unknown },
) => {
  await page.addInitScript((responses) => {
    const w = window as Window & {
      __E2E_TAURI_OVERRIDES__?: Record<string, unknown>;
      __E2E_GLANCE_CALLS__?: Array<Record<string, unknown>>;
    };
    w.__E2E_TAURI_OVERRIDES__ = w.__E2E_TAURI_OVERRIDES__ || {};
    w.__E2E_GLANCE_CALLS__ = [];
    w.__E2E_TAURI_OVERRIDES__['accounts_glance'] = (args: Record<string, unknown>) => {
      w.__E2E_GLANCE_CALLS__!.push(args);
      return args && args.sinceBaseline ? responses.baseline : responses.today;
    };
  }, responses);
};

const glanceCalls = (page: Page): Promise<Array<Record<string, unknown>>> =>
  page.evaluate(
    () => (window as Window & { __E2E_GLANCE_CALLS__?: unknown[] }).__E2E_GLANCE_CALLS__ ?? [],
  ) as Promise<Array<Record<string, unknown>>>;

/** Same recording approach as `mockGlanceRecording`, for `account_history`. */
const mockHistoryRecording = async (page: Page, response: unknown) => {
  await page.addInitScript((response) => {
    const w = window as Window & {
      __E2E_TAURI_OVERRIDES__?: Record<string, unknown>;
      __E2E_HISTORY_CALLS__?: Array<Record<string, unknown>>;
    };
    w.__E2E_TAURI_OVERRIDES__ = w.__E2E_TAURI_OVERRIDES__ || {};
    w.__E2E_HISTORY_CALLS__ = [];
    w.__E2E_TAURI_OVERRIDES__['account_history'] = (args: Record<string, unknown>) => {
      w.__E2E_HISTORY_CALLS__!.push(args);
      return response;
    };
  }, response);
};

const historyCalls = (page: Page): Promise<Array<Record<string, unknown>>> =>
  page.evaluate(
    () => (window as Window & { __E2E_HISTORY_CALLS__?: unknown[] }).__E2E_HISTORY_CALLS__ ?? [],
  ) as Promise<Array<Record<string, unknown>>>;

/**
 * `--since-baseline` rows (D2): `tf-m1` has a recorded baseline, `h004` does
 * not — the D3 "no baseline" case. `window_source: 'baseline'` on at least
 * one row is also the D6 signal a real cold boot would probe for.
 */
const BASELINE_GLANCE = {
  environment: 'practice',
  days: null,
  since: null,
  to: '2026-08-25T20:00:00Z',
  generated_at: '2026-08-25T20:00:00Z',
  accounts: [
    {
      account: 'tf-m1',
      names: ['tf-m1'],
      account_id: '101-001-00000000-002',
      currency: 'USD',
      nav: '99976.89',
      balance: '99976.89',
      unrealized_pl: '0',
      open_trade_count: 0,
      realized: '47.20',
      trades: 6,
      wins: 4,
      losses: 2,
      win_rate: 0.667,
      window_start: '2026-08-25T00:36:00Z',
      window_source: 'baseline',
      note: null,
      error: null,
    },
    {
      account: 'h004',
      names: ['h004'],
      account_id: '101-001-00000000-001',
      currency: 'USD',
      nav: null,
      balance: null,
      unrealized_pl: null,
      open_trade_count: null,
      realized: null,
      trades: null,
      wins: null,
      losses: null,
      win_rate: null,
      window_start: null,
      window_source: 'baseline',
      note: 'no baseline recorded',
      error: null,
    },
  ],
};

/** An ordinary non-baseline glance response — no row carries
 *  `window_source: 'baseline'`, so D6's probe falls back to `today`. */
const TODAY_GLANCE = {
  environment: 'practice',
  days: null,
  since: '2026-08-25T06:00:00Z',
  to: '2026-08-25T20:00:00Z',
  generated_at: '2026-08-25T20:00:00Z',
  accounts: [
    {
      account: 'tf-m1',
      names: ['tf-m1'],
      account_id: '101-001-00000000-002',
      currency: 'USD',
      nav: '10012.00',
      balance: '10012.00',
      unrealized_pl: '0',
      open_trade_count: 0,
      realized: '12.00',
      trades: 2,
      wins: 1,
      losses: 1,
      win_rate: 0.5,
      window_start: '2026-08-25T06:00:00Z',
      window_source: 'since',
      note: null,
      error: null,
    },
  ],
};

const HISTORY_BASELINE = {
  account: 'tf-m1',
  account_id: '101-001-00000000-002',
  environment: 'practice',
  baseline: { balance: '100000', date: '2026-08-25T00:36:00Z' },
  since: '2026-08-25T00:36:00Z',
  count: 1,
  realized: '47.20',
  blended_exits: 0,
  decomposed_exits: 0,
  decompose_error: null,
  truncated: false,
  trades: [
    {
      id: 'trade-1',
      instrument: 'EUR_USD',
      side: 'long',
      units: '2000',
      strategy: 'rahagod',
      entry: { time: '2026-08-25T09:00:00Z', price: '1.14000' },
      exit: { time: '2026-08-25T09:12:00Z', price: '1.14024', count: 1, blended: false },
      realized_pl: '47.20',
      duration_secs: 720,
    },
  ],
};

test.describe('Account windows — baseline preset & custom range (AGT-1134)', () => {
  test('selecting since baseline sends sinceBaseline: true and shows the hero label', async ({
    appPage,
  }) => {
    // Deterministic single-fetch boot: skip the D6 probe by opening on `today`.
    await seedWindow(appPage.page, { kind: 'today' });
    await mockGlanceRecording(appPage.page, { today: TODAY_GLANCE, baseline: BASELINE_GLANCE });
    await appPage.goto('local');

    await expect(appPage.page.getByTestId('accounts-window-label')).toHaveText('Today');

    await appPage.page.getByTestId('accounts-window-baseline').click();

    await expect(appPage.page.getByTestId('accounts-window-label')).toHaveText('Since baseline');
    await expect(appPage.page.getByTestId('accounts-window-baseline')).toHaveAttribute(
      'aria-pressed',
      'true',
    );

    const calls = await glanceCalls(appPage.page);
    // D2: `since_baseline` is mutually exclusive with `since`/`days` — the
    // wire arg is camelCased `sinceBaseline` (AGT-1131's Tauri-camelCasing
    // gotcha), and no `since`/`days` accompanies it.
    expect(calls.at(-1)).toMatchObject({ sinceBaseline: true, since: null, days: null });
  });

  test('an account with no baseline renders "no baseline", never $0.00, and is excluded from the hero (D3)', async ({
    appPage,
  }) => {
    await seedWindow(appPage.page, { kind: 'baseline' });
    await mockGlanceRecording(appPage.page, { today: TODAY_GLANCE, baseline: BASELINE_GLANCE });
    await appPage.goto('local');

    // Only tf-m1 (the measured account) counts toward the hero.
    await expect(appPage.page.getByTestId('accounts-hero')).toHaveText('+$47.20');
    await expect(appPage.page.getByTestId('accounts-summary-line')).toContainText(
      'realized across 1 account',
    );
    await expect(appPage.page.getByTestId('accounts-unmeasured-summary')).toContainText(
      '1 no baseline',
    );

    const measured = appPage.page.getByTestId('account-tile').filter({ hasText: 'tf-m1' });
    expect(await measured.getAttribute('data-unmeasured')).toBeNull();
    // The per-tile "since <date>" footer renders from this row's window_start.
    await expect(measured.getByTestId('account-since')).toHaveText(
      sinceLabelFor('2026-08-25T00:36:00Z'),
    );

    const unmeasured = appPage.page.getByTestId('account-tile').filter({ hasText: 'h004' });
    await expect(unmeasured).toHaveAttribute('data-unmeasured', 'true');
    await expect(unmeasured.getByTestId('account-no-baseline')).toHaveText('no baseline');
    await expect(unmeasured).toContainText('wickd trade baseline set --account h004');
    // The load-bearing honesty check: an unmeasured account never reads as a
    // flat $0.00 day, and it carries no window-start footer of its own.
    await expect(unmeasured).not.toContainText('$0.00');
    await expect(unmeasured.getByTestId('account-since')).toHaveCount(0);
  });

  test('a custom range sends a closed window with the end date inclusive for the human (D4)', async ({
    appPage,
  }) => {
    await seedWindow(appPage.page, { kind: 'today' });
    await mockGlanceRecording(appPage.page, { today: TODAY_GLANCE, baseline: BASELINE_GLANCE });
    await appPage.goto('local');

    await appPage.page.getByTestId('accounts-window-custom').click();
    await expect(appPage.page.getByTestId('accounts-range')).toBeVisible();

    await appPage.page.getByTestId('accounts-range-start').fill('2026-08-01');
    await appPage.page.getByTestId('accounts-range-end').fill('2026-08-05');
    await appPage.page.getByTestId('accounts-range-apply').click();

    // Aug 1 – Aug 5 is what a human reads off the two dates picked, even
    // though the machine's `to` lands on the morning of Aug 6.
    await expect(appPage.page.getByTestId('accounts-window-label')).toHaveText('Aug 1 – Aug 5');
    await expect(appPage.page.getByTestId('accounts-window-custom')).toHaveAttribute(
      'aria-pressed',
      'true',
    );

    const expectedFrom = localMidnight(new Date(2026, 7, 1));
    const expectedTo = localMidnight(new Date(2026, 7, 6)); // day AFTER the end date (D4)

    const calls = await glanceCalls(appPage.page);
    const last = calls.at(-1);
    expect(last?.since).toBe(expectedFrom);
    expect(last?.to).toBe(expectedTo);
    expect(last?.sinceBaseline).toBeFalsy();
    expect(last?.days).toBeNull();
  });

  test('the picker persists the selected window across a reload (D7)', async ({ appPage }) => {
    await mockGlanceRecording(appPage.page, { today: TODAY_GLANCE, baseline: BASELINE_GLANCE });
    await appPage.goto('local');

    await appPage.page.getByTestId('accounts-window-30d').click();
    await expect(appPage.page.getByTestId('accounts-window-label')).toHaveText('Last 30d');
    await expect(appPage.page.getByTestId('accounts-window-30d')).toHaveAttribute(
      'aria-pressed',
      'true',
    );

    await appPage.page.reload();

    // A persisted choice always wins over the D6 default — the reload opens
    // straight back on 30d, not baseline.
    await expect(appPage.page.getByTestId('accounts-window-label')).toHaveText('Last 30d');
    await expect(appPage.page.getByTestId('accounts-window-30d')).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });

  test('the drill-down repeats the window label and honours the same window in account_history (D8)', async ({
    appPage,
  }) => {
    await seedWindow(appPage.page, { kind: 'baseline' });
    await mockGlanceRecording(appPage.page, { today: TODAY_GLANCE, baseline: BASELINE_GLANCE });
    await mockHistoryRecording(appPage.page, HISTORY_BASELINE);
    await appPage.goto('local');

    const tile = appPage.page.getByTestId('account-tile').filter({ hasText: 'tf-m1' });
    await tile.click();

    const modal = appPage.page.getByTestId('account-history-modal');
    await expect(modal).toBeVisible();
    await expect(modal.getByTestId('history-window-label')).toHaveText('Since baseline');

    let calls = await historyCalls(appPage.page);
    // D8: baseline passes neither since nor to — `account_history` already
    // defaults to since-baseline per account when `since` is omitted.
    expect(calls.at(-1)).toEqual({ account: 'tf-m1', since: null, to: null });

    await appPage.page.keyboard.press('Escape');
    await expect(modal).toHaveCount(0);

    // Switch to a custom range and reopen — the drill-down must follow the
    // section's window, not keep showing the previous one (D8).
    await appPage.page.getByTestId('accounts-window-custom').click();
    await appPage.page.getByTestId('accounts-range-start').fill('2026-08-01');
    await appPage.page.getByTestId('accounts-range-end').fill('2026-08-05');
    await appPage.page.getByTestId('accounts-range-apply').click();
    await expect(appPage.page.getByTestId('accounts-window-label')).toHaveText('Aug 1 – Aug 5');

    await tile.click();
    await expect(modal.getByTestId('history-window-label')).toHaveText('Aug 1 – Aug 5');

    const expectedFrom = localMidnight(new Date(2026, 7, 1));
    const expectedTo = localMidnight(new Date(2026, 7, 6));
    calls = await historyCalls(appPage.page);
    expect(calls.at(-1)).toEqual({ account: 'tf-m1', since: expectedFrom, to: expectedTo });
  });
});
