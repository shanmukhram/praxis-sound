import { describe, expect, it } from 'vitest';
import { applyPatch, resolveWritePattern, splitOut } from '../../src/engine/mock/apply';
import { MockEngine } from '../../src/engine/MockEngine';
import { derivePattern, editPatternInRange, muteLayer, toggleNote } from '../../src/score/patchBuilder';
import type { NScore, Note, PatchOp, ScorePatch } from '../../src/score/types';
import { eventKey, layerEvents } from '../../src/score/timeline';
import { validate } from '../../src/engine/mock/validate';
import { decodeWav } from '../../src/engine/wav';
import { fixture, unlocked } from './fixtures';

const scoped = (
  ops: PatchOp[],
  bars?: { start: number; end: number },
  layers: string[] = [],
  origin: ScorePatch['origin'] = 'user',
): ScorePatch => ({
  description: 'test',
  origin,
  scope: bars ? { bars, layers } : { layers },
  ops,
});

const kickPattern = (id: string) => ({
  id,
  grid: 'eighth' as const,
  length_steps: 8,
  notes: [{ step: 0, pitch: 36, velocity: 0.7, accent: 'strong' as const, length: 1 }],
});

function expectError(s: NScore, p: ScorePatch, kind: string) {
  const r = applyPatch(s, p);
  expect(r.ok).toBe(false);
  if (!r.ok) expect(r.error.kind).toBe(kind);
  return r;
}

function expectInverseRestores(s: NScore, p: ScorePatch) {
  const r = applyPatch(s, p);
  expect(r.ok, r.ok ? '' : JSON.stringify(r.error)).toBe(true);
  if (!r.ok) return;
  const back = applyPatch(r.score, r.inverse);
  expect(back.ok, back.ok ? '' : JSON.stringify(back.error)).toBe(true);
  if (back.ok) expect(back.score).toEqual(s);
}

describe('fixtures load clean', () => {
  for (const name of ['acceptance1', 'acceptance1_bell', 'empty']) {
    it(`${name} validates with no errors`, () => {
      expect(validate(fixture(name)).errors).toEqual([]);
    });
  }
});

describe('locks (contract rule 2)', () => {
  const s = fixture();
  it('rejects a bar edit on the locked piano', () => {
    expectError(
      s,
      scoped([{ op: 'clear_bars', layer: 'piano', bars: { start: 13, end: 16 } }], { start: 13, end: 16 }, ['piano']),
      'locked_layer',
    );
  });
  it('lets a user mute and solo a locked layer (a lock protects content, not auditioning)', () => {
    const r = applyPatch(s, muteLayer(s, 'piano', true));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.score.layers[0].muted).toBe(true);
    expect(r.score.layers[0].locked).toBe(true);
    const solo = applyPatch(s, scoped([{ op: 'update_layer', id: 'piano', update: { muted: true, solo: true } }], undefined, ['piano']));
    expect(solo.ok).toBe(true);
    const back = applyPatch(r.score, r.inverse);
    expect(back.ok && back.score).toEqual(s);
  });
  it('still refuses audition changes from a model or system origin', () => {
    expectError(s, { ...muteLayer(s, 'piano', true), origin: 'model' }, 'locked_layer');
    expectError(s, { ...muteLayer(s, 'piano', true), origin: 'system' }, 'locked_layer');
  });
  it('still refuses any other field on a locked layer, even alongside mute', () => {
    expectError(s, scoped([{ op: 'update_layer', id: 'piano', update: { muted: true, gain_db: -3 } }]), 'locked_layer');
    expectError(s, scoped([{ op: 'update_layer', id: 'piano', update: { name: 'Piano' } }]), 'locked_layer');
  });
  it('only a user may unlock', () => {
    expectError(s, scoped([{ op: 'set_lock', id: 'piano', locked: false }], undefined, [], 'model'), 'unlock_not_allowed');
    const r = applyPatch(s, scoped([{ op: 'set_lock', id: 'piano', locked: false }]));
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.score.layers[0].locked).toBe(false);
  });
  it('rejects replace_pattern of a pattern a locked layer plays', () => {
    const pat = { ...s.patterns[0], notes: [] };
    expectError(s, scoped([{ op: 'replace_pattern', pattern: pat }]), 'locked_layer');
  });
});

