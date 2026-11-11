// Mock patch application: a TypeScript stand-in for crates/praxis-score/src/apply/ (WP-A),
// keeping the rules in docs/contracts/patch.md for the ops the UI emits:
//   1 atomic, 2 locks (user mute/solo allowed on locked layers), 3 scope, 4 copy-on-write
//   splits with phase kept and write_bars id resolution, 5 exact inverse, 7 insert positions.
// Supported ops are listed in MOCK_SUPPORTED_OPS; anything else is rejected as `invalid`.

import type { ApplyResult, PatchError } from '../EngineAdapter';
import type { BarRange, Clip, Key, LayerUpdate, NLayer, NScore, OpName, OpOf, Origin, Pattern, PatchOp, ScorePatch } from '../../score/types';
import { clone, deepEqual, normalizeLayer, normalizePattern, rangeContains, rangesOverlap } from '../../score/types';
import { isUnpitchedClass, semitone, stepsPerBar } from '../../score/theory';
import { composeId } from '../../score/ids';
import { eventKey, layerEvents } from '../../score/timeline';
import { validate } from './validate';

export const MOCK_SUPPORTED_OPS: OpName[] = [
  'set_title',
  'set_tempo',
  'set_seed',
  'set_key',
  'set_time_signature',
  'set_bars',
  'set_playback',
  'set_chords',
  'add_section',
  'update_section',
  'remove_section',
  'add_layer',
  'remove_layer',
  'update_layer',
  'set_lock',
  'add_pattern',
  'replace_pattern',
  'remove_pattern',
  'write_bars',
  'clear_bars',
  'place_clip',
  'set_clips',
];

const GLOBAL_OPS: OpName[] = [
  'set_title',
  'set_meta',
  'set_tempo',
  'set_key',
  'set_time_signature',
  'set_bars',
  'set_playback',
  'set_constraints',
  'set_master',
  'set_seed',
];

const ID_RE = /^[a-z0-9_-]{1,64}$/;

/** Fields of a layer update that are auditioning, not content (rule 2). */
const AUDITION_FIELDS = new Set(['muted', 'solo']);

/** True when an `origin: user` update_layer changes only `muted`/`solo` (allowed on a locked layer). */
export function isAuditionOnly(update: LayerUpdate, origin: Origin): boolean {
  return origin === 'user' && Object.entries(update).every(([k, v]) => v === null || v === undefined || AUDITION_FIELDS.has(k));
}

function sameContent(a: Pattern, b: Pattern): boolean {
  return a.grid === b.grid && a.length_steps === b.length_steps && deepEqual(a.notes, b.notes) && (a.spoken ?? null) === (b.spoken ?? null);
}

/**
 * Where a write_bars pattern lives (ids.rs `resolve_write_pattern`): its own id when valid and
 * free; an identical existing pattern is reused; otherwise `<layer>_b<start>-<end>`, `_2`, ...
 * until the id is free or holds identical content. Never DuplicateId.
 */
export function resolveWritePattern(s: NScore, layer: string, bars: BarRange, pattern: Pattern): { id: string; add: boolean } {
  if (ID_RE.test(pattern.id)) {
    const existing = s.patterns.find((p) => p.id === pattern.id);
    if (!existing) return { id: pattern.id, add: true };
    if (deepEqual(existing, normalizePattern(pattern))) return { id: pattern.id, add: false };
  }
  const tail = `_b${bars.start}-${bars.end}`;
  for (let n = 1; n <= 10000; n++) {
    const id = composeId(layer, tail, n);
    const existing = s.patterns.find((p) => p.id === id);
    if (!existing) return { id, add: true };
    if (sameContent(existing, normalizePattern(pattern))) return { id, add: false };
  }
  throw new Error(`no free pattern id for layer \`${layer}\` ${tail}`);
}

/** `PatchOp::is_global`: valid only when `scope.bars` is unset. */
export function isGlobalOp(op: PatchOp): boolean {
  return GLOBAL_OPS.includes(op.op);
}

/** The layer an op changes, if it is a layer op. */
export function opLayer(op: PatchOp): string | null {
  switch (op.op) {
    case 'write_bars':
    case 'clear_bars':
    case 'place_clip':
    case 'set_clips':
      return op.layer;
    case 'update_layer':
    case 'set_lock':
    case 'remove_layer':
      return op.id;
    case 'add_layer':
      return op.layer.id;
    default:
      return null;
  }
}

