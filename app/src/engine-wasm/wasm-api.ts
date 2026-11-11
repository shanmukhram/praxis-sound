// The surface of crates/praxis-wasm as wasm-bindgen exports it (`wasm-bindgen --target web`
// writes the glue to ./pkg/, built by `npm run build:wasm`). Written by hand so the app
// type-checks without the generated files; the glue's own .d.ts must match it.

/** `Engine` in crates/praxis-wasm/src/lib.rs: pack files in memory, presets, offline renders. */
export interface WasmEngineHandle {
  insert_pack_file(path: string, bytes: Uint8Array): void;
  pack_needs(score: string): string;
  missing_pack_files(pack: string, preset: string): string;
  presets(): string;
  pack_size(): Float64Array;
  render(score: string, settings: string): WasmRendered;
  free(): void;
}

/** `Rendered`: one offline render. */
export interface WasmRendered {
  ok(): boolean;
  result(): string;
  take_left(): Float32Array;
  take_right(): Float32Array;
  free(): void;
}

/** `Realtime`: the real-time engine an AudioWorklet runs. */
export interface WasmRealtimeHandle {
  sample_rate(): number;
  insert_pack_file(path: string, bytes: Uint8Array): void;
  missing_pack_files(pack: string, preset: string): string;
  load(score: string, masterGainDb: number): string;
  play(): boolean;
  stop(): boolean;
  seek(position: number): boolean;
  set_loop(start: number, end: number): boolean;
  set_layer_mute(layer: string, muted: boolean): boolean;
  set_layer_solo(layer: string, solo: boolean): boolean;
  set_layer_gain(layer: string, gainDb: number): boolean;
  set_master_gain(gainDb: number): boolean;
  render(frames: number): number;
  process(left: Float32Array, right: Float32Array): void;
  left_ptr(): number;
  right_ptr(): number;
  status_ptr(): number;
  status(): Float64Array;
  free(): void;
}

/** The generated module (`pkg/praxis_wasm.js`). */
export interface PraxisGlue {
  initSync(input: { module: WebAssembly.Module | BufferSource }): { memory: WebAssembly.Memory };
  version(): string;
  validate_score(json: string): string;
  plan_brief(brief: string, seed: number): string;
  compose_plan(plan: string, seed: number): string;
  plan_edit(score: string, instruction: string, scope: string, seed: number): string;
  apply_patch(score: string, patch: string): string;
  verify(score: string, left: Float32Array, right: Float32Array, sampleRate: number): string;
  analyse(left: Float32Array, right: Float32Array, sampleRate: number, fileName: string, channels: number, beatsPerBar: number): string;
  Engine: new () => WasmEngineHandle;
  Realtime: {
    new (sampleRate: number): WasmRealtimeHandle;
    max_frames(): number;
    status_fields(): string;
  };
}

/** Status slots of `Realtime.status()` (crates/praxis-wasm/src/realtime.rs `STATUS_FIELDS`). */
export const STATUS = {
  position: 0,
  playing: 1,
  ended: 2,
  peakL: 3,
  peakR: 4,
  rmsL: 5,
  rmsR: 6,
  callbacks: 7,
  frames: 8,
  swaps: 9,
  rejectedSwaps: 10,
  crossfading: 11,
} as const;

export const STATUS_LEN = 12;
