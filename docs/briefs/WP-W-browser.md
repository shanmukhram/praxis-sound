# Brief: WP-W the browser build (Phase 8)

**Start building only after WP-U6 is merged** (app files move until then); then merge
`build/integration` into your branch. Read first: `docs/BRIEFING.md`, PRD "Why Rust" (same engine
in WebAssembly) and Phase 8, ADR-0002 (browser build reuses the UI), `crates/praxis-wasm/`,
`app/src/engine/EngineAdapter.ts` and the MockEngine, the real-time engine
`crates/praxis-engine/src/realtime/`, and how the core crates stay wasm-safe.

## Build

- `praxis-wasm` exposes, through `wasm-bindgen`: validate, compose a ScorePlan into a Score
  (offline planner), apply patches, offline render (to a Float32Array), verify, analyse a clip's
  PCM, and a real-time engine instance whose `process(left, right)` runs inside an AudioWorklet.
- `app/src/engine-wasm/`: a `WasmEngine` adapter implementing `EngineAdapter` (takes kept in the
  browser's storage, audio from the worklet), the AudioWorklet processor, and a build that
  produces the browser app (`npm run build:web` or similar) with the wasm module.
- `app/src/main.ts`: pick the WasmEngine in a browser build, Tauri inside the desktop shell, the
  mock only in tests.
- Instruments that need sample packs load them over HTTP from the served `packs/` folder when
  present, and fall back to the synth presets with a plain message.
- Tooling: generating the JS glue needs `wasm-bindgen-cli` at the exact version of the
  `wasm-bindgen` crate. The owner approved downloads (2026-11-11): install it from crates.io
  (`cargo install wasm-bindgen-cli --version <exact> --locked`, MIT/Apache-2.0, 5 jobs) and record
  it in your report for `THIRD_PARTY.md`.

## You own

`crates/praxis-wasm/**`, `app/src/engine-wasm/**`, `app/src/main.ts`, `app/vite.config.ts` and
`app/package.json` scripts for the web build, `app/tests/` new files for the wasm adapter.

## Rules

`PRAXIS_KEYCHAIN=off`; never play audio (tests mute Chromium, or use an OfflineAudioContext); 5
jobs / 5 test threads via the worktrees cargo config.

## Done when

The browser build opens in Chromium (muted), makes a take of the acceptance-1 brief through the
wasm engine with 8 of 8 checks, plays through the AudioWorklet (verified with an
OfflineAudioContext render compared to the native offline render within a stated tolerance), and
Playwright covers it; fmt, workspace clippy, the wasm build, tsc, Vitest pass. Report the wasm
size and load time. Report as in docs/BRIEFING.md.
