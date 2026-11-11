// The browser adapter on the real engine: crates/praxis-wasm compiled to WebAssembly and run in
// Node (the same module the browser loads), with the take bookkeeping, pack loading, patches and
// the store of app/src/engine-wasm. Needs `npm run build:wasm` first; skipped (and says so) when
// the wasm build is missing. The piano pack comes from PRAXIS_PACKS or the nearest packs/ folder
// that holds it; without it the piano plays the synth stand-in and those tests check that.

import { describe, expect, it, beforeAll } from 'vitest';
// Node's file and path modules, typed here (the app's tsconfig has no Node types).
interface NodeFs {
  existsSync(p: string): boolean;
  readFileSync(p: string): Uint8Array;
}
interface NodePath {
  dirname(p: string): string;
  join(...p: string[]): string;
  resolve(...p: string[]): string;
}
interface NodeUrl {
  fileURLToPath(u: string): string;
}
const proc = (globalThis as unknown as { process: { env: Record<string, string | undefined>; getBuiltinModule(id: string): unknown } }).process;
const { existsSync, readFileSync } = proc.getBuiltinModule('node:fs') as NodeFs;
const { dirname, join, resolve } = proc.getBuiltinModule('node:path') as NodePath;
const { fileURLToPath } = proc.getBuiltinModule('node:url') as NodeUrl;
import type { PraxisGlue } from '../../src/engine-wasm/wasm-api';
import { STATUS } from '../../src/engine-wasm/wasm-api';
import { PraxisCore, localClient, type CoreClient } from '../../src/engine-wasm/core';
import { PackLoader, type PackFetcher } from '../../src/engine-wasm/packs';
import { MemoryStore } from '../../src/engine-wasm/store';
import { WasmEngine } from '../../src/engine-wasm/WasmEngine';
import { chooseEngine } from '../../src/engine-wasm/choose';
import type { Take } from '../../src/engine/EngineAdapter';

const BRIEF =
  '16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt piano playing a repeating rhythmic motif on an eighth-note grid, with accents and overlapping sustain. Bars 9-16: the identical piano part, plus a soft muted kick layered underneath. No new melody, no fills. No risers, sweeps, impacts or pads anywhere.';

const here = dirname(fileURLToPath(import.meta.url));
const wasmPath = resolve(here, '../../src/engine-wasm/pkg/praxis_wasm_bg.wasm');
const glues = import.meta.glob<PraxisGlue>('../../src/engine-wasm/pkg/praxis_wasm.js');
const built = existsSync(wasmPath) && Object.keys(glues).length > 0;

function packRoot(): string | null {
  const env = proc.env.PRAXIS_PACKS;
  const has = (d: string) => existsSync(join(d, 'upright-piano-kw', 'pack.json'));
  if (env) return has(env) ? env : null;
  let d = here;
  for (let i = 0; i < 10; i++) {
    if (has(join(d, 'packs'))) return join(d, 'packs');
    const up = dirname(d);
    if (up === d) break;
    d = up;
  }
  return null;
}

function fsFetcher(root: string | null): PackFetcher {
  return async (path) => {
    if (!root) return null;
    try {
      const b = readFileSync(join(root, ...path.split('/')));
      return b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) as ArrayBuffer;
    } catch {
      return null;
    }
  };
}

let glue: PraxisGlue;
let memory: WebAssembly.Memory;
let client: CoreClient;
const packs = packRoot();

beforeAll(async () => {
  if (!built) return;
  glue = await Object.values(glues)[0]();
  memory = glue.initSync({ module: readFileSync(wasmPath) as Uint8Array<ArrayBuffer> }).memory;
  client = localClient(new PraxisCore(glue));
});

/** An engine with its own wasm `Engine` (its own pack files) over one compiled module. */
function engine(opts: { packs?: string | null; store?: MemoryStore } = {}): WasmEngine {
  const store = opts.store ?? new MemoryStore();
  const core = localClient(new PraxisCore(glue));
  return new WasmEngine({ core, packs: new PackLoader(core, fsFetcher(opts.packs === undefined ? packs : opts.packs)), store });
}

