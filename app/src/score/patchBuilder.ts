// Turns UI intents into ScorePatches with exact scope. The UI never mutates a Score directly:
// it builds a patch here and hands it to the engine adapter, which applies it (or rejects it).

import type {
  Accent,
  BarRange,
  Clip,
  Grid,
  LayerUpdate,
  NLayer,
  NScore,
  Note,
  Pattern,
  PatchOp,
  ScorePatch,
} from './types';
import { clone, deepEqual, formatRange, rangeContains, rangesOverlap } from './types';
import { accentVelocity, stepsPerBar } from './theory';
import { barSeconds, layerEvents, stepSeconds } from './timeline';

// ---------------------------------------------------------------- layer properties

export function layerName(score: NScore, id: string): string {
  const l = score.layers.find((x) => x.id === id);
  return l?.name || id;
}

function layerPatch(score: NScore, id: string, update: LayerUpdate, description: string): ScorePatch {
  return {
    description: `${description} ${layerName(score, id)}`,
    origin: 'user',
    scope: { layers: [id] },
    ops: [{ op: 'update_layer', id, update }],
  };
}

export function muteLayer(score: NScore, id: string, muted: boolean): ScorePatch {
  return layerPatch(score, id, { muted }, muted ? 'Mute' : 'Unmute');
}

export function soloLayer(score: NScore, id: string, solo: boolean): ScorePatch {
  return layerPatch(score, id, { solo }, solo ? 'Solo' : 'Unsolo');
}

export function setLayerGain(score: NScore, id: string, gain_db: number): ScorePatch {
  return layerPatch(score, id, { gain_db }, `Set gain ${gain_db.toFixed(1)} dB on`);
}

export function lockLayer(score: NScore, id: string, locked: boolean): ScorePatch {
  return {
    description: `${locked ? 'Lock' : 'Unlock'} ${layerName(score, id)}`,
    origin: 'user',
    scope: { layers: [id] },
    ops: [{ op: 'set_lock', id, locked }],
  };
}

export function setTempo(bpm: number): ScorePatch {
  return { description: `Set tempo ${bpm} BPM`, origin: 'user', scope: { layers: [] }, ops: [{ op: 'set_tempo', bpm }] };
}

export function clearBars(score: NScore, layerId: string, bars: BarRange): ScorePatch {
  return {
    description: `Clear ${formatRange(bars)} of ${layerName(score, layerId)}`,
    origin: 'user',
    scope: { bars: { ...bars }, layers: [layerId] },
    ops: [{ op: 'clear_bars', layer: layerId, bars: { ...bars } }],
  };
}

// ---------------------------------------------------------------- pattern editing in a bar range

export interface DerivedPattern {
  /** What the layer plays in the range, as one editable pattern starting at the range's first bar. */
  pattern: Pattern;
  /**
   * `loop`: one clip covers the range and the range is a whole number of pattern repeats, so an
   * edit applies to every repeat in the range. `literal`: the range is shown note for note.
   */
  mode: 'loop' | 'literal';
  /** Pattern id the range plays now (loop mode), else null. */
  source: string | null;
  /**
   * True when the source pattern is used only by one clip covering exactly this range, from
   * step 0, untransposed: the edit can be a `replace_pattern` instead of a new pattern.
   */
  replaceable: boolean;
  stepsPerBar: number;
}

const DEFAULT_GRID: Grid = 'sixteenth';

function sortNotes(notes: Note[]): Note[] {
  return notes.sort((a, b) => a.step - b.step || a.pitch - b.pitch);
}

function coveringClips(layer: NLayer, bars: BarRange): Clip[] {
  return layer.clips.filter((c) => rangesOverlap(c.bars, bars));
}

