// Building a sound by hand (WP-U5): every control in the Sound panel turns into a ScorePatch here
// (pure functions, tested without a DOM). Patches go through `applyPatch`, so they are heard at
// once, undoable and checked by the same rules as any other edit; "Make a take" records the live
// score as one take afterwards.

import type { Take } from '../engine/EngineAdapter';
import type { PresetInfo } from '../engine/EngineAdapter';
import { presetLabel } from '../engine/presets';
import type { Accent, BarRange, Clip, Grid, Key, LayerUpdate, NLayer, NScore, Note, PatchOp, ScorePatch, TimeSignature } from '../score/types';
import { clone, deepEqual, formatRange, rangesOverlap } from '../score/types';
import { inKey, isUnpitchedClass, keyLabel, semitone, stepsPerBar } from '../score/theory';

const user = (description: string, ops: PatchOp[], layers: string[] = []): ScorePatch => ({ description, origin: 'user', scope: { layers }, ops });

const ID_OK = /^[a-z0-9_-]{1,64}$/;

/** A free id from `stem` (`stem`, `stem_2`, ...), valid as a score id. */
export function freshId(stem: string, taken: Set<string>): string {
  const base = stem.toLowerCase().replace(/[^a-z0-9_-]+/g, '_').replace(/^_+|_+$/g, '').slice(0, 56) || 'layer';
  if (!taken.has(base) && ID_OK.test(base)) return base;
  for (let n = 2; ; n++) {
    const id = `${base}_${n}`;
    if (!taken.has(id)) return id;
  }
}

/** The grid a new pattern uses: eighth notes when they divide the bar, else the first that does. */
export function newGrid(ts: TimeSignature): Grid {
  for (const g of ['eighth', 'sixteenth', 'quarter'] as Grid[]) if (stepsPerBar(ts, g) !== null) return g;
  return 'quarter';
}

// ---------------------------------------------------------------- layers

/**
 * Adds a layer playing `preset` in every bar, with an empty pattern to write notes into: two bars
 * for a pitched instrument (room for a motif), one bar for drums.
 */
export function addLayerPatch(score: NScore, preset: PresetInfo): ScorePatch {
  const id = freshId(preset.name, new Set(score.layers.map((l) => l.id)));
  const patternId = freshId(`${id}_p1`, new Set(score.patterns.map((p) => p.id)));
  const grid = newGrid(score.time_signature);
  const spb = stepsPerBar(score.time_signature, grid) ?? 1;
  const unpitched = isUnpitchedClass(preset.class) || !preset.pitched;
  const bars = unpitched || score.bars < 2 ? 1 : 2;
  const name = presetLabel(preset.name);
  const used = new Set(score.layers.map((l) => l.name));
  const label = used.has(name) ? `${name} ${score.layers.filter((l) => l.name.startsWith(name)).length + 1}` : name;
  return user(
    `Added ${label}`,
    [
      { op: 'add_pattern', pattern: { id: patternId, grid, length_steps: bars * spb, notes: [] } },
      {
        op: 'add_layer',
        layer: {
          id,
          name: label,
          class: preset.class,
          instrument: { kind: preset.kind, preset: preset.name, params: {} },
          clips: [{ bars: { start: 1, end: score.bars }, pattern: patternId }],
          gain_db: 0,
          pan: 0,
        },
      },
    ],
    [id],
  );
}

function layerOf(score: NScore, id: string): NLayer {
  const l = score.layers.find((x) => x.id === id);
  if (!l) throw new Error(`no layer ${id}`);
  return l;
}

export function nameOf(l: NLayer): string {
  return l.name || l.id;
}

/** Removes a layer and the patterns only it played. */
export function removeLayerPatch(score: NScore, id: string): ScorePatch {
  const l = layerOf(score, id);
  const mine = new Set(l.clips.map((c) => c.pattern));
  const others = new Set(score.layers.filter((x) => x.id !== id).flatMap((x) => x.clips.map((c) => c.pattern)));
  const ops: PatchOp[] = [{ op: 'remove_layer', id }];
  for (const p of score.patterns) if (mine.has(p.id) && !others.has(p.id)) ops.push({ op: 'remove_pattern', id: p.id });
  return user(`Removed ${nameOf(l)}`, ops, [id]);
}

