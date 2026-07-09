import { test, expect } from '../helpers/app-fixture';

test.describe('BUG-038: Trade markers display when chart opens from menu', () => {
  test('trade legend shows when the local store has closed trades for the instrument', async ({
    appPage,
  }) => {
    // Seed the local store with closed trades for EUR_USD (default chart
    // instrument). Since AGT-647 the chart reads trades from the local store
    // (local_list_closed_trades_by_instrument), not Zero.
    const closedTrades = [
      {
        id: 'trade-001',
        account_id: '101-001-1234567-001',
        instrument: 'EUR_USD',
        units: '10000',
        open_price: '1.08500',
        close_price: '1.08750',
        open_time: 1733050800000, // 2024-12-01T10:00:00Z in ms
        close_time: 1733065200000, // 2024-12-01T14:00:00Z in ms
        realized_pl: '25.00',
        state: 'CLOSED',
        synced_at: Date.now(),
        created_at: Date.now(),
        updated_at: Date.now(),
      },
      {
        id: 'trade-002',
        account_id: '101-001-1234567-001',
        instrument: 'EUR_USD',
        units: '-5000',
        open_price: '1.09000',
        close_price: '1.08800',
        open_time: 1733137200000, // 2024-12-02T10:00:00Z in ms
        close_time: 1733151600000, // 2024-12-02T14:00:00Z in ms
        realized_pl: '10.00',
        state: 'CLOSED',
        synced_at: Date.now(),
        created_at: Date.now(),
        updated_at: Date.now(),
      },
    ];

    await appPage.page.addInitScript((trades) => {
      (window as Window & { __E2E_LOCAL_TRADES__?: unknown[] }).__E2E_LOCAL_TRADES__ = trades;
    }, closedTrades);

    // Open chart from menu (no trade URL params)
    await appPage.goto('chart');

    // The TradeLegend component renders when effectiveTrades is non-null.
    // It shows "{count} trades shown" text.
    await expect(appPage.page.getByText('2 trades shown')).toBeVisible({ timeout: 5000 });
  });

  test('no trade legend when the local store has no trades for the instrument', async ({
    appPage,
  }) => {
    // Default bridge state has an empty local trade store
    await appPage.goto('chart');

    // ChartApp renders the WindowHeader with title "Charting"
    await expect(appPage.page.getByText('Charting')).toBeVisible();

    // TradeLegend should NOT be visible when there are no trades
    await expect(appPage.page.getByText(/\d+ trades shown/)).not.toBeVisible();
  });
});
