// Building acceptance test 1 by hand on the mock engine (WP-U5): a blank sound, layers and notes
// as live edits (no take per click), one take for the batch, the next batch as the next take,
// and undo across them, as crates/praxis-app/src/hand.rs does on the desktop.

import { describe, expect, it } from 'vitest';
import { MockEngine } from '../../src/engine/MockEngine';
import { DRAFT_ASK } from '../../src/engine/blank';
import type { BlankSound, TakeState } from '../../src/engine/EngineAdapter';
import { derivePattern, editPatternInRange } from '../../src/score/patchBuilder';
import type { NScore, ScorePatch } from '../../src/score/types';
import { addLayerPatch, layerBarsPatch, parseSpoken, tempoPatch, writeSpoken } from '../../src/ui/handEdits';

const A1: BlankSound = { title: 'Calm piano by hand', key: { tonic: 'A', mode: 'minor' }, bpm: 128, timeSignature: { numerator: 4, denominator: 4 }, bars: 16, loop: true };

async function live(engine: MockEngine, patch: ScorePatch | null): Promise<NScore> {
  expect(patch).not.toBeNull();
  const r = await engine.applyPatch(patch!);
  if (!r.ok) throw new Error(JSON.stringify(r.error));
  return r.score;
}

/** Says each bar of a layer's pattern over its bars, as the Rhythm view's "Say a bar" does. */
function say(score: NScore, layer: string, bars: { start: number; end: number }, words: string[], pitch: number, unpitched: boolean): ScorePatch | null {
  const d = derivePattern(score, layer, bars)!;
  let notes = d.pattern.notes;
  words.forEach((w, i) => {
    const r = parseSpoken(w);
    if (!r.ok) throw new Error(r.message);
    notes = writeSpoken(notes, i * d.stepsPerBar, d.pattern.length_steps, r.steps, pitch, unpitched);
  });
  return editPatternInRange(score, layer, bars, notes, `Wrote ${words.join(' | ')}`);
}