export function renameLayerPatch(score: NScore, id: string, name: string): ScorePatch | null {
  const l = layerOf(score, id);
  const n = name.trim();
  if (!n || n === l.name) return null;
  return user(`Renamed ${nameOf(l)} to ${n}`, [{ op: 'update_layer', id, update: { name: n } }], [id]);
}

/**
 * Changes a layer's instrument. The layer keeps its class unless the new instrument is a drum
 * where it was pitched (or the other way round); a name that was the old instrument's follows.
 */
export function instrumentPatch(score: NScore, id: string, preset: PresetInfo): ScorePatch | null {
  const l = layerOf(score, id);
  if (l.instrument.preset === preset.name && l.instrument.kind === preset.kind) return null;
  const update: Record<string, unknown> = { instrument: { kind: preset.kind, preset: preset.name, params: {} } };
  const wasUnpitched = isUnpitchedClass(l.class);
  const nowUnpitched = isUnpitchedClass(preset.class) || !preset.pitched;
  if (wasUnpitched !== nowUnpitched) update.class = preset.class;
  if (l.name === presetLabel(l.instrument.preset) || !l.name) update.name = presetLabel(preset.name);
  return user(`${nameOf(l)} now plays ${presetLabel(preset.name).toLowerCase()}`, [{ op: 'update_layer', id, update: update as LayerUpdate }], [id]);
}

export function gainPatch(score: NScore, id: string, gain_db: number): ScorePatch | null {
  const l = layerOf(score, id);
  const g = Math.round(gain_db * 2) / 2;
  if (g === l.gain_db) return null;
  return user(`${nameOf(l)} level ${dbText(g)}`, [{ op: 'update_layer', id, update: { gain_db: g } }], [id]);
}

export function panPatch(score: NScore, id: string, pan: number): ScorePatch | null {
  const l = layerOf(score, id);
  const p = Math.max(-1, Math.min(1, Math.round(pan * 20) / 20));
  if (p === l.pan) return null;
  return user(`${nameOf(l)} pan ${panText(p).toLowerCase()}`, [{ op: 'update_layer', id, update: { pan: p } }], [id]);
}

export function dbText(db: number): string {
  return `${db > 0 ? '+' : db < 0 ? '−' : ''}${Math.abs(db).toFixed(1)} dB`;
}

export function panText(pan: number): string {
  if (Math.abs(pan) < 0.025) return 'Centre';
  return `${Math.round(Math.abs(pan) * 100)}% ${pan < 0 ? 'left' : 'right'}`;
}

/** First and last bar a layer plays, or null when it plays nowhere. */
export function layerSpan(l: NLayer): BarRange | null {
  if (!l.clips.length) return null;
  return { start: Math.min(...l.clips.map((c) => c.bars.start)), end: Math.max(...l.clips.map((c) => c.bars.end)) };
}

const mod = (a: number, n: number) => ((a % n) + n) % n;

/**
 * Makes a layer play bars `from`-`to` only: bars outside are cleared, bars inside that are silent
 * get the layer's pattern, in phase with where it already plays (so a motif keeps its shape).
 * Null when the layer already plays exactly those bars.
 */
