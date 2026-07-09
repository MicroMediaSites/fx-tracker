/**
 * AGT-645 — strategies + backtests domain served from the local store.
 *
 * Two claims, as far as the E2E harness can prove them:
 *
 *   1. The offline local window (?window=local, no auth, no Zero, all
 *      non-localhost requests aborted) renders a strategy's saved backtest
 *      runs — runs list, metrics, equity curve, trades — entirely from the
 *      local-store command surface (AC2 + AC3).
 *   2. The backtest window's strategy list is served by
 *      `local_list_strategies` (not Zero), and a completed backtest run is
 *      persisted through `local_save_backtest` (AC1).
 *
 * Screenshots are written to review-evidence/ as visual evidence for the
 * stamp review.
 */

import { test, expect } from '../helpers/app-fixture';
import {
  makeStrategy,
  makeBacktestResultPayload,
  makeLocalBacktestRow,
} from '../fixtures/strategy-fixtures';

const EVIDENCE_DIR = 'review-evidence';

test.describe('Local window backtests view (AGT-645)', () => {
  test('renders runs, equity curve, and trades offline from the local store', async ({
    appPage,
  }) => {
    const { page } = appPage;

    // Abort every non-localhost request: the whole domain must work offline.
    const externalRequests: string[] = [];
    await page.route('**/*', (route) => {
      const url = new URL(route.request().url());
      const isLocalAsset = url.hostname === 'localhost' || url.hostname === '127.0.0.1';
      if (!isLocalAsset) {
        externalRequests.push(url.toString());
        return route.abort();
      }
      return route.continue();
    });

    const strat = makeStrategy({ name: 'Ichimoku H4 breakout' });
    await appPage.setLocalStrategies([strat]);
    await appPage.setLocalBacktests([
      makeLocalBacktestRow(strat.id, {
        runNumber: 1,
        startDate: '2025-01-01',
        endDate: '2025-03-31',
        createdAt: 1_735_689_600_000,
      }),
      makeLocalBacktestRow(strat.id, {
        runNumber: 2,
        startDate: '2025-04-01',
        endDate: '2025-06-30',
        createdAt: 1_743_465_600_000,
        result: makeBacktestResultPayload({
          metrics: {
            ...makeBacktestResultPayload().metrics,
            totalPnl: '-120.00',
            totalReturnPct: '-12.00',
          },
        }),
      }),
    ]);

    await appPage.goto('local');

    // Strategy list served from the local store.
    const row = page.getByTestId('local-strategy-row');
    await expect(row).toHaveCount(1);
    await expect(row.first()).toContainText('Ichimoku H4 breakout');

    // Select the strategy -> backtests section appears.
    await row.first().click();
    await expect(page.getByTestId('local-backtests')).toBeVisible();

    // Both saved runs render.
    const runRows = page.getByTestId('local-backtest-run-row');
    await expect(runRows).toHaveCount(2);

    // Latest run (#2) is selected by default; its metrics render.
    const detail = page.getByTestId('local-backtest-detail');
    await expect(detail).toBeVisible();
    await expect(page.getByTestId('local-backtest-metrics')).toContainText('-12.00%');

    // Select run #1 -> metrics, equity curve, and trades from local data.
    await runRows.filter({ hasText: '#1' }).click();
    await expect(runRows.filter({ hasText: '#1' })).toHaveAttribute('aria-pressed', 'true');
    await expect(page.getByTestId('local-backtest-metrics')).toContainText('$250.00');
    await expect(page.getByTestId('local-backtest-metrics')).toContainText('25.00%');
    await expect(page.getByTestId('local-equity-curve')).toBeVisible();
    const tradeRows = page.getByTestId('local-trade-row');
    await expect(tradeRows).toHaveCount(4);
    await expect(tradeRows.first()).toContainText('1.08500');

    // Let the selection highlight's 150ms color transition finish so the
    // evidence screenshot shows the selected run, not the mid-transition one.
    await page.waitForTimeout(300);

    await page.screenshot({
      path: `${EVIDENCE_DIR}/AGT-645-local-backtests-runs-equity-trades.png`,
      fullPage: true,
    });

    // Zero external requests: no Clerk, no Zero, no queries-service.
    expect(externalRequests).toEqual([]);
  });

  test('shows the empty state for a strategy with no saved runs', async ({ appPage }) => {
    const { page } = appPage;
    const strat = makeStrategy({ name: 'Fresh strategy' });
    await appPage.setLocalStrategies([strat]);

    await appPage.goto('local');
    await page.getByTestId('local-strategy-row').first().click();
    await expect(page.getByTestId('local-backtests-empty')).toBeVisible();
  });
});