describe('building a sound by hand (mock engine)', () => {
  it('starts blank, takes live edits without takes, and records them as take 1 and take 2', async () => {
    const engine = new MockEngine();
    const states: TakeState[] = [];
    engine.onState((s) => states.push(s));

    expect((await engine.startBlank({ ...A1, bpm: 1000 })).ok).toBe(false);
    expect(await engine.startBlank(A1)).toEqual({ ok: true });
    let st = engine.state();
    expect(st.draft).toBe(true);
    expect(st.takes).toEqual([]);
    expect(st.score!.tempo.bpm).toBe(128);
    expect(st.score!.layers).toEqual([]);

    // No take yet: a request in words is refused plainly, and an empty sound makes no take.
    const asked = await engine.ask({ text: 'make it darker' });
    expect(asked).toEqual({ ok: false, message: DRAFT_ASK });
    const empty = await engine.makeTake('nothing');
    expect(empty.ok).toBe(false);
    if (!empty.ok) expect(empty.message).toMatch(/add a layer first/i);

    // Layers and notes are live edits: no take per click.
    const presets = await engine.presets();
    const piano = presets.find((p) => p.name === 'felt_piano')!;
    const kick = presets.find((p) => p.name === 'soft_kick')!;
    let s = await live(engine, addLayerPatch(engine.state().score!, piano));
    s = await live(engine, say(s, 'felt_piano', { start: 1, end: 16 }, ['DON din don din TIN din don din', 'DON din don din TIN din don din'], 57, false));
    s = await live(engine, addLayerPatch(s, kick));
    s = await live(engine, say(s, 'soft_kick', { start: 1, end: 16 }, ['DON - - - don - - -'], 36, true));
    s = await live(engine, layerBarsPatch(s, 'soft_kick', 9, 16));
    expect(engine.state().takes).toEqual([]);
    expect(engine.state().canUndo).toBe(true);
    expect(s.layers.map((l) => l.id)).toEqual(['felt_piano', 'soft_kick']);
    expect(s.layers[1].clips.map((c) => [c.bars.start, c.bars.end])).toEqual([[9, 16]]);

    // One take for the batch: take 1 of a new sound (no patch).
    const t1 = await engine.makeTake('By hand: felt piano motif, soft kick in bars 9-16');
    expect(t1.ok).toBe(true);
    if (!t1.ok) return;
    expect(t1.take.n).toBe(1);
    expect(t1.take.patch).toBeNull();
    expect(t1.take.title).toBe('Built by hand');
    expect(t1.take.say).toBe('Built by hand: felt piano, soft kick, 16 bars at 128 BPM in A minor.');
    expect(t1.take.checks.checks.length).toBeGreaterThan(0);
    st = engine.state();
    expect(st.draft).toBe(false);
    expect(st.current).toBe(1);

    // Nothing changed since take 1: refused.
    const again = await engine.makeTake('again');
    expect(again.ok).toBe(false);
    if (!again.ok) expect(again.message).toBe('Nothing changed since take 1: edit the sound, then make a take.');

    // Requests in words work after the first take.
    expect((await engine.ask({ text: 'Make the kick quieter' })).ok).toBe(true);
    expect(engine.state().current).toBe(2);

    // The next batch: one live edit, then take 3, whose patch is the diff from take 2.
    s = await live(engine, tempoPatch(engine.state().score!, 120));
    const t3 = await engine.makeTake('By hand: tempo 120 BPM');
    expect(t3.ok).toBe(true);
    if (!t3.ok) return;
    expect(t3.take.n).toBe(3);
    expect(t3.take.patch!.ops).toContainEqual({ op: 'set_tempo', bpm: 120 });
    expect(t3.take.say).toBe('Changed by hand: tempo 120 BPM.');
    expect(Object.keys(t3.take.changed).sort()).toEqual(['felt_piano', 'soft_kick']);

    // Undo the edit: the live score is take 2's again, so take 2 is current; redo returns to 3.
    expect(await engine.undo()).toBe(true);
    expect(engine.state().current).toBe(2);
    expect(engine.state().score!.tempo.bpm).toBe(128);
    expect(await engine.redo()).toBe(true);
    expect(engine.state().current).toBe(3);
    expect(states.some((x) => x.draft)).toBe(true);
  });

  it('applies the global ops a hand edit uses with exact inverses (key, bars, time, loop mode)', async () => {
    const engine = new MockEngine();
    await engine.startBlank(A1);
    for (const p of [
      { description: 'key', origin: 'user', scope: { layers: [] }, ops: [{ op: 'set_key', key: { tonic: 'D', mode: 'dorian' }, transpose_notes: true }] },
      { description: 'bars', origin: 'user', scope: { layers: [] }, ops: [{ op: 'set_bars', bars: 8 }] },
      { description: 'time', origin: 'user', scope: { layers: [] }, ops: [{ op: 'set_time_signature', time_signature: { numerator: 3, denominator: 4 } }] },
      { description: 'loop', origin: 'user', scope: { layers: [] }, ops: [{ op: 'set_playback', playback: { mode: 'one_shot', tail_seconds: 4 } }] },
    ] as ScorePatch[]) {
      const before = engine.state().score!;
      const r = await engine.applyPatch(p);
      expect(r.ok).toBe(true);
      expect(await engine.undo()).toBe(true);
      expect(engine.state().score).toEqual(before);
      expect(await engine.redo()).toBe(true);
    }
    const s = engine.state().score!;
    expect(s.key).toEqual({ tonic: 'D', mode: 'dorian' });
    expect(s.bars).toBe(8);
    expect(s.time_signature).toEqual({ numerator: 3, denominator: 4 });
    expect(s.playback).toEqual({ mode: 'one_shot', tail_seconds: 4 });
  });

  it('plays a blank sound before its first take and seeks and loops it', async () => {
    const engine = new MockEngine();
    await engine.startBlank(A1);
    const pos: number[] = [];
    engine.onPosition((p) => pos.push(p.seconds));
    await engine.seek(15);
    await engine.setLoop({ start: 13, end: 16 });
    await engine.play();
    await engine.stop();
    expect(pos.some((x) => Math.abs(x - 15) < 0.1)).toBe(true);
    engine.dispose();
  });
});