export function layerBarsPatch(score: NScore, id: string, from: number, to: number): ScorePatch | null {
  const l = layerOf(score, id);
  const range = { start: Math.max(1, Math.min(from, to)), end: Math.min(score.bars, Math.max(from, to)) };
  const ops: PatchOp[] = [];
  const covered = (bar: number) => l.clips.some((c) => bar >= c.bars.start && bar <= c.bars.end);
  if (range.start > 1 && l.clips.some((c) => rangesOverlap(c.bars, { start: 1, end: range.start - 1 }))) {
    ops.push({ op: 'clear_bars', layer: id, bars: { start: 1, end: range.start - 1 } });
  }
  if (range.end < score.bars && l.clips.some((c) => rangesOverlap(c.bars, { start: range.end + 1, end: score.bars }))) {
    ops.push({ op: 'clear_bars', layer: id, bars: { start: range.end + 1, end: score.bars } });
  }
  // The pattern to fill silent bars with, and the phase it plays at bar x.
  const anchor = l.clips[0];
  let patternId = anchor?.pattern;
  let phase: (bar: number) => number = () => 0;
  if (anchor) {
    const pat = score.patterns.find((p) => p.id === anchor.pattern);
    const spb = pat ? stepsPerBar(score.time_signature, pat.grid) : null;
    if (pat && spb && pat.length_steps > 0) phase = (bar) => mod((anchor.pattern_offset_steps ?? 0) + (bar - anchor.bars.start) * spb, pat.length_steps);
  } else {
    const grid = newGrid(score.time_signature);
    const spb = stepsPerBar(score.time_signature, grid) ?? 1;
    patternId = freshId(`${id}_p1`, new Set(score.patterns.map((p) => p.id)));
    ops.push({ op: 'add_pattern', pattern: { id: patternId, grid, length_steps: (isUnpitchedClass(l.class) ? 1 : 2) * spb, notes: [] } });
  }
  let gap: number | null = null;
  const flush = (end: number) => {
    if (gap === null) return;
    const clip: Clip = { bars: { start: gap, end }, pattern: patternId! };
    const off = phase(gap);
    if (off) clip.pattern_offset_steps = off;
    ops.push({ op: 'place_clip', layer: id, clip });
    gap = null;
  };
  for (let bar = range.start; bar <= range.end; bar++) {
    if (!covered(bar)) gap ??= bar;
    else flush(bar - 1);
  }
  flush(range.end);
  if (!ops.length) return null;
  const words = range.start === range.end ? `bar ${range.start}` : `bars ${range.start}–${range.end}`;
  return user(`${nameOf(l)} plays in ${words}`, ops, [id]);
}

// ---------------------------------------------------------------- sound settings (global ops)

export function keyPatch(score: NScore, key: Key): ScorePatch | null {
  if (deepEqual(key, score.key)) return null;
  // Notes move with the key (always in key): by the smallest step between the tonics.
  return user(`Key ${keyLabel(key)}`, [{ op: 'set_key', key: { ...key }, transpose_notes: true }]);
}

export function tempoPatch(score: NScore, bpm: number): ScorePatch | null {
  const b = Math.round(bpm * 100) / 100;
  if (!Number.isFinite(b) || b === score.tempo.bpm) return null;
  return user(`Tempo ${b} BPM`, [{ op: 'set_tempo', bpm: b }]);
}

export function playbackPatch(score: NScore, loop: boolean): ScorePatch | null {
  if ((score.playback.mode === 'loop') === loop) return null;
  return user(loop ? 'Plays as a loop' : 'Plays once (one-shot)', [{ op: 'set_playback', playback: loop ? { mode: 'loop' } : { mode: 'one_shot', tail_seconds: 4 } }]);
}

export function timeSignaturePatch(score: NScore, ts: TimeSignature): ScorePatch | null {
  if (deepEqual(ts, score.time_signature)) return null;
  return user(`Time ${ts.numerator}/${ts.denominator}`, [{ op: 'set_time_signature', time_signature: { ...ts } }]);
}

/** Why the patterns cannot be read in `ts` (their length is not whole bars), or null. */
export function timeSignatureProblem(score: NScore, ts: TimeSignature): string | null {
  for (const p of score.patterns) {
    const spb = stepsPerBar(ts, p.grid);
    if (spb === null || p.length_steps % spb !== 0) return `The notes are written in ${score.time_signature.numerator}/${score.time_signature.denominator}; they do not fit whole bars of ${ts.numerator}/${ts.denominator}. Change the time signature before writing notes.`;
  }
  return null;
}

/**
 * Changes the number of bars. Shortening trims what plays past the new end (clips, sections and
 * chords) in the same patch, so nothing is left outside the sound; lengthening leaves layers
 * where they are (set which bars each plays).
 */
