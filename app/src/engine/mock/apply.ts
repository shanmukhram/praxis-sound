// Mock patch application: a TypeScript stand-in for crates/praxis-score/src/apply.rs (WP-A),
// keeping the rules in docs/contracts/patch.md for the ops the UI emits:
//   1 atomic, 2 locks, 3 scope, 4 copy-on-write splits with phase kept, 5 exact inverse.
// Supported ops are listed in MOCK_SUPPORTED_OPS; anything else is rejected as `invalid`.

import type { ApplyResult, PatchError } from '../EngineAdapter';
import type { BarRange, Clip, NLayer, NScore, OpName, OpOf, Pattern, PatchOp, ScorePatch } from '../../score/types';
import { clone, normalizeLayer, normalizePattern, rangeContains, rangesOverlap } from '../../score/types';
import { stepsPerBar } from '../../score/theory';
import { eventKey, layerEvents } from '../../score/timeline';
import { validate } from './validate';

export const MOCK_SUPPORTED_OPS: OpName[] = [
  'set_title',
  'set_tempo',
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
];

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
      } else if (layer!.locked) {
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
    case 'add_layer': {
      if (s.layers.some((l) => l.id === op.layer.id)) f.duplicate('layer', op.layer.id);
      s.layers.push(normalizeLayer(clone(op.layer)));
      return [{ op: 'remove_layer', id: op.layer.id }];
    }
    case 'remove_layer': {
      const idx = s.layers.indexOf(layer!);
      s.layers.splice(idx, 1);
      // Contract gap: add_layer has no index, so this inverse appends (see app/README.md).
      return [{ op: 'add_layer', layer: clone(layer!) }];
    }
    case 'update_layer':
      return [updateLayer(layer!, op)];
    case 'set_lock': {
      const prev = layer!.locked;
      layer!.locked = op.locked;
      return [{ op: 'set_lock', id: layer!.id, locked: prev }];
    }
    case 'add_pattern': {
      if (s.patterns.some((p) => p.id === op.pattern.id)) f.duplicate('pattern', op.pattern.id);
      s.patterns.push(normalizePattern(clone(op.pattern)));
      return [{ op: 'remove_pattern', id: op.pattern.id }];
    }
    case 'remove_pattern': {
      const idx = s.patterns.findIndex((p) => p.id === op.id);
      if (idx < 0) f.unknown('pattern', op.id);
      const user = s.layers.find((l) => l.clips.some((c) => c.pattern === op.id));
      if (user) f.fail({ kind: 'invalid', message: `pattern \`${op.id}\` is still used by layer \`${user.id}\`` });
      const [removed] = s.patterns.splice(idx, 1);
      return [{ op: 'add_pattern', pattern: removed }];
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
      if (s.patterns.some((p) => p.id === op.pattern.id)) f.duplicate('pattern', op.pattern.id);
      const prev = clone(layer!.clips);
      s.patterns.push(normalizePattern(clone(op.pattern)));
      layer!.clips = sortClips([...splitOut(s, layer!.clips, op.bars), { bars: { ...op.bars }, pattern: op.pattern.id, transpose: 0, velocity_scale: 1 }]);
      return [
        { op: 'set_clips', layer: layer!.id, clips: prev },
        { op: 'remove_pattern', id: op.pattern.id },
      ];
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