describe('scope (contract rule 3)', () => {
  const s = fixture();
  it('rejects a bar op reaching outside scope.bars, never clipping it', () => {
    expectError(
      s,
      scoped([{ op: 'clear_bars', layer: 'kick', bars: { start: 12, end: 16 } }], { start: 13, end: 16 }, ['kick']),
      'out_of_scope',
    );
  });
  it('rejects a layer op on a layer not in scope.layers', () => {
    expectError(s, scoped([{ op: 'update_layer', id: 'kick', update: { gain_db: -6 } }], undefined, ['piano']), 'out_of_scope');
  });
  it('rejects a global op when scope.bars is set', () => {
    expectError(s, scoped([{ op: 'set_tempo', bpm: 120 }], { start: 1, end: 16 }), 'out_of_scope');
  });
  it('rejects set_clips that changes notes outside the scope', () => {
    expectError(s, scoped([{ op: 'set_clips', layer: 'kick', clips: [] }], { start: 13, end: 16 }, ['kick']), 'out_of_scope');
  });
  it('accepts set_clips that only changes in-scope bars', () => {
    const clips = [{ bars: { start: 9, end: 12 }, pattern: 'kick_soft', transpose: 0, velocity_scale: 1 }];
    const r = applyPatch(s, scoped([{ op: 'set_clips', layer: 'kick', clips }], { start: 13, end: 16 }, ['kick']));
    expect(r.ok).toBe(true);
  });
  it('rejects unknown layers', () => {
    expectError(s, scoped([{ op: 'clear_bars', layer: 'bass', bars: { start: 1, end: 1 } }]), 'unknown_id');
  });
  it('rejects a sound-changing update_layer when scope.bars is set (renames are fine)', () => {
    expectError(s, scoped([{ op: 'update_layer', id: 'kick', update: { gain_db: -6 } }], { start: 13, end: 16 }, ['kick']), 'out_of_scope');
    expect(applyPatch(s, scoped([{ op: 'update_layer', id: 'kick', update: { name: 'Kick' } }], { start: 13, end: 16 }, ['kick'])).ok).toBe(true);
  });
});

describe('atomic and valid (contract rule 1)', () => {
  it('leaves the score untouched when a later op fails', () => {
    const s = fixture();
    const before = structuredClone(s);
    const p = scoped([
      { op: 'update_layer', id: 'kick', update: { gain_db: -8 } },
      { op: 'clear_bars', layer: 'piano', bars: { start: 1, end: 2 } },
    ]);
    expectError(s, p, 'locked_layer');
    expect(s).toEqual(before);
  });
  it('rejects a result that does not validate (off-scale note on the pitched piano)', () => {
    const s = unlocked();
    const pat = {
      id: 'piano_b13-16',
      grid: 'eighth' as const,
      length_steps: 16,
      notes: [{ step: 0, pitch: 58, velocity: 0.5, accent: 'normal' as const, length: 1 }],
    };
    const r = expectError(
      s,
      scoped([{ op: 'write_bars', layer: 'piano', bars: { start: 13, end: 16 }, pattern: pat }], { start: 13, end: 16 }, ['piano']),
      'invalid',
    );
    if (!r.ok && r.error.kind === 'invalid') expect(r.error.message).toContain('off_scale');
  });
  it('rejects ops the mock does not implement, plainly', () => {
    const r = expectError(fixture(), scoped([{ op: 'set_bars', bars: 8 }]), 'invalid');
    if (!r.ok && r.error.kind === 'invalid') expect(r.error.message).toContain('does not implement');
  });
});