export function barsPatch(score: NScore, bars: number): ScorePatch | null {
  if (!Number.isInteger(bars) || bars < 1 || bars === score.bars) return null;
  const ops: PatchOp[] = [];
  if (bars < score.bars) {
    for (const l of score.layers) {
      if (!l.clips.some((c) => c.bars.end > bars)) continue;
      const clips = l.clips.filter((c) => c.bars.start <= bars).map((c) => ({ ...clone(c), bars: { start: c.bars.start, end: Math.min(c.bars.end, bars) } }));
      ops.push({ op: 'set_clips', layer: l.id, clips });
    }
    for (const sec of score.sections) {
      if (sec.bars.start > bars) ops.push({ op: 'remove_section', id: sec.id });
      else if (sec.bars.end > bars) ops.push({ op: 'update_section', section: { ...clone(sec), bars: { start: sec.bars.start, end: bars } } });
    }
    if (score.chords.some((c) => c.bars.end > bars)) {
      ops.push({ op: 'set_chords', chords: score.chords.filter((c) => c.bars.start <= bars).map((c) => ({ ...clone(c), bars: { start: c.bars.start, end: Math.min(c.bars.end, bars) } })) });
    }
  }
  ops.push({ op: 'set_bars', bars });
  return user(`${bars} bars`, ops);
}

// ---------------------------------------------------------------- notes

/** The in-key pitch nearest `pitch` (ties go up); any pitch for unpitched layers or chromatic keys. */
export function snapToKey(key: Key, pitch: number): number {
  const p = Math.max(0, Math.min(127, Math.round(pitch)));
  if (key.mode === 'chromatic' || inKey(key, p)) return p;
  for (let d = 1; d < 12; d++) {
    if (p + d <= 127 && inKey(key, p + d)) return p + d;
    if (p - d >= 0 && inKey(key, p - d)) return p - d;
  }
  return p;
}

/** The key's tonic nearest middle C (A minor -> A3, 57): where a new pitched part starts. */
export function tonicNear(key: Key, around = 60): number {
  const t = semitone(key.tonic);
  const base = around - mod(around - t, 12);
  return around - base > 6 ? base + 12 : base;
}

export type SpokenStep = { kind: 'hit'; accent: Accent; velocity: number } | { kind: 'rest' } | { kind: 'tie' };

const SYLLABLES: Record<string, [Accent, number]> = {
  DON: ['strong', 0.8],
  don: ['normal', 0.65],
  Don: ['normal', 0.65],
  din: ['soft', 0.55],
  Din: ['soft', 0.55],
  DIN: ['normal', 0.6],
  TIN: ['bright', 0.7],
  tin: ['bright', 0.6],
  Tin: ['bright', 0.6],
};

/**
 * Spoken rhythm (FR-5), as crates/praxis-score/src/rhythm.rs reads it: one step per syllable,
 * DON strong, don normal, din soft, TIN bright, "-" or "." a rest, "~" a tie; "|" is only visual.
 */
export function parseSpoken(text: string): { ok: true; steps: SpokenStep[] } | { ok: false; message: string } {
  const steps: SpokenStep[] = [];
  let canTie = false;
  const tokens: string[] = [];
  let word = '';
  for (const c of text) {
    if (/\s/.test(c) || c === '|' || c === '-' || c === '.' || c === '~') {
      if (word) tokens.push(word);
      word = '';
      if (c === '-' || c === '.' || c === '~') tokens.push(c);
    } else word += c;
  }
  if (word) tokens.push(word);
  for (const t of tokens) {
    if (t === '-' || t === '.') {
      canTie = false;
      steps.push({ kind: 'rest' });
    } else if (t === '~') {
      if (!canTie) return { ok: false, message: 'A "~" needs a hit before it to hold.' };
      steps.push({ kind: 'tie' });
    } else {
      const s = SYLLABLES[t];
      if (!s) return { ok: false, message: `"${t}" is not a rhythm word: use DON, don, din, TIN, "-" for a rest or "~" to hold.` };
      canTie = true;
      steps.push({ kind: 'hit', accent: s[0], velocity: s[1] });
    }
  }
  return { ok: true, steps };
}

/**
 * Writes spoken steps into a pattern's notes from `start`: a hit keeps the pitch already at that
 * step (else `pitch`), a rest silences the step, a tie holds the previous hit one step longer.
 * Pitched notes ring 1.6 steps so they connect; drum hits are one step.
 */
