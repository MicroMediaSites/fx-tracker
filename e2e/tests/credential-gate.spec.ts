import { test, expect } from '../helpers/app-fixture';

test.describe('Credential Gate', () => {
  test('vault unlocked renders app content', async ({ appPage }) => {
    await appPage.goto('watcher');

    // With vault unlocked (default), should see the Live Monitor content
    await expect(appPage.page.getByTestId('daemon-status')).toBeVisible();
  });

  test('vault locked shows unlock UI', async ({ appPage }) => {
    // Override vault to be locked
    await appPage.page.addInitScript(() => {
      window.__E2E_TAURI_OVERRIDES__ = window.__E2E_TAURI_OVERRIDES__ || {};
      (window.__E2E_TAURI_OVERRIDES__ as Record<string, unknown>)['is_vault_unlocked'] = false;

      // Also update localStorage to reflect locked state
      const vaultState = JSON.stringify({
        state: {
          status: 'locked',
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
    });

    await appPage.goto('watcher');

    // UnlockVaultModal renders with a "Master password" placeholder input
    await expect(
      appPage.page.getByPlaceholder('Master password')
    ).toBeVisible();

    // Should NOT show the Live Monitor content
    await expect(appPage.page.getByTestId('daemon-status')).not.toBeVisible();
  });

  test('no credentials shows onboarding flow', async ({ appPage }) => {
    // Override to simulate no credentials
    await appPage.page.addInitScript(() => {
      window.__E2E_TAURI_OVERRIDES__ = window.__E2E_TAURI_OVERRIDES__ || {};
      (window.__E2E_TAURI_OVERRIDES__ as Record<string, unknown>)['is_vault_unlocked'] = false;
      (window.__E2E_TAURI_OVERRIDES__ as Record<string, unknown>)['has_practice_credentials'] = false;

      // Update localStorage - no credentials
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

    // Should NOT show the Live Monitor content
    await expect(appPage.page.getByTestId('daemon-status')).not.toBeVisible();
  });
});
