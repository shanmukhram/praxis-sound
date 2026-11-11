// Finds the wasm-bindgen output in ./pkg/ (written by `npm run build:wasm`). Globs, not imports,
// so the app still type-checks and builds without it; the browser build then says the engine
// is missing instead of failing to load.

import type { PraxisGlue } from './wasm-api';

const glueModules = import.meta.glob<PraxisGlue>('./pkg/praxis_wasm.js');
const wasmUrls = import.meta.glob<string>('./pkg/praxis_wasm_bg.wasm', { query: '?url', import: 'default', eager: true });

/** True when the wasm build is present in this bundle. */
export function wasmBuilt(): boolean {
  return Object.keys(glueModules).length > 0 && Object.keys(wasmUrls).length > 0;
}

/** URL of the wasm binary, or null when it was not built. */
export function wasmUrl(): string | null {
  return Object.values(wasmUrls)[0] ?? null;
}

/** The generated JS glue (not yet initialised). */
export async function loadGlue(): Promise<PraxisGlue> {
  const load = Object.values(glueModules)[0];
  if (!load) throw new Error(NOT_BUILT);
  return load();
}

export const NOT_BUILT =
  'The browser engine (WebAssembly) is not in this build. Run `npm run build:wasm` in app/ (it needs the wasm32-unknown-unknown Rust target and wasm-bindgen-cli), then build again.';

/** Fetches and compiles the wasm binary once; the worker and the worklet both use the module. */
export async function compileWasm(url = wasmUrl()): Promise<{ module: WebAssembly.Module; bytes: number; ms: number }> {
  if (!url) throw new Error(NOT_BUILT);
  const t0 = performance.now();
  const res = await fetch(url);
  if (!res.ok) throw new Error(`could not load the engine (${url}): HTTP ${res.status}`);
  const buf = await res.arrayBuffer();
  const module = await WebAssembly.compile(buf);
  return { module, bytes: buf.byteLength, ms: performance.now() - t0 };
}
