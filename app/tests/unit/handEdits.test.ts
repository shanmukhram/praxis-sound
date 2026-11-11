// The hand-building patch builders (WP-U5): each control's patch, applied by the mock engine's
// implementation of the patch rules (contracts/patch.md), validates and has an exact inverse.

import { describe, expect, it } from 'vitest';
import { applyPatch } from '../../src/engine/mock/apply';
import { blankScore, solveBars, solveTempo } from '../../src/engine/blank';
import { STANDARD_PRESETS, groupPresets, presetLabel, presetsFromPayload } from '../../src/engine/presets';
import type { PresetInfo, Take } from '../../src/engine/EngineAdapter';
import { derivePattern, editPatternInRange } from '../../src/score/patchBuilder';
import { layerEvents } from '../../src/score/timeline';
import type { NScore, ScorePatch } from '../../src/score/types';
import { deepEqual } from '../../src/score/types';
import {
  addLayerPatch,
  addToBatch,
  barsPatch,
  batchAsk,
  batchKey,
  batchScope,
  gainPatch,
  hasUnsaved,
  instrumentPatch,
  keyPatch,
  layerBarsPatch,
  panPatch,
  panText,
  parseSpoken,
  playbackPatch,
  removeLayerPatch,
  renameLayerPatch,
  snapToKey,
  tempoPatch,
  timeSignaturePatch,
  timeSignatureProblem,
  tonicNear,
  writeSpoken,
} from '../../src/ui/handEdits';
import { unlocked } from './fixtures';

const preset = (name: string): PresetInfo => STANDARD_PRESETS.find((p) => p.name === name)!;

function blank(): NScore {
  return blankScore({ title: 'Hand loop', key: { tonic: 'A', mode: 'minor' }, seconds: 30, timeSignature: { numerator: 4, denominator: 4 }, bars: 16, loop: true });
}

/** Applies a patch, checks its inverse restores the score exactly, and returns the result. */
function apply(s: NScore, p: ScorePatch | null): NScore {
  expect(p).not.toBeNull();
  const r = applyPatch(s, p!);
  if (!r.ok) throw new Error(JSON.stringify(r.error));
  const back = applyPatch(r.score, r.inverse);
  if (!back.ok) throw new Error(`inverse refused: ${JSON.stringify(back.error)}`);
  expect(deepEqual(back.score, s)).toBe(true);
  return r.score;
}

const MOTIF = 'DON din don din TIN din don din';

describe('a blank sound', () => {
  it('fits the bars into a length with the duration solver, as the Rust solver does', () => {
    expect(solveTempo(16, { numerator: 4, denominator: 4 }, 30)).toBe(128);
    expect(solveBars(128, { numerator: 4, denominator: 4 }, 31)).toEqual({ bars: 17, seconds: 31.875 });
    const s = blank();
    expect(s.tempo.bpm).toBe(128);
    expect(s.layers).toEqual([]);
    expect(s.playback).toEqual({ mode: 'loop' });
    expect(s.title).toBe('Hand loop');
    expect(() => blankScore({ key: { tonic: 'C', mode: 'major' }, bpm: 900, timeSignature: { numerator: 4, denominator: 4 }, bars: 4, loop: false })).toThrow(/between 20 and 400/);
  });
});

