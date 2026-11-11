// Browser entry: the workstation on the WebAssembly engine. Compiles the wasm module once,
// starts the offline engine in a worker, keeps sounds in IndexedDB, fetches sample packs from the
// served `packs/` folder, and plays through the engine in an AudioWorklet (started on the first
// Play, which is a user gesture).

import { Shell, type SoundEntry } from '../ui/shell';
import { h } from '../ui/dom';
import { compileWasm, NOT_BUILT, wasmBuilt, wasmUrl } from './glue';
import { workerClient } from './workerClient';
import { PackLoader, httpFetcher } from './packs';
import { openStore, type SoundSummary } from './store';
import { WasmEngine, type BrowserPlayback } from './WasmEngine';
import { renderThroughWorklet, WorkletPlayer } from './player';

/** Load measurements, shown in the devtools and read by the browser tests. */
export interface WasmLoadInfo {
  version: string;
  wasmBytes: number;
  /** Fetch and compile of the module, milliseconds. */
  compileMs: number;
  /** From the start of the entry to the engine answering in its worker, milliseconds. */
  readyMs: number;
}

function entry(engine: WasmEngine, s: SoundSummary): SoundEntry {
  return {
    id: s.id,
    name: s.name,
    sub: s.sub,
    open: () =>
      engine.openSound(s.id).catch((e: unknown) => {
        console.error(e instanceof Error ? e.message : e);
        return engine.openSound(null);
      }),
  };
}

function fail(root: HTMLElement, text: string): void {
  root.replaceChildren(h('div', { class: 'boot-error', role: 'alert', 'data-testid': 'boot-error', style: 'padding:32px;max-width:640px;line-height:1.5' }, text));
}

/** Which engine plays, under the Sound panel's title (the desktop has its own line). */
function mountPlaybackLine(root: HTMLElement, engine: WasmEngine): void {
  const head = root.querySelector('.panel-head > div');
  if (!head) return;
  const line = h('small', { 'data-testid': 'playback-line', style: 'display:block;opacity:.75;margin-top:2px' });
  head.append(line);
  const text = (p: BrowserPlayback) =>
    p.message ?? (p.mode === 'worklet' ? `Browser engine (WebAssembly) · AudioWorklet · ${p.sampleRate / 1000} kHz` : 'Browser engine (WebAssembly)');
  engine.onPlayback((p) => {
    line.textContent = text(p);
  });
}

export async function startBrowser(root: HTMLElement): Promise<{ shell: Shell; engine: WasmEngine; wasm: unknown } | null> {
  const t0 = performance.now();
  if (!wasmBuilt()) {
    fail(root, NOT_BUILT);
    return null;
  }
  let compiled: Awaited<ReturnType<typeof compileWasm>>;
  let client: Awaited<ReturnType<typeof workerClient>>;
  try {
    compiled = await compileWasm();
    client = await workerClient(compiled.module);
  } catch (e) {
    fail(root, `The browser engine could not start: ${e instanceof Error ? e.message : String(e)}`);
    return null;
  }
  const load: WasmLoadInfo = { version: client.version, wasmBytes: compiled.bytes, compileMs: compiled.ms, readyMs: performance.now() - t0 };
  console.info(`Praxis engine ${load.version}: ${(load.wasmBytes / 1048576).toFixed(2)} MiB wasm, compiled in ${load.compileMs.toFixed(0)} ms, ready in ${load.readyMs.toFixed(0)} ms`);

  const store = await openStore();
  const packs = new PackLoader(client.client, httpFetcher());
  const bytes = async () => (await fetch(wasmUrl()!)).arrayBuffer();
  const engine = new WasmEngine({
    core: client.client,
    packs,
    store,
    player:
      typeof AudioContext === 'undefined' || typeof AudioWorkletNode === 'undefined'
        ? undefined
        : async () => WorkletPlayer.create(new AudioContext({ latencyHint: 'interactive' }), compiled.module, bytes),
  });

  const list = await engine.listSounds();
  const sounds: SoundEntry[] = list.map((s) => entry(engine, s));
  sounds.push({ id: 'new', name: 'Untitled sound', sub: 'New', open: () => engine.openSound(null) });
  const shell = new Shell(root, engine, sounds);
  mountPlaybackLine(root, engine);
  engine.onSoundCreated((s) => {
    if (!sounds.some((x) => x.id === s.id)) sounds.unshift(entry(engine, s));
    void shell.openSound(s.id);
  });
  const wanted = new URLSearchParams(location.search).get('sound');
  await shell.openSound(wanted && sounds.some((s) => s.id === wanted) ? wanted : (list[0]?.id ?? 'new'));

  // For the devtools console and the browser tests; not an API.
  const wasm = {
    load,
    module: compiled.module,
    packs,
    /** Renders the live score through the worklet engine in an OfflineAudioContext. */
    renderThroughWorklet: (frames: number, sampleRate = 48_000) =>
      renderThroughWorklet({
        module: compiled.module,
        bytes,
        score: engine.current(),
        masterGainDb: engine.playerGainDb(),
        frames,
        sampleRate,
        packFiles: packs.all(),
      }),
  };
  return { shell, engine, wasm };
}
