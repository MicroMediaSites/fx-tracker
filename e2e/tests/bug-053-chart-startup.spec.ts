import { test, expect } from '../helpers/app-fixture';

test.describe('BUG-053: Chart window opens on startup', () => {
  test('chart window renders when navigated via ?window=chart', async ({ appPage }) => {
    // The fix added a "charting" | "chart" match arm to open_startup_windows
    // in src-tauri/src/commands/window.rs. This test verifies the frontend
    // route works — when the Rust backend creates a window with ?window=chart,
    // ChartApp renders correctly.
    await appPage.goto('chart');

    // ChartApp renders WindowHeader with title "Charting"
    await expect(appPage.page.getByText('Charting')).toBeVisible();
  });
});