describe('layers by hand', () => {
  it('adds a felt piano (a two-bar pattern) and a soft kick (one bar), each over every bar', () => {
    let s = apply(blank(), addLayerPatch(blank(), preset('felt_piano')));
    const piano = s.layers[0];
    expect(piano).toMatchObject({ id: 'felt_piano', name: 'Felt piano', class: 'motif', instrument: { kind: 'sampler', preset: 'felt_piano' } });
    expect(piano.clips).toEqual([{ bars: { start: 1, end: 16 }, pattern: 'felt_piano_p1', transpose: 0, velocity_scale: 1 }]);
    expect(s.patterns[0]).toMatchObject({ id: 'felt_piano_p1', grid: 'eighth', length_steps: 16, notes: [] });
    s = apply(s, addLayerPatch(s, preset('soft_kick')));
    expect(s.layers[1]).toMatchObject({ id: 'soft_kick', class: 'drums', instrument: { kind: 'drum', preset: 'soft_kick' } });
    expect(s.patterns[1].length_steps).toBe(8);
    // A second piano gets its own id and name.
    s = apply(s, addLayerPatch(s, preset('felt_piano')));
    expect(s.layers[2].id).toBe('felt_piano_2');
    expect(s.layers[2].name).toBe('Felt piano 2');
  });

  it('removes a layer with the patterns only it played; renames; changes the instrument', () => {
    let s = apply(blank(), addLayerPatch(blank(), preset('felt_piano')));
    s = apply(s, renameLayerPatch(s, 'felt_piano', 'Motif'));
    expect(s.layers[0].name).toBe('Motif');
    expect(renameLayerPatch(s, 'felt_piano', '  ')).toBeNull();
    s = apply(s, instrumentPatch(s, 'felt_piano', preset('bell')));
    expect(s.layers[0].instrument.preset).toBe('bell');
    expect(s.layers[0].name).toBe('Motif'); // a name the person chose stays
    expect(s.layers[0].class).toBe('motif');
    // Pitched -> drum moves the class; the default name follows the instrument.
    let k = apply(blank(), addLayerPatch(blank(), preset('felt_piano')));
    k = apply(k, instrumentPatch(k, 'felt_piano', preset('soft_kick')));
    expect(k.layers[0]).toMatchObject({ class: 'drums', name: 'Soft kick' });
    const gone = apply(s, removeLayerPatch(s, 'felt_piano'));
    expect(gone.layers).toEqual([]);
    expect(gone.patterns).toEqual([]);
  });

  it('sets level and pan in plain words', () => {
    let s = apply(blank(), addLayerPatch(blank(), preset('soft_kick')));
    s = apply(s, gainPatch(s, 'soft_kick', -4.2));
    expect(s.layers[0].gain_db).toBe(-4);
    s = apply(s, panPatch(s, 'soft_kick', -0.3));
    expect(s.layers[0].pan).toBe(-0.3);
    expect(panText(-0.3)).toBe('30% left');
    expect(panText(0)).toBe('Centre');
    expect(gainPatch(s, 'soft_kick', -4)).toBeNull();
  });

  it('sets which bars a layer plays, keeping a motif in phase when bars come back', () => {
    let s = apply(blank(), addLayerPatch(blank(), preset('felt_piano')));
    const d = derivePattern(s, 'felt_piano', { start: 1, end: 16 })!;
    const steps = parseSpoken(`${MOTIF} DON din don din TIN din don din`);
    if (!steps.ok) throw new Error(steps.message);
    const notes = writeSpoken(d.pattern.notes, 0, 16, steps.steps, 57, false);
    s = apply(s, editPatternInRange(s, 'felt_piano', { start: 1, end: 16 }, notes, 'motif'));
    const full = layerEvents(s, 'felt_piano');

    const later = apply(s, layerBarsPatch(s, 'felt_piano', 2, 16));
    expect(later.layers[0].clips.map((c) => [c.bars.start, c.bars.end])).toEqual([[2, 16]]);
    const back = apply(later, layerBarsPatch(later, 'felt_piano', 1, 16));
    // Bar 1 comes back exactly as it was: same notes, same phase.
    expect(layerEvents(back, 'felt_piano').map((e) => [e.bar, e.stepInBar, e.pitch])).toEqual(full.map((e) => [e.bar, e.stepInBar, e.pitch]));

    // The soft kick in bars 9-16 only.
    let k = apply(s, addLayerPatch(s, preset('soft_kick')));
    k = apply(k, layerBarsPatch(k, 'soft_kick', 9, 16));
    expect(k.layers[1].clips.map((c) => [c.bars.start, c.bars.end])).toEqual([[9, 16]]);
    expect(layerBarsPatch(k, 'soft_kick', 9, 16)).toBeNull();
  });
});

