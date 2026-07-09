/**
 * AGT-650 — Zero/Clerk/queries-service removal: app windows work offline.
 *
 * AC4: the APP WINDOWS (not just the local boot window) mount with no
 * sign-in and no cloud, behind only the local credential gate. Each test
 * aborts all non-localhost traffic — the assertion is that surfaces render
 * real data with zero external requests.
 *
 * AGT-652 slimmed the surface to local/chart/backtest/watcher, so the
 * offline regression now covers the watcher window in its daemon-client
 * form (the account/ticket window tests were removed with those windows).
 */

import { test, expect } from '../helpers/app-fixture';

const EVIDENCE_DIR = 'review-evidence';

/** Abort everything that isn't the local dev server (offline stand-in). */
async function goOffline(appPage: { page: import('@playwright/test').Page }) {
  const externalRequests: string[] = [];
  await appPage.page.route('**/*', (route) => {
    const url = new URL(route.request().url());
    const isLocalAsset = url.hostname === 'localhost' || url.hostname === '127.0.0.1';
    if (!isLocalAsset) {
      externalRequests.push(url.toString());
      return route.abort();
    }
    return route.continue();
  });
  return externalRequests;
}

test.describe('AGT-650 — app windows offline, no auth, no Zero', () => {
  test('watcher window renders the daemon state offline with zero external requests', async ({
    appPage,
  }) => {
    await appPage.page.addInitScript(() => {
      (window as unknown as Record<string, unknown>).__E2E_DAEMON_STATUS__ = {
        watchers: [
          {
            pid: 4242,
            command: 'wickd watch revert_adx EUR_USD,GBP_USD --granularity H1',
            strategy: 'revert_adx',
            instruments: ['EUR_USD', 'GBP_USD'],
          },
        ],
        hub_socket_present: true,
        pending_count: 0,
        queue_len: 0,
      };
      (window as unknown as Record<string, unknown>).__E2E_HUB_STREAM__ = {
        mode: 'client',
        observed: ['EUR_USD', 'GBP_USD'],
        direct: [],
        last_line_ms: Date.now(),
      };
    });

    const externalRequests = await goOffline(appPage);
    await appPage.goto('watcher');

    await expect(appPage.page.getByText('Live Monitor')).toBeVisible();
    await expect(appPage.page.getByTestId('daemon-status')).toBeVisible();
    await expect(appPage.page.getByText('1 wickd watch daemon running')).toBeVisible();

    await appPage.page.screenshot({
      path: `${EVIDENCE_DIR}/AGT-650-watcher-local-store-offline.png`,
      fullPage: true,
    });

    expect(externalRequests).toEqual([]);
  });
});
