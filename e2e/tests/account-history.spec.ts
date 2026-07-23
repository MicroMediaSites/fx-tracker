/**
 * Account trade-history drill-down — click a tile, see its trades.
 *
 * Each trade shows entry → exit, side, size, P&L, and strategy. Two honesty
 * affordances are asserted because they are the point: a multi-exit trade is
 * marked "avg" (its exit price is a blended average, not a single fill), and a
 * truncated history says so rather than reading as complete.
 */
import { test, expect } from '../helpers/app-fixture';

const GLANCE = {
  environment: 'practice',
  days: null,
  since: '2026-07-20T06:00:00Z',
  generated_at: '2026-07-20T22:00:00Z',
  accounts: [
    {
      account: 'tf-m1',
      names: ['tf-m1'],
      account_id: '101-001-26151603-002',
      currency: 'USD',
      nav: '99976.89',
      balance: '99976.89',
      unrealized_pl: '0',
      open_trade_count: 0,
      realized: '-15.70',
      trades: 47,
      wins: 5,
      losses: 42,
      win_rate: 0.106,
      error: null,
    },
  ],
};

const HISTORY = {
  account: 'tf-m1',
  account_id: '101-001-26151603-002',
  environment: 'practice',
  baseline: { balance: '100000', date: '2026-07-16T23:15:49Z' },
  since: '2026-07-16T23:15:49Z',
  count: 2,
  realized: '-0.42',
  blended_exits: 1,
  truncated: false,
  trades: [
    {
      id: 'trade-1',
      instrument: 'EUR_USD',
      side: 'long',
      units: '2000',
      strategy: 'rahagod',
      entry: { time: '2026-07-20T09:00:00Z', price: '1.14066' },
      exit: { time: '2026-07-20T09:12:00Z', price: '1.14060', count: 1, blended: false },
      realized_pl: '-0.12',
      duration_secs: 720,
    },
    {
      id: 'trade-2',
      instrument: 'GBP_USD',
      side: 'short',
      units: '1500',
      strategy: 'rahagod',
      entry: { time: '2026-07-20T10:00:00Z', price: '1.29000' },
      exit: { time: '2026-07-20T11:30:00Z', price: '1.28950', count: 3, blended: true },
      realized_pl: '-0.30',
      duration_secs: 5400,
    },
  ],
};

test.describe('Account trade-history drill-down', () => {
  test('the account number suffix shows on the tile', async ({ appPage }) => {
    await appPage.mockTauriCommand('accounts_glance', GLANCE);
    await appPage.goto('local');

    // Last three digits of 101-001-26151603-002.
    await expect(appPage.page.getByTestId('account-tile').filter({ hasText: 'tf-m1' })).toContainText(
      '002'
    );
  });

  test('clicking a tile opens its trade history with entry and exit detail', async ({ appPage }) => {
    await appPage.mockTauriCommand('accounts_glance', GLANCE);
    await appPage.mockTauriCommand('account_history', HISTORY);
    await appPage.goto('local');

    await appPage.page.getByTestId('account-tile').filter({ hasText: 'tf-m1' }).click();

    const modal = appPage.page.getByTestId('account-history-modal');
    await expect(modal).toBeVisible();
    await expect(modal).toContainText('tf-m1');
    await expect(modal).toContainText('since experiment start');

    const rows = appPage.page.getByTestId('history-trade-row');
    await expect(rows).toHaveCount(2);

    const first = rows.filter({ hasText: 'EUR_USD' });
    await expect(first).toContainText('1.14066'); // entry
    await expect(first).toContainText('1.14060'); // exit
    await expect(first).toContainText('rahagod');
    await expect(first).toContainText('12m'); // held
  });

  test('a multi-exit trade is flagged as an average, not a single fill', async ({ appPage }) => {
    await appPage.mockTauriCommand('accounts_glance', GLANCE);
    await appPage.mockTauriCommand('account_history', HISTORY);
    await appPage.goto('local');

    await appPage.page.getByTestId('account-tile').filter({ hasText: 'tf-m1' }).click();

    const shortTrade = appPage.page.getByTestId('history-trade-row').filter({ hasText: 'GBP_USD' });
    await expect(shortTrade.getByTestId('history-blended')).toContainText('3 exits');
    // The clean single-exit trade carries no such flag.
    const longTrade = appPage.page.getByTestId('history-trade-row').filter({ hasText: 'EUR_USD' });
    await expect(longTrade.getByTestId('history-blended')).toHaveCount(0);
  });

  test('a truncated history says it may be incomplete', async ({ appPage }) => {
    await appPage.mockTauriCommand('accounts_glance', GLANCE);
    await appPage.mockTauriCommand('account_history', { ...HISTORY, truncated: true });
    await appPage.goto('local');

    await appPage.page.getByTestId('account-tile').filter({ hasText: 'tf-m1' }).click();

    await expect(appPage.page.getByTestId('history-truncated')).toContainText("history cap");
  });

  test('Escape closes the drill-down', async ({ appPage }) => {
    await appPage.mockTauriCommand('accounts_glance', GLANCE);
    await appPage.mockTauriCommand('account_history', HISTORY);
    await appPage.goto('local');

    await appPage.page.getByTestId('account-tile').filter({ hasText: 'tf-m1' }).click();
    await expect(appPage.page.getByTestId('account-history-modal')).toBeVisible();

    await appPage.page.keyboard.press('Escape');
    await expect(appPage.page.getByTestId('account-history-modal')).toHaveCount(0);
  });
});
