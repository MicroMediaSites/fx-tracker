import { test, expect } from '../helpers/app-fixture';
import { makeStrategy } from '../fixtures/strategy-fixtures';

test.describe('Strategy Parameters', () => {
  test('strategy with parameters shows parameter names in sidebar', async ({ appPage }) => {
    const strat = makeStrategy({ name: 'Param Render Test' });
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    // Strategy auto-selected — verify header
    await expect(appPage.page.getByRole('heading', { name: 'Param Render Test' })).toBeVisible();

    // Select Simple Historical methodology to reveal parameter panel
    await appPage.page.getByRole('button', { name: 'Select methodology...' }).click();
    await appPage.page.getByText('Simple Historical').click();

    // Parameters should now be visible: RSI Period and EMA Period
    await expect(appPage.page.getByText('RSI Period')).toBeVisible();
    await expect(appPage.page.getByText('EMA Period')).toBeVisible();
  });

  test('strategy with no parameters does not show parameter panel', async ({ appPage }) => {
    const strat = makeStrategy({
      name: 'No Params Strategy',
      parameters: JSON.stringify([]),
    });
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    // Select methodology
    await appPage.page.getByRole('button', { name: 'Select methodology...' }).click();
    await appPage.page.getByText('Simple Historical').click();

    // With no parameters, RSI Period / EMA Period should not appear
    await expect(appPage.page.getByText('RSI Period')).not.toBeVisible();
    await expect(appPage.page.getByText('EMA Period')).not.toBeVisible();
  });

  test('methodology selector shows available options', async ({ appPage }) => {
    const strat = makeStrategy({ name: 'Methodology Test' });
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    // Click to expand methodology dropdown
    await appPage.page.getByRole('button', { name: 'Select methodology...' }).click();

    // Should see methodology options
    await expect(appPage.page.getByText('Simple Historical')).toBeVisible();
    await expect(appPage.page.getByText('Walk-Forward')).toBeVisible();
  });
});
