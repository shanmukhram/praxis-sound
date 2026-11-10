import { describe, expect, it } from 'vitest';
import {
  clearBars,
  derivePattern,
  describeScope,
  editPatternInRange,
  freshPatternId,
  lockLayer,
  muteLayer,
  restorePatch,
  setTempo,
  soloLayer,
  toggleNote,
  updateNote,
} from '../../src/score/patchBuilder';
import type { ScorePatch } from '../../src/score/types';
import { applyPatch } from '../../src/engine/mock/apply';
import { checkSchema } from './support/jsonSchema';
import { fixture, patchSchema, unlocked } from './fixtures';

const D = { velocity: 0.7, length: 1, accent: 'normal' as const };

function expectValidPatch(p: ScorePatch | null): asserts p is ScorePatch {
  expect(p).not.toBeNull();
  expect(checkSchema(patchSchema, p)).toEqual([]);
}

describe('layer patches carry exact scope', () => {
  const s = fixture();
  it('mute names only that layer and no bars', () => {
    const p = muteLayer(s, 'kick', true);
    expectValidPatch(p);
    expect(p.scope).toEqual({ layers: ['kick'] });
    expect(p.ops).toEqual([{ op: 'update_layer', id: 'kick', update: { muted: true } }]);
    expect(p.origin).toBe('user');
  });
  it('solo and lock likewise', () => {
    const solo = soloLayer(s, 'kick', true);
    const lock = lockLayer(s, 'piano', false);
    expectValidPatch(solo);
    expectValidPatch(lock);
    expect(solo.scope).toEqual({ layers: ['kick'] });
    expect(lock.ops).toEqual([{ op: 'set_lock', id: 'piano', locked: false }]);
  });
  it('tempo is a whole-score patch with no bar scope', () => {
    const p = setTempo(120);
    expectValidPatch(p);
    expect(p.scope?.bars).toBeUndefined();
  });
  it('clear bars is scoped to the bars and layer', () => {
    const p = clearBars(s, 'kick', { start: 13, end: 16 });
    expectValidPatch(p);
    expect(p.scope).toEqual({ bars: { start: 13, end: 16 }, layers: ['kick'] });
  });
});

describe('step edits in a bar range', () => {
  it('derives the looping kick pattern for bars 13-16', () => {
    const d = derivePattern(fixture(), 'kick', { start: 13, end: 16 })!;
    expect(d.mode).toBe('loop');
    expect(d.source).toBe('kick_soft');
    expect(d.replaceable).toBe(false); // the clip spans 9-16
    expect(d.pattern.length_steps).toBe(8);
    expect(d.pattern.notes.map((n) => n.step)).toEqual([0, 4]);
  });

  it('a step edit in bars 13-16 writes a new pattern scoped to exactly those bars', () => {
    const s = fixture();
    const d = derivePattern(s, 'kick', { start: 13, end: 16 })!;
    const notes = toggleNote(d.pattern.notes, 6, 36, D);
    const p = editPatternInRange(s, 'kick', { start: 13, end: 16 }, notes, 'Edit steps');
    expectValidPatch(p);
    expect(p.scope).toEqual({ bars: { start: 13, end: 16 }, layers: ['kick'] });
    expect(p.ops).toHaveLength(1);
    const op = p.ops[0];
    expect(op.op).toBe('write_bars');
    if (op.op !== 'write_bars') return;
    expect(op.bars).toEqual({ start: 13, end: 16 });
    expect(op.pattern.id).toBe('kick_b13-16');
    expect(op.pattern.notes.map((n) => n.step)).toEqual([0, 4, 6]);
    expect(describeScope(p)).toBe('bars 13-16 · layers kick');
  });

  it('a second edit of the same range replaces the pattern it wrote', () => {
    const s0 = fixture();
    const range = { start: 13, end: 16 };
    const d0 = derivePattern(s0, 'kick', range)!;
    const p1 = editPatternInRange(s0, 'kick', range, toggleNote(d0.pattern.notes, 6, 36, D), 'Edit')!;
    const r1 = applyPatch(s0, p1);
    expect(r1.ok).toBe(true);
    if (!r1.ok) return;
    const d1 = derivePattern(r1.score, 'kick', range)!;
    expect(d1.replaceable).toBe(true);
    const p2 = editPatternInRange(r1.score, 'kick', range, updateNote(d1.pattern.notes, 6, 36, { accent: 'strong' }), 'Edit');
    expectValidPatch(p2);
    expect(p2.ops[0].op).toBe('replace_pattern');
    expect(applyPatch(r1.score, p2).ok).toBe(true);
  });

  it('an unchanged note list is a no-op', () => {
    const s = fixture();
    const d = derivePattern(s, 'kick', { start: 9, end: 16 })!;
    expect(editPatternInRange(s, 'kick', { start: 9, end: 16 }, d.pattern.notes, 'x')).toBeNull();
  });

  it('an empty range derives a literal pattern on the layer grid', () => {
    const d = derivePattern(fixture(), 'kick', { start: 1, end: 2 })!;
    expect(d.mode).toBe('literal');
    expect(d.pattern.grid).toBe('eighth');
    expect(d.pattern.length_steps).toBe(16);
    expect(d.pattern.notes).toEqual([]);
  });

  it('a range straddling a clip edge is shown note for note', () => {
    const d = derivePattern(fixture(), 'kick', { start: 8, end: 9 })!;
    expect(d.mode).toBe('literal');
    expect(d.pattern.notes.map((n) => n.step)).toEqual([8, 12]);
  });

  it('pattern ids are deterministic with a numeric suffix on collision', () => {
    const s = fixture();
    expect(freshPatternId(s, 'kick', { start: 13, end: 16 })).toBe('kick_b13-16');
    s.patterns.push({ id: 'kick_b13-16', grid: 'eighth', length_steps: 8, notes: [] });
    expect(freshPatternId(s, 'kick', { start: 13, end: 16 })).toBe('kick_b13-16_2');
  });
});

describe('note list helpers', () => {
  it('toggle adds then removes', () => {
    const a = toggleNote([], 3, 60, D);
    expect(a).toHaveLength(1);
    expect(toggleNote(a, 3, 60, D)).toEqual([]);
  });
  it('update clamps velocity and allows overlap above 100%', () => {
    const n = updateNote(toggleNote([], 0, 60, D), 0, 60, { velocity: 1.7, length: 2.5 });
    expect(n[0].velocity).toBe(1);
    expect(n[0].length).toBe(2.5);
  });
});

describe('restore version patch', () => {
  it('turns the current score back into the saved one', () => {
    const saved = unlocked();
    const r = applyPatch(saved, clearBars(saved, 'kick', { start: 13, end: 16 }));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    const p = restorePatch(r.score, saved, 'v1');
    expectValidPatch(p);
    const back = applyPatch(r.score, p);
    expect(back.ok).toBe(true);
    if (back.ok) expect(back.score).toEqual(saved);
  });
});
