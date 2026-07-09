import { test, expect } from '../helpers/app-fixture';

test.describe('Settings Persistence', () => {
  test('settings hydrate from localStorage', async ({ appPage }) => {
    await appPage.goto('watcher');

    // Settings store has completedTours with all windows marked true, so no tour should appear
    // The FirstRunTour should not be visible
    const hasTourOverlay = await appPage.page
      .getByText(/welcome to wickd|let.*show you/i)
      .first()
      .isVisible({ timeout: 2000 })
      .catch(() => false);

    expect(hasTourOverlay).toBeFalsy();
  });

  test('tour is skipped when completedTours includes the window', async ({ appPage }) => {
    await appPage.goto('watcher');

    // Wait for app to fully render
    await expect(appPage.page.getByTestId('daemon-status')).toBeVisible();

    // Tour overlay should not be present
    const tourElements = await appPage.page.locator('[data-tour-overlay]').count();
    expect(tourElements).toBe(0);
  });

  test('app renders with demo data source', async ({ appPage }) => {
    await appPage.goto('watcher');

    // With practice credentials, the header environment badge reads "Demo"
    // (the account-alias display left with the deleted ProfileMenu, AGT-652)
    await expect(appPage.page.getByText('Demo', { exact: true })).toBeVisible();
  });
});
