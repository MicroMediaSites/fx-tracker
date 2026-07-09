import { test, expect } from '../helpers/app-fixture';

test.describe('BUG-054: Update notification shown when update available', () => {
  test('UpdateModal opens when check-for-updates event is fired from menu', async ({ appPage }) => {
    await appPage.goto('local');

    // Wait for the local boot window to fully render (owns the updater since AGT-652)
    await expect(appPage.page.getByTestId('local-app')).toBeVisible();

    // Wait for event listeners to be registered (useEffect is async)
    await appPage.page.waitForTimeout(500);

    // Fire the 'check-for-updates' event (simulates View > Check for Updates menu)
    await appPage.page.evaluate(() => {
      window.__E2E_EMIT_EVENT__?.('check-for-updates', undefined);
    });

    // The UpdateModal should now be visible with "Software Update" header
    await expect(appPage.page.getByText('Software Update')).toBeVisible();

    // In e2e (dev mode), the updater shows "disabled in development mode" error.
    // This validates that the event -> modal pipeline works correctly.
    // In production builds, the check() call would proceed to show available/up-to-date.
    await expect(appPage.page.getByText('Unable to check for updates')).toBeVisible();
  });

  test('UpdateModal closes when Close button is clicked', async ({ appPage }) => {
    await appPage.goto('local');
    await expect(appPage.page.getByTestId('local-app')).toBeVisible();

    // Wait for event listeners to be registered
    await appPage.page.waitForTimeout(500);

    // Open update modal via menu event
    await appPage.page.evaluate(() => {
      window.__E2E_EMIT_EVENT__?.('check-for-updates', undefined);
    });

    // Wait for modal to appear
    await expect(appPage.page.getByText('Software Update')).toBeVisible();

    // Click "Close" to dismiss
    await appPage.page.getByRole('button', { name: 'Close' }).click();

    // Modal should close
    await expect(appPage.page.getByText('Software Update')).not.toBeVisible();
  });

  test('UpdateModal is not shown on page load without trigger', async ({ appPage }) => {
    await appPage.goto('local');

    // Wait for the local boot window to fully render (owns the updater since AGT-652)
    await expect(appPage.page.getByTestId('local-app')).toBeVisible();

    // Give time for any auto-check to fire (it shouldn't in dev mode)
    await appPage.page.waitForTimeout(3000);

    // No update modal should be shown automatically
    await expect(appPage.page.getByText('Software Update')).not.toBeVisible();
  });
});
