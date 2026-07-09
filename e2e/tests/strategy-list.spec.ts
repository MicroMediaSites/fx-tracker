import { test, expect } from '../helpers/app-fixture';
import {
  makeStrategy,
  makeArchivedStrategy,
  makePromotedStrategy,
} from '../fixtures/strategy-fixtures';

test.describe('Strategy List & CRUD', () => {
  test('renders strategy list with names from Zero data', async ({ appPage }) => {
    const s1 = makeStrategy({ name: 'Ichimoku Cloud Breakout' });
    const s2 = makeStrategy({ name: 'RSI Mean Reversion' });

    await appPage.setLocalStrategies([s1, s2]);
    await appPage.goto('backtest');

    await expect(appPage.page.getByText('Strategy Development')).toBeVisible();
    // Strategy name appears in both sidebar selector and header — use heading for header
    await expect(appPage.page.getByRole('heading', { name: 'Ichimoku Cloud Breakout' })).toBeVisible();
  });

  test('empty state shows create prompt', async ({ appPage }) => {
    await appPage.setLocalStrategies([]);
    await appPage.goto('backtest');

    await expect(appPage.page.getByText('No active strategies')).toBeVisible();
  });

  test('selecting a strategy shows indicator and rule counts', async ({ appPage }) => {
    const s1 = makeStrategy({ name: 'Test Strat Alpha' });
    await appPage.setLocalStrategies([s1]);
    await appPage.goto('backtest');

    // Strategy stats line: "2 indicators · 1 entry · 2 exit"
    await expect(appPage.page.getByText('2 indicators')).toBeVisible();
    await expect(appPage.page.getByText('1 entry')).toBeVisible();
  });

  test('show archived checkbox reveals archived strategies', async ({ appPage }) => {
    const active = makeStrategy({ name: 'Active Strategy' });
    const archived = makeArchivedStrategy({ name: 'Old Strategy' });
    await appPage.setLocalStrategies([active, archived]);
    await appPage.goto('backtest');

    // Initially first strategy auto-selected
    await expect(appPage.page.getByRole('heading', { name: 'Active Strategy' })).toBeVisible();

    // Check "Show Archived"
    const checkbox = appPage.page.getByLabel('Show Archived');
    await checkbox.check();

    // Expand dropdown to see both — click the strategy selector button
    const selector = appPage.page.getByRole('button', { name: /Active Strategy/ });
    await selector.click();

    // Archived strategy should appear in dropdown with "Archived" badge
    await expect(appPage.page.getByText('Old Strategy')).toBeVisible();
  });

  test('locked live strategy renders its Live badge without edit affordances', async ({ appPage }) => {
    const locked = makePromotedStrategy({ name: 'Live Strategy' });
    await appPage.setLocalStrategies([locked]);
    await appPage.goto('backtest');

    await expect(appPage.page.getByRole('heading', { name: 'Live Strategy' })).toBeVisible();
    // AGT-651: the builder is gone — no Edit/Clone/+New buttons anywhere.
    await expect(appPage.page.getByRole('button', { name: 'Clone' })).toHaveCount(0);
    await expect(appPage.page.getByRole('button', { name: 'Edit' })).toHaveCount(0);
    await expect(appPage.page.getByRole('button', { name: '+ New' })).toHaveCount(0);
  });
});
