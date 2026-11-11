// What crates/praxis-app's commands return, and how each payload maps onto the adapter types.
// Take records come from the command layer's take ops (praxis_cli::ops, `takes.json`), the same
// records the CLI and the MCP server return, so a take an agent made reads like one made here.

import { normalizeScore, type NScore, type Score, type ScorePatch } from '../../score/types';
import type { RenderReport, Take, TakeScope, VerifyCheck, VerifyReport, ViewName, Who } from '../EngineAdapter';
import { type ClipKind, type LibraryEntry, profileSummary, type SoundProfile, splitReference } from '../reference';

/** `SoundInfo` in crates/praxis-app/src/sounds.rs. */
export interface SoundInfo {
  id: string;
  name: string;
  path: string;
  sub: string;
  takes: number;
  modified_ms: number;
}

/** `Live` in crates/praxis-app/src/commands.rs. */
export interface LivePayload {
  score: Score;
  can_undo: boolean;
  can_redo: boolean;
  /** The sound's current take after the call. */
  current: number;
}

export interface AppliedPayload {
  live: LivePayload;
  inverse: ScorePatch;
}

/** `NewSound` in crates/praxis-app/src/hand.rs: the blank sound's folder and live score. */
export interface NewSoundPayload {
  sound: string;
  revision: number;
  live: LivePayload;
}

export interface SteppedPayload {
  moved: boolean;
  live: LivePayload;
}

/** The CLI `render` payload (docs/contracts/cli.md), or the engine's `RenderReport`. */
export interface RenderPayload {
  output?: string;
  sample_rate?: number;
  frames?: number;
  length_samples?: number;
  tail_samples?: number;
  is_loop?: boolean;
  loudness_lufs?: number | null;
  loudness_before_lufs?: number | null;
  true_peak_dbtp?: number | null;
  applied_gain_db?: number;
  warmup_cycles?: number;
  notes_rendered?: number;
  blocked_layers?: string[];
  render_seconds?: number;
  warnings?: string[];
  check?: Partial<VerifyReport> & { failed?: string[] };
}

/** One take record (docs/contracts/cli.md, "The take record"); `render.path` is absolute. */
export interface TakeRecord {
  n: number;
  who: string;
  ask: string;
  title: string;
  say: string;
  patch: ScorePatch | null;
  changed: Record<string, [number, number]>;
  untouched: string;
  render: { path: string; report: RenderPayload };
  checks: Partial<VerifyReport> | null;
  opens?: ViewName | null;
  scope?: TakeScope | null;
  /** The take's score (its project version `take-<n>`); praxis-app always asks for it. */
  score?: Score | null;
  version?: string;
  from?: number;
  identical_bars?: [number, number][];
  provider?: string;
  seed?: number;
}

/** A view request in `takes.json` (`open_view`): `by` is "Praxis" or the agent's name. */
export interface ViewRequest {
  revision: number;
  view: ViewName;
  by: string;
  take: number;
}

/** The `takes` payload: the whole history of one sound. */
export interface TakesPayload {
  sound: string;
  revision: number;
  current: number;
  takes: TakeRecord[];
  /** The live score, null before the first take. */
  score: Score | null;
  can_undo: boolean;
  can_redo: boolean;
  views: ViewRequest[];
}

/** The `ask` / `keep_variation` payload: the new take and where the sound now stands. */
export interface AskPayload {
  sound: string;
  revision: number;
  current: number;
  /** Null for a turn of the sound assistant that made no take, or a lock/mute/solo patch. */
  take: TakeRecord | null;
  can_undo: boolean;
  can_redo: boolean;
  provider_note?: string | null;
  /** Why no take was made (`take` null): docs/contracts/cli.md, "Assistant agent". */
  no_take?: 'question' | 'refused' | 'undo' | 'view' | 'nothing_to_change' | 'live';
  /** The assistant's answer (with or without a take). */
  reply?: string;
  agent?: { outcome: string; reply: string; steps: { tool: string; words: string; ok: boolean; result: string }[] } | null;
}