/** The bar range a bar op touches. */
export function opBars(op: PatchOp): BarRange | null {
  switch (op.op) {
    case 'write_bars':
    case 'clear_bars':
      return op.bars;
    case 'place_clip':
      return op.clip.bars;
    default:
      return null;
  }
}

class Reject {
  constructor(readonly error: PatchError) {}
}

export function applyPatch(score: NScore, patch: ScorePatch): ApplyResult {
  try {
    return applyInner(score, patch);
  } catch (e) {
    if (e instanceof Reject) return { ok: false, error: e.error };
    throw e;
  }
}

function applyInner(input: NScore, patch: ScorePatch): ApplyResult {
  const s = clone(input);
  const origin = patch.origin ?? 'user';
  const scope = { bars: patch.scope?.bars ?? null, layers: patch.scope?.layers ?? [] };
  let inverse: PatchOp[] = [];

  patch.ops.forEach((op, index) => {
    const fail = (error: PatchError): never => {
      throw new Reject(error);
    };
    const outOfScope = (reason: string) => fail({ kind: 'out_of_scope', index, op: op.op, reason });
    const unknown = (what: string, id: string) => fail({ kind: 'unknown_id', index, op: op.op, what, id });
    const duplicate = (what: string, id: string) => fail({ kind: 'duplicate_id', index, op: op.op, what, id });

    if (!MOCK_SUPPORTED_OPS.includes(op.op)) {
      fail({ kind: 'invalid', message: `the mock engine does not implement op \`${op.op}\` yet` });
    }

    // Rule 3: scope.
    if (isGlobalOp(op) && scope.bars) outOfScope('global ops need a whole-score scope (scope.bars is set)');
    const lid = opLayer(op);
    if (lid !== null && scope.layers.length > 0 && !scope.layers.includes(lid)) {
      outOfScope(`layer \`${lid}\` is not in scope.layers [${scope.layers.join(', ')}]`);
    }
    const bars = opBars(op);
    if (bars && scope.bars && !rangeContains(scope.bars, bars)) {
      outOfScope(`bars ${bars.start}-${bars.end} are not inside scope bars ${scope.bars.start}-${scope.bars.end}`);
    }

    // Rule 2: locks (add_layer creates the layer, so there is nothing locked yet).
    let layer: NLayer | undefined;
    if (lid !== null && op.op !== 'add_layer') {
      layer = s.layers.find((l) => l.id === lid);
      if (!layer) unknown('layer', lid);
      if (op.op === 'set_lock') {
        if (!op.locked && layer!.locked && origin !== 'user') fail({ kind: 'unlock_not_allowed', layer: lid });
      } else if (layer!.locked && !(op.op === 'update_layer' && isAuditionOnly(op.update, origin))) {
        fail({ kind: 'locked_layer', layer: lid });
      }
    }

    const opInverse = applyOp(s, op, layer, scope, { outOfScope, unknown, duplicate, fail });
    inverse = [...opInverse, ...inverse];
  });

  // Rule 1: the result must validate.
  const report = validate(s);
  if (report.errors.length > 0) {
    const first = report.errors.slice(0, 3).map((e) => `${e.code} at ${e.path}: ${e.message}`);
    const more = report.errors.length > 3 ? ` (+${report.errors.length - 3} more)` : '';
    return { ok: false, error: { kind: 'invalid', message: first.join('; ') + more } };
  }

  const inv: ScorePatch = {
    description: patch.description ? `Undo: ${patch.description}` : 'Undo',
    origin,
    scope: patch.scope ?? { layers: [] },
    ops: inverse,
  };
  return { ok: true, score: s, inverse: inv };
}

interface Fails {
  outOfScope: (reason: string) => never;
  unknown: (what: string, id: string) => never;
  duplicate: (what: string, id: string) => never;
  fail: (e: PatchError) => never;
}

interface ScopeN {
  bars: BarRange | null;
  layers: string[];
}

