/**
 * AGT-653 — de-SaaS cleanup: billing, entitlements, marketing, Clerk shell.
 *
 * Proves the post-teardown surface:
 *
 *   1. the default window still cold-boots fully offline (zero non-localhost
 *      requests — nothing tries to reach Stripe/Clerk/marketing), and
 *   2. formerly tier-gated UI is ungated: the backtest methodology selector
 *      shows no lock icons / tier badges / upgrade prompts, and no pricing
 *      surface exists anywhere in the window.
 *
 * Screenshots are written to review-evidence/ as visual evidence for the
 * stamp review.
 */

import { test, expect } from '../helpers/app-fixture';
import { makeStrategy } from '../fixtures/strategy-fixtures';

const EVIDENCE_DIR = 'review-evidence';

test.describe('De-SaaS cleanup (AGT-653)', () => {
  test('default window boots offline with no billing/auth surface', async ({ appPage }) => {
    const { page } = appPage;

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

    await appPage.goto('local');

    await expect(page.getByTestId('local-app')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'wickd' })).toBeVisible();

    // No auth or billing surface on the boot path.
    await expect(page.getByText('Sign In', { exact: true })).toHaveCount(0);
    await expect(page.getByText(/upgrade/i)).toHaveCount(0);
    await expect(page.getByText(/subscription/i)).toHaveCount(0);

    await page.screenshot({
      path: `${EVIDENCE_DIR}/AGT-653-offline-boot-de-saas.png`,
      fullPage: true,
    });

    // Zero non-localhost requests: no Stripe, no Clerk, no marketing site.
    expect(externalRequests).toEqual([]);
  });

  test('methodology selector is ungated: no locks, tiers, or upgrade prompts', async ({
    appPage,
  }) => {
    const { page } = appPage;
    const strat = makeStrategy({ name: 'De-SaaS Gate Check' });
    await appPage.setLocalStrategies([strat]);
    await appPage.goto('backtest');

    await expect(page.getByRole('heading', { name: 'De-SaaS Gate Check' })).toBeVisible();

    // Expand the methodology dropdown — every methodology is selectable with
    // no tier machinery attached.
    await page.getByRole('button', { name: 'Select methodology...' }).click();
    await expect(page.getByText('Simple Historical')).toBeVisible();
    await expect(page.getByText('Walk-Forward')).toBeVisible();

    // Formerly premium/pro-gated options carry no lock or tier badge.
    await expect(page.getByText('(premium)')).toHaveCount(0);
    await expect(page.getByText('(pro)')).toHaveCount(0);
    await expect(page.getByText(/upgrade to/i)).toHaveCount(0);

    await page.screenshot({
      path: `${EVIDENCE_DIR}/AGT-653-methodology-ungated.png`,
      fullPage: true,
    });

    // Selecting a formerly-gated methodology just works (no pricing modal).
    await page.getByText('Walk-Forward').first().click();
    await expect(page.getByText(/premium feature/i)).toHaveCount(0);
    await expect(page.getByText(/upgrade/i)).toHaveCount(0);
  });
});