/** Event the shell sends for each step of the sound assistant's turn (crates/praxis-app). */
export const AGENT_EVENT = 'praxis://agent';

/** `AgentProgress` in crates/praxis-app/src/commands.rs: the sound plus one `AgentEvent`. */
export type AgentEventPayload = { sound: string } & (
  | { event: 'started'; request: string }
  | { event: 'thinking'; n: number }
  | { event: 'said'; text: string }
  | { event: 'step'; n: number; tool: string; words: string }
  | { event: 'step_done'; n: number; tool: string; ok: boolean; words: string }
  | { event: 'take_made'; n: number }
  | { event: 'finished'; outcome: string; reply: string }
);

const num = (v: unknown, d = 0): number => (typeof v === 'number' && Number.isFinite(v) ? v : d);
const numOrNull = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);

export function toRenderReport(p: RenderPayload): RenderReport {
  const sampleRate = num(p.sample_rate, 48000);
  return {
    mock: false,
    sample_rate: sampleRate,
    length_samples: num(p.length_samples, num(p.frames)),
    tail_samples: num(p.tail_samples),
    is_loop: Boolean(p.is_loop),
    loudness_before_lufs: num(p.loudness_before_lufs, -Infinity),
    loudness_lufs: num(p.loudness_lufs, -Infinity),
    true_peak_dbtp: num(p.true_peak_dbtp, -Infinity),
    applied_gain_db: num(p.applied_gain_db),
    warmup_cycles: num(p.warmup_cycles),
    notes_rendered: num(p.notes_rendered),
    blocked_layers: p.blocked_layers ?? [],
    render_seconds: num(p.render_seconds),
    warnings: p.warnings ?? [],
  };
}

export function toVerifyReport(c: Partial<VerifyReport> | null | undefined): VerifyReport {
  const checks: VerifyCheck[] = (c?.checks ?? []).map((k) => ({
    name: String(k.name),
    passed: Boolean(k.passed),
    measured: String(k.measured ?? ''),
    expected: String(k.expected ?? ''),
    inconclusive: typeof k.inconclusive === 'boolean' ? k.inconclusive : undefined,
  }));
  return {
    checks,
    tempo_bpm: numOrNull(c?.tempo_bpm),
    key: typeof c?.key === 'string' ? c.key : null,
    grid_deviation_ms: numOrNull(c?.grid_deviation_ms),
    onsets_on_grid: numOrNull(c?.onsets_on_grid),
    integrated_lufs: numOrNull(c?.integrated_lufs),
    true_peak_dbtp: numOrNull(c?.true_peak_dbtp),
    seam_jump_ratio: numOrNull(c?.seam_jump_ratio),
    seam_excess_db: numOrNull(c?.seam_excess_db),
    passed: typeof c?.passed === 'boolean' ? c.passed : checks.length > 0 && checks.every((k) => k.passed),
  };
}

/** "You" stays "You"; any other maker is an agent ("Agent · cut-assistant"). */
export function toWho(who: string | undefined): Who {
  const w = (who ?? '').trim();
  if (!w || w === 'You' || w === 'you') return 'You';
  if (w.startsWith('Agent · ')) return w as Who;
  return `Agent · ${w}`;
}

/** The plain agent name of a `Who` ("cut-assistant"), or null for the person. */
export function agentName(who: Who): string | null {
  return who === 'You' ? null : who.slice('Agent · '.length);
}

/**
 * A take record as the adapter's `Take`. `score` is the take's own score when the record has it,
 * else `fallback` (the live score).
 */
export function toTake(r: TakeRecord, fileUrl: (path: string) => string, fallback: NScore | null): Take {
  const score = r.score ? normalizeScore(r.score) : fallback;
  if (!score) throw new Error(`take ${r.n} has no score`);
  // A request with a reference carries its profile in words after the typed text.
  const asked = splitReference(r.ask ?? '');
  const take: Take = {
    n: r.n,
    who: toWho(r.who),
    ask: asked.ask,
    title: r.title,
    say: r.say,
    score,
    patch: r.patch ?? null,
    changed: r.changed ?? {},
    untouched: r.untouched ?? '',
    render: { audioUrl: fileUrl(r.render.path), report: toRenderReport(r.render.report ?? {}) },
    checks: toVerifyReport(r.checks ?? r.render.report?.check),
  };
  if (r.opens) take.opens = r.opens;
  if (r.scope) take.scope = r.scope;
  if (asked.reference) take.reference = asked.reference;
  return take;
}