/** Applies one op in place and returns the ops that undo it (in order). */
function applyOp(s: NScore, op: PatchOp, layer: NLayer | undefined, scope: ScopeN, f: Fails): PatchOp[] {
  switch (op.op) {
    case 'set_title': {
      const prev = s.title;
      s.title = op.title;
      return [{ op: 'set_title', title: prev }];
    }
    case 'set_tempo': {
      const prev = s.tempo.bpm;
      s.tempo = { ...s.tempo, bpm: op.bpm };
      return [{ op: 'set_tempo', bpm: prev }];
    }
    case 'set_seed': {
      const prev = s.seed;
      s.seed = op.seed;
      return [{ op: 'set_seed', seed: prev }];
    }
    case 'set_key':
      return setKey(s, op);
    case 'set_time_signature': {
      const prev = clone(s.time_signature);
      s.time_signature = clone(op.time_signature);
      return [{ op: 'set_time_signature', time_signature: prev }];
    }
    case 'set_bars': {
      const prev = s.bars;
      s.bars = op.bars;
      return [{ op: 'set_bars', bars: prev }];
    }
    case 'set_playback': {
      const prev = clone(s.playback);
      s.playback = clone(op.playback);
      return [{ op: 'set_playback', playback: prev }];
    }
    case 'set_chords': {
      const prev = clone(s.chords);
      s.chords = clone(op.chords);
      return [{ op: 'set_chords', chords: prev }];
    }
    case 'add_section': {
      if (s.sections.some((x) => x.id === op.section.id)) f.duplicate('section', op.section.id);
      if (scope.bars && !rangeContains(scope.bars, op.section.bars)) f.outOfScope('section bars are outside scope bars');
      insertAt(s.sections, clone(op.section), op.index, f);
      return [{ op: 'remove_section', id: op.section.id }];
    }
    case 'update_section': {
      const i = s.sections.findIndex((x) => x.id === op.section.id);
      if (i < 0) f.unknown('section', op.section.id);
      if (scope.bars && (!rangeContains(scope.bars, s.sections[i].bars) || !rangeContains(scope.bars, op.section.bars))) f.outOfScope('section bars are outside scope bars');
      const prev = s.sections[i];
      s.sections[i] = clone(op.section);
      return [{ op: 'update_section', section: prev }];
    }
    case 'remove_section': {
      const i = s.sections.findIndex((x) => x.id === op.id);
      if (i < 0) f.unknown('section', op.id);
      if (scope.bars && !rangeContains(scope.bars, s.sections[i].bars)) f.outOfScope('section bars are outside scope bars');
      const [removed] = s.sections.splice(i, 1);
      return [{ op: 'add_section', section: removed, index: i }];
    }
    case 'add_layer': {
      if (s.layers.some((l) => l.id === op.layer.id)) f.duplicate('layer', op.layer.id);
      for (const c of op.layer.clips ?? []) {
        if (scope.bars && !rangeContains(scope.bars, c.bars)) f.outOfScope(`clip bars ${c.bars.start}-${c.bars.end} are outside scope bars`);
        if (!s.patterns.some((p) => p.id === c.pattern)) f.unknown('pattern', c.pattern);
      }
      insertAt(s.layers, normalizeLayer(clone(op.layer)), op.index, f);
      // A layer added locked has to be unlocked before its removal can undo the add.
      const inv: PatchOp[] = op.layer.locked ? [{ op: 'set_lock', id: op.layer.id, locked: false }] : [];
      return [...inv, { op: 'remove_layer', id: op.layer.id }];
    }
    case 'remove_layer': {
      const idx = s.layers.indexOf(layer!);
      const outside = scope.bars ? layer!.clips.find((c) => !rangeContains(scope.bars!, c.bars)) : undefined;
      if (outside) f.outOfScope(`layer \`${layer!.id}\` plays bars ${outside.bars.start}-${outside.bars.end}, outside scope bars`);
      s.layers.splice(idx, 1);
      return [{ op: 'add_layer', layer: clone(layer!), index: idx }];
    }
    case 'update_layer': {
      if (scope.bars && changesSound(layer!, op.update)) {
        f.outOfScope(`with scope bars ${scope.bars.start}-${scope.bars.end} update_layer may only rename layer \`${layer!.id}\`; other properties change every bar`);
      }
      return [updateLayer(layer!, op)];
    }
    case 'set_lock': {
      const prev = layer!.locked;
      layer!.locked = op.locked;
      return [{ op: 'set_lock', id: layer!.id, locked: prev }];
    }
    case 'add_pattern': {
      if (s.patterns.some((p) => p.id === op.pattern.id)) f.duplicate('pattern', op.pattern.id);
      insertAt(s.patterns, normalizePattern(clone(op.pattern)), op.index, f);
      return [{ op: 'remove_pattern', id: op.pattern.id }];
    }
    case 'remove_pattern': {
      const idx = s.patterns.findIndex((p) => p.id === op.id);
      if (idx < 0) f.unknown('pattern', op.id);
      const user = s.layers.find((l) => l.clips.some((c) => c.pattern === op.id));
      if (user?.locked) f.fail({ kind: 'locked_layer', layer: user.id });
      if (user) f.fail({ kind: 'invalid', message: `pattern \`${op.id}\` is still played by layer \`${user.id}\`` });
      const [removed] = s.patterns.splice(idx, 1);
      return [{ op: 'add_pattern', pattern: removed, index: idx }];
    }
    case 'replace_pattern': {
      const idx = s.patterns.findIndex((p) => p.id === op.pattern.id);
      if (idx < 0) f.unknown('pattern', op.pattern.id);
      for (const l of s.layers) {
        for (const c of l.clips) {
          if (c.pattern !== op.pattern.id) continue;
          if (l.locked) f.fail({ kind: 'locked_layer', layer: l.id });
          if (scope.layers.length > 0 && !scope.layers.includes(l.id)) {
            f.outOfScope(`pattern \`${op.pattern.id}\` is also used by layer \`${l.id}\``);
          }
          if (scope.bars && !rangeContains(scope.bars, c.bars)) {
            f.outOfScope(`pattern \`${op.pattern.id}\` is also used in bars ${c.bars.start}-${c.bars.end}`);
          }
        }
      }
      const prev = s.patterns[idx];
      s.patterns[idx] = normalizePattern(clone(op.pattern));
      return [{ op: 'replace_pattern', pattern: prev }];
    }
    case 'write_bars': {
      const { id, add } = resolveWritePattern(s, layer!.id, op.bars, op.pattern);
      const prev = clone(layer!.clips);
      if (add) s.patterns.push(normalizePattern({ ...clone(op.pattern), id }));
      layer!.clips = sortClips([...splitOut(s, layer!.clips, op.bars), { bars: { ...op.bars }, pattern: id, transpose: 0, velocity_scale: 1 }]);
      const inv: PatchOp[] = [{ op: 'set_clips', layer: layer!.id, clips: prev }];
      if (add) inv.push({ op: 'remove_pattern', id });
      return inv;
    }
    case 'clear_bars': {
      const prev = clone(layer!.clips);
      layer!.clips = splitOut(s, layer!.clips, op.bars);
      return [{ op: 'set_clips', layer: layer!.id, clips: prev }];
    }
    case 'place_clip': {
      if (!s.patterns.some((p) => p.id === op.clip.pattern)) f.unknown('pattern', op.clip.pattern);
      const prev = clone(layer!.clips);
      layer!.clips = sortClips([...splitOut(s, layer!.clips, op.clip.bars), clone(op.clip)]);
      return [{ op: 'set_clips', layer: layer!.id, clips: prev }];
    }
    case 'set_clips': {
      const prev = clone(layer!.clips);
      const before = scope.bars ? outsideEvents(s, layer!.id, scope.bars) : null;
      layer!.clips = clone(op.clips);
      if (before !== null && scope.bars) {
        const after = outsideEvents(s, layer!.id, scope.bars);
        if (before !== after) f.outOfScope(`set_clips changes notes outside bars ${scope.bars.start}-${scope.bars.end}`);
      }
      return [{ op: 'set_clips', layer: layer!.id, clips: prev }];
    }
    default:
      return f.fail({ kind: 'invalid', message: `the mock engine does not implement op \`${op.op}\` yet` });
  }
}