/** Pattern the editors show for `layerId` in `bars`. Returns null for an unknown layer. */
export function derivePattern(score: NScore, layerId: string, bars: BarRange): DerivedPattern | null {
  const layer = score.layers.find((l) => l.id === layerId);
  if (!layer) return null;
  const clips = coveringClips(layer, bars);
  const nbars = bars.end - bars.start + 1;

  if (clips.length === 1 && rangeContains(clips[0].bars, bars)) {
    const clip = clips[0];
    const pat = score.patterns.find((p) => p.id === clip.pattern);
    const spb = pat ? stepsPerBar(score.time_signature, pat.grid) : null;
    if (pat && spb !== null && pat.length_steps > 0 && (nbars * spb) % pat.length_steps === 0) {
      const len = pat.length_steps;
      const rot = ((clip.pattern_offset_steps ?? 0) + (bars.start - clip.bars.start) * spb) % len;
      const transpose = clip.transpose ?? 0;
      const vs = clip.velocity_scale ?? 1;
      const notes = pat.notes
        .filter((n) => n.step < len)
        .map((n) => ({ ...clone(n), step: (n.step - rot + len) % len, pitch: n.pitch + transpose, velocity: Math.min(1, n.velocity * vs) }));
      const usedElsewhere = score.layers.some((l) =>
        l.clips.some((c) => c.pattern === pat.id && c !== clip),
      );
      const replaceable =
        rot === 0 && transpose === 0 && vs === 1 && clip.bars.start === bars.start && clip.bars.end === bars.end && !usedElsewhere;
      return {
        pattern: { id: pat.id, grid: pat.grid, length_steps: len, notes: sortNotes(notes) },
        mode: 'loop',
        source: pat.id,
        replaceable,
        stepsPerBar: spb,
      };
    }
  }

  // Literal: every note the layer plays in the range, on the grid of the first clip found.
  const gridPat = (clips.length ? clips : layer.clips)
    .map((c) => score.patterns.find((p) => p.id === c.pattern))
    .find((p) => p && stepsPerBar(score.time_signature, p.grid) !== null);
  let grid: Grid = gridPat?.grid ?? DEFAULT_GRID;
  let spb = stepsPerBar(score.time_signature, grid);
  if (spb === null) {
    grid = 'eighth';
    spb = stepsPerBar(score.time_signature, grid) ?? 1;
  }
  const notes: Note[] = [];
  const startS = (bars.start - 1) * barSeconds(score);
  const stepS = stepSeconds(score, grid);
  for (const e of layerEvents(score, layerId, bars.start, bars.end)) {
    let step: number;
    let length = e.noteLength;
    if (e.grid === grid) {
      step = (e.bar - bars.start) * spb + e.stepInBar;
    } else {
      const ownStepS = stepSeconds(score, e.grid);
      step = Math.round((e.startS - e.noteOffset * ownStepS - startS) / stepS);
      length = (e.noteLength * ownStepS) / stepS;
    }
    const n: Note = {
      step,
      pitch: e.pitch,
      velocity: Math.min(1, e.noteVelocity * e.velocityScale),
      accent: e.accent,
      length: Math.min(64, Math.max(0.01, length)),
    };
    if (e.noteOffset) n.offset = e.noteOffset;
    notes.push(n);
  }
  return {
    pattern: { id: '', grid, length_steps: nbars * spb, notes: sortNotes(notes) },
    mode: 'literal',
    source: null,
    replaceable: false,
    stepsPerBar: spb,
  };
}

/** Deterministic id for a pattern written into bars of a layer (contract rule 7 style). */
export function freshPatternId(score: NScore, layerId: string, bars: BarRange): string {
  const base = `${layerId}_b${bars.start}-${bars.end}`.slice(0, 58);
  const taken = new Set(score.patterns.map((p) => p.id));
  if (!taken.has(base)) return base;
  for (let i = 2; ; i++) {
    const id = `${base}_${i}`;
    if (!taken.has(id)) return id;
  }
}

/**
 * Patch that makes `layerId` play `notes` (a pattern of the derived grid and length, starting at
 * bars.start) in `bars`. Scope is exactly those bars and that layer. Returns null for a no-op.
 */
export function editPatternInRange(
  score: NScore,
  layerId: string,
  bars: BarRange,
  notes: Note[],
  description: string,
): ScorePatch | null {
  const d = derivePattern(score, layerId, bars);
  if (!d) return null;
  const next = sortNotes(clone(notes));
  if (deepEqual(next, d.pattern.notes)) return null;
  let op: PatchOp;
  if (d.replaceable && d.source) {
    const orig = score.patterns.find((p) => p.id === d.source)!;
    op = { op: 'replace_pattern', pattern: { ...clone(orig), notes: next } };
  } else {
    op = {
      op: 'write_bars',
      layer: layerId,
      bars: { ...bars },
      pattern: { id: freshPatternId(score, layerId, bars), grid: d.pattern.grid, length_steps: d.pattern.length_steps, notes: next },
    };
  }
  return {
    description: `${description} in ${formatRange(bars)} of ${layerName(score, layerId)}`,
    origin: 'user',
    scope: { bars: { ...bars }, layers: [layerId] },
    ops: [op],
  };
}

// ---------------------------------------------------------------- pure note-list edits

export interface NoteDefaults {
  velocity: number;
  length: number;
  accent: Accent;
}

export function findNote(notes: Note[], step: number, pitch: number): Note | undefined {
  return notes.find((n) => n.step === step && n.pitch === pitch);
}

/** Adds a note at (step, pitch), or removes the one that is there. */
export function toggleNote(notes: Note[], step: number, pitch: number, d: NoteDefaults): Note[] {
  const existing = findNote(notes, step, pitch);
  if (existing) return notes.filter((n) => n !== existing);
  return sortNotes([...notes, { step, pitch, velocity: d.velocity, accent: d.accent, length: d.length }]);
}

