/**
 * BUG-076: React error #310 (too many re-renders) when dragging the AI terminal overlay handle.
 *
 * Root cause: The drag useEffect in WindowHeader.tsx and ModalTerminalDrawer.tsx included
 * `terminalHeight` in its dependency array. Each mousemove during drag called
 * setTerminalHeight, which triggered a re-render, which re-ran the effect (because
 * terminalHeight changed), which removed and re-added event listeners on every pixel
 * of movement. React hit its render limit and threw error #310.
 *
 * Fix: Removed `terminalHeight` from the drag useEffect dependency array. Instead,
 * a ref (`terminalHeightRef`) tracks the current height so `handleMouseUp` can read
 * the latest value without being in the dependency array. The effect now only depends
 * on `isDragging`.
 */
import { test, expect } from '../helpers/app-fixture';

test.describe('BUG-076: Terminal drag handle', () => {
  test('dragging terminal handle does not crash with React error #310', async ({ appPage }) => {
    await appPage.goto('watcher');

    // Wait for page to load
    await expect(appPage.page.getByTestId('daemon-status')).toBeVisible({ timeout: 5000 });

    const handle = appPage.page.locator('[data-testid="terminal-drag-handle"]');
    await expect(handle).toBeVisible();

    const box = await handle.boundingBox();
    if (!box) throw new Error('Terminal drag handle bounding box not found');

    const centerX = box.x + box.width / 2;
    const centerY = box.y + box.height / 2;

    // Simulate drag: mousedown, multiple mousemoves, mouseup
    await appPage.page.mouse.move(centerX, centerY);
    await appPage.page.mouse.down();

    // Move in small increments to simulate real drag (10 steps of 15px each = 150px)
    for (let i = 1; i <= 10; i++) {
      await appPage.page.mouse.move(centerX, centerY + i * 15);
    }
    await appPage.page.mouse.up();

    // Page should still be functional (no crash from error #310)
    await expect(appPage.page.getByTestId('daemon-status')).toBeVisible();

    // Terminal portal should be visible and have expanded
    const portal = appPage.page.locator('[data-testid="terminal-portal"]');
    await expect(portal).toBeVisible();

    // Verify no React error overlay appeared
    const errorOverlay = appPage.page.locator('text=Too many re-renders');
    await expect(errorOverlay).not.toBeVisible();
  });
});
