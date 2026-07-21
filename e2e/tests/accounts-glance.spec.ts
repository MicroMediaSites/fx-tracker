/**
 * AccountsSection — the accounts-at-a-glance panel on the HOME window.
 *
 * Renders `accounts_glance`, which shells out to `wickd trade glance`. These
 * specs mock the command — no CLI, no keychain, no OANDA.
 *
 * The cases here are the ones the panel's design turns on: aliased accounts
 * collapsing to one row, a null win rate rendering "—" rather than "0%", and a
 * per-account failure degrading to an error row while its healthy siblings
 * still render.
 */
import { test, expect } from '../helpers/app-fixture';

/** Shaped after Matt's real practice config (the TF ladder + h004). */
const GLANCE = {
  environment: 'practice',
  days: 7,
  since: '2026-07-13T03:00:00Z',
  generated_at: '2026-07-20T03:00:00Z',
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
      realized: '-23.1058',
      trades: 68,
      wins: 7,
      losses: 61,
      win_rate: 0.103,
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

test.describe('Accounts at a glance', () => {
  test('renders a row per account with realized P&L and win rate', async ({ appPage }) => {
    // mockTauriCommand is an init script — set it before goto().
    await appPage.mockTauriCommand('accounts_glance', GLANCE);
    await appPage.goto('local');

    const rows = appPage.page.locator('[data-testid="account-row"]');
    await expect(rows).toHaveCount(4);

    // Winning account: signed, green.
    const h004 = rows.filter({ hasText: 'h004' });
    await expect(h004.locator('[data-testid="account-realized"]')).toHaveText('+$47.20');
    await expect(h004).toContainText('67% W');
    await expect(h004).toContainText('6 trades');
    // Open position is reported separately from the window figure.
    await expect(h004).toContainText('open');

    // Losing account: the minus is a real sign, not a hyphen in the amount.
    const m1 = rows.filter({ hasText: 'tf-m1' });
    await expect(m1.locator('[data-testid="account-realized"]')).toHaveText('−$23.11');
    await expect(m1).toContainText('10% W');
    await expect(m1).toContainText('68 trades');
  });

  test('an idle account shows "—" for win rate, never 0%', async ({ appPage }) => {
    await appPage.mockTauriCommand('accounts_glance', GLANCE);
    await appPage.goto('local');

    const m30 = appPage.page.locator('[data-testid="account-row"]').filter({ hasText: 'tf-m30' });
    await expect(m30).toContainText('— W');
    await expect(m30).not.toContainText('0% W');
    await expect(m30).toContainText('0 trades');
  });

  test('aliased accounts collapse into one row', async ({ appPage }) => {
    await appPage.mockTauriCommand('accounts_glance', GLANCE);
    await appPage.goto('local');

    // `default` appears only as an alias marker on tf-m30's row, never as its
    // own row — it is the same OANDA account.
    const rows = appPage.page.locator('[data-testid="account-row"]');
    await expect(rows.filter({ hasText: 'tf-m30' })).toContainText('+default');
    await expect(rows.filter({ hasText: /^default/ })).toHaveCount(0);
  });

  test('one failed account does not blank its healthy siblings', async ({ appPage }) => {
    await appPage.mockTauriCommand('accounts_glance', GLANCE);
    await appPage.goto('local');

    const failed = appPage.page.locator('[data-testid="account-row"]').filter({ hasText: 'tf-h1' });
    await expect(failed.locator('[data-testid="account-error"]')).toContainText('401 Unauthorized');
    // The other three still rendered.
    await expect(appPage.page.locator('[data-testid="account-realized"]')).toHaveCount(3);
  });

  test('empty config explains how to fix it', async ({ appPage }) => {
    await appPage.mockTauriCommand('accounts_glance', { ...GLANCE, accounts: [] });
    await appPage.goto('local');

    await expect(appPage.page.getByText('wickd login')).toBeVisible();
  });
});