/** Updates fields of the note at (step, pitch). Unknown notes leave the list unchanged. */
export function updateNote(notes: Note[], step: number, pitch: number, change: Partial<Pick<Note, 'velocity' | 'accent' | 'length' | 'pitch' | 'step'>>): Note[] {
  return sortNotes(
    notes.map((n) => {
      if (n.step !== step || n.pitch !== pitch) return n;
      const out = { ...n, ...change };
      out.velocity = clampVelocity(out.velocity);
      out.length = Math.min(64, Math.max(0.05, out.length));
      return out;
    }),
  );
}

export function clampVelocity(v: number): number {
  return Math.min(1, Math.max(0.01, Math.round(v * 100) / 100));
}

/** Velocity the engine plays for a note after its accent (clamped to 1). */
export function playedVelocity(n: Note): number {
  return Math.min(1, n.velocity * accentVelocity(n.accent ?? 'normal'));
}

// ---------------------------------------------------------------- versions

const LAYER_FIELDS: (keyof LayerUpdate)[] = [
  'name',
  'class',
  'instrument',
  'muted',
  'solo',
  'gain_db',
  'pan',
  'effects',
  'automation',
  'seed_offset',
];

/**
 * A whole-score patch that turns `current` into `target` (restoring a named version, FR-31).
 * Layer and pattern order can differ afterwards because add_layer/add_pattern append.
 */
export function restorePatch(current: NScore, target: NScore, name: string): ScorePatch {
  const ops: PatchOp[] = [];
  const g = <K extends keyof NScore>(k: K) => !deepEqual(current[k], target[k]);
  if (g('title')) ops.push({ op: 'set_title', title: target.title });
  if (g('meta')) ops.push({ op: 'set_meta', meta: clone(target.meta) });
  if (current.tempo.bpm !== target.tempo.bpm) ops.push({ op: 'set_tempo', bpm: target.tempo.bpm });
  if (g('key')) ops.push({ op: 'set_key', key: clone(target.key), transpose_notes: false });
  if (g('time_signature')) ops.push({ op: 'set_time_signature', time_signature: clone(target.time_signature) });
  if (g('bars')) ops.push({ op: 'set_bars', bars: target.bars });
  if (g('playback')) ops.push({ op: 'set_playback', playback: clone(target.playback) });
  if (g('constraints')) ops.push({ op: 'set_constraints', constraints: clone(target.constraints) });
  if (g('master')) ops.push({ op: 'set_master', master: clone(target.master) });
  if (g('chords')) ops.push({ op: 'set_chords', chords: clone(target.chords) });

  const cur = new Map(current.layers.map((l) => [l.id, l]));
  const tgt = new Map(target.layers.map((l) => [l.id, l]));
  const curP = new Map(current.patterns.map((p) => [p.id, p]));
  const tgtP = new Map(target.patterns.map((p) => [p.id, p]));

  for (const [id, l] of cur) if (tgt.has(id) && l.locked && !tgt.get(id)!.locked) ops.push({ op: 'set_lock', id, locked: false });
  for (const [id, p] of tgtP) if (!curP.has(id)) ops.push({ op: 'add_pattern', pattern: clone(p) });
  for (const [id, p] of tgtP) if (curP.has(id) && !deepEqual(curP.get(id), p)) ops.push({ op: 'replace_pattern', pattern: clone(p) });
  for (const [id, l] of tgt) if (!cur.has(id)) ops.push({ op: 'add_layer', layer: clone(l) });
  for (const [id, t] of tgt) {
    const c = cur.get(id);
    if (!c) continue;
    const update: Record<string, unknown> = {};
    for (const f of LAYER_FIELDS) if (!deepEqual(c[f], t[f])) update[f] = clone(t[f]);
    if (Object.keys(update).length) ops.push({ op: 'update_layer', id, update: update as LayerUpdate });
    if (!deepEqual(c.clips, t.clips)) ops.push({ op: 'set_clips', layer: id, clips: clone(t.clips) });
  }
  for (const id of cur.keys()) if (!tgt.has(id)) ops.push({ op: 'remove_layer', id });
  for (const id of curP.keys()) if (!tgtP.has(id)) ops.push({ op: 'remove_pattern', id });
  for (const [id, l] of tgt) if (cur.has(id) && l.locked && !cur.get(id)!.locked) ops.push({ op: 'set_lock', id, locked: true });

  return { description: `Restore version "${name}"`, origin: 'user', scope: { layers: [] }, ops };
}

// ---------------------------------------------------------------- display

/** "bars 13-16 · layers kick" style description of a patch scope. */
export function describeScope(patch: ScorePatch): string {
  const bars = patch.scope?.bars ? formatRange(patch.scope.bars) : 'all bars';
  const layers = patch.scope?.layers?.length ? `layers ${patch.scope.layers.join(', ')}` : 'any unlocked layer';
  return `${bars} · ${layers}`;
}
