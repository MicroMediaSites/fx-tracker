/**
 * AGT-648 — imported CandleSight archive rows are visible, badged, and
 * filterable in the local window.
 *
 * The import CLI (`cargo run --bin import_candlesight`) tags every restored
 * row with `source = 'candlesight'`. This spec proves the app side of the AC:
 * imported strategies render with a distinguishing badge and the source
 * filter separates them from native wickd strategies.
 *
 * Screenshots are written to review-evidence/ as visual evidence for the
 * stamp review.
 */

import { test, expect } from '../helpers/app-fixture';
import { makeStrategy } from '../fixtures/strategy-fixtures';

const EVIDENCE_DIR = 'review-evidence';

test.describe('CandleSight import visibility (AGT-648)', () => {
  test('imported strategies are badged and filterable by source', async ({ appPage }) => {
    const { page } = appPage;

    const native = makeStrategy({ name: 'Fresh wickd idea' });
    const imported = makeStrategy({
      name: 'Ichi w MACD Confirm (recovered)',
      source: 'candlesight',
    });
    const importedArchived = makeStrategy({
      name: 'Old HiLo Open (recovered)',
      source: 'candlesight',
      is_archived: true,
    });
    await appPage.setLocalStrategies([native, imported, importedArchived]);

    await appPage.goto('local');

    // All three render; only the imported ones carry the candlesight badge.
    const rows = page.getByTestId('local-strategy-row');
    await expect(rows).toHaveCount(3);
    await expect(page.getByTestId('candlesight-badge')).toHaveCount(2);
    await expect(
      rows.filter({ hasText: 'Fresh wickd idea' }).getByTestId('candlesight-badge')
    ).toHaveCount(0);

    await page.screenshot({
      path: `${EVIDENCE_DIR}/AGT-648-local-candlesight-badges.png`,
      fullPage: true,
    });

    // Filter down to the imported rows only.
    const filter = page.getByTestId('local-source-filter');
    await expect(filter).toBeVisible();
    await filter.selectOption('candlesight');
    await expect(rows).toHaveCount(2);
    await expect(rows.filter({ hasText: 'Fresh wickd idea' })).toHaveCount(0);

    await page.screenshot({
      path: `${EVIDENCE_DIR}/AGT-648-local-candlesight-filter.png`,
      fullPage: true,
    });

    // And to native wickd rows only.
    await filter.selectOption('wickd');
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText('Fresh wickd idea');
    await expect(page.getByTestId('candlesight-badge')).toHaveCount(0);

    // Back to everything.
    await filter.selectOption('all');
    await expect(rows).toHaveCount(3);
  });

  test('the source filter is hidden when nothing was imported', async ({ appPage }) => {
    const { page } = appPage;
    await appPage.setLocalStrategies([makeStrategy({ name: 'Only native data' })]);

    await appPage.goto('local');
    await expect(page.getByTestId('local-strategy-row')).toHaveCount(1);
    await expect(page.getByTestId('local-source-filter')).toHaveCount(0);
    await expect(page.getByTestId('candlesight-badge')).toHaveCount(0);
  });
});