export function writeSpoken(notes: Note[], start: number, length: number, steps: SpokenStep[], pitch: number, unpitched: boolean): Note[] {
  let out = notes.map((n) => ({ ...n }));
  let last: Note | null = null;
  steps.forEach((st, i) => {
    const step = start + i;
    if (step >= length) return;
    const here = out.filter((n) => n.step === step).sort((a, b) => b.velocity - a.velocity)[0];
    out = out.filter((n) => n.step !== step);
    if (st.kind === 'hit') {
      last = { step, pitch: here?.pitch ?? pitch, velocity: st.velocity, accent: st.accent, length: unpitched ? 1 : 1.6 };
      out.push(last);
    } else if (st.kind === 'tie' && last) {
      last.length = Math.min(64, last.length + 1);
    } else {
      last = null;
    }
  });
  return out.sort((a, b) => a.step - b.step || a.pitch - b.pitch);
}

// ---------------------------------------------------------------- the batch

/** A score's content: what a take renders, without the auditioning switches (mute, solo, lock). */
function content(score: NScore): NScore {
  const s = clone(score);
  s.layers.forEach((l) => {
    l.muted = false;
    l.solo = false;
    l.locked = false;
  });
  return s;
}

/**
 * True when the live score holds edits no take has recorded: a blank sound with a layer, or a
 * live score whose content differs from the current take's (mute, solo and lock aside).
 */
export function hasUnsaved(take: Take | null, score: NScore | null, draft: boolean): boolean {
  if (!score) return false;
  if (!take) return draft && score.layers.length > 0;
  return !deepEqual(content(take.score), content(score));
}

/** One hand edit in the batch since the last take. */
export interface BatchEdit {
  label: string;
  layer?: string;
  bars?: BarRange;
  /** Edits of the same setting share a key; a later one replaces the earlier in the wording. */
  key?: string | null;
}

/**
 * The setting a patch sets, when it sets one (the tempo, a layer's level...), so the batch reads
 * "tempo 128 BPM" once, not every value dragged through on the way.
 */
export function batchKey(patch: ScorePatch): string | null {
  const ops = patch.ops;
  if (ops.some((o) => o.op === 'set_bars')) return 'set_bars';
  if (ops.length !== 1) return null;
  const op = ops[0];
  switch (op.op) {
    case 'set_tempo':
    case 'set_key':
    case 'set_playback':
    case 'set_time_signature':
      return op.op;
    case 'update_layer': {
      const keys = Object.keys(op.update).filter((k) => (op.update as Record<string, unknown>)[k] !== undefined);
      if (keys.includes('instrument')) return `${op.id}:instrument`;
      return keys.length === 1 ? `${op.id}:${keys[0]}` : null;
    }
    default:
      return null;
  }
}

/** Adds an edit to the batch, dropping an earlier edit of the same setting. */
export function addToBatch(batch: BatchEdit[], edit: BatchEdit): BatchEdit[] {
  const out = edit.key ? batch.filter((b) => b.key !== edit.key) : [...batch];
  out.push(edit);
  return out;
}

/** The request a batch reads as in the thread: "By hand: added Felt piano; ... and 2 more". */
export function batchAsk(batch: BatchEdit[]): string {
  if (!batch.length) return 'Hand edits';
  const words = batch.map((b) => b.label.charAt(0).toLowerCase() + b.label.slice(1));
  const shown = words.slice(0, 4).join('; ');
  return `By hand: ${shown}${words.length > 4 ? `; and ${words.length - 4} more` : ''}`;
}

/** The scope of a batch: bars of one layer, when every edit in it was there. */
export function batchScope(batch: BatchEdit[]): { layer: string; bars: BarRange } | undefined {
  if (!batch.length || batch.some((b) => !b.layer || !b.bars)) return undefined;
  const layer = batch[0].layer!;
  if (batch.some((b) => b.layer !== layer)) return undefined;
  return { layer, bars: { start: Math.min(...batch.map((b) => b.bars!.start)), end: Math.max(...batch.map((b) => b.bars!.end)) } };
}

export function rangeWords(r: BarRange): string {
  return formatRange(r).replace('-', '–');
}
