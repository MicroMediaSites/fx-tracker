/**
 * BUG-075: "Forgot password? Reset Credentials" button does nothing.
 *
 * Root cause: The ConfirmModal rendered at z-[150], the same z-index as the
 * UnlockVaultModal. In Tauri's webview, same-level z-index siblings can
 * render in unpredictable order, causing the ConfirmModal to be hidden
 * behind the opaque UnlockVaultModal.
 *
 * Fix: Raised ConfirmModal z-index to z-[200] so it always renders above
 * other modals when used for dangerous confirmation actions.
 */
import { test, expect, type AppPage } from '../helpers/app-fixture';

/**
 * Helper to set the vault to "locked" state before navigation.
 * Must be called BEFORE goto() since addInitScript only takes effect
 * on the next navigation.
 */
async function setVaultLocked(appPage: AppPage) {
  await appPage.page.addInitScript(() => {
    window.__E2E_TAURI_OVERRIDES__ = window.__E2E_TAURI_OVERRIDES__ || {};
    (window.__E2E_TAURI_OVERRIDES__ as Record<string, unknown>)['is_vault_unlocked'] = false;

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
}

test.describe('BUG-075: Reset credentials confirmation modal', () => {
  test('reset credentials modal appears above unlock modal', async ({ appPage }) => {
    await setVaultLocked(appPage);
    await appPage.goto('watcher');

    // Verify the UnlockVaultModal is showing
    await expect(
      appPage.page.getByPlaceholder('Master password')
    ).toBeVisible({ timeout: 5000 });

    // Click "Forgot password? Reset credentials"
    const resetButton = appPage.page.getByText('Forgot password? Reset credentials');
    await expect(resetButton).toBeVisible();
    await resetButton.click();

    // Verify the ConfirmModal appears with the correct content
    await expect(
      appPage.page.getByRole('heading', { name: 'Reset Credentials' })
    ).toBeVisible({ timeout: 3000 });
    await expect(
      appPage.page.getByText('permanently delete your stored OANDA credentials')
    ).toBeVisible();
  });

  test('cancel button closes the confirmation modal', async ({ appPage }) => {
    await setVaultLocked(appPage);
    await appPage.goto('watcher');

    // Wait for unlock modal
    await expect(
      appPage.page.getByPlaceholder('Master password')
    ).toBeVisible({ timeout: 5000 });

    // Open the reset confirmation
    await appPage.page.getByText('Forgot password? Reset credentials').click();
    await expect(
      appPage.page.getByRole('heading', { name: 'Reset Credentials' })
    ).toBeVisible({ timeout: 3000 });

    // Click Cancel
    await appPage.page.getByRole('button', { name: 'Cancel' }).click();

    // ConfirmModal should be gone
    await expect(
      appPage.page.getByRole('heading', { name: 'Reset Credentials' })
    ).not.toBeVisible();

    // UnlockVaultModal should still be showing
    await expect(
      appPage.page.getByPlaceholder('Master password')
    ).toBeVisible();
  });

  test('confirm button triggers credential reset', async ({ appPage }) => {
    // Keep the local-store credential row intact (bridge default) so
    // deleteCredentials actually runs (vs silently returning)
    await setVaultLocked(appPage);
    await appPage.goto('watcher');

    // Wait for unlock modal
    await expect(
      appPage.page.getByPlaceholder('Master password')
    ).toBeVisible({ timeout: 5000 });

    // Verify the credentials setup flag exists before reset
    const flagBefore = await appPage.page.evaluate(() =>
      localStorage.getItem('credentials_setup_complete_e2e-device-001')
    );
    expect(flagBefore).toBe('true');

    // Open the reset confirmation
    await appPage.page.getByText('Forgot password? Reset credentials').click();
    await expect(
      appPage.page.getByRole('heading', { name: 'Reset Credentials' })
    ).toBeVisible({ timeout: 3000 });

    // Click the "Reset Credentials" confirm button (exact match to avoid
    // matching the "Forgot password? Reset credentials" button underneath)
    await appPage.page.getByRole('button', { name: 'Reset Credentials', exact: true }).click();

    // The ConfirmModal should close after the reset completes
    await expect(
      appPage.page.getByRole('heading', { name: 'Reset Credentials' })
    ).not.toBeVisible({ timeout: 5000 });

    // Verify deleteCredentials actually ran: localStorage flag should be cleared
    // This proves the confirm button invoked the reset function, not just closed the modal
    const flagAfter = await appPage.page.evaluate(() =>
      localStorage.getItem('credentials_setup_complete_e2e-device-001')
    );
    expect(flagAfter).toBeNull();
  });
});
