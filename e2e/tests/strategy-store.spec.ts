/**
 * Unified `.rhai` strategy store — app viewer/runner surface (AGT-651).
 *
 * The app lists strategies from `~/.wickd/strategies/` (mocked via
 * `__E2E_STORE_STRATEGIES__`) alongside local-store rows, badges them as
 * read-only store entries, and offers a read-only source viewer. Authoring
 * affordances (builder, +New, Edit, Clone) no longer exist anywhere.
 */
import { test, expect } from '../helpers/app-fixture';
import { makeStrategy } from '../fixtures/strategy-fixtures';

const EVIDENCE_DIR = 'review-evidence';

const RHAI_SOURCE = `// @parameters: [ { "id": "period", "name": "Period", "type": "integer", "default": 14 } ]
fn on_candle() {
    let p = param("period");
    "hold"
}
`;

const storeEntry = (name: string) => ({
  name,
  path: `/e2e-home/.wickd/strategies/${name}.rhai`,
  valid: true,
  parameters: [
    {
      id: 'period',
      name: 'Period',
      description: null,
      type: 'integer',
      default: 14,
      min: 2,
      max: 50,
      step: 1,
      options: null,
      group: null,
    },
  ],
  indicators: [{ id: 'rsi', type: 'rsi', params: { period: 14 }, symbol: null, timeframe: null }],
  modified_at: 1751000000000,
  content_hash: 'deadbeefdeadbeef',
  source: RHAI_SOURCE,
});

test.describe('Unified .rhai strategy store (viewer/runner)', () => {
  test('store strategies appear in the list with the store badge and read-only source viewer', async ({
    appPage,
  }) => {
    const { page } = appPage;
    await appPage.setStoreStrategies([storeEntry('revert_adx')]);
    await appPage.setLocalStrategies([makeStrategy({ name: 'Local Rules Strategy' })]);
    await appPage.goto('backtest');

    // Both worlds are listed; select the store strategy from the dropdown
    // (initial auto-select races the async store load, so select explicitly).
    await page.getByRole('button', { name: /Strategy|revert_adx/ }).first().click();
    await page.getByRole('button', { name: /revert_adx/ }).last().click();
    await expect(page.getByRole('heading', { name: 'revert_adx' })).toBeVisible();
    await expect(page.getByTestId('store-badge').first()).toBeVisible();

    // No authoring affordances anywhere (builder is deleted).
    await expect(page.getByRole('button', { name: '+ New' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Edit' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Clone' })).toHaveCount(0);
    // Store entries cannot be promoted from the app (CLI-managed).
    await expect(page.getByRole('button', { name: 'Go Live' })).toHaveCount(0);

    await page.screenshot({
      path: `${EVIDENCE_DIR}/AGT-651-store-strategy-list.png`,
      fullPage: true,
    });

    // Read-only source viewer.
    await page.getByTestId('view-source-button').click();
    await expect(page.getByTestId('source-viewer-modal')).toBeVisible();
    await expect(page.getByText('fn on_candle()')).toBeVisible();
    await expect(page.getByText('Read-only')).toBeVisible();
    // ScriptPanel is not editable here: no Save button in the modal.
    await expect(
      page.getByTestId('source-viewer-modal').getByRole('button', { name: 'Save' })
    ).toHaveCount(0);

    await page.screenshot({
      path: `${EVIDENCE_DIR}/AGT-651-source-viewer.png`,
      fullPage: true,
    });

    await page.getByTestId('source-viewer-close').click();
    await expect(page.getByTestId('source-viewer-modal')).toHaveCount(0);

    // The local-store row is still reachable through the dropdown (viewer
    // keeps serving archived/imported rows).
    await page.getByRole('button', { name: /revert_adx/ }).first().click();
    await expect(page.getByText('Local Rules Strategy').first()).toBeVisible();
  });

  test('store strategy parameters drive the runner parameter panel', async ({ appPage }) => {
    const { page } = appPage;
    await appPage.setStoreStrategies([storeEntry('mean_revert')]);
    await appPage.setLocalStrategies([]);
    await appPage.goto('backtest');

    await expect(page.getByRole('heading', { name: 'mean_revert' })).toBeVisible();

    // Pick the simple methodology so the testable-parameters panel renders
    // from the script's @parameters metadata.
    await page.getByRole('button', { name: 'Select methodology...' }).click();
    await page.getByRole('button', { name: /Simple Historical/ }).click();
    await expect(page.getByText('Period', { exact: false }).first()).toBeVisible();

    await page.screenshot({
      path: `${EVIDENCE_DIR}/AGT-651-store-strategy-runner.png`,
      fullPage: true,
    });
  });
});
