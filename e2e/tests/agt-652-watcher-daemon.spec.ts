/**
 * AGT-652 — one watcher engine: the app is a client of the wickd CLI watcher.
 *
 * AC1/AC2 evidence: the app hosts no engine; the Live Monitor renders the
 * daemon's client-visible outputs — running `wickd watch` processes, the
 * durable signal feed (alert-queue.ndjson), and the semi-auto pending/approve
 * queue (pending.json) — read-only, with a `wickd approve <id>` affordance
 * instead of any in-app execution. Screenshots land in review-evidence/ for
 * the stamp review.
 */

import { test, expect } from '../helpers/app-fixture';

const EVIDENCE_DIR = 'review-evidence';

const PENDING_SIGNAL = {
  id: 'e6a1f1cc-0001-4c8e-9d0e-agt652demo01',
  ts: '2026-07-06T14:30:00+00:00',
  instrument: 'EUR_USD',
  side: 'long',
  units: 1000,
  suggested_units: 2400,
  strategy: 'revert_adx',
  reason: 'ADX below threshold with RSI reversion from oversold',
  sl: '1.0800',
  tp: '1.0950',
  entry_price: '1.0850',
  status: 'pending',
};

const QUEUE_ALERTS = [
  {
    id: 'queue-0001',
    ts: '2026-07-06T14:30:00+00:00',
    payload: {
      kind: 'strategy-signal',
      instrument: 'EUR_USD',
      signal: 'buy',
      proposal: PENDING_SIGNAL,
    },
  },
  {
    id: 'queue-0002',
    ts: '2026-07-06T15:10:00+00:00',
    payload: {
      kind: 'price-level',
      instrument: 'GBP_USD',
      level: '1.2700',
      direction: 'cross-up',
      price: '1.2703',
    },
  },
];

