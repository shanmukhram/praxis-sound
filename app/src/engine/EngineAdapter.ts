// The one interface the workstation talks to. Implementations:
//   MockEngine     - TypeScript, this package (browser dev mode and tests)
//   TauriEngine    - desktop: Tauri IPC to the Rust real-time engine (later package)
//   WasmEngine     - browser build: praxis-wasm in an AudioWorklet (later package)
// Every call is async so IPC and worker-backed adapters fit without changes.

import type { BarRange, GeneratorClass, InstrumentKind, Key, NScore, Score, ScorePatch, TimeSignature } from '../score/types';
import type { AnalyzeResult, CompareLevel, LibraryEntry, LibraryResult, ReferenceClip, SoundProfile } from './reference';

export type { AnalyzeResult, ClipKind, CompareLevel, LibraryEntry, LibraryResult, ReferenceClip, SoundProfile } from './reference';

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
  /** False for a failed check and for an inconclusive one. */
  passed: boolean;
  /** An inconclusive check's ends with "; can't tell: <reason in plain words>". */
  measured: string;
  expected: string;
  /**
   * The material cannot decide this check (tempo or key): "can't tell from these notes". Neither
   * passed nor failed; `VerifyReport.passed` ignores it.
   */
  inconclusive?: boolean;
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
  /** The reference (a clip's file name or a library style) the request carried, if any. */
  reference?: string;
}

export interface AskRequest {
  text: string;
  scope?: TakeScope;
  /**
   * A reference: a dropped clip's profile or a library style (FR-11, FR-19). Its profile goes to
   * the planner and the assistant with the request.
   */
  reference?: ReferenceClip;
  /** Called with each step of the sound assistant while it works (the desktop engine). */
  onProgress?: (p: AskProgress) => void;
}

/** One step of the sound assistant's work, in plain words. */
export type AskProgress =
  | { kind: 'step'; n: number; words: string }
  | { kind: 'step_done'; n: number; ok: boolean; words: string }
  | { kind: 'said'; text: string };

/**
 * Why a request made no take though it went fine: the assistant asked a question, refused (a
 * locked layer), stepped the history (undo), only opened a view, found nothing to change, or the
 * request only locked, muted or soloed layers (`live`).
 */
export type NoTake = 'question' | 'refused' | 'undo' | 'view' | 'nothing_to_change' | 'live';

/**
 * A take; or a reply without a take (`take: null`, shown as the assistant's answer, never as an
 * error); or why the request failed (a refused patch, a request that could not be carried out).
 */
export type AskResult = TakeResult | { ok: true; take: null; reply: string; noTake: NoTake };

/** A take, or why none was made (making a take by hand, going back). */
export type TakeResult = { ok: true; take: Take } | { ok: false; message: string; error?: PatchError };

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
  makeTake(ask: string, options?: MakeTakeOptions): Promise<TakeResult>;
  takes(): Promise<Take[]>;
  state(): TakeState;
  /** Goes back to take n through a patch, so it can be undone. */
  restore(n: number): Promise<TakeResult>;
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

  // Reference clips (FR-11, FR-12, FR-19, FR-30). A profile holds measurements, never audio.
  /** Measures a dropped audio or video clip: its Sound Profile, and a URL to play it for A/B. */
  analyzeClip(file: ClipFile): Promise<AnalyzeResult>;
  /** The reference library (entries without their profiles). */
  library(): Promise<LibraryEntry[]>;
  /** One library entry's full profile (null when it is gone). */
  libraryProfile(id: string): Promise<SoundProfile | null>;
  /** Saves a reference's profile (never its audio) to the library under a name, with tags. */
  saveReference(ref: ReferenceClip, name: string, tags: string[]): Promise<LibraryResult>;
  /** Replaces a library entry's tags. */
  tagReference(id: string, tags: string[]): Promise<LibraryResult>;
  removeReference(id: string): Promise<boolean>;
  /**
   * The take's side of a loudness-matched A/B: a trim (or silence) on what the transport plays
   * while comparing; null puts it back. Never saved, never in a render or an export.
   */
  setCompare(level: CompareLevel | null): Promise<void>;

  dispose(): void;
}

/** A clip as the front end has it (a dropped or chosen `File`). */
export interface ClipFile {
  readonly name: string;
  readonly size: number;
  arrayBuffer(): Promise<ArrayBuffer>;
}