/** Smallest move from one tonic to another, -5..=6 semitones (apply/ops.rs `tonic_interval`). */
export function tonicInterval(from: Key['tonic'], to: Key['tonic']): number {
  const d = (((semitone(to) - semitone(from)) % 12) + 12) % 12;
  return d > 6 ? d - 12 : d;
}

/**
 * set_key. With `transpose_notes`, patterns played only by pitched layers move by the smallest
 * interval between the tonics (a pattern shared with a drum layer is left alone and the pitched
 * clips get `transpose` instead), as crates/praxis-score/src/apply/ops.rs does.
 */
function setKey(s: NScore, op: OpOf<'set_key'>): PatchOp[] {
  const old = clone(s.key);
  const inv: PatchOp[] = [];
  const d = tonicInterval(old.tonic, op.key.tonic);
  if (op.transpose_notes && d !== 0) {
    const pitched = s.layers.filter((l) => !isUnpitchedClass(l.class));
    const drumUses = new Set(s.layers.filter((l) => isUnpitchedClass(l.class)).flatMap((l) => l.clips.map((c) => c.pattern)));
    const used = new Set(pitched.flatMap((l) => l.clips.map((c) => c.pattern)));
    for (const id of used) {
      if (drumUses.has(id)) continue;
      const i = s.patterns.findIndex((p) => p.id === id);
      if (i < 0) continue;
      const prev = s.patterns[i];
      s.patterns[i] = { ...clone(prev), notes: prev.notes.map((n) => ({ ...n, pitch: n.pitch + d })) };
      inv.push({ op: 'replace_pattern', pattern: prev });
    }
    for (const l of pitched) {
      if (!l.clips.some((c) => drumUses.has(c.pattern))) continue;
      const prev = clone(l.clips);
      l.clips = l.clips.map((c) => (drumUses.has(c.pattern) ? { ...c, transpose: (c.transpose ?? 0) + d } : c));
      inv.push({ op: 'set_clips', layer: l.id, clips: prev });
    }
  }
  s.key = clone(op.key);
  inv.push({ op: 'set_key', key: old, transpose_notes: false });
  return inv;
}