test.describe('AGT-652 — Live Monitor as a wickd daemon client', () => {
  test('renders daemon status, signal feed, and read-only pending queue', async ({ appPage }) => {
    await appPage.page.addInitScript(
      ({ pending, queue }) => {
        const w = window as unknown as Record<string, unknown>;
        w.__E2E_DAEMON_STATUS__ = {
          watchers: [
            {
              pid: 61105,
              command:
                'wickd watch revert_adx EUR_USD,GBP_USD,USD_CHF,EUR_GBP --granularity H1 --env practice --auto',
              strategy: 'revert_adx',
              instruments: ['EUR_USD', 'GBP_USD', 'USD_CHF', 'EUR_GBP'],
            },
          ],
          hub_socket_present: true,
          pending_count: 1,
          queue_len: queue.length,
        };
        w.__E2E_DAEMON_QUEUE__ = queue;
        w.__E2E_DAEMON_PENDING__ = [pending];
        w.__E2E_HUB_STREAM__ = {
          mode: 'client',
          observed: ['EUR_USD', 'GBP_USD'],
          direct: [],
          last_line_ms: Date.now(),
        };
      },
      { pending: PENDING_SIGNAL, queue: QUEUE_ALERTS },
    );

    await appPage.goto('watcher');

    // Watcher status renders from the daemon (AC2): process + stream state.
    await expect(appPage.page.getByTestId('daemon-status')).toBeVisible();
    await expect(appPage.page.getByText('1 wickd watch daemon running')).toBeVisible();
    await expect(appPage.page.getByText('attached to wickd stream hub')).toBeVisible();
    // CLI/launchd-managed watchers render as read-only external rows on the
    // per-instrument tiles of the merged "Watching" section: one row per
    // (watcher, instrument) with strategy + timeframe; pid in the tooltip.
    const externalRows = appPage.page.getByTestId('external-watcher-row');
    await expect(externalRows).toHaveCount(4); // one watcher x 4 instruments
    await expect(externalRows.first().getByText('revert_adx')).toBeVisible();
    await expect(externalRows.first().getByText('H1')).toBeVisible();
    await expect(externalRows.first()).toHaveAttribute('title', /pid 61105/);

    // The signal feed moved to the Home window ("Signals") — the Monitor
    // stays reserved for actionable state.
    await expect(appPage.page.getByTestId('queue-alert-row')).toHaveCount(0);

    // The semi-auto pending/approve queue renders read-only (AC1/AC2):
    // approval is a CLI action, so the affordance is the exact command.
    const pendingRow = appPage.page.getByTestId('pending-signal-row');
    await expect(pendingRow).toHaveCount(1);
    await expect(pendingRow.getByText('EUR_USD')).toBeVisible();
    await expect(
      appPage.page.getByTestId('copy-approve-command'),
    ).toHaveText(`wickd approve ${PENDING_SIGNAL.id}`);
    // No in-app execute affordance exists anymore.
    await expect(appPage.page.getByRole('button', { name: /execute/i })).toHaveCount(0);

    await appPage.page.screenshot({
      path: `${EVIDENCE_DIR}/AGT-652-watcher-daemon-client.png`,
      fullPage: true,
    });
  });

  test('renders the idle state when no daemon is running', async ({ appPage }) => {
    await appPage.goto('watcher');

    await expect(appPage.page.getByText('no wickd watch daemon running')).toBeVisible();
    await expect(appPage.page.getByTestId('pending-empty')).toBeVisible();

    await appPage.page.screenshot({
      path: `${EVIDENCE_DIR}/AGT-652-watcher-daemon-idle.png`,
      fullPage: true,
    });
  });

  test('signal feed renders on the Home window as "Signals"', async ({ appPage }) => {
    await appPage.page.addInitScript(
      ({ queue }) => {
        (window as unknown as Record<string, unknown>).__E2E_DAEMON_QUEUE__ = queue;
      },
      { queue: QUEUE_ALERTS },
    );

    await appPage.goto('local');

    const alertRows = appPage.page.getByTestId('queue-alert-row');
    await expect(alertRows).toHaveCount(2);
    await expect(alertRows.filter({ hasText: 'buy' })).toHaveCount(1);
    await expect(alertRows.filter({ hasText: 'buy' }).getByText('revert_adx')).toBeVisible();
    await expect(appPage.page.getByText('cross-up 1.2700 @ 1.2703')).toBeVisible();
  });

  test('instrument-first flow: pin a price window, attach a strategy under it', async ({ appPage }) => {
    await appPage.mockTauriCommand('store_list_strategies', [
      { name: 'kijun_revert_trend', valid: true },
      { name: 'broken_script', valid: false },
    ]);
    await appPage.mockTauriCommand('start_watcher', 12345);
    await appPage.mockTauriCommand('stop_watcher', null);

    await appPage.goto('watcher');

    // Pin a pair: the ghost tile's header spot opens the pair menu in place.
    await appPage.page.getByTestId('add-pair-trigger').click();
    const pairMenu = appPage.page.getByTestId('add-pair-menu');
    await expect(pairMenu).toBeVisible();
    await expect(appPage.page.getByTestId('add-pair-all')).toBeVisible();
    await pairMenu.getByText('EUR/USD', { exact: true }).click();
    await expect(pairMenu).not.toBeVisible();
    const grid = appPage.page.getByTestId('price-grid');
    await expect(grid.getByText('EUR/USD', { exact: true })).toBeVisible();

    // "+ strategy" appends an inline-editable row with its name menu already
    // open (inline editing philosophy — the row is the editor).
    await appPage.page.getByTestId('add-strategy-button').click();
    const row = appPage.page.getByTestId('strategy-row');
    await expect(row).toHaveCount(1);
    const nameMenu = appPage.page.getByTestId('strategy-row-name-menu');
    await expect(nameMenu).toBeVisible();
    // Only valid store strategies (plus builtins) are offered.
    await expect(nameMenu.locator('button')).toHaveText([
      'kijun_revert_trend',
      'ma-crossover',
      'rsi',
    ]);
    await nameMenu.getByText('kijun_revert_trend').click();
    await expect(nameMenu).not.toBeVisible();
    await expect(row.getByTestId('strategy-row-name')).toHaveText('kijun_revert_trend');

    // Timeframe edits in place the same way.
    await row.getByTestId('strategy-row-timeframe').click();
    const timeframeMenu = appPage.page.getByTestId('strategy-row-timeframe-menu');
    await expect(timeframeMenu).toBeVisible();
    await timeframeMenu.getByText('H4', { exact: true }).click();
    await expect(row.getByTestId('strategy-row-timeframe')).toHaveText('H4');

    // A fresh row is disarmed; the state icon cycles the trust ladder
    // disarmed -> monitor -> semi-auto -> disarmed (semi-auto is the UI
    // ceiling — no --auto state exists). Arming renders immediately via the
    // config's own pid, before any daemon poll.
    const stateButton = row.getByTestId('strategy-row-state');
    await expect(stateButton).toHaveAttribute('title', /Disarmed/);
    await stateButton.click();
    await expect(stateButton).toHaveAttribute('title', /Monitoring/);
    await stateButton.click();
    await expect(stateButton).toHaveAttribute('title', /Semi-auto/);
    await stateButton.click();
    await expect(stateButton).toHaveAttribute('title', /Disarmed/);
    await expect(row.getByTestId('strategy-row-error')).toHaveCount(0);

    // Disarmed rows can be deleted.
    await expect(row.getByTestId('strategy-row-delete')).toBeVisible();
  });
});
