import { test, expect } from '../helpers/app-fixture';
import {
  makeStrategy,
  mockBacktestResult,
  mockWalkForwardResult,
} from '../fixtures/strategy-fixtures';

test.describe('Backtest Execution', () => {
  test('backtest page renders with strategy selected', async ({ appPage }) => {
    const strat = makeStrategy({ name: 'Backtest Flow Test' });
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    await expect(appPage.page.getByText('Strategy Development')).toBeVisible();
    await expect(appPage.page.getByRole('heading', { name: 'Backtest Flow Test' })).toBeVisible();
    await expect(appPage.page.getByText('Backtest Config')).toBeVisible();
  });

  test('selecting methodology shows methodology info', async ({ appPage }) => {
    const strat = makeStrategy({ name: 'Methodology Test' });
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    // Click methodology dropdown
    await appPage.page.getByRole('button', { name: 'Select methodology...' }).click();
    await appPage.page.getByText('Simple Historical').click();

    // Methodology info should appear (replaces "Select a methodology" text)
    await expect(appPage.page.getByText('Select a methodology to begin backtesting.')).not.toBeVisible();
  });

  test('run backtest command can be mocked', async ({ appPage }) => {
    const strat = makeStrategy({ name: 'Run Backtest Test' });
    await appPage.setLocalStrategies([strat]);
    await appPage.mockTauriCommand('run_backtest', mockBacktestResult);
    await appPage.mockTauriCommand('validate_strategy_json', { valid: true, errors: [] });
    await appPage.goto('backtest');

    await expect(appPage.page.getByRole('heading', { name: 'Run Backtest Test' })).toBeVisible();
  });

  test('walk-forward progress event does not crash page', async ({ appPage }) => {
    const strat = makeStrategy({ name: 'WF Progress Test' });
    await appPage.setLocalStrategies([strat]);
    await appPage.mockTauriCommand('run_walk_forward', mockWalkForwardResult);
    await appPage.mockTauriCommand('validate_strategy_json', { valid: true, errors: [] });
    await appPage.goto('backtest');

    // Emit a walk-forward progress event
    await appPage.emitTauriEvent('wf-progress', {
      phase: 'optimization',
      windowNum: 1,
      totalWindows: 4,
      percent: 25,
      strategyId: strat.id,
    });

    // The page should handle the event without crashing
    await expect(appPage.page.getByRole('heading', { name: 'WF Progress Test' })).toBeVisible();
  });

  test('walk-forward completion event does not crash page', async ({ appPage }) => {
    const strat = makeStrategy({ name: 'WF Complete Test' });
    await appPage.setLocalStrategies([strat]);
    await appPage.mockTauriCommand('run_walk_forward', mockWalkForwardResult);
    await appPage.goto('backtest');

    // Emit completion event
    await appPage.emitTauriEvent('wf-complete', {
      result: mockWalkForwardResult,
      strategyId: strat.id,
    });

    // Page should still be stable
    await expect(appPage.page.getByText('Strategy Development')).toBeVisible();
  });

  test('Go Live button visible for selected strategy', async ({ appPage }) => {
    const strat = makeStrategy({ name: 'Promote Test' });
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    await expect(appPage.page.getByRole('button', { name: 'Go Live', exact: true })).toBeVisible();
  });
});
