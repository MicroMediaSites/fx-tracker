import { test, expect } from '../helpers/app-fixture';
import {
  makeParameterizedStrategy,
  makeChainedTriggerStrategy,
  makeMultiIndicatorStrategy,
  makeVariableStrategy,
  makePivotStrategy,
  makeNewIndicatorStrategy,
  makeSessionFilterStrategy,
  makeCandlestickPatternStrategy,
  makePendingOrderStrategy,
} from '../fixtures/strategy-fixtures';

test.describe('Strategy Data Integrity', () => {
  test('strategy with parameterized values renders without crash', async ({ appPage }) => {
    const strat = makeParameterizedStrategy();
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    // Strategy with $param references should render without crashing
    await expect(appPage.page.getByRole('heading', { name: 'Parameterized Strategy' })).toBeVisible();
    // Should show indicator/rule counts in sidebar
    await expect(appPage.page.getByText('2 indicators')).toBeVisible();
  });

  test('strategy with chained AND/OR triggers renders', async ({ appPage }) => {
    const strat = makeChainedTriggerStrategy();
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    await expect(appPage.page.getByRole('heading', { name: 'Chained Trigger Strategy' })).toBeVisible();
    // AGT-651: the builder is gone — the list view rendering without crash is the contract.
    await expect(appPage.page.getByText('1 entry')).toBeVisible();
  });

  test('strategy with multiple indicator types renders indicator list', async ({ appPage }) => {
    const strat = makeMultiIndicatorStrategy();
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    await expect(appPage.page.getByRole('heading', { name: 'Multi-Indicator Strategy' })).toBeVisible();
    // Strategy should render without crashing
    await expect(appPage.page.getByText('indicators')).toBeVisible();
  });

  test('strategy with all variable expression types renders', async ({ appPage }) => {
    const strat = makeVariableStrategy();
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    await expect(appPage.page.getByRole('heading', { name: 'Variable Strategy' })).toBeVisible();
    // Variables render only in the retired builder; list rendering is the contract now.
    await expect(appPage.page.getByText('indicators')).toBeVisible();
  });

  test('strategy with conditional and rolling variables renders', async ({ appPage }) => {
    const strat = makeVariableStrategy({ name: 'Extended Variable Strategy' });
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    await expect(appPage.page.getByRole('heading', { name: 'Extended Variable Strategy' })).toBeVisible();
    await expect(appPage.page.getByText('indicators')).toBeVisible();
  });

  test('strategy with VWAP, Parabolic SAR, and SuperTrend indicators renders', async ({ appPage }) => {
    const strat = makeNewIndicatorStrategy();
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    await expect(appPage.page.getByRole('heading', { name: 'New Indicator Strategy' })).toBeVisible();
    // Strategy with new indicator types (VWAP, Parabolic SAR, SuperTrend) should render without crashing
    await expect(appPage.page.getByText('indicators')).toBeVisible();
  });

  test('strategy with pivot config renders without crash', async ({ appPage }) => {
    const strat = makePivotStrategy();
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    await expect(appPage.page.getByRole('heading', { name: 'Pivot Strategy' })).toBeVisible();
    // Strategy should load without crashing even with pivot config
    await expect(appPage.page.getByText('2 indicators')).toBeVisible();
  });

  test('strategy with session filter triggers renders without crash', async ({ appPage }) => {
    const strat = makeSessionFilterStrategy();
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    await expect(appPage.page.getByRole('heading', { name: 'Session Filter Strategy' })).toBeVisible();
    // Strategy with time_in_range and day_of_week triggers should render without crashing
    await expect(appPage.page.getByText('2 indicators')).toBeVisible();
  });

  test('strategy with candlestick pattern data source renders without crash', async ({ appPage }) => {
    const strat = makeCandlestickPatternStrategy();
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    await expect(appPage.page.getByRole('heading', { name: 'Candlestick Pattern Strategy' })).toBeVisible();
    // Strategy with pattern data source should render without crashing
    await expect(appPage.page.getByText('1 indicator')).toBeVisible();
  });

  test('strategy with pending order config renders without crash', async ({ appPage }) => {
    const strat = makePendingOrderStrategy();
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    await expect(appPage.page.getByRole('heading', { name: 'Pending Order Strategy' })).toBeVisible();
    // Strategy with pending_order on entry rule should render without crashing
    await expect(appPage.page.getByText('2 indicators')).toBeVisible();
  });
});
