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
    const processRow = appPage.page.getByTestId('watch-process-row');
    await expect(processRow).toHaveCount(1);
    await expect(processRow.getByText('revert_adx')).toBeVisible();
    await expect(processRow.getByText('pid 61105')).toBeVisible();

    // Live signals render from the daemon's durable queue (AC2).
    const alertRows = appPage.page.getByTestId('queue-alert-row');
    await expect(alertRows).toHaveCount(2);
    // Newest first: the 15:10 price-level alert precedes the 14:30 buy signal.
    await expect(alertRows.filter({ hasText: 'buy' })).toHaveCount(1);
    await expect(alertRows.filter({ hasText: 'buy' }).getByText('revert_adx')).toBeVisible();
    await expect(appPage.page.getByText('cross-up 1.2700 @ 1.2703')).toBeVisible();

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
    await expect(appPage.page.getByTestId('queue-empty')).toBeVisible();

    await appPage.page.screenshot({
      path: `${EVIDENCE_DIR}/AGT-652-watcher-daemon-idle.png`,
      fullPage: true,
    });
  });
});
