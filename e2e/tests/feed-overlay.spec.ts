/**
 * FeedOverlay — the pull-down market-awareness feed in the header drawer.
 *
 * The drawer shell (drag handle, ⌘K) previously hosted the retired AI chat
 * terminal; it now renders items the `wickd feed tick` launchd producer
 * appends to `~/.wickd/feed.ndjson`, read via the offline `feed_list`
 * command. These specs mock `feed_list` — no producer, no network.
 */
import { test, expect } from '../helpers/app-fixture';

const FEED_ITEMS = [
  {
    id: 'feed-1',
    ts: new Date().toISOString(),
    run_id: 'run-1',
    severity: 'urgent',
    pairs: ['EUR_USD'],
    headline: 'US Core CPI in 45 minutes',
    body: 'High-impact USD print lands inside the H1 session.',
    kind: 'calendar',
    sources: ['calendar'],
  },
  {
    id: 'feed-2',
    ts: new Date(Date.now() - 20 * 60 * 1000).toISOString(),
    run_id: 'run-1',
    severity: 'watch',
    pairs: ['GBP_USD', 'EUR_GBP'],
    headline: 'Sterling softening across pairs',
    body: 'GBP legs lower on the last three H1 closes.',
    kind: 'price',
    sources: ['candles'],
  },
];

const openDrawer = async (appPage: { page: import('@playwright/test').Page }) => {
  // Click the handle's toggle icon to open at minimal height, then drag the
  // handle taller from an off-center point (the centered icon swallows
  // mousedown for its click behavior, so drags must start beside it).
  await appPage.page.locator('[title="Open feed (⌘K)"]').click();
  const handle = appPage.page.locator('[data-testid="terminal-drag-handle"]');
  await expect(handle).toBeVisible();
  const box = await handle.boundingBox();
  if (!box) throw new Error('drag handle bounding box not found');
  const dragX = box.x + box.width / 4;
  const centerY = box.y + box.height / 2;
  await appPage.page.mouse.move(dragX, centerY);
  await appPage.page.mouse.down();
  await appPage.page.mouse.move(dragX, centerY + 220);
  await appPage.page.mouse.up();
};

test.describe('Feed overlay drawer', () => {
  test('renders feed items with severity and pairs', async ({ appPage }) => {
    // mockTauriCommand is an init script — set it before goto().
    await appPage.mockTauriCommand('feed_list', FEED_ITEMS);
    await appPage.goto('watcher');

    await openDrawer(appPage);

    const overlay = appPage.page.getByTestId('feed-overlay');
    await expect(overlay).toBeVisible();

    const rows = appPage.page.getByTestId('feed-item-row');
    await expect(rows).toHaveCount(2);
    await expect(rows.first()).toContainText('US Core CPI in 45 minutes');
    await expect(rows.first()).toContainText('urgent');
    await expect(rows.first()).toContainText('EUR_USD');
    await expect(rows.nth(1)).toContainText('Sterling softening across pairs');
    await expect(rows.nth(1)).toContainText('EUR_GBP');
  });

  test('shows the empty state when the producer has not written yet', async ({ appPage }) => {
    await appPage.goto('watcher');

    await openDrawer(appPage);

    await expect(appPage.page.getByTestId('feed-overlay')).toBeVisible();
    await expect(appPage.page.getByTestId('feed-empty')).toBeVisible();
    await expect(appPage.page.getByTestId('feed-empty')).toContainText('every 15 minutes');
  });

  test('follow-up input asks via feed_ask and renders the transcript', async ({ appPage }) => {
    await appPage.mockTauriCommand('feed_list', FEED_ITEMS);
    await appPage.mockTauriCommand('feed_ask', 'GBP softness is a UK data story.');
    await appPage.goto('watcher');

    await openDrawer(appPage);

    const input = appPage.page.getByTestId('feed-ask-input');
    await expect(input).toBeVisible();
    await input.fill('why is sterling soft?');
    await input.press('Enter');

    const lines = appPage.page.getByTestId('feed-ask-line');
    await expect(lines).toHaveCount(2);
    await expect(lines.first()).toContainText('> why is sterling soft?');
    await expect(lines.nth(1)).toContainText('← GBP softness is a UK data story.');
  });
});
