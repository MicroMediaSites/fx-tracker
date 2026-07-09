import { test, expect } from '../helpers/app-fixture';

test.describe('BUG-039: Trade marker removed on instrument change', () => {
  test('trade legend disappears when instrument changes away from trade instrument', async ({ appPage }) => {
    // Trade data in the format that useChartParams expects from URL params
    const tradeData = [
      {
        id: 'trade-001',
        instrument: 'EUR_USD',
        units: '10000',
        open_price: '1.08500',
        close_price: '1.08750',
        open_time: 1701424800000,
        close_time: 1701439200000,
        realized_pl: '25.00',
      },
    ];

    const tradesParam = encodeURIComponent(JSON.stringify(tradeData));

    // Navigate to chart with trade data in URL params
    await appPage.page.goto(
      `/?window=chart&instrument=EUR_USD&granularity=H1&trades=${tradesParam}`,
    );
    await appPage.page.waitForLoadState('domcontentloaded');

    // Verify trade legend is visible
    await expect(appPage.page.getByText('1 trades shown')).toBeVisible();

    // Click the instrument input to open the SymbolPicker dropdown
    const symbolInput = appPage.page.locator('input[placeholder="EUR/USD"]');
    await symbolInput.click();

    // Type GBP to filter, then select GBP/USD
    await symbolInput.fill('GBP');
    await appPage.page.getByText('GBP/USD').click();

    // Verify trade legend is gone because trades were cleared on instrument change
    await expect(appPage.page.getByText('trades shown')).not.toBeVisible();
  });
});
