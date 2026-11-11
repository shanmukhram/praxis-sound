// Phase 5 in the release desktop app (WP-U6). Run by hand, not in CI:
//
//   cargo build --release -p praxis-cli               (target/release/praxis.exe)
//   npm run tauri -- build --no-bundle                 (target/release/praxis-app.exe)
//   node tests/exit/reference-clips.mjs [home]
//
// Renders three clips with the engine (no downloads): the acceptance-1 felt-piano loop (music),
// an impact (a sound effect) and a long moving noise bed (ambience), from the scores in the
// repository. Starts the release exe on the fake output (PRAXIS_AUDIO_OUTPUT=fake: silent) with
// its own PRAXIS_HOME and the WebView muted, and drives the window over the WebView2 debugging
// port: drops each clip on the window, reads its profile view and takes a screenshot; makes a
// take "like" the music clip, runs the loudness-matched A/B against it and reads the trim the
// audio service holds; saves the ambience to the reference library. Prints what it read, writes
// app/screenshots/release-reference-*.png. Never plays through the speakers, never calls the
// Claude API (PRAXIS_KEYCHAIN=off; the offline planner answers).

import { chromium } from '@playwright/test';
import { spawn, spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '../../..');
const exe = path.join(repo, 'target', 'release', 'praxis-app.exe');
const cli = path.join(repo, 'target', 'release', 'praxis.exe');
const home = path.resolve(process.argv[2] ?? path.join(repo, 'target', 'u6-release-home'));
const shot = (name) => path.join(repo, 'app', 'screenshots', name);
const PORT = 9224;
const packs = process.env.PRAXIS_PACKS ?? path.join(repo, 'packs');
const env = { ...process.env, PRAXIS_HOME: home, PRAXIS_KEYCHAIN: 'off', ANTHROPIC_API_KEY: '', PRAXIS_PACKS: packs };

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (...a) => console.log('[u6]', ...a);

rmSync(home, { recursive: true, force: true });
const clipsDir = path.join(home, 'made-clips');
mkdirSync(path.join(home, 'sounds'), { recursive: true });
mkdirSync(clipsDir, { recursive: true });

// The three clips, rendered by the engine.
const CLIPS = [
  { kind: 'music', file: 'felt piano loop.wav', score: 'fixtures/acceptance1.score.json' },
  { kind: 'sfx', file: 'impact.wav', score: 'crates/praxis-analyze/tests/clips/s05_impact.score.json' },
  { kind: 'ambience', file: 'moving noise bed.wav', score: 'crates/praxis-analyze/tests/clips/a09_long_moving_noise_bed.score.json' },
];
for (const c of CLIPS) {
  const out = path.join(clipsDir, c.file);
  const r = spawnSync(cli, ['render', path.join(repo, c.score), '-o', out], { env, encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`render ${c.score}: ${r.stderr || r.stdout}`);
  c.path = out;
  log('rendered', c.file);
}

const app = spawn(exe, [], {
  env: {
    ...env,
    PRAXIS_AUDIO_OUTPUT: 'fake',
    // The WebView plays the clip side of the A/B through <audio>: muted here.
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT} --mute-audio`,
  },
  stdio: 'inherit',
});

async function connect() {
  for (let i = 0; i < 60; i++) {
    try {
      return await chromium.connectOverCDP(`http://127.0.0.1:${PORT}`);
    } catch {
      await sleep(500);
    }
  }
  throw new Error('the window did not open its debugging port');
}

let browser;
const report = {};
try {
  browser = await connect();
  const page = browser.contexts()[0].pages()[0];
  await page.waitForSelector('[data-testid=new-sound]');
  const tid = (id) => page.getByTestId(id);
  const drop = (name, bytes) =>
    page.evaluate(
      ({ name, b64 }) => {
        const data = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
        const dt = new DataTransfer();
        dt.items.add(new File([data], name, { type: 'audio/wav' }));
        window.dispatchEvent(new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true }));
      },
      { name, b64: bytes.toString('base64') },
    );
  const lines = () =>
    page.evaluate(() =>
      Object.fromEntries([...document.querySelectorAll('[data-testid=profile-lines] dd')].map((d) => [d.dataset.line, d.querySelector('span')?.textContent ?? ''])),
    );

  await tid('new-sound').click();
  for (const c of CLIPS) {
    const t0 = Date.now();
    await drop(c.file, readFileSync(c.path));
    await page.waitForFunction((n) => document.querySelector('[data-testid=ref-name]')?.textContent === n, c.file, { timeout: 120_000 });
    const ms = Date.now() - t0;
    const kind = await tid('ref-kind').textContent();
    report[c.kind] = { file: c.file, measuredMs: ms, kind, lines: await lines(), chip: await tid('clip-chip').textContent() };
    log(c.file, '->', kind, `(${ms} ms)`);
    await tid('panel-body').evaluate((b) => (b.scrollTop = 0));
    await sleep(200);
    await page.screenshot({ path: shot(`release-reference-${c.kind}.png`) });
  }

  // Save the ambience (the clip on screen now) to the library.
  await tid('ref-save-open').click();
  await tid('ref-save-name').fill('Moving noise bed');
  await tid('ref-save-tags').fill('noise, bed, night');
  await tid('ref-save-go').click();
  await page.waitForSelector('[data-testid=library] li');
  report.library = await page.evaluate(() => [...document.querySelectorAll('[data-testid=library] li')].map((l) => l.textContent));
  log('library', report.library);

  // A take like the music clip, then the A/B against it.
  await drop(CLIPS[0].file, readFileSync(CLIPS[0].path));
  await page.waitForFunction((n) => document.querySelector('[data-testid=ref-name]')?.textContent === n, CLIPS[0].file, { timeout: 120_000 });
  await tid('ref-like').click();
  await page.waitForSelector('[data-testid=take-current]', { timeout: 180_000 });
  report.take = await page.evaluate(() => {
    const t = window.praxis.engine.state().takes.at(-1);
    return { n: t.n, ask: t.ask, reference: t.reference, title: t.title, lufs: t.checks.integrated_lufs };
  });
  log('take', report.take);
  // The new sound opens on its Overview; the reference waits in its own tab.
  await tid('tab-reference').click();
  report.abWords = await tid('ab-levels').textContent();
  await tid('ab-play').click();
  await sleep(1500);
  const status = () => page.evaluate(() => window.__TAURI_INTERNALS__.invoke('transport_status'));
  const a = await status();
  await tid('ab-b').click();
  await sleep(500);
  const b = await status();
  report.ab = { words: report.abWords, a: { playing: a.playing, compare: a.compare, rms_db: a.rms_db }, b: { playing: b.playing, compare: b.compare, rms_db: b.rms_db } };
  log('A/B', JSON.stringify(report.ab));
  await tid('ab').scrollIntoViewIfNeeded();
  await sleep(200);
  await page.screenshot({ path: shot('release-reference-ab.png') });
  await tid('ab-play').click();
  await sleep(300);
  report.afterStop = (await status()).compare;
  console.log(JSON.stringify(report, null, 2));
} finally {
  await browser?.close().catch(() => undefined);
  app.kill();
}