/** `TransportEvent` in crates/praxis-app/src/audio.rs, sent as `praxis://transport` ~30 times a second. */
export interface TransportEventPayload {
  /** An output device is open and running the real-time engine. */
  available: boolean;
  device: string | null;
  sample_rate: number;
  /** Why there is no device, or why the sound could not be built for playback. */
  error: string | null;
  playing: boolean;
  ended: boolean;
  position: number;
  seconds: number;
  /** dBFS, floored at -120 (silence). */
  peak_db: [number, number];
  rms_db: [number, number];
  /** Xruns while the engine played (audible). */
  xruns: number;
  /** Every xrun since the device opened, stopped time included. */
  xruns_total: number;
  late_wakeups: number;
  device_errors: number;
  min_callback_frames: number;
  max_callback_frames: number;
  callbacks: number;
  swaps: number;
  preparing: boolean;
  stalled: boolean;
}

/** `TransportStatus` (the event's fields plus what the last graph build said). */
export interface TransportStatusPayload extends TransportEventPayload {
  requested_buffer_frames: number | null;
  loaded: boolean;
  prepare_ms: number | null;
  warnings: string[];
  max_render_ms: number;
  p99_render_ms: number;
  layers: Record<string, LayerLive>;
  loop_range: { start: number; end: number } | null;
}

/** A live layer override (`LayerLive`): heard at once, not saved. Left-out fields keep their value. */
export interface LayerLive {
  gain_db?: number;
  muted?: boolean;
  solo?: boolean;
}

/** Meter floor the engine sends for silence. */
export const METER_FLOOR_DB = -120;

// ---------------------------------------------------------------- reference clips

/** `Clip` in crates/praxis-app/src/clips.rs: the shell's copy of a dropped clip and its profile. */
export interface ClipPayload {
  path: string;
  profile: unknown;
}

/** `LibraryEntry` in crates/praxis-app/src/clips.rs: a named, tagged profile (no audio). */
export interface LibraryEntryPayload {
  id: string;
  name: string;
  tags?: string[];
  kind?: string;
  duration_s?: number;
  /** The analyzer's one line in plain words. */
  summary?: string;
  /** The profile (`library_show`, `library_add`); null in a listing. */
  profile?: unknown;
  /** The same clip was already kept: that entry got the name and tags. */
  already?: boolean;
}

const arr = <T>(v: unknown, f: (x: unknown) => T): T[] => (Array.isArray(v) ? v.map(f) : []);
const obj = (v: unknown): Record<string, any> => (v && typeof v === 'object' ? (v as Record<string, any>) : {});
const KINDS: ClipKind[] = ['music', 'sfx', 'ambience'];

/**
 * A Sound Profile from the analyzer's JSON (`praxis analyze`, the profile contract), checked for
 * the fields the app reads. Accepts the profile itself or a payload wrapping it (`{profile}`).
 * Throws when it is not a profile at all.
 */