describe('write_bars pattern ids (rule 4): never DuplicateId', () => {
  const range = { start: 13, end: 16 };
  const write = (id: string, notes: Note[] = kickPattern(id).notes) =>
    scoped([{ op: 'write_bars', layer: 'kick', bars: range, pattern: { ...kickPattern(id), notes } }], range, ['kick']);
  const kickAt13 = (x: NScore) => x.layers[1].clips.find((c) => c.bars.start === 13)!.pattern;

  it('uses the given id when it is free', () => {
    const r = applyPatch(fixture(), write('my_fill'));
    expect(r.ok && kickAt13(r.score)).toBe('my_fill');
    if (r.ok) expect(r.inverse.ops.map((o) => o.op)).toEqual(['set_clips', 'remove_pattern']);
  });

  it('reuses an existing identical pattern under the given id, without adding or removing it', () => {
    const s0 = fixture();
    s0.patterns.push(kickPattern('my_fill'));
    const r = applyPatch(s0, write('my_fill'));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(kickAt13(r.score)).toBe('my_fill');
    expect(r.score.patterns.length).toBe(s0.patterns.length);
    expect(r.inverse.ops.map((o) => o.op)).toEqual(['set_clips']);
    expect(applyPatch(r.score, r.inverse)).toMatchObject({ ok: true, score: s0 });
  });

  it('stores a different pattern with a taken id under the derived id, leaving the original untouched', () => {
    const s0 = fixture();
    const r = applyPatch(s0, write('kick_soft', [{ step: 2, pitch: 36, velocity: 0.5, accent: 'soft', length: 1 }]));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(kickAt13(r.score)).toBe('kick_b13-16');
    expect(r.score.patterns.find((p) => p.id === 'kick_soft')).toEqual(s0.patterns.find((p) => p.id === 'kick_soft'));
    expectInverseRestores(s0, write('kick_soft', [{ step: 2, pitch: 36, velocity: 0.5, accent: 'soft', length: 1 }]));
  });

  it('suffixes the derived id on collision and reuses a derived id holding identical content', () => {
    const s0 = fixture();
    s0.patterns.push({ ...kickPattern('kick_b13-16'), notes: [] });
    const different = [{ step: 3, pitch: 36, velocity: 0.5, accent: 'normal' as const, length: 1 }];
    expect(resolveWritePattern(s0, 'kick', range, { ...kickPattern('kick_soft'), notes: different })).toEqual({ id: 'kick_b13-16_2', add: true });
    expect(resolveWritePattern(s0, 'kick', range, { ...kickPattern('kick_soft'), notes: [] })).toEqual({ id: 'kick_b13-16', add: false });
  });

  it('derives an id when the given id is invalid, and keeps derived ids within 64 chars', () => {
    expect(resolveWritePattern(fixture(), 'kick', range, kickPattern('Bad Id!'))).toEqual({ id: 'kick_b13-16', add: true });
    const long = 'k'.repeat(64);
    const { id } = resolveWritePattern(fixture(), long, range, kickPattern('kick_soft_x'.repeat(10)));
    expect(id.length).toBe(64);
    expect(id.endsWith('_b13-16')).toBe(true);
  });
});

