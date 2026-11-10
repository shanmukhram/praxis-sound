// The one interface the workstation talks to. Implementations:
//   MockEngine     - TypeScript, this package (browser dev mode and tests)
//   TauriEngine    - desktop: Tauri IPC to the Rust real-time engine (later package)
//   WasmEngine     - browser build: praxis-wasm in an AudioWorklet (later package)
// Every call is async so IPC and worker-backed adapters fit without changes.

import type { BarRange, NScore, Score, ScorePatch } from '../score/types';

/** Why a patch was rejected. Mirrors `PatchError` in crates/praxis-score/src/patch.rs. */
export type PatchError =
  | { kind: 'locked_layer'; layer: string }
  | { kind: 'out_of_scope'; index: number; op: string; reason: string }
  | { kind: 'unknown_id'; index: number; op: string; what: string; id: string }
  | { kind: 'duplicate_id'; index: number; op: string; what: string; id: string }
  | { kind: 'unlock_not_allowed'; layer: string }
  | { kind: 'invalid'; message: string };

export type ApplyResult = { ok: true; score: NScore; inverse: ScorePatch } | { ok: false; error: PatchError };

/** Same wording as the Rust `Display` impl, so the UI and CLI read alike. */
export function patchErrorMessage(e: PatchError): string {
  switch (e.kind) {
    case 'locked_layer':
      return `layer \`${e.layer}\` is locked`;
    case 'out_of_scope':
      return `op ${e.index} (${e.op}) is outside the patch scope: ${e.reason}`;
    case 'unknown_id':
      return `op ${e.index} (${e.op}) references unknown ${e.what} \`${e.id}\``;
    case 'duplicate_id':
      return `op ${e.index} (${e.op}) would duplicate ${e.what} \`${e.id}\``;
    case 'unlock_not_allowed':
      return `only a user may unlock layer \`${e.layer}\``;
    case 'invalid':
      return `the patched score is invalid: ${e.message}`;
  }
}

/** One validation issue (`validate::Issue`). */
export interface Issue {
  code: string;
  path: string;
  message: string;
}

export interface LoadResult {
  score: NScore;
  errors: Issue[];
  warnings: Issue[];
}

export type SampleFormat = 'wav16' | 'wav24' | 'wav32f';

export interface RenderOptions {
  sampleRate?: number;
  format?: SampleFormat;
  /** Render only these layers (stems); default all audible layers. */
  layers?: string[];
}

/** Mirrors `RenderReport` in crates/praxis-engine/src/api.rs, plus `mock`. */
export interface RenderReport {
  mock: boolean;
  sample_rate: number;
  length_samples: number;
  tail_samples: number;
  is_loop: boolean;
  loudness_before_lufs: number;
  loudness_lufs: number;
  true_peak_dbtp: number;
  applied_gain_db: number;
  warmup_cycles: number;
  notes_rendered: number;
  blocked_layers: string[];
  render_seconds: number;
  warnings: string[];
}

export interface RenderResult {
  /** URL of the rendered WAV (blob: in the browser, a file URL from the desktop shell). */
  audioUrl: string;
  report: RenderReport;
}

export interface TransportPosition {
  playing: boolean;
  /** Seconds from the start of bar 1. */
  seconds: number;
  /** Fractional 1-based bar position. */
  bar: number;
  loop: BarRange | null;
}

/** Output meters in dBFS (-Infinity for silence). */
export interface Meters {
  peakL: number;
  peakR: number;
  rmsL: number;
  rmsR: number;
}

export type ExportKind = 'wav16' | 'wav24' | 'wav32f' | 'flac' | 'stems' | 'midi' | 'project';

export interface ExportRequest {
  kinds: ExportKind[];
  /** Output folder or base path chosen by the user. */
  destination: string;
  baseName: string;
}

export interface ExportResult {
  ok: boolean;
  /** Files the engine wrote (or, for the mock, would write). */
  files: { kind: ExportKind; path: string }[];
  message: string;
}

export type Unsubscribe = () => void;

export interface EngineAdapter {
  /** Which implementation this is; the UI labels mock output plainly. */
  readonly kind: 'mock' | 'tauri' | 'wasm';
  readonly label: string;

  load(score: Score): Promise<LoadResult>;
  /** Applies a patch atomically: the new score and its exact inverse, or why it was rejected. */
  applyPatch(patch: ScorePatch): Promise<ApplyResult>;
  render(options?: RenderOptions): Promise<RenderResult>;

  play(): Promise<void>;
  stop(): Promise<void>;
  seek(seconds: number): Promise<void>;
  setLoop(range: BarRange | null): Promise<void>;
  onPosition(cb: (p: TransportPosition) => void): Unsubscribe;
  onMeters(cb: (m: Meters) => void): Unsubscribe;

  export(request: ExportRequest): Promise<ExportResult>;
  dispose(): void;
}