export function toProfile(value: unknown): SoundProfile {
  const v = obj(value);
  const p = obj(v.source ? v : v.profile);
  if (!p.source || !p.class || !p.loudness) throw new Error('the analyzer did not return a Sound Profile');
  const src = obj(p.source);
  const cls = obj(p.class);
  const loud = obj(p.loudness);
  const env = obj(p.envelope);
  const sp = obj(p.spectral);
  const kind: ClipKind = KINDS.includes(cls.kind) ? cls.kind : 'music';
  const scores = arr(cls.scores, (x) => num(x));
  const tempo = p.tempo ? obj(p.tempo) : null;
  const key = p.key ? obj(p.key) : null;
  const groove = p.groove ? obj(p.groove) : null;
  const reverb = p.reverb ? obj(p.reverb) : null;
  return {
    version: num(p.version, 1),
    source: { file_name: String(src.file_name ?? ''), duration_s: num(src.duration_s), sample_rate: num(src.sample_rate, 48000), channels: num(src.channels, 2), pcm_hash: String(src.pcm_hash ?? '') },
    class: { kind, confidence: num(cls.confidence), scores: [scores[0] ?? 0, scores[1] ?? 0, scores[2] ?? 0] },
    tempo: tempo && Number.isFinite(tempo.bpm)
      ? { bpm: num(tempo.bpm), confidence: num(tempo.confidence), beats_s: arr(tempo.beats_s, (x) => num(x)), downbeat_s: num(tempo.downbeat_s), beats_per_bar: num(tempo.beats_per_bar, 4) }
      : null,
    key: key && key.key && typeof key.key === 'object' ? { key: key.key, confidence: num(key.confidence), chroma: arr(key.chroma, (x) => num(x)) } : null,
    loudness: {
      integrated_lufs: num(loud.integrated_lufs, -Infinity),
      range_lu: num(loud.range_lu),
      true_peak_dbtp: num(loud.true_peak_dbtp, -Infinity),
      sample_peak_dbfs: num(loud.sample_peak_dbfs, -Infinity),
      rms_dbfs: num(loud.rms_dbfs, -Infinity),
      crest_db: num(loud.crest_db),
    },
    envelope: { attack_s: num(env.attack_s), decay_s: num(env.decay_s), sustain: num(env.sustain), release_s: num(env.release_s), frames_db: arr(env.frames_db, (x) => num(x, -120)), hop_s: num(env.hop_s, 0.01) },
    spectral: {
      centroid_hz: num(sp.centroid_hz),
      rolloff_hz: num(sp.rolloff_hz),
      flatness: num(sp.flatness),
      bands_db: arr(sp.bands_db, (x) => num(x, -120)),
      tilt_db_per_oct: num(sp.tilt_db_per_oct),
      centroid_track_hz: arr(sp.centroid_track_hz, (x) => num(x)),
    },
    groove: groove
      ? { grid: groove.grid ?? 'sixteenth', steps_per_bar: num(groove.steps_per_bar, 16), step_weights: arr(groove.step_weights, (x) => num(x)), swing: num(groove.swing), onsets_per_bar: num(groove.onsets_per_bar) }
      : null,
    onsets_s: arr(p.onsets_s, (x) => num(x)),
    pitch: { hop_s: num(obj(p.pitch).hop_s, 0.01), midi: arr(obj(p.pitch).midi, (x) => (typeof x === 'number' && Number.isFinite(x) ? x : null)) },
    tonality: num(p.tonality),
    stereo: { width: num(obj(p.stereo).width), correlation: num(obj(p.stereo).correlation, 1) },
    reverb: reverb ? { rt60_s: num(reverb.rt60_s), confidence: num(reverb.confidence) } : null,
    sections: arr(p.sections, (x) => ({ start_s: num(obj(x).start_s), end_s: num(obj(x).end_s), label: String(obj(x).label ?? '') })),
    features: arr(p.features, (x) => num(x)),
    tags: arr(p.tags, (x) => String(x)),
    style_name: typeof p.style_name === 'string' ? p.style_name : null,
    ...(typeof p.summary === 'string' ? { summary: p.summary } : {}),
  };
}

export function toLibraryEntry(e: LibraryEntryPayload): LibraryEntry {
  const profile = e.profile ? toProfile(e.profile) : null;
  const kind: ClipKind = profile?.class.kind ?? (KINDS.includes(e.kind as ClipKind) ? (e.kind as ClipKind) : 'music');
  return {
    id: String(e.id),
    name: String(e.name ?? e.id),
    tags: (e.tags ?? profile?.tags ?? []).map(String),
    kind,
    summary: e.summary || (profile ? (profile.summary ?? profileSummary(profile)) : ''),
    profile,
  };
}