describe('copy-on-write and isolation (rules 4 and 6)', () => {
  it('write_bars splits the clip and keeps the right-hand phase', () => {
    const s = unlocked();
    const pat = {
      id: 'piano_b5',
      grid: 'eighth' as const,
      length_steps: 8,
      notes: [{ step: 0, pitch: 57, velocity: 0.5, accent: 'normal' as const, length: 1 }],
    };
    const r = applyPatch(
      s,
      scoped([{ op: 'write_bars', layer: 'piano', bars: { start: 5, end: 5 }, pattern: pat }], { start: 5, end: 5 }, ['piano']),
    );
    expect(r.ok, r.ok ? '' : JSON.stringify(r.error)).toBe(true);
    if (!r.ok) return;
    const clips = r.score.layers[0].clips;
    expect(clips.map((c) => [c.bars.start, c.bars.end, c.pattern, c.pattern_offset_steps ?? 0])).toEqual([
      [1, 4, 'piano_motif', 0],
      [5, 5, 'piano_b5', 0],
      [6, 16, 'piano_motif', 8], // bar 6 is the second half of the 2-bar motif
    ]);
    const outside = (x: NScore) =>
      layerEvents(x, 'piano')
        .filter((e) => e.bar !== 5)
        .map(eventKey);
    expect(outside(r.score)).toEqual(outside(s));
  });

  it('a kick edit in bars 13-16 leaves every other layer and bar identical', () => {
    const s = fixture();
    const range = { start: 13, end: 16 };
    const d = derivePattern(s, 'kick', range)!;
    const notes = toggleNote(d.pattern.notes, 6, 36, { velocity: 0.6, length: 1, accent: 'soft' });
    const p = editPatternInRange(s, 'kick', range, notes, 'Edit')!;
    const r = applyPatch(s, p);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.score.layers[0]).toEqual(s.layers[0]);
    const evs = (x: NScore, from: number, to: number) => layerEvents(x, 'kick', from, to).map(eventKey);
    expect(evs(r.score, 1, 12)).toEqual(evs(s, 1, 12));
    expect(evs(r.score, 13, 16).length).toBe(evs(s, 13, 16).length + 4);
  });

  it('splitOut with a range covering the whole clip removes it', () => {
    const s = fixture();
    expect(splitOut(s, s.layers[1].clips, { start: 1, end: 16 })).toEqual([]);
  });
});

describe('exact inverse (rule 5)', () => {
  const s = unlocked();
  const cases: [string, ScorePatch][] = [
    [
      'write_bars',
      scoped([{ op: 'write_bars', layer: 'kick', bars: { start: 13, end: 16 }, pattern: kickPattern('kick_b13-16') }], { start: 13, end: 16 }, ['kick']),
    ],
    ['clear_bars', scoped([{ op: 'clear_bars', layer: 'piano', bars: { start: 3, end: 6 } }], { start: 3, end: 6 }, ['piano'])],
    [
      'place_clip',
      scoped([{ op: 'place_clip', layer: 'kick', clip: { bars: { start: 2, end: 3 }, pattern: 'kick_soft' } }], { start: 2, end: 3 }, ['kick']),
    ],
    ['set_clips', scoped([{ op: 'set_clips', layer: 'kick', clips: [] }])],
    ['update_layer', scoped([{ op: 'update_layer', id: 'kick', update: { muted: true, gain_db: -9, name: 'Kick' } }])],
    ['set_lock', scoped([{ op: 'set_lock', id: 'kick', locked: true }])],
    ['set_tempo', scoped([{ op: 'set_tempo', bpm: 100 }])],
    ['set_seed', scoped([{ op: 'set_seed', seed: 99 }])],
    ['add_layer', scoped([{ op: 'add_layer', layer: { id: 'hat', class: 'percussion', instrument: { kind: 'drum', preset: 'hat' } } }])],
    ['remove_layer (last)', scoped([{ op: 'remove_layer', id: 'kick' }])],
    ['add_pattern', scoped([{ op: 'add_pattern', pattern: kickPattern('extra') }])],
    [
      'multi-op',
      scoped([
        { op: 'set_tempo', bpm: 90 },
        { op: 'update_layer', id: 'piano', update: { solo: true } },
        { op: 'clear_bars', layer: 'kick', bars: { start: 9, end: 10 } },
      ]),
    ],
  ];
  for (const [name, p] of cases) {
    it(`${name}: apply then inverse gives back the original`, () => {
      expectInverseRestores(s, p);
    });
  }

  it('removing a middle layer or pattern re-inserts it at its old index (rule 7)', () => {
    const three = unlocked();
    three.layers.push({ ...structuredClone(three.layers[1]), id: 'kick2', name: 'Kick 2' });
    three.patterns.splice(1, 0, kickPattern('spare'));
    const r = applyPatch(three, scoped([{ op: 'remove_layer', id: 'kick' }, { op: 'remove_pattern', id: 'spare' }]));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.inverse.ops).toEqual([
      { op: 'add_pattern', pattern: kickPattern('spare'), index: 1 },
      { op: 'add_layer', layer: three.layers[1], index: 1 },
    ]);
    const back = applyPatch(r.score, r.inverse);
    expect(back.ok && back.score).toEqual(three);
  });

  it('add_layer and add_pattern insert at index; past the end is invalid', () => {
    const s0 = unlocked();
    const hat = { id: 'hat', class: 'percussion' as const, instrument: { kind: 'drum' as const, preset: 'hat' } };
    const r = applyPatch(s0, scoped([{ op: 'add_layer', layer: hat, index: 0 }, { op: 'add_pattern', pattern: kickPattern('first'), index: 0 }]));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.score.layers.map((l) => l.id)).toEqual(['hat', 'piano', 'kick']);
    expect(r.score.patterns[0].id).toBe('first');
    expect(applyPatch(r.score, r.inverse).ok && applyPatch(r.score, r.inverse)).toMatchObject({ score: s0 });
    expectError(s0, scoped([{ op: 'add_layer', layer: hat, index: 3 }]), 'invalid');
  });

  it('a layer added locked is unlocked by the inverse before it is removed', () => {
    const s0 = unlocked();
    const hat = { id: 'hat', class: 'percussion' as const, instrument: { kind: 'drum' as const, preset: 'hat' }, locked: true };
    expectInverseRestores(s0, scoped([{ op: 'add_layer', layer: hat }]));
  });
});

