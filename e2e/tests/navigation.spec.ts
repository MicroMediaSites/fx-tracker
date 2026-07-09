import { test, expect } from '../helpers/app-fixture';

test.describe('Window Routing', () => {
  test('?window=local renders the local boot window', async ({ appPage }) => {
    await appPage.goto('local');

    await expect(appPage.page.getByTestId('local-app')).toBeVisible();
  });

  test('?window=backtest renders the strategy viewer/runner window', async ({ appPage }) => {
    await appPage.goto('backtest');

    // BacktestApp renders WindowHeader with title "Strategy Development"
    await expect(appPage.page.getByText('Strategy Development')).toBeVisible();
  });

  test('?window=chart renders chart app', async ({ appPage }) => {
    await appPage.goto('chart');

    // ChartApp renders WindowHeader with title "Charting"
    await expect(appPage.page.getByText('Charting')).toBeVisible();
  });

  test('?window=watcher renders the Live Monitor (daemon client)', async ({ appPage }) => {
    await appPage.goto('watcher');

    // StrategyWatcherApp renders WindowHeader with title "Live Monitor"
    await expect(appPage.page.getByText('Live Monitor')).toBeVisible();
    await expect(appPage.page.getByTestId('daemon-status')).toBeVisible();
  });

  test('no window param boots the local window (login window is gone, AGT-652)', async ({
    appPage,
  }) => {
    await appPage.goto();

    await expect(appPage.page.getByTestId('local-app')).toBeVisible();
  });
});
