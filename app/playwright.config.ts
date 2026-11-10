import { defineConfig, devices } from '@playwright/test';

// Browser: Playwright's bundled Chromium (`npx playwright install chromium` once), or set
// PW_CHANNEL=msedge / chrome to use an installed browser instead.
const channel = process.env.PW_CHANNEL || undefined;

export default defineConfig({
  testDir: 'tests/e2e',
  timeout: 60_000,
  fullyParallel: false,
  reporter: [['list']],
  use: {
    baseURL: 'http://localhost:5173',
    viewport: { width: 1440, height: 900 },
    colorScheme: 'dark',
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'browser', use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 900 }, channel } }],
  webServer: {
    command: 'npm run dev',
    url: 'http://localhost:5173',
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
