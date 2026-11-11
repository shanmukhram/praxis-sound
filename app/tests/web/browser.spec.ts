// The browser build end to end, in Chromium with audio muted: the WebAssembly engine loads, the
// acceptance-1 brief makes a take with 8 of 8 checks, the AudioWorklet plays it (rendered in an
// OfflineAudioContext and compared with the offline render, and with the native render of the
// same brief), Play runs the worklet transport, and the sound is kept across a reload.
//
// Run `npm run build:web` first; `npm run test:web` writes the native render, then runs this.

import { test, expect, type Page } from '@playwright/test';
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const BRIEF =
  '16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt piano playing a repeating rhythmic motif on an eighth-note grid, with accents and overlapping sustain. Bars 9-16: the identical piano part, plus a soft muted kick layered underneath. No new melody, no fills. No risers, sweeps, impacts or pads anywhere.';

/** The native render of the same brief (`cargo run -p praxis-wasm --example native_render`). */
const NATIVE = process.env.PRAXIS_NATIVE_RENDER ?? fileURLToPath(new URL('../../../target/praxis-native/acceptance1.f32', import.meta.url));

/** Worklet cycle 2 vs the wasm offline loop: the same code in the same engine. */
const WORKLET_TOLERANCE = 1e-4;
/** WebAssembly vs native (different maths libraries for sin/exp/pow): max sample difference. */
const NATIVE_TOLERANCE = 1e-3;

async function engineReady(page: Page): Promise<{ version: string; wasmBytes: number; compileMs: number; readyMs: number }> {
  await page.waitForFunction(() => !!(window as unknown as { praxis?: { wasm?: unknown } }).praxis?.wasm, null, { timeout: 60_000 });
  return page.evaluate(() => (window as unknown as { praxis: { wasm: { load: never } } }).praxis.wasm.load);
}

test('the browser build makes the acceptance-1 take on the wasm engine and plays it through the worklet', async ({ page }) => {
  page.on('console', (m) => {
    if (m.type() === 'error') console.log(`[page] ${m.text()}`);
  });
  if (existsSync(NATIVE)) {
    await page.route('**/__native__/acceptance1.f32', (r) => r.fulfill({ body: readFileSync(NATIVE), contentType: 'application/octet-stream' }));
  }
  const t0 = Date.now();
  await page.goto('/');
  const load = await engineReady(page);
  console.log(
    `engine ${load.version}: ${(load.wasmBytes / 1048576).toFixed(2)} MiB wasm, compile ${load.compileMs.toFixed(0)} ms, ready ${load.readyMs.toFixed(0)} ms (page to ready ${Date.now() - t0} ms)`,
  );
  await expect(page.getByTestId('boot-error')).toHaveCount(0);

  await page.getByTestId('prompt').fill(BRIEF);
  await page.getByTestId('send').click();
  const current = page.getByTestId('take-current');
  await expect(current).toHaveAttribute('data-take', '1', { timeout: 240_000 });
  await expect(current.getByTestId('checks')).toContainText('8 of 8 checks passed');
  const take = await page.evaluate(() => {
    const e = (window as unknown as { praxis: { engine: { state(): { takes: { render: { report: Record<string, unknown> }; say: string; checks: { checks: { name: string; passed: boolean }[] } }[] } } } }).praxis.engine;
    const t = e.state().takes[0];
    return { report: t.render.report, say: t.say, checks: t.checks.checks.map((c) => `${c.name}:${c.passed}`) };
  });
  console.log(`take 1: ${take.checks.join(' ')}; rendered in ${Number(take.report.render_seconds).toFixed(2)} s`);
  expect(take.report.mock).toBe(false);
  expect(take.checks).toHaveLength(8);
  expect(take.say).not.toMatch(/stand-in/);

  // The AudioWorklet engine, rendered offline: two loop cycles, the second compared with the
  // offline render (the first has no tails from a previous cycle yet).
  const cmp = await page.evaluate(async (hasNative) => {
    type W = {
      praxis: {
        engine: { takeChannels(n: number): Float32Array[] };
        wasm: { renderThroughWorklet(frames: number): Promise<{ left: Float32Array; right: Float32Array; ms: number }> };
      };
    };
    const p = (window as unknown as W).praxis;
    const off = p.engine.takeChannels(1);
    const n = off[0].length;
    const w = await p.wasm.renderThroughWorklet(2 * n);
    const diff = (a: Float32Array, b: Float32Array, from = 0) => {
      let m = 0;
      let e = 0;
      let s = 0;
      for (let i = 0; i < n; i++) {
        const d = a[from + i] - b[i];
        m = Math.max(m, Math.abs(d));
        e += d * d;
        s += b[i] * b[i];
      }
      return { max: m, residualDb: 10 * Math.log10(e / s) };
    };
    const worklet = [diff(w.left, off[0], n), diff(w.right, off[1], n)];
    let native: { max: number; residualDb: number }[] | null = null;
    if (hasNative) {
      const buf = await (await fetch('/__native__/acceptance1.f32')).arrayBuffer();
      const frames = new DataView(buf).getUint32(0, true);
      if (frames === n) {
        const l = new Float32Array(buf, 4, n);
        const r = new Float32Array(buf, 4 + 4 * n, n);
        native = [diff(off[0], l), diff(off[1], r)];
      } else {
        native = [{ max: Infinity, residualDb: 0 }];
      }
    }
    return { worklet, native, workletMs: w.ms, seconds: (2 * n) / 48_000 };
  }, existsSync(NATIVE));
  console.log(`worklet (offline context) vs wasm offline render, cycle 2: ${JSON.stringify(cmp.worklet)}; ${cmp.seconds} s rendered in ${cmp.workletMs.toFixed(0)} ms`);
  for (const c of cmp.worklet) expect(c.max).toBeLessThanOrEqual(WORKLET_TOLERANCE);
  if (cmp.native) {
    console.log(`wasm offline render vs native render: ${JSON.stringify(cmp.native)}`);
    for (const c of cmp.native) expect(c.max).toBeLessThanOrEqual(NATIVE_TOLERANCE);
  } else {
    console.log(`no native render at ${NATIVE}: run \`npm run test:web\` to compare with it`);
  }

  // Play (muted): the worklet transport runs and the playhead moves.
  await current.getByRole('button', { name: 'Play take 1' }).click();
  await expect(page.getByTestId('playback-line')).toContainText('AudioWorklet');
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { praxis: { engine: { playback: { mode: string } } } }).praxis.engine.playback.mode), { timeout: 20_000 })
    .toBe('worklet');
  const pos = () =>
    page.evaluate(() => new Promise<number>((resolve) => {
      const e = (window as unknown as { praxis: { engine: { onPosition(cb: (p: { seconds: number }) => void): () => void } } }).praxis.engine;
      const un = e.onPosition((p) => {
        resolve(p.seconds);
        queueMicrotask(() => un());
      });
    }));
  await expect.poll(pos, { timeout: 20_000 }).toBeGreaterThan(0.5);
  await current.getByRole('button', { name: 'Stop take 1' }).click();

  // Kept in the browser: a reload opens the same sound with its take.
  await page.reload();
  await engineReady(page);
  await expect(page.getByTestId('take-current')).toHaveAttribute('data-take', '1', { timeout: 60_000 });
  await expect(page.getByTestId('take-current').getByTestId('checks')).toContainText('8 of 8 checks passed');
});
