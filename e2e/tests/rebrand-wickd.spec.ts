import { test, expect } from '../helpers/app-fixture';

/**
 * AGT-640 rebrand evidence: the rendered app shell presents as wickd and no
 * user-visible CandleSight copy remains. Screenshots are written to
 * review-evidence/ and cited in REVIEW-NOTES.md for the stamp review.
 */
test.describe('wickd rebrand (AGT-640)', () => {
  test('watcher window shows wickd branding, no CandleSight copy', async ({ appPage }) => {
    await appPage.goto('watcher');
    await expect(appPage.page.getByTestId('daemon-status')).toBeVisible();

    // Window header logo alt text is rebranded
    await expect(appPage.page.getByAltText('wickd').first()).toBeVisible();

    // No user-visible CandleSight copy anywhere in the rendered shell
    const bodyText = await appPage.page.locator('body').innerText();
    expect(bodyText).not.toMatch(/candlesight/i);

    await appPage.page.screenshot({
      path: 'review-evidence/watcher-window-wickd.png',
      fullPage: true,
    });
  });

  test('onboarding welcome copy reads "Welcome to wickd"', async ({ appPage }) => {
    // Simulate a fresh install (no credentials) to surface onboarding
    await appPage.page.addInitScript(() => {
      window.__E2E_TAURI_OVERRIDES__ = window.__E2E_TAURI_OVERRIDES__ || {};
      (window.__E2E_TAURI_OVERRIDES__ as Record<string, unknown>)['is_vault_unlocked'] = false;
      (window.__E2E_TAURI_OVERRIDES__ as Record<string, unknown>)['has_practice_credentials'] = false;

      const vaultState = JSON.stringify({
        state: {
          status: 'loading',
          deviceId: 'e2e-device-001',
          hasPracticeCredentials: false,
          hasLiveCredentials: false,
          unlockedAt: null,
          rateLimitMessage: null,
          rateLimitSeconds: 0,
          error: null,
        },
        version: 0,
      });
      localStorage.setItem('candlesight-vault', vaultState);
      localStorage.removeItem('credentials_setup_complete_e2e-device-001');

      // Also clear the stored credential row in the local-store mock (AGT-650)
      (window as unknown as Record<string, unknown>).__E2E_LOCAL_CREDENTIAL__ = null;
    });

    await appPage.goto('watcher');

    await expect(appPage.page.getByText(/welcome to wickd/i)).toBeVisible();
    await expect(appPage.page.getByText(/welcome to candlesight/i)).not.toBeVisible();

    await appPage.page.screenshot({
      path: 'review-evidence/onboarding-welcome-wickd.png',
      fullPage: true,
    });
  });
});
