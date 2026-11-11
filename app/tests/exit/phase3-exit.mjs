// Phase 3 exit, by hand, in the release desktop app (WP-U5). Run by hand, not in CI:
//
//   npm run tauri -- build --no-bundle            (target/release/praxis-app.exe)
//   node tests/exit/phase3-exit.mjs [home]
//
// Starts the release exe on the fake output (PRAXIS_AUDIO_OUTPUT=fake: the real-time engine,
// silent) with its own PRAXIS_HOME, and drives the window over the WebView2 debugging port the
// way a person would: a blank sound (A minor, 30 s = 128 BPM, 16 bars, loop); a felt-piano layer
// with the acceptance-1 motif entered in Rhythm (the rhythm, said a bar at a time) and Notes (the
// pitches, note by note); a soft-kick layer in bars 9-16; one take; 60 s of playback through the
// real-time engine; WAV, FLAC, stems and MIDI exported. It prints the numbers and writes
// screenshots to app/screenshots/exit-*.png. Never plays through the speakers.

import { chromium } from '@playwright/test';
import { spawn } from 'node:child_process';
import { mkdirSync, rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '../../..');
const exe = path.join(repo, 'target', 'release', 'praxis-app.exe');
const home = path.resolve(process.argv[2] ?? path.join(repo, 'target', 'exit-home'));
const shot = (name) => path.join(repo, 'app', 'screenshots', name);
const PORT = 9223;

rmSync(home, { recursive: true, force: true });
mkdirSync(path.join(home, 'sounds'), { recursive: true });

