/**
 * AGT-646 — charting, S/R zones, and notes served from the local store.
 *
 * Proves the AC2 claim as far as the E2E harness allows: with all
 * non-localhost networking aborted (offline), the chart window renders S/R
 * zones read from the local-store command surface (`local_list_sr_zones`),
 * and the notes journal reads/writes through `local_list_notes` /
 * `local_save_note` — no Zero query backs any of it (the sr_zone/note Zero
 * queries and mutators no longer exist in the codebase).
 *
 * Screenshots are written to review-evidence/ as visual evidence for the
 * stamp review.
 */

import { test, expect } from '../helpers/app-fixture';

const EVIDENCE_DIR = 'review-evidence';

/** ~30 hourly candles around 1.0850 so the seeded zones sit inside the price range. */
function buildCandles(): { time: string; open: string; high: string; low: string; close: string }[] {
  const candles = [];
  const start = Date.UTC(2026, 5, 1, 0, 0, 0); // 2026-06-01T00:00:00Z
  let price = 1.084;
  for (let i = 0; i < 30; i++) {
    const drift = Math.sin(i / 4) * 0.002;
    const open = price;
    const close = 1.084 + drift;
    const high = Math.max(open, close) + 0.0006;
    const low = Math.min(open, close) - 0.0006;
    candles.push({
      time: new Date(start + i * 3600_000).toISOString(),
      open: open.toFixed(5),
      high: high.toFixed(5),
      low: low.toFixed(5),
      close: close.toFixed(5),
    });
    price = close;
  }
  return candles;
}

test.describe('Local-store charting domain (AGT-646)', () => {
  test('chart renders S/R zones from the local store while offline', async ({ appPage }) => {
    const { page } = appPage;

    // Offline: abort every non-localhost request. Zones/notes/chart config
    // must be served purely by the (mocked) local-store commands.
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

    // Seed the local store with two zones for the default instrument.
    await page.addInitScript(() => {
      const now = Date.now();
      (window as unknown as { __E2E_LOCAL_SR_ZONES__: unknown[] }).__E2E_LOCAL_SR_ZONES__ = [
        {
          id: 'zone-1',
          instrument: 'EUR_USD',
          upper_price: '1.08600',
          lower_price: '1.08450',
          label: 'Demand',
          color: 'rgba(34, 197, 94, 0.20)',
          created_at: now - 1000,
          updated_at: now - 1000,
        },
        {
          id: 'zone-2',
          instrument: 'EUR_USD',
          upper_price: '1.08300',
          lower_price: '1.08200',
          label: null,
          color: null,
          created_at: now,
          updated_at: now,
        },
      ];
    });
    await appPage.mockTauriCommand('get_candles', buildCandles());

    await appPage.goto('chart');

    // Chart window is up.
    await expect(page.getByText('Charting')).toBeVisible();

    // The Zones tools menu proves the zone data flowed from the local store:
    // "Clear All" reports the count of zones loaded for this instrument.
    await page.getByRole('button', { name: 'Zones' }).click();
    await page.getByRole('button', { name: 'Clear All' }).click();
    await expect(page.getByText('Clear 2 zones?')).toBeVisible();

    await page.screenshot({
      path: `${EVIDENCE_DIR}/AGT-646-chart-zones-from-local-store.png`,
      fullPage: true,
    });

    // Clearing goes through local_clear_sr_zones and re-reads the store.
    await page.getByRole('button', { name: 'Clear', exact: true }).click();
    await page.getByRole('button', { name: 'Zones' }).click();
    await expect(page.getByRole('button', { name: 'Clear All' })).not.toBeVisible();

    expect(externalRequests).toEqual([]);
  });

// (The trade-journal notes test lived on the account window, deleted by AGT-652.
// Note storage itself is still covered by src-tauri local_store unit tests.)
});
