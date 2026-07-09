import { defineConfig, devices } from '@playwright/test';

// Overridable so concurrent worktrees (parallel build agents) can run E2E
// side by side without fighting over one port.
const E2E_PORT = process.env.E2E_PORT || '1422';

export default defineConfig({
  testDir: './e2e/tests',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI ? 'html' : 'list',
  timeout: 60000,

  use: {
    baseURL: `http://localhost:${E2E_PORT}`,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },

  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],

  webServer: {
    command: `npx vite --config vite.config.e2e.ts --mode e2e --port ${E2E_PORT} --strictPort`,
    url: `http://localhost:${E2E_PORT}`,
    reuseExistingServer: !process.env.CI,
    timeout: 30000,
    env: {
    },
  },
});