const app = spawn(exe, [], {
  env: {
    ...process.env,
    PRAXIS_HOME: home,
    PRAXIS_AUDIO_OUTPUT: 'fake',
    PRAXIS_PACKS: process.env.PRAXIS_PACKS ?? path.join(repo, 'packs'),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`,
  },
  stdio: 'inherit',
});

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (...a) => console.log('[exit]', ...a);

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

const MOTIF = [57, 64, 69, 64, 72, 64, 69, 64, 53, 60, 65, 60, 69, 60, 67, 64];
const BASS = [
  [0, 45],
  [8, 41],
];
const NAMES = ['C', 'C#', 'D', 'Eb', 'E', 'F', 'F#', 'G', 'Ab', 'A', 'Bb', 'B'];
const midiName = (p) => `${NAMES[p % 12]}${Math.floor(p / 12) - 1}`;

let browser;
try {
  browser = await connect();
  const page = browser.contexts()[0].pages()[0];
  await page.waitForSelector('[data-testid=new-sound]');
  const tid = (id) => page.getByTestId(id);
  const state = () => page.evaluate(() => {
    const e = window.praxis.engine;
    const s = e.state();
    return { takes: s.takes.length, current: s.current, draft: !!s.draft, score: s.score, last: s.takes.at(-1) ?? null, sound: e.soundPath(), playback: e.playback() };
  });

  // A blank sound: A minor, 16 bars of 4/4 fitted to 30 s (128 BPM), a loop.
  await tid('new-sound').click();
  await tid('start-blank').click();
  await tid('blank-title').fill('Acceptance 1 by hand');
  await tid('blank-tonic').selectOption('A');
  await tid('blank-mode').selectOption('minor');
  await tid('blank-bars').fill('16');
  await tid('blank-by-length').click();
  await tid('blank-seconds').fill('30');
  await tid('blank-loop').click();
  log(await tid('blank-length-line').textContent());
  await page.screenshot({ path: shot('exit-1-blank-form.png') });
  await tid('blank-go').click();
  await page.waitForFunction(() => window.praxis.engine.state().draft === true);
  await page.waitForSelector('[data-testid=sound-settings]');
  log('blank sound at', (await state()).sound);

  // The felt piano, its rhythm said a bar at a time.
  await tid('add-layer').click();
  await tid('quick-felt_piano').click();
  await page.waitForSelector('[data-testid=cell-felt_piano-1].on');
  await tid('tab-rhythm').click();
  for (const [bar, words] of [
    ['0', 'DON din don din TIN din don din'],
    ['1', 'DON din don din TIN din don din'],
  ]) {
    await tid('say-bar-which').selectOption(bar);
    await tid('say-bar').fill(words);
    await tid('say-bar-go').click();
    await page.waitForFunction((b) => document.querySelector(`[data-testid=step-${Number(b) * 8}]`)?.classList.contains('strong'), bar);
  }
  await page.screenshot({ path: shot('exit-2-rhythm.png') });

  // The pitches in Notes: each note moved off the tonic (remove, then add at its pitch), then the
  // two low notes under the bar starts.
  await tid('tab-notes').click();
  const roll = tid('roll');
  const pitchesNow = () => page.evaluate(() => window.praxis.engine.state().score.patterns.find((p) => p.id === 'felt_piano_p1').notes.map((n) => n.pitch));
  /** Clicks the roll where (step of the motif, pitch) is, using the roll's own layout (notes.ts). */
  const range = async () => ({ lo: Number(await roll.getAttribute('data-lo')), hi: Number(await roll.getAttribute('data-hi')) });
  const clickAt = async (step, pitch) => {
    // Move the pitch window an octave at a time until the pitch is in it, as a person would.
    for (let i = 0; i < 6; i++) {
      const r = await range();
      if (pitch < r.lo + 1) await tid('notes-down').click();
      else if (pitch > r.hi - 1) await tid('notes-up').click();
      else break;
    }
    const { lo, hi } = await range();
    const H = 220;
    const box = await roll.boundingBox();
    const y = H - 14 - ((pitch - lo) / (hi - lo)) * (H - 28);
    const x = ((step + 0.5) / 64) * box.width; // the roll shows bars 1-8, 8 steps a bar
    await roll.click({ position: { x, y } });
  };
  for (let step = 0; step < 16; step++) {
    const want = MOTIF[step];
    if (want === 57) continue;
    const bar = step < 8 ? 1 : 2;
    const before = (await pitchesNow()).length;
    await page.locator(`.roll .n[aria-label="A3 at bar ${bar}, step ${(step % 8) + 1}"]`).click();
    await page.waitForFunction((n) => window.praxis.engine.state().score.patterns.find((p) => p.id === 'felt_piano_p1').notes.length === n, before - 1);
    await clickAt(step, want);
    await page.waitForFunction((n) => window.praxis.engine.state().score.patterns.find((p) => p.id === 'felt_piano_p1').notes.length === n, before);
  }
  for (const [step, pitch] of BASS) {
    const before = (await pitchesNow()).length;
    await clickAt(step, pitch);
    await page.waitForFunction((n) => window.praxis.engine.state().score.patterns.find((p) => p.id === 'felt_piano_p1').notes.length === n, before + 1);
  }
  const motif = (await state()).score.patterns.find((p) => p.id === 'felt_piano_p1').notes.map((n) => [n.step, n.pitch]);
  log('motif', JSON.stringify(motif));
  const wanted = [...MOTIF.map((p, s) => [s, p]), ...BASS].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const got = [...motif].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  log('motif matches acceptance 1:', JSON.stringify(got) === JSON.stringify(wanted), got.map(([s, p]) => `${s}:${midiName(p)}`).join(' '));
  await page.screenshot({ path: shot('exit-3-notes.png') });

  // The soft kick in bars 9-16, a little lower, its beat said in Rhythm.
  await tid('tab-overview').click();
  await tid('add-layer').click();
  await tid('quick-soft_kick').click();
  await tid('edit-soft_kick').click();
  await tid('layer-range-soft_kick-second').click();
  await page.waitForSelector('[data-testid=cell-soft_kick-9].on');
  await tid('layer-gain-soft_kick').fill('-4');
  await page.screenshot({ path: shot('exit-4-layers.png') });
  await tid('edit-soft_kick').click();
  await tid('tab-rhythm').click();
  await tid('view-layer-soft_kick').click();
  await tid('say-bar').fill('DON - - - don - - -');
  await tid('say-bar-go').click();
  await page.waitForFunction(() => document.querySelector('[data-testid=step-4]')?.classList.contains('normal'));
  let st = await state();
  log('before the take: takes', st.takes, 'draft', st.draft, 'layers', st.score.layers.map((l) => `${l.id} ${JSON.stringify(l.clips.map((c) => [c.bars.start, c.bars.end]))} ${l.gain_db} dB`).join('; '));

  // One take for all of it.
  const t0 = Date.now();
  await tid('make-take').click();
  await page.waitForSelector('[data-testid=take-current][data-take="1"]', { timeout: 180_000 });
  st = await state();
  log(`take 1 in ${((Date.now() - t0) / 1000).toFixed(1)} s:`, st.last.title, '|', st.last.say);
  log('checks:', st.last.checks.passed ? 'all passed' : 'FAILED', st.last.checks.checks.map((c) => `${c.name} ${c.passed ? 'ok' : 'FAIL'} (${c.measured})`).join('; '));
  await page.waitForTimeout(800);
  await page.screenshot({ path: shot('exit-5-take.png') });

  // 60 s through the real-time engine (fake output: silent), from the keyboard.
  await page.locator('[data-testid=thread]').click({ position: { x: 4, y: 4 } });
  await page.keyboard.press('Space');
  await page.waitForFunction(() => window.praxis.engine.playback().mode === 'engine' && document.querySelector('[data-testid=transport-play]')?.getAttribute('aria-label') === 'Stop (Space)');
  const startStatus = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke('transport_status'));
  log('playing on', startStatus.device, startStatus.sample_rate, 'Hz, callbacks', startStatus.min_callback_frames, '-', startStatus.max_callback_frames, 'frames');
  await page.waitForTimeout(30_000);
  await page.screenshot({ path: shot('exit-6-playing.png') });
  await page.waitForTimeout(30_000);
  const status = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke('transport_status'));
  await page.keyboard.press('Space');
  log(
    `after 60 s: playing ${status.playing}, xruns while playing ${status.xruns}, xruns total ${status.xruns_total}, callbacks ${status.callbacks}, render p99 ${status.p99_render_ms?.toFixed(3)} ms, max ${status.max_render_ms?.toFixed(3)} ms (budget ${((status.max_callback_frames / status.sample_rate) * 1000).toFixed(1)} ms), device errors ${status.device_errors}, swaps ${status.swaps}, position ${status.seconds?.toFixed(1)} s`,
  );

  // Export WAV (24-bit) with stems and MIDI, then FLAC.
  await tid('tab-export').click();
  await tid('export-wav24').check();
  await tid('export-stems').check();
  await tid('export-midi').check();
  await tid('export-go').click();
  await page.waitForFunction(() => document.querySelector('[data-testid=export-result] .files li'), null, { timeout: 180_000 });
  const first = await tid('export-result').innerText();
  await tid('export-flac').check();
  await tid('export-stems').uncheck();
  await tid('export-midi').uncheck();
  await tid('export-go').click();
  await page.waitForFunction(() => /\.flac/.test(document.querySelector('[data-testid=export-result]')?.textContent ?? ''), null, { timeout: 180_000 });
  const second = await tid('export-result').innerText();
  log('export 1:\n' + first);
  log('export 2:\n' + second);
  await page.screenshot({ path: shot('exit-7-export.png') });
  log('sound folder:', (await state()).sound);
} finally {
  await browser?.close().catch(() => undefined);
  app.kill();
}