function insertAt<T>(list: T[], item: T, index: number | null | undefined, f: Fails): void {
  if (index === null || index === undefined) {
    list.push(item);
    return;
  }
  if (!Number.isInteger(index) || index < 0 || index > list.length) {
    f.fail({ kind: 'invalid', message: `index ${index} is past the end of the list (${list.length} items)` });
  }
  list.splice(index, 0, item);
}

/** True when an update changes anything but the layer's name (that affects every bar). */
function changesSound(layer: NLayer, u: LayerUpdate): boolean {
  const rec = layer as unknown as Record<string, unknown>;
  return Object.entries(u).some(([k, v]) => k !== 'name' && v !== null && v !== undefined && !deepEqual(v, rec[k]));
}

function updateLayer(layer: NLayer, op: OpOf<'update_layer'>): PatchOp {
  const u = op.update;
  const back: OpOf<'update_layer'>['update'] = {};
  const rec = layer as unknown as Record<string, unknown>;
  const backRec = back as Record<string, unknown>;
  for (const [k, v] of Object.entries(u)) {
    if (v === null || v === undefined) continue;
    backRec[k] = clone(rec[k]);
    rec[k] = clone(v);
  }
  return { op: 'update_layer', id: layer.id, update: back };
}

function outsideEvents(s: NScore, layerId: string, bars: BarRange): string {
  return layerEvents(s, layerId)
    .filter((e) => e.bar < bars.start || e.bar > bars.end)
    .map(eventKey)
    .sort()
    .join('\n');
}

function sortClips(clips: Clip[]): Clip[] {
  return clips.sort((a, b) => a.bars.start - b.bars.start);
}

/**
 * Removes `range` from a clip list, splitting clips at its edges. The right-hand piece of a split
 * gets `pattern_offset_steps` so it keeps playing the steps it played before (rule 4).
 */
export function splitOut(s: NScore, clips: Clip[], range: BarRange): Clip[] {
  const out: Clip[] = [];
  for (const c of clips) {
    if (!rangesOverlap(c.bars, range)) {
      out.push(c);
      continue;
    }
    if (c.bars.start < range.start) {
      out.push({ ...c, bars: { start: c.bars.start, end: range.start - 1 } });
    }
    if (c.bars.end > range.end) {
      const start = range.end + 1;
      const pat: Pattern | undefined = s.patterns.find((p) => p.id === c.pattern);
      const right: Clip = { ...c, bars: { start, end: c.bars.end } };
      const spb = pat ? stepsPerBar(s.time_signature, pat.grid) : null;
      if (pat && spb !== null && pat.length_steps > 0) {
        const off = ((c.pattern_offset_steps ?? 0) + (start - c.bars.start) * spb) % pat.length_steps;
        if (off !== 0) right.pattern_offset_steps = off;
        else delete right.pattern_offset_steps;
      }
      out.push(right);
    }
  }
  return out;
}
