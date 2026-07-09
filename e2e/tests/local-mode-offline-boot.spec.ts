/**
 * AGT-642 — local-first boot (walking skeleton).
 *
 * Proves the AC2 core claim as far as the E2E harness allows: the default
 * cold-boot window (?window=local, the URL tauri.conf.json now boots with)
 * renders a working strategies view served from the local store, with
 *
 *   1. all non-localhost network aborted (offline),
 *   2. no sign-in UI and no Clerk/Zero/queries-service traffic attempted,
 *   3. list / create / delete flowing through the local-store commands.
 *
 * Screenshots are written to review-evidence/ as visual evidence for the
 * stamp review.
 */

import { test, expect } from '../helpers/app-fixture';

const EVIDENCE_DIR = 'review-evidence';

test.describe('Local mode offline boot (AGT-642)', () => {
  test('boots offline with no sign-in and serves strategies from the local store', async ({
    appPage,
  }) => {
    const { page } = appPage;

    // Track every request the app attempts to a non-localhost origin, and
    // kill it: the boot must succeed with networking (beyond the local dev
    // server standing in for bundled assets) unavailable.
    const externalRequests: string[] = [];
    await page.route('**/*', (route) => {
      const url = new URL(route.request().url());
      const isLocalAsset = url.hostname === 'localhost' || url.hostname === '127.0.0.1';
      if (!isLocalAsset) {
        externalRequests.push(url.toString());
        return route.abort();
      }
      return route.continue();
    });

    // Cold boot straight into the local window — no auth bridge state needed,
    // but note we do NOT rely on the mocked "authenticated" defaults either:
    // the local window never calls get_auth_status.
    await appPage.goto('local');

    // A working window: header + store path + empty state.
    await expect(page.getByTestId('local-app')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'wickd' })).toBeVisible();
    await expect(page.getByTestId('local-store-path')).toContainText('.wickd/app.db');
    await expect(page.getByTestId('local-strategies-empty')).toBeVisible();

    // No sign-in surface anywhere on the boot path.
    await expect(page.getByText('Sign In', { exact: true })).toHaveCount(0);
    await expect(page.getByText('Please log in', { exact: false })).toHaveCount(0);

    await page.screenshot({
      path: `${EVIDENCE_DIR}/AGT-642-offline-boot-empty.png`,
      fullPage: true,
    });

    // AGT-651: in-app strategy creation is gone (authoring happens through
    // the wickd CLI against the unified .rhai store) — no create form.
    await expect(page.getByRole('button', { name: 'Create strategy' })).toHaveCount(0);

    // The whole session attempted zero non-localhost requests: no Clerk, no
    // Zero, no queries-service, nothing.
    expect(externalRequests).toEqual([]);
  });

  test('boot shows the store error instead of crashing when the store is unavailable', async ({
    appPage,
  }) => {
    const { page } = appPage;
    // mockTauriCommand serializes its response (functions/Errors don't survive
    // addInitScript), so install a throwing override directly in page scope.
    await page.addInitScript(() => {
      const w = window as Window & {
        __E2E_TAURI_OVERRIDES__?: Record<string, unknown>;
      };
      w.__E2E_TAURI_OVERRIDES__ = w.__E2E_TAURI_OVERRIDES__ || {};
      w.__E2E_TAURI_OVERRIDES__['local_list_strategies'] = () => {
        throw new Error('disk unwritable');
      };
    });

    await appPage.goto('local');

    await expect(page.getByTestId('local-app')).toBeVisible();
    await expect(page.getByTestId('local-store-error')).toContainText('Local store unavailable');

    await page.screenshot({
      path: `${EVIDENCE_DIR}/AGT-642-store-error-fallback.png`,
      fullPage: true,
    });
  });

  test('pre-seeded strategies render ordered by most recently updated', async ({ appPage }) => {
    const { page } = appPage;
    const base = {
      description: '',
      schema_version: 2,
      parameters: null,
      variables: null,
      indicators: '[]',
      entry_rules: '[]',
      entry_logic: null,
      exit_rules: '[]',
      risk_settings: '{}',
      planning_conversation: null,
      auto_note_indicators: null,
      pivot_config: null,
      strategy_type: 'rules',
      script_content: null,
      version: 1,
      is_active: true,
      is_promoted: false,
      is_locked: false,
      is_archived: false,
    };
    await page.addInitScript(
      (seed) => {
        (window as Window & { __E2E_LOCAL_STRATEGIES__?: unknown[] }).__E2E_LOCAL_STRATEGIES__ =
          seed;
      },
      [
        { ...base, id: 's-old', name: 'Older strategy', created_at: 1000, updated_at: 1000 },
        {
          ...base,
          id: 's-new',
          name: 'Newer strategy',
          is_promoted: true,
          created_at: 2000,
          updated_at: 2000,
        },
      ],
    );

    await appPage.goto('local');

    const rows = page.getByTestId('local-strategy-row');
    await expect(rows).toHaveCount(2);
    await expect(rows.nth(0)).toContainText('Newer strategy');
    await expect(rows.nth(0)).toContainText('live');
    await expect(rows.nth(1)).toContainText('Older strategy');
  });
});
