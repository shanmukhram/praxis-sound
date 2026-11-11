// The one interface the workstation talks to. Implementations:
//   MockEngine     - TypeScript, this package (browser dev mode and tests)
//   TauriEngine    - desktop: Tauri IPC to the Rust real-time engine (later package)
//   WasmEngine     - browser build: praxis-wasm in an AudioWorklet (later package)
// Every call is async so IPC and worker-backed adapters fit without changes.

import type { BarRange, GeneratorClass, InstrumentKind, Key, NScore, Score, ScorePatch, TimeSignature } from '../score/types';

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

// ---------------------------------------------------------------- takes (ADR-0011)

/** One measured check. Mirrors `Check` in crates/praxis-analyze/src/verify.rs. */
export interface VerifyCheck {
  /** "tempo", "key", "grid", "loudness", "true_peak", "loop_seam", "clipping", "silence". */
  name: string;
  passed: boolean;
  measured: string;
  expected: string;
}

/**
 * Mirrors `VerifyReport` (contracts/sound-profile.md, verify.rs) and the `praxis check --json`
 * payload, so a real adapter can fill it from the CLI unchanged.
 */
export interface VerifyReport {
  checks: VerifyCheck[];
  tempo_bpm: number | null;
  key: string | null;
  grid_deviation_ms: number | null;
  onsets_on_grid: number | null;
  integrated_lufs: number | null;
  true_peak_dbtp: number | null;
  seam_jump_ratio: number | null;
  seam_excess_db: number | null;
  passed: boolean;
}

export type ViewName = 'overview' | 'rhythm' | 'notes' | 'mix' | 'export';

/** Who made a take: the person, or a connected agent by name. */
export type Who = 'You' | `Agent · ${string}`;

/** A bar range on one layer: the scope of a pointed-at request. */
export interface TakeScope {
  layer: string;
  bars: BarRange;
}

/** One playable result of a request (a project version, FR-31). */
export interface Take {
  /** 1-based take number within the sound. */
  n: number;
  who: Who;
  /** The request as asked (with the scope appended when there was one). */
  ask: string;
  title: string;
  /** Short plain-language reply (the planner's `PlanOutcome` summary). */
  say: string;
  score: NScore;
  /** The patch that made this take from the previous one; null for a new sound. */
  patch: ScorePatch | null;
  /** Bars each changed layer was changed in: `{layer: [from, to]}`. Empty for a new sound. */
  changed: Record<string, [number, number]>;
  /** What stayed identical, in words (e.g. "Bars 1-12 sample-identical to take 2."). */
  untouched: string;
  render: RenderResult;
  checks: VerifyReport;
  /** A panel view the maker opened to show what it did. */
  opens?: ViewName;
  scope?: TakeScope;
}

export interface AskRequest {
  text: string;
  scope?: TakeScope;
  /** A dropped reference clip (FR-11). */
  clipFile?: File | { name: string };
}

/** A take, or why no take was made (a refused patch, or a request the planner cannot do). */
export type AskResult = { ok: true; take: Take } | { ok: false; message: string; error?: PatchError };

export interface TakeState {
  takes: Take[];
  /** Number of the take that is playing now. */
  current: number;
  /** The live score: the current take plus mute/solo/lock changes and hand edits made since. */
  score: NScore | null;
  canUndo: boolean;
  canRedo: boolean;
  /**
   * True for a sound started blank by hand that has no take yet: the live score can be edited,
   * played and made into its first take, but not asked about in words.
   */
  draft?: boolean;
}

// ---------------------------------------------------------------- building by hand (WP-U5)

/** What a blank sound starts as. Give `bpm`, or `seconds` and the duration solver picks the tempo. */
export interface BlankSound {
  title?: string;
  key: Key;
  bpm?: number;
  seconds?: number;
  timeSignature: TimeSignature;
  bars: number;
  loop: boolean;
}

/** An instrument preset a layer can use (`praxis presets`: the planner catalogue plus engine-only presets). */
export interface PresetInfo {
  name: string;
  kind: InstrumentKind;
  class: GeneratorClass;
  pitched: boolean;
  description: string;
  /** False when it cannot render here (a pack not installed); `reason` says why. */
  available: boolean;
  reason?: string;
}

export interface MakeTakeOptions {
  title?: string;
  say?: string;
  /** Bars of one layer, when every edit in the batch was there. */
  scope?: TakeScope;
}

export type StartResult = { ok: true } | { ok: false; message: string };

export interface OpenViewEvent {
  view: ViewName;
  /** "Praxis" (the assistant) or an agent's name. */
  by: string;
}

export interface AgentInfo {
  name: string;
  connected: boolean;
  /** What it is for, in a few words. */
  about: string;
}

export interface EngineAdapter {
  /** Which implementation this is; the UI labels mock output plainly. */
  readonly kind: 'mock' | 'tauri' | 'wasm';
  readonly label: string;

  load(score: Score): Promise<LoadResult>;
  /**
   * Applies a patch atomically: the new score and its exact inverse, or why it was rejected.
   * Recorded in the session's undo history without making a take (mute, solo, lock).
   */
  applyPatch(patch: ScorePatch): Promise<ApplyResult>;

  // Takes (ADR-0011). `praxis generate|edit|check` and the MCP `praxis_takes` stream fill these.
  /** A request in words (optionally scoped to bars of one layer) -> a rendered, checked take. */
  ask(request: AskRequest): Promise<AskResult>;
  /** A hand edit from a panel view (Rhythm, Notes) as an explicit patch -> a take. */
  edit(patch: ScorePatch, ask: string, scope?: TakeScope): Promise<AskResult>;

  // Building by hand (WP-U5). Hand edits are `applyPatch` calls (heard at once, no take);
  // `makeTake` records the live score as one take.
  /** Starts a new, blank sound (no layers, no take yet) and makes it the open sound. */
  startBlank(settings: BlankSound): Promise<StartResult>;
  /** Instrument presets for the layer picker. */
  presets(): Promise<PresetInfo[]>;
  /**
   * Renders, checks and records the live score as one take: the first take of a blank sound, or
   * every hand edit since the current take. Refused when nothing changed or there is no layer.
   */
  makeTake(ask: string, options?: MakeTakeOptions): Promise<AskResult>;
  takes(): Promise<Take[]>;
  state(): TakeState;
  /** Goes back to take n through a patch, so it can be undone. */
  restore(n: number): Promise<AskResult>;
  undo(): Promise<boolean>;
  redo(): Promise<boolean>;
  /** Suggestion chips generated from the current sound. */
  suggestions(): Promise<string[]>;
  agents(): Promise<AgentInfo[]>;
  onTake(cb: (take: Take) => void): Unsubscribe;
  /** Any change to takes, the current take, the live score or undo state. */
  onState(cb: (state: TakeState) => void): Unsubscribe;
  onOpenView(cb: (e: OpenViewEvent) => void): Unsubscribe;
  /** Opens a panel view on behalf of the assistant or an agent (MCP `praxis_open_view`). */
  openView(view: ViewName, by?: string): void;
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
