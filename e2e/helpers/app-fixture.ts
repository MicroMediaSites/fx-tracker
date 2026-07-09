/**
 * Custom Playwright fixture for wickd E2E tests
 *
 * Provides an `appPage` fixture that:
 * 1. Injects Tauri bridge (IPC mock, includes the local-store stand-in)
 * 2. Pre-seeds localStorage for Zustand stores
 * 3. Provides helpers for per-test overrides
 */

import { test as base, type Page } from '@playwright/test';
import { getTauriBridgeScript } from '../mocks/tauri-bridge';

export type AppFixtures = {
  appPage: AppPage;
};

export class AppPage {
  constructor(public page: Page) {}

  /**
   * Override a Tauri command response for this test.
   * Must be called BEFORE goto() — uses addInitScript which only
   * takes effect on the next navigation.
   */
  async mockTauriCommand(name: string, response: unknown) {
    await this.page.addInitScript(
      ({ name, response }) => {
        window.__E2E_TAURI_OVERRIDES__ = window.__E2E_TAURI_OVERRIDES__ || {};
        (window.__E2E_TAURI_OVERRIDES__ as Record<string, unknown>)[name] = response;
      },
      { name, response },
    );
  }

  /**
   * Pre-seed an arbitrary local-store dataset global (AGT-650), e.g.
   * '__E2E_LOCAL_STRATEGY_WATCHERS__'. Must be called BEFORE goto().
   */
  async setLocalDataset(globalName: string, data: unknown) {
    await this.page.addInitScript(
      ({ globalName, data }) => {
        (window as unknown as Record<string, unknown>)[globalName] = data;
      },
      { globalName, data },
    );
  }

  /**
   * Pre-seed the local-store strategies mock (AGT-642/645).
   * Must be called BEFORE goto().
   */
  async setLocalStrategies(rows: unknown[]) {
    await this.page.addInitScript((data) => {
      (window as Window & { __E2E_LOCAL_STRATEGIES__?: unknown[] }).__E2E_LOCAL_STRATEGIES__ =
        data;
    }, rows);
  }

  /**
   * Pre-seed the unified `.rhai` strategy store mock (AGT-651). Entries may
   * carry a `source` field consumed by store_read_strategy.
   * Must be called BEFORE goto().
   */
  async setStoreStrategies(rows: unknown[]) {
    await this.page.addInitScript((data) => {
      (window as Window & { __E2E_STORE_STRATEGIES__?: unknown[] }).__E2E_STORE_STRATEGIES__ =
        data;
    }, rows);
  }

  /**
   * Pre-seed the local-store saved backtest runs mock (AGT-645).
   * Must be called BEFORE goto().
   */
  async setLocalBacktests(rows: unknown[]) {
    await this.page.addInitScript((data) => {
      (window as Window & { __E2E_LOCAL_BACKTESTS__?: unknown[] }).__E2E_LOCAL_BACKTESTS__ =
        data;
    }, rows);
  }

  /**
   * Navigate to a specific window type
   */
  async goto(windowType?: string) {
    const url = windowType ? `/?window=${windowType}` : '/';
    await this.page.goto(url);
    await this.page.waitForLoadState('domcontentloaded');
  }

  /**
   * Emit a Tauri event to all registered listeners in the page.
   * Simulates cross-window event broadcast (e.g., environment-changed).
   * Must be called AFTER goto() — requires the app to be loaded.
   */
  async emitTauriEvent(eventName: string, payload: unknown) {
    await this.page.evaluate(
      ({ eventName, payload }) => {
        const emitFn = (window as Window & { __E2E_EMIT_EVENT__?: (event: string, payload?: unknown) => Promise<void> }).__E2E_EMIT_EVENT__;
        if (emitFn) {
          emitFn(eventName, payload);
        } else {
          throw new Error('__E2E_EMIT_EVENT__ not available - ensure tauri-api-event mock is loaded');
        }
      },
      { eventName, payload },
    );
  }
}

export const test = base.extend<AppFixtures>({
  appPage: async ({ page }, use) => {
    // Inject Tauri bridge before page loads
    await page.addInitScript(getTauriBridgeScript());

    // Pre-seed localStorage for Zustand stores
    await page.addInitScript(() => {
      const vaultState = JSON.stringify({
        state: {
          status: 'unlocked',
          deviceId: 'e2e-device-001',
          hasPracticeCredentials: true,
          hasLiveCredentials: false,
          unlockedAt: Date.now(),
          rateLimitMessage: null,
          rateLimitSeconds: 0,
          error: null,
        },
        version: 0,
      });

      const settingsState = JSON.stringify({
        state: {
          dataSource: 'demo',
          completedTours: { account: true, chart: true, backtest: true, watcher: true, tradeanalysis: true },
          startupWindows: ['watcher'],
          chartDefaultTimeframe: 'H4',
          chartDefaultInstrument: 'EUR_USD',
          desktopNotifications: false,
          devUsePracticeUrlForLive: false,
        },
        version: 2,
      });

      localStorage.setItem('candlesight-vault', vaultState);
      localStorage.setItem('candlesight-settings', settingsState);
      localStorage.setItem('credentials_setup_complete_e2e-device-001', 'true');
    });

    const appPage = new AppPage(page);
    await use(appPage);
  },
});

export { expect } from '@playwright/test';
