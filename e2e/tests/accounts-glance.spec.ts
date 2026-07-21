/**
 * AccountsSection — the dashboard's lead block on the HOME window.
 *
 * Renders `accounts_glance`, which shells out to `wickd trade glance`. These
 * specs mock the command — no CLI, no keychain, no OANDA.
 *
 * The cases here are the ones the design turns on: a hero total you can read
 * without scanning, per-account tiles rather than rows, an idle account that
 * recedes without lying, a failure that doesn't blank its siblings, and a
 * refusal to total across currencies.
 */
import { test, expect } from '../helpers/app-fixture';

/** Shaped after the real practice config (the TF ladder + h004). */
const GLANCE = {
  environment: 'practice',
  days: null,
  since: '2026-07-20T06:00:00Z',
  generated_at: '2026-07-20T22:00:00Z',
  accounts: [
    {
      account: 'h004',
      names: ['h004'],
      account_id: '101-001-26151603-001',
      currency: 'USD',
      nav: '10047.2000',
      balance: '10000.0000',
      unrealized_pl: '12.4000',
      open_trade_count: 1,
      realized: '47.2000',
      trades: 6,
      wins: 4,
      losses: 2,
      win_rate: 0.667,
      error: null,
    },
    {
      account: 'tf-m1',
      names: ['tf-m1'],
      account_id: '101-001-26151603-002',
      currency: 'USD',
      nav: '99976.8953',
      balance: '99976.8953',
      unrealized_pl: '0.0000',
      open_trade_count: 0,
      realized: '-15.7000',
      trades: 47,
      wins: 5,
      losses: 42,
      win_rate: 0.106,
      error: null,
    },
    {
      // Aliased: `default` and `tf-m30` are the same OANDA account.
      account: 'tf-m30',
      names: ['tf-m30', 'default'],
      account_id: '101-001-26151603-005',
      currency: 'USD',
      nav: '99999.9998',
      balance: '99999.9998',
      unrealized_pl: '0.0000',
      open_trade_count: 0,
      realized: '0',
      trades: 0,
      wins: 0,
      losses: 0,
      win_rate: null,
      error: null,
    },
    {
      account: 'tf-h1',
      names: ['tf-h1'],
      account_id: '101-001-26151603-006',
      currency: null,
      nav: null,
      balance: null,
      unrealized_pl: null,
      open_trade_count: null,
      realized: null,
      trades: null,
      wins: null,
      losses: null,
      win_rate: null,
      error: 'OANDA account fetch failed: 401 Unauthorized',
    },
  ],
};

test.describe('Accounts dashboard', () => {
  test('leads with a hero total across accounts', async ({ appPage }) => {
    // mockTauriCommand is an init script — set it before goto().
    await appPage.mockTauriCommand('accounts_glance', GLANCE);
    await appPage.goto('local');

    // 47.20 − 15.70 + 0 = 31.50 (the errored account contributes nothing).
    await expect(appPage.page.getByTestId('accounts-hero')).toHaveText('+$31.50');
    await expect(appPage.page.getByTestId('accounts-dashboard')).toContainText('53 trades');
    await expect(appPage.page.getByTestId('accounts-dashboard')).toContainText('1 unavailable');
  });

  test('breaks down into one tile per account, not a list of rows', async ({ appPage }) => {
    await appPage.mockTauriCommand('accounts_glance', GLANCE);
    await appPage.goto('local');

    const tiles = appPage.page.getByTestId('account-tile');
    await expect(tiles).toHaveCount(4);

    const m1 = tiles.filter({ hasText: 'tf-m1' });
    // The minus is a real sign, not a hyphen inside the amount.
    await expect(m1.getByTestId('account-realized')).toHaveText('−$15.70');
    await expect(m1).toContainText('47t');
    await expect(m1).toContainText('11%');
  });

  test('an idle account recedes and says so', async ({ appPage }) => {
    await appPage.mockTauriCommand('accounts_glance', GLANCE);
    await appPage.goto('local');

    const m30 = appPage.page.getByTestId('account-tile').filter({ hasText: 'tf-m30' });
    await expect(m30).toHaveAttribute('data-idle', 'true');
    await expect(m30).toContainText('no activity');
    // Never "0%" for a window where nothing was decided.
    await expect(m30).not.toContainText('0%');
  });

  test('aliased accounts collapse into one tile', async ({ appPage }) => {
    await appPage.mockTauriCommand('accounts_glance', GLANCE);
    await appPage.goto('local');

    const tiles = appPage.page.getByTestId('account-tile');
    // `default` shows only as an alias count on tf-m30, never as its own tile.
    await expect(tiles.filter({ hasText: 'tf-m30' })).toContainText('+1');
    await expect(tiles.filter({ hasText: /^default/ })).toHaveCount(0);
  });

  test('one failed account does not blank its healthy siblings', async ({ appPage }) => {
    await appPage.mockTauriCommand('accounts_glance', GLANCE);
    await appPage.goto('local');

    const failed = appPage.page.getByTestId('account-tile').filter({ hasText: 'tf-h1' });
    await expect(failed).toContainText('unavailable');
    await expect(failed.getByTestId('account-error')).toContainText('401 Unauthorized');
    // The other three still render their numbers.
    await expect(appPage.page.getByTestId('account-realized')).toHaveCount(3);
  });

  test('refuses to total across mixed currencies', async ({ appPage }) => {
    // Adding USD to JPY would be a confident, meaningless number.
    await appPage.mockTauriCommand('accounts_glance', {
      ...GLANCE,
      accounts: [
        { ...GLANCE.accounts[0], currency: 'USD', realized: '100' },
        { ...GLANCE.accounts[1], currency: 'JPY', realized: '5000' },
      ],
    });
    await appPage.goto('local');

    await expect(appPage.page.getByTestId('accounts-hero')).toHaveCount(0);
    await expect(appPage.page.getByTestId('accounts-hero-mixed')).toContainText(
      'different currencies'
    );
  });

  test('defaults to the today window', async ({ appPage }) => {
    // "Was today profitable" is the cold-boot question, so today leads — and it
    // goes out as a `since` instant, not `days: 1`, which is a different span.
    await appPage.mockTauriCommand('accounts_glance', GLANCE);
    await appPage.goto('local');

    await expect(appPage.page.getByTestId('accounts-window-today')).toHaveAttribute(
      'aria-pressed',
      'true'
    );
    await expect(appPage.page.getByTestId('accounts-window-7d')).toHaveAttribute(
      'aria-pressed',
      'false'
    );
    await expect(appPage.page.getByTestId('accounts-dashboard')).toContainText('Today');
  });

  test('empty config explains how to fix it', async ({ appPage }) => {
    await appPage.mockTauriCommand('accounts_glance', { ...GLANCE, accounts: [] });
    await appPage.goto('local');

    await expect(appPage.page.getByText('wickd login')).toBeVisible();
  });
});
