import { defineConfig, devices } from '@playwright/test';
import { fileURLToPath } from 'node:url';

// The browser build (`npm run build:web`) served by `vite preview --mode web`, in Chromium with
// its audio muted: nothing is ever heard. Playwright's bundled Chromium (`npx playwright install
// chromium` once), or PW_CHANNEL=msedge / chrome for an installed browser. Sample packs are
// served from PRAXIS_PACKS (else the repository's packs/ folder).

const appDir = fileURLToPath(new URL('../..', import.meta.url));
const channel = process.env.PW_CHANNEL || undefined;
const launchOptions = { args: ['--mute-audio', '--autoplay-policy=no-user-gesture-required'] };

export default defineConfig({
  testDir: '.',
  testMatch: '*.spec.ts',
  timeout: 300_000,
  fullyParallel: false,
  workers: 1,
  reporter: [['list']],
  use: {
    baseURL: 'http://localhost:4174',
    viewport: { width: 1440, height: 900 },
    colorScheme: 'dark',
    trace: 'retain-on-failure',
    launchOptions,
  },
  projects: [{ name: 'web', use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 900 }, channel, launchOptions } }],
  webServer: {
    command: 'npm run preview:web',
    cwd: appDir,
    url: 'http://localhost:4174',
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