test.describe('Backtest window on the local store (AGT-645)', () => {
  test('strategy list is served by local_list_strategies and a run persists via local_save_backtest', async ({
    appPage,
  }) => {
    const { page } = appPage;

    // Seed ONLY the local store — no Zero strategy data anywhere. If any
    // Zero query still fed this domain, the list would be empty.
    const strat = makeStrategy({ name: 'Local Store Strategy' });
    await appPage.setLocalStrategies([strat]);
    await appPage.mockTauriCommand('validate_strategy_json', { valid: true, errors: [] });
    await appPage.mockTauriCommand('run_custom_backtest', makeBacktestResultPayload());

    await appPage.goto('backtest');

    await expect(page.getByText('Strategy Development')).toBeVisible();
    await expect(
      page.getByRole('heading', { name: 'Local Store Strategy' })
    ).toBeVisible();

    // Pick the Simple Historical methodology and run a custom date range.
    await page.getByRole('button', { name: 'Select methodology...' }).click();
    await page.getByText('Simple Historical').click();
    // DateInput's label is not programmatically associated with its input,
    // so target the two date inputs (From, To) positionally.
    const dateInputs = page.locator('input[type="date"]');
    await dateInputs.nth(0).fill('2025-01-01');
    await dateInputs.nth(1).fill('2025-03-31');
    await page.getByRole('button', { name: 'Run Custom Range' }).click();

    // The run's results render (metrics from the mocked backtest)...
    await expect(page.getByText('Run History')).toBeVisible();

    // ...and the run was persisted to the local store through
    // local_save_backtest (AC1's backtest-result write path).
    await expect
      .poll(async () =>
        page.evaluate(
          () =>
            (window as Window & { __E2E_LOCAL_BACKTESTS__?: unknown[] })
              .__E2E_LOCAL_BACKTESTS__?.length ?? 0
        )
      )
      .toBe(1);
    const saved = await page.evaluate(
      () =>
        (window as Window & { __E2E_LOCAL_BACKTESTS__?: Array<Record<string, unknown>> })
          .__E2E_LOCAL_BACKTESTS__?.[0]
    );
    expect(saved?.strategy_id).toBe(strat.id);
    expect(saved?.instrument).toBe('EUR_USD');

    await page.screenshot({
      path: `${EVIDENCE_DIR}/AGT-645-backtest-window-local-run.png`,
      fullPage: true,
    });
  });

  test('saved runs rehydrate into the backtest window from the local store', async ({
    appPage,
  }) => {
    const { page } = appPage;

    const strat = makeStrategy({ name: 'Rehydrate Test' });
    await appPage.setLocalStrategies([strat]);
    await appPage.setLocalBacktests([
      makeLocalBacktestRow(strat.id, {
        runNumber: 1,
        startDate: '2025-01-01',
        endDate: '2025-03-31',
      }),
    ]);
    await appPage.mockTauriCommand('validate_strategy_json', { valid: true, errors: [] });

    await appPage.goto('backtest');
    await expect(page.getByRole('heading', { name: 'Rehydrate Test' })).toBeVisible();

    await page.getByRole('button', { name: 'Select methodology...' }).click();
    await page.getByText('Simple Historical').click();

    // The persisted run drives the composite header without running anything.
    await expect(page.getByText('Composite')).toBeVisible();
    await expect(page.getByText('+25.0%').first()).toBeVisible();
  });
});
