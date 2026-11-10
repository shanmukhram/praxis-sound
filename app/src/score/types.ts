// Score and patch types for the UI. The shapes come from the frozen schemas (generated.ts);
// this module adds normalised variants (schema defaults filled in) and small helpers.

import type {
  BarRange,
  Clip,
  Layer,
  Note,
  Pattern,
  PatchOp,
  Score,
  ScorePatch,
  Section,
} from './generated';

export type * from './generated';

export const SCHEMA_VERSION = 1;

/** A layer with every schema default filled in. */
export type NLayer = Layer &
  Required<
    Pick<
      Layer,
      'automation' | 'clips' | 'effects' | 'gain_db' | 'locked' | 'muted' | 'name' | 'pan' | 'seed_offset' | 'solo'
    >
  >;

/** A score with every top-level and layer default filled in. */
export type NScore = Omit<Score, 'layers' | 'patterns' | 'sections'> &
  Required<Pick<Score, 'chords' | 'constraints' | 'markers' | 'master' | 'meta' | 'playback' | 'seed' | 'title'>> & {
    layers: NLayer[];
    patterns: Pattern[];
    sections: Section[];
  };

export type OpName = PatchOp['op'];

/** Narrow a PatchOp to one variant by its tag. */
export type OpOf<K extends OpName> = Extract<PatchOp, { op: K }>;

/** One applied edit and its exact inverse (mirrors `HistoryEntry` in project.rs). */
export interface HistoryEntry {
  seq: number;
  patch: ScorePatch;
  inverse: ScorePatch;
}

/** A named snapshot (mirrors `Version` in project.rs). */
export interface Version {
  name: string;
  seq: number;
  score: NScore;
}

/** The `.praxis` project file (mirrors `Project` in project.rs). */
export interface Project {
  format: 'praxis-project';
  version: 1;
  score: NScore;
  undo: HistoryEntry[];
  redo: HistoryEntry[];
  next_seq: number;
  versions: Version[];
  references: { name: string; profile: string }[];
}

export function clone<T>(v: T): T {
  return structuredClone(v);
}

/** Fills schema defaults so the UI never has to guess. Does not validate. */
export function normalizeScore(input: Score): NScore {
  const s = clone(input);
  return {
    ...s,
    title: s.title ?? '',
    seed: s.seed ?? 0,
    meta: s.meta ?? { provenance: {}, reference_profiles: [], tags: [] },
    playback: s.playback ?? { mode: 'one_shot', tail_seconds: 4 },
    constraints: s.constraints ?? { avoid: [] },
    master: s.master ?? { effects: [], loudness: { normalize: true, target_lufs: -16, true_peak_dbtp: -1 } },
    markers: s.markers ?? [],
    chords: s.chords ?? [],
    sections: s.sections ?? [],
    patterns: (s.patterns ?? []).map(normalizePattern),
    layers: (s.layers ?? []).map(normalizeLayer),
  };
}

export function normalizePattern(p: Pattern): Pattern {
  return { ...p, notes: p.notes.map(normalizeNote) };
}

export function normalizeNote(n: Note): Note {
  return { ...n, accent: n.accent ?? 'normal' };
}

export function normalizeClip(c: Clip): Clip {
  return { ...c, transpose: c.transpose ?? 0, velocity_scale: c.velocity_scale ?? 1 };
}

export function normalizeLayer(l: Layer): NLayer {
  return {
    ...l,
    name: l.name ?? '',
    automation: l.automation ?? [],
    clips: (l.clips ?? []).map(normalizeClip),
    effects: l.effects ?? [],
    gain_db: l.gain_db ?? 0,
    locked: l.locked ?? false,
    muted: l.muted ?? false,
    pan: l.pan ?? 0,
    seed_offset: l.seed_offset ?? 0,
    solo: l.solo ?? false,
  };
}

/** Structural equality on JSON values (key order ignored, `undefined` fields ignored). */
export function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== typeof b || a === null || b === null || typeof a !== 'object') return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (Array.isArray(a)) {
    const bb = b as unknown[];
    return a.length === bb.length && a.every((v, i) => deepEqual(v, bb[i]));
  }
  const ao = a as Record<string, unknown>;
  const bo = b as Record<string, unknown>;
  const keys = new Set([...Object.keys(ao), ...Object.keys(bo)]);
  for (const k of keys) {
    if (ao[k] === undefined && bo[k] === undefined) continue;
    if (!deepEqual(ao[k], bo[k])) return false;
  }
  return true;
}

export function rangeContains(outer: BarRange, inner: BarRange): boolean {
  return inner.start >= outer.start && inner.end <= outer.end;
}

export function rangesOverlap(a: BarRange, b: BarRange): boolean {
  return a.start <= b.end && b.start <= a.end;
}

export function formatRange(r: BarRange): string {
  return r.start === r.end ? `bar ${r.start}` : `bars ${r.start}-${r.end}`;
}
