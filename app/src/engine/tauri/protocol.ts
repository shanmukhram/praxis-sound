// What crates/praxis-app's commands return, and how each payload maps onto the adapter types.
// Take records come from the command layer's take ops (praxis_cli::ops, `takes.json`), the same
// records the CLI and the MCP server return, so a take an agent made reads like one made here.

import { normalizeScore, type NScore, type Score, type ScorePatch } from '../../score/types';
import type { RenderReport, Take, TakeScope, VerifyCheck, VerifyReport, ViewName, Who } from '../EngineAdapter';

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
  take: TakeRecord;
  can_undo: boolean;
  can_redo: boolean;
  provider_note?: string | null;
}

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
  const take: Take = {
    n: r.n,
    who: toWho(r.who),
    ask: r.ask,
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