describe('MockEngine adapter', () => {
  it('loads, applies, renders a mock preview and records exports', async () => {
    const e = new MockEngine();
    const loaded = await e.load(fixture());
    expect(loaded.errors).toEqual([]);
    const r = await e.applyPatch(muteLayer(loaded.score, 'kick', true));
    expect(r.ok).toBe(true);
    expect(e.current()!.layers[1].muted).toBe(true);
    const out = await e.render({ sampleRate: 8000 });
    expect(out.report.mock).toBe(true);
    expect(out.report.is_loop).toBe(true);
    expect(out.report.length_samples).toBe(8000 * 30);
    expect(out.report.notes_rendered).toBe(144); // piano only: 8 repeats of 18 notes
    expect(out.report.loudness_lufs).toBeCloseTo(-16, 6); // normalised to the score's target
    expect(out.report.true_peak_dbtp).toBeLessThanOrEqual(-1);
    const wav = decodeWav(await (await fetch(out.audioUrl)).arrayBuffer());
    expect(wav.channels[0].length).toBe(8000 * 30);
    const res = await e.export({ kinds: ['wav24', 'stems', 'midi'], destination: 'out', baseName: 'loop' });
    expect(res.files.map((f) => f.path)).toEqual(['out/loop.24bit.wav', 'out/loop.stem-piano.wav', 'out/loop.stem-kick.wav', 'out/loop.mid']);
    expect(e.exports).toHaveLength(1);
    e.dispose();
  });

  it('renders deterministically', async () => {
    const a = new MockEngine();
    const b = new MockEngine();
    await a.load(fixture());
    await b.load(fixture());
    const bytes = async (e: MockEngine) => new Uint8Array(await (await fetch((await e.render({ sampleRate: 8000 })).audioUrl)).arrayBuffer());
    expect(await bytes(a)).toEqual(await bytes(b));
  });

  it('runs the transport clock without WebAudio and reports bars', async () => {
    const e = new MockEngine();
    await e.load(fixture());
    const seen: number[] = [];
    e.onPosition((p) => seen.push(p.bar));
    await e.seek(1.875 * 4); // bar 5 at 128 BPM
    expect(seen.at(-1)).toBeCloseTo(5, 9);
    await e.setLoop({ start: 13, end: 16 });
    e.dispose();
  });
});