describe('choosing the engine', () => {
  it('runs Tauri in the shell, the wasm engine in the web build and the mock in the dev server', () => {
    expect(chooseEngine({ tauri: true, mode: 'web', param: 'mock' })).toBe('tauri');
    expect(chooseEngine({ tauri: false, mode: 'web', param: null })).toBe('wasm');
    expect(chooseEngine({ tauri: false, mode: 'development', param: null })).toBe('mock');
    expect(chooseEngine({ tauri: false, mode: 'development', param: 'wasm' })).toBe('wasm');
    expect(chooseEngine({ tauri: false, mode: 'web', param: 'mock' })).toBe('mock');
  });
});

describe.skipIf(!built)('WasmEngine on the real engine (wasm in Node)', () => {
  it('makes the acceptance-1 take from one brief with 8 of 8 checks', { timeout: 240_000 }, async () => {
    if (!packs) console.warn('the upright-piano-kw pack is not installed: the piano plays its synth stand-in');
    const e = engine();
    const steps: string[] = [];
    const r = await e.ask({ text: BRIEF, onProgress: (p) => p.kind === 'step' && steps.push(p.words) });
    expect(r.ok, JSON.stringify(r)).toBe(true);
    const take = (r as { take: Take }).take;
    expect(take.n).toBe(1);
    expect(take.render.report.mock).toBe(false);
    expect(take.checks.checks).toHaveLength(8);
    expect(take.checks.checks.filter((c) => c.passed).map((c) => c.name)).toHaveLength(8);
    expect(take.checks.passed).toBe(true);
    expect(take.score.layers.map((l) => l.instrument.preset).sort()).toEqual(['felt_piano', 'soft_kick']);
    // The master gain is frozen after the first render, so later edits keep untouched bars.
    expect(take.score.master.frozen_gain_db).toBeCloseTo(take.render.report.applied_gain_db, 9);
    expect(steps[0]).toMatch(/Planning/);
    expect(steps.some((s) => /Rendering 16 bars/.test(s))).toBe(true);
    if (packs) {
      expect(steps.some((s) => /Loading the upright-piano-kw samples/.test(s))).toBe(true);
      expect(take.say).not.toMatch(/stand-in/);
    }
    const ch = e.takeChannels(1)!;
    expect(ch[0].length).toBe(30 * 48_000);
  });

  it('edits bars 13-16 of the piano in words, keeps the rest, and undoes it', { timeout: 240_000 }, async () => {
    const e = engine();
    await e.ask({ text: BRIEF });
    const piano = e.state().score!.layers.find((l) => l.instrument.preset === 'felt_piano')!.id;
    const r = await e.ask({ text: 'make it softer', scope: { layer: piano, bars: { start: 13, end: 16 } } });
    expect(r.ok, JSON.stringify(r)).toBe(true);
    const take = (r as { take: Take }).take;
    expect(take.n).toBe(2);
    expect(take.title).toMatch(/^Softer, bars 13/);
    expect(take.changed[piano]).toEqual([13, 16]);
    expect(take.untouched).toMatch(/Bars 1–12 (sample-identical|identical note for note) to take 1/);
    expect(take.checks.checks).toHaveLength(8);
    // With the frozen gain, bars 3-12 of the master differ only by float rounding (the loop's
    // reverb tails summed at another level; the native render shows the same 6e-8), and bar 1
    // carries the edited bars' wrapped tails.
    const a = e.takeChannels(1)!;
    const b = e.takeChannels(2)!;
    const bar = 90_000;
    let rounding = 0;
    for (let i = 2 * bar; i < 12 * bar - 1000; i++) rounding = Math.max(rounding, Math.abs(a[0][i] - b[0][i]), Math.abs(a[1][i] - b[1][i]));
    expect(rounding).toBeLessThanOrEqual(1e-6);
    let edited = 0;
    for (let i = 12 * bar; i < 16 * bar; i++) edited = Math.max(edited, Math.abs(a[0][i] - b[0][i]));
    expect(edited).toBeGreaterThan(0.01);
    expect(e.state().canUndo).toBe(true);
    expect(await e.undo()).toBe(true);
    expect(e.state().current).toBe(1);
    expect(await e.redo()).toBe(true);
    expect(e.state().current).toBe(2);
    const back = await e.restore(1);
    expect(back.ok).toBe(true);
    expect(e.state().current).toBe(1);
  });

  it('refuses a patch on a locked layer in the app words and leaves the score as it was', { timeout: 240_000 }, async () => {
    const e = engine({ packs: null });
    await e.ask({ text: BRIEF });
    const id = e.state().score!.layers[0].id;
    const lock = await e.applyPatch({ description: 'lock', origin: 'user', scope: {}, ops: [{ op: 'set_lock', id, locked: true }] } as never);
    expect(lock.ok).toBe(true);
    const before = e.state().score;
    const r = await e.edit({ description: 'mute it', origin: 'model', scope: {}, ops: [{ op: 'update_layer', id, update: { muted: true } }] } as never, 'Mute it');
    expect(r.ok).toBe(false);
    expect((r as { error?: { kind: string } }).error?.kind).toBe('locked_layer');
    expect(e.state().score).toEqual(before);
  });

  it('plays the synth stand-in and says so when the pack is not served', { timeout: 240_000 }, async () => {
    const e = engine({ packs: null });
    const r = await e.ask({ text: BRIEF });
    expect(r.ok).toBe(true);
    const take = (r as { take: Take }).take;
    expect(take.say).toMatch(/upright-piano-kw pack is not on this server/);
    expect(take.render.report.warnings.join(' ')).toMatch(/soft keys synth/);
    // The take's score keeps the felt piano: only the render stands in.
    expect(take.score.layers.some((l) => l.instrument.preset === 'felt_piano')).toBe(true);
    expect(take.checks.checks).toHaveLength(8);
  });

  it('keeps sounds in the store and reopens them with their takes', { timeout: 240_000 }, async () => {
    const store = new MemoryStore();
    const e = engine({ packs: null, store });
    const made: string[] = [];
    e.onSoundCreated((s) => made.push(s.id));
    await e.ask({ text: 'A warm logo sting in D major, 4 seconds' });
    expect(made).toHaveLength(1);
    const list = await e.listSounds();
    expect(list.map((s) => s.id)).toEqual(made);
    expect(list[0].sub).toMatch(/D major/);
    const again = engine({ packs: null, store });
    await again.openSound(made[0]);
    const st = again.state();
    expect(st.takes).toHaveLength(1);
    expect(st.current).toBe(1);
    expect(st.score?.master.frozen_gain_db).not.toBeUndefined();
    expect(await store.loadAudio(made[0], 1)).not.toBeNull();
  });

  it('lists presets from the engine and the starter suggestions', async () => {
    const e = engine({ packs: null });
    const presets = await e.presets();
    expect(presets.some((p) => p.name === 'felt_piano' && p.available)).toBe(true);
    expect(presets.some((p) => p.name === 'soft_kick')).toBe(true);
    expect((await e.suggestions())[0]).toMatch(/felt piano loop/);
    expect(await e.agents()).toEqual([]);
  });

  it('runs the worklet engine block by block through views on the wasm memory', { timeout: 120_000 }, async () => {
    const plan = await client.planBrief('A warm logo sting in D major, 4 seconds', 1);
    expect(plan.ok).toBe(true);
    if (!plan.ok) return;
    const off = await client.render(plan.score, { sample_rate: 48_000 });
    expect(off.ok).toBe(true);
    if (!off.ok) return;
    const rt = new glue.Realtime(48_000);
    const info = JSON.parse(rt.load(JSON.stringify(plan.score), Number(off.report.applied_gain_db)));
    expect(info.ok).toBe(true);
    rt.play();
    const n = off.left.length;
    const left = new Float32Array(n);
    for (let i = 0; i < n; i += 128) {
      const k = rt.render(Math.min(128, n - i));
      // The worklet's way: a view on the engine's own buffer (rebuilt if the memory grew).
      left.set(new Float32Array(memory.buffer, rt.left_ptr(), k), i);
    }
    const status = new Float64Array(memory.buffer, rt.status_ptr(), 12);
    expect(status[STATUS.playing]).toBe(1);
    let d = 0;
    for (let i = 0; i < n - 480; i++) d = Math.max(d, Math.abs(left[i] - off.left[i]));
    expect(d).toBeLessThanOrEqual(1e-6);
    rt.free();
  });
});