describe('sound settings', () => {
  it('changes key (notes move with it), tempo, loop mode and time signature', () => {
    const s0 = unlocked();
    const d = apply(s0, keyPatch(s0, { tonic: 'D', mode: 'minor' }));
    expect(d.key).toEqual({ tonic: 'D', mode: 'minor' });
    expect(d.patterns.find((p) => p.id === 'piano_motif')!.notes[0].pitch).toBe(62); // A3 -> D4
    expect(d.patterns.find((p) => p.id === 'kick_soft')!.notes[0].pitch).toBe(s0.patterns.find((p) => p.id === 'kick_soft')!.notes[0].pitch); // drums never move
    expect(apply(s0, tempoPatch(s0, 120)).tempo.bpm).toBe(120);
    expect(apply(s0, playbackPatch(s0, false)).playback).toEqual({ mode: 'one_shot', tail_seconds: 4 });
    expect(playbackPatch(s0, true)).toBeNull();
    const b = blank();
    expect(apply(b, timeSignaturePatch(b, { numerator: 3, denominator: 4 })).time_signature).toEqual({ numerator: 3, denominator: 4 });
    expect(timeSignatureProblem(s0, { numerator: 3, denominator: 4 })).toMatch(/do not fit whole bars of 3\/4/);
    expect(timeSignatureProblem(s0, { numerator: 2, denominator: 4 })).toBeNull();
  });

  it('shortens the sound and trims what played past the end; lengthens it', () => {
    const s0 = unlocked();
    const short = apply(s0, barsPatch(s0, 12));
    expect(short.bars).toBe(12);
    expect(short.layers.find((l) => l.id === 'kick')!.clips.map((c) => [c.bars.start, c.bars.end])).toEqual([[9, 12]]);
    expect(short.sections.map((x) => [x.bars.start, x.bars.end])).toEqual([
      [1, 8],
      [9, 12],
    ]);
    const shorter = apply(s0, barsPatch(s0, 6));
    expect(shorter.sections.map((x) => x.id)).toEqual(['a']);
    expect(shorter.layers.find((l) => l.id === 'kick')!.clips).toEqual([]);
    const longer = apply(s0, barsPatch(s0, 24));
    expect(longer.bars).toBe(24);
    expect(barsPatch(s0, 16)).toBeNull();
  });
});

describe('notes and spoken rhythm', () => {
  it('reads spoken rhythm like rhythm.rs: syllables, rests, ties and errors', () => {
    const r = parseSpoken('DON din~ don- TIN|tin');
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.steps.map((s) => (s.kind === 'hit' ? s.accent : s.kind))).toEqual(['strong', 'soft', 'tie', 'normal', 'rest', 'bright', 'bright']);
    expect(parseSpoken('~ DON').ok).toBe(false);
    const bad = parseSpoken('DON bom');
    expect(bad.ok).toBe(false);
    if (!bad.ok) expect(bad.message).toContain('"bom" is not a rhythm word');
  });

  it('writes a spoken bar into a pattern, keeping pitches already there', () => {
    const r = parseSpoken('DON din~ - TIN');
    if (!r.ok) throw new Error(r.message);
    const before = [{ step: 1, pitch: 64, velocity: 0.5, accent: 'normal' as const, length: 1 }, { step: 6, pitch: 69, velocity: 0.5, accent: 'normal' as const, length: 1 }];
    const out = writeSpoken(before, 0, 8, r.steps, 57, false);
    expect(out).toEqual([
      { step: 0, pitch: 57, velocity: 0.8, accent: 'strong', length: 1.6 },
      { step: 1, pitch: 64, velocity: 0.55, accent: 'soft', length: 2.6 },
      { step: 4, pitch: 57, velocity: 0.7, accent: 'bright', length: 1.6 },
      { step: 6, pitch: 69, velocity: 0.5, accent: 'normal', length: 1 },
    ]);
    // Drums: one step each; nothing past the pattern.
    const drums = parseSpoken('DON don DON');
    if (!drums.ok) throw new Error(drums.message);
    expect(writeSpoken([], 6, 8, drums.steps, 36, true).map((n) => [n.step, n.length])).toEqual([
      [6, 1],
      [7, 1],
    ]);
  });

  it('snaps pitches to the key and starts a part on the tonic near middle C', () => {
    expect(snapToKey({ tonic: 'A', mode: 'minor' }, 61)).toBe(62); // C# -> D (ties go up)
    expect(snapToKey({ tonic: 'A', mode: 'minor' }, 60)).toBe(60);
    expect(snapToKey({ tonic: 'C', mode: 'chromatic' }, 61)).toBe(61);
    expect(tonicNear({ tonic: 'A', mode: 'minor' })).toBe(57);
    expect(tonicNear({ tonic: 'E', mode: 'minor' })).toBe(64);
    expect(tonicNear({ tonic: 'C', mode: 'major' })).toBe(60);
  });
});

