// Which engine the workstation runs on: the real engine through Tauri inside the desktop shell;
// in a browser the WebAssembly engine in the web build (`vite build --mode web`, `npm run
// build:web`) and the mock in the dev server the UI tests drive. `?engine=wasm` or `?engine=mock`
// overrides the browser default (inside Tauri the desktop engine always runs).

export type EngineChoice = 'tauri' | 'wasm' | 'mock';

export function chooseEngine(opts: { tauri: boolean; mode: string; param: string | null }): EngineChoice {
  if (opts.tauri) return 'tauri';
  if (opts.param === 'wasm' || opts.param === 'mock') return opts.param;
  return opts.mode === 'web' ? 'wasm' : 'mock';
}