describe('the batch', () => {
  it('knows when the live score holds edits no take has, ignoring mute, solo and lock', () => {
    const s = unlocked();
    const take = { score: s } as Take;
    expect(hasUnsaved(take, s, false)).toBe(false);
    const muted = structuredClone(s);
    muted.layers[0].muted = true;
    muted.layers[1].locked = true;
    expect(hasUnsaved(take, muted, false)).toBe(false);
    expect(hasUnsaved(take, apply(s, tempoPatch(s, 100)), false)).toBe(true);
    expect(hasUnsaved(null, blank(), true)).toBe(false); // nothing to render yet
    expect(hasUnsaved(null, apply(blank(), addLayerPatch(blank(), preset('soft_kick'))), true)).toBe(true);
  });

  it('reads as one request, with a scope only when every edit was on the same bars of one layer', () => {
    expect(batchAsk([{ label: 'Added Felt piano' }, { label: 'Wrote "DON din" in bar 1' }])).toBe('By hand: added Felt piano; wrote "DON din" in bar 1');
    expect(batchAsk(Array.from({ length: 6 }, (_, i) => ({ label: `Edit ${i}` })))).toBe('By hand: edit 0; edit 1; edit 2; edit 3; and 2 more');
    const at = (a: number, b: number) => ({ label: 'x', layer: 'piano', bars: { start: a, end: b } });
    expect(batchScope([at(9, 12), at(13, 16)])).toEqual({ layer: 'piano', bars: { start: 9, end: 16 } });
    expect(batchScope([at(9, 12), { label: 'tempo' }])).toBeUndefined();
  });

  it('keeps one wording per setting: a later tempo replaces the earlier', () => {
    const s = unlocked();
    const tempo = (b: number) => ({ label: `Tempo ${b} BPM`, key: batchKey(tempoPatch(s, b)!) });
    let batch = addToBatch([], tempo(120));
    batch = addToBatch(batch, { label: 'Added Bell', key: batchKey(addLayerPatch(s, preset('bell'))) });
    batch = addToBatch(batch, tempo(124));
    expect(batch.map((b) => b.label)).toEqual(['Added Bell', 'Tempo 124 BPM']);
    expect(batchKey(gainPatch(s, 'kick', -6)!)).toBe('kick:gain_db');
    expect(batchKey(instrumentPatch(s, 'kick', preset('kick'))!)).toBe('kick:instrument');
    expect(batchKey(barsPatch(s, 12)!)).toBe('set_bars');
  });
});

describe('presets in plain words', () => {
  it('groups the catalogue and reads the desktop payload with availability', () => {
    expect(presetLabel('fm_keys')).toBe('Electric piano');
    expect(presetLabel('new_thing')).toBe('New thing');
    const groups = groupPresets(STANDARD_PRESETS);
    expect(groups.map((g) => g.name)).toEqual(['Keys, bells and plucks', 'Bass', 'Drums', 'Pads and textures', 'Sound effects']);
    expect(groups[0].presets[0].name).toBe('felt_piano');
    expect(groups[2].presets[0].name).toBe('soft_kick');
    const list = presetsFromPayload({
      presets: [{ name: 'felt_piano', kind: 'sampler', class: 'motif', pitched: true, description: 'felt', available: false, reason: 'pack upright-piano-kw is not installed' }],
      engine_only: [{ name: 'tape_hiss', kind: 'synth', class: 'texture' }],
    });
    expect(list).toEqual([
      { name: 'felt_piano', kind: 'sampler', class: 'motif', pitched: true, description: 'felt', available: false, reason: 'pack upright-piano-kw is not installed' },
      { name: 'tape_hiss', kind: 'synth', class: 'texture', pitched: true, description: '', available: true },
    ]);
  });
});
