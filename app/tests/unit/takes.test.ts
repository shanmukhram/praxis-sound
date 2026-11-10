import { describe, expect, it } from 'vitest';
import { MockEngine, identicalOutside } from '../../src/engine/MockEngine';
import type { Take, VerifyReport } from '../../src/engine/EngineAdapter';
import { changedFromPatch, changedText, untouchedText } from '../../src/engine/takes';
import { changeMapModel } from '../../src/ui/changeMapModel';
import { summarizeChecks } from '../../src/ui/checks';
import { checkSchema } from './support/jsonSchema';
import { fixture, patchSchema, rawFixtures } from './fixtures';
import type { Score } from '../../src/score/types';

async function demo(): Promise<MockEngine> {
  const e = new MockEngine();
  await e.seedDemo(rawFixtures.acceptance1 as Score);
  return e;
}

describe('the demo conversation', () => {
  it('has three takes: piano, + kick, an agent lift in bars 13-16', async () => {
    const e = await demo();
    const takes = await e.takes();
    expect(takes.map((t) => t.n)).toEqual([1, 2, 3]);
    expect(takes.map((t) => t.who)).toEqual(['You', 'You', 'Agent · cut-assistant']);
    expect(takes[0].patch).toBeNull();
    expect(takes[0].changed).toEqual({});
    expect(takes[1].changed).toEqual({ kick: [9, 16] });
    expect(takes[2].changed).toEqual({ piano: [13, 16] });
    expect(takes[2].opens).toBe('notes');
    expect(e.state().current).toBe(3);
    for (const t of takes) {
      expect(t.render.report.mock).toBe(true);
      expect(t.checks.checks.length).toBeGreaterThan(0);
      expect(checkSchema(patchSchema, t.patch ?? { ops: [] })).toEqual([]);
    }
  });

  it('every demo take passes its checks, and untouched bars are proven sample-identical', async () => {
    const takes = await (await demo()).takes();
    for (const t of takes) expect(t.checks.checks.filter((c) => !c.passed)).toEqual([]);
    // Adding a layer re-normalises the master, so take 2 can only claim note-for-note identity.
    expect(takes[1].untouched).toBe('Bars 1–8 identical note for note to take 1. Felt piano unchanged.');
    expect(takes[2].untouched).toBe('Bars 1–12 sample-identical to take 2. Soft kick unchanged.');
    expect(changedText(takes[2])).toBe('Changed felt piano in bars 13–16.');
  });
});

describe('change map', () => {
  it("lights exactly the cells in the take's changed ranges", async () => {
    const e = await demo();
    const take = (await e.takes())[2];
    const map = changeMapModel(take, e.state().score!, null);
    const lit = map.lanes.flatMap((l) => l.cells.filter((c) => c.chg).map((c) => `${l.layer}:${c.bar}`));
    expect(lit).toEqual(['piano:13', 'piano:14', 'piano:15', 'piano:16']);
    const kick = map.lanes.find((l) => l.layer === 'kick')!;
    expect(kick.cells.filter((c) => c.on).map((c) => c.bar)).toEqual([9, 10, 11, 12, 13, 14, 15, 16]);
    expect(map.sections.map((s) => s.name)).toEqual(['Piano alone', 'Piano and kick']);
    expect(map.legend).toBe('Changed felt piano in bars 13–16. Bars 1–12 sample-identical to take 2. Soft kick unchanged.');
  });

  it('changes when `changed` changes, and a new sound lights nothing', async () => {
    const e = await demo();
    const [t1, , t3] = await e.takes();
    const fake: Take = { ...t3, changed: { kick: [9, 10] } };
    const lit = changeMapModel(fake, e.state().score!, null).lanes.flatMap((l) => l.cells.filter((c) => c.chg).map((c) => `${l.layer}:${c.bar}`));
    expect(lit).toEqual(['kick:9', 'kick:10']);
    expect(changeMapModel(t1, e.state().score!, null).lanes.some((l) => l.cells.some((c) => c.chg))).toBe(false);
  });

  it('marks the dragged selection', async () => {
    const e = await demo();
    const map = changeMapModel((await e.takes())[2], e.state().score!, { layer: 'kick', bars: { start: 10, end: 11 } });
    expect(map.lanes[1].cells.filter((c) => c.sel).map((c) => c.bar)).toEqual([10, 11]);
    expect(map.lanes[0].cells.some((c) => c.sel)).toBe(false);
  });
});

describe('asking', () => {
  it('a scoped ask produces a patch whose scope is exactly the dragged bars and layer', async () => {
    const e = await demo();
    const r = await e.ask({ text: 'make these softer', scope: { layer: 'piano', bars: { start: 5, end: 8 } } });
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.take.n).toBe(4);
    expect(r.take.patch!.scope).toEqual({ bars: { start: 5, end: 8 }, layers: ['piano'] });
    expect(r.take.changed).toEqual({ piano: [5, 8] });
    expect(r.take.ask).toBe('make these softer (Bars 5–8 · Felt piano)');
    expect(checkSchema(patchSchema, r.take.patch)).toEqual([]);
  });

  it('a suggestion makes take 4 and opens the view it names', async () => {
    const e = await demo();
    const opened: string[] = [];
    e.onOpenView((v) => opened.push(`${v.view} by ${v.by}`));
    const chips = await e.suggestions();
    expect(chips).toContain('Make it a little darker');
    const r = await e.ask({ text: chips[0] });
    expect(r.ok && r.take.n).toBe(4);
    expect(r.ok && r.take.changed).toEqual({ piano: [1, 16] });
    expect(opened).toEqual(['mix by Praxis']);
  });

  it('a request on a locked layer is refused by the lock rule and makes no take', async () => {
    const e = await demo();
    const lock = await e.applyPatch({ origin: 'user', scope: { layers: ['piano'] }, ops: [{ op: 'set_lock', id: 'piano', locked: true }] });
    expect(lock.ok).toBe(true);
    const r = await e.ask({ text: 'lift', scope: { layer: 'piano', bars: { start: 13, end: 16 } } });
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.error?.kind).toBe('locked_layer');
      expect(r.message).toContain('layer `piano` is locked');
    }
    expect((await e.takes()).length).toBe(3);
  });

  it('a request the mock cannot plan says so and makes no take', async () => {
    const e = await demo();
    const r = await e.ask({ text: 'make it sound like a sunset' });
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.message).toContain('mock engine understands');
  });

  it('an agent take is made under its name', async () => {
    const e = await demo();
    const r = await e.agentAsk('trailer-bot', 'Change the last four bars');
    expect(r.ok && r.take.who).toBe('Agent · trailer-bot');
  });

  it('a new sound from words starts take 1 in the key asked for', async () => {
    const e = new MockEngine();
    const r = await e.ask({ text: 'A warm 20-second loop in D major' });
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.take.n).toBe(1);
    expect(r.take.score.key).toEqual({ tonic: 'D', mode: 'major' });
    expect(r.take.checks.checks.find((c) => c.name === 'key')?.passed).toBe(true);
  });
});

describe('restore and undo', () => {
  it('going back to take 1 is an undoable patch', async () => {
    const e = await demo();
    const [t1, , t3] = await e.takes();
    const r = await e.restore(1);
    expect(r.ok).toBe(true);
    expect(e.state().current).toBe(1);
    expect(e.state().score).toEqual(t1.score);
    expect(e.state().canUndo).toBe(true);
    expect(await e.undo()).toBe(true);
    expect(e.state().current).toBe(3);
    expect(e.state().score).toEqual(t3.score);
    expect(await e.redo()).toBe(true);
    expect(e.state().current).toBe(1);
  });

  it('undoing a take moves back to the previous take; the take stays listed', async () => {
    const e = await demo();
    await e.ask({ text: 'Make the kick quieter' });
    expect(e.state().current).toBe(4);
    await e.undo();
    expect(e.state().current).toBe(3);
    expect(e.state().takes.length).toBe(4);
    expect(e.state().score).toEqual((await e.takes())[2].score);
  });

  it('undo calls fired without waiting each undo one step (operations are serialised)', async () => {
    const e = await demo();
    await Promise.all([e.undo(), e.undo()]);
    expect(e.state().current).toBe(1);
    expect(e.state().score).toEqual((await e.takes())[0].score);
    expect(e.state().canUndo).toBe(false); // the history held takes 2 and 3; take 1 is where it starts
  });

  it('mute on the live score is undoable and makes no take', async () => {
    const e = await demo();
    await e.applyPatch({ origin: 'user', scope: { layers: ['kick'] }, ops: [{ op: 'update_layer', id: 'kick', update: { muted: true } }] });
    expect(e.state().takes.length).toBe(3);
    expect(e.state().score!.layers[1].muted).toBe(true);
    await e.undo();
    expect(e.state().score!.layers[1].muted).toBe(false);
  });
});

describe('checks summary', () => {
  const report = (passes: boolean[]): VerifyReport => ({
    checks: passes.map((p, i) => ({ name: ['key', 'grid', 'loudness', 'true_peak', 'loop_seam'][i], passed: p, measured: 'm', expected: 'e' })),
    tempo_bpm: 128,
    key: 'a minor',
    grid_deviation_ms: 1,
    onsets_on_grid: 0.98,
    integrated_lufs: -16,
    true_peak_dbtp: -1.1,
    seam_jump_ratio: 0.5,
    seam_excess_db: -6,
    passed: passes.every(Boolean),
  });
  it('counts passes from the report', () => {
    const s = summarizeChecks(report([true, true, true, true, true]));
    expect([s.passed, s.total, s.label]).toEqual([5, 5, '5 of 5 checks passed']);
    expect(s.items.map((i) => i.text)).toEqual([
      'in key (A minor)',
      'on the beat (98% of notes within 10 ms)',
      'loudness (-16.0 LUFS)',
      'peak (-1.1 dB)',
      'seamless loop (m)',
    ]);
  });
  it('reports failures honestly', () => {
    const s = summarizeChecks(report([true, false, true, false, true]));
    expect([s.passed, s.total, s.allPassed, s.label]).toEqual([3, 5, false, '2 of 5 checks failed']);
  });
});

describe('take helpers', () => {
  it('changedFromPatch counts bar ops by range and layer-wide changes by extent', () => {
    const s = fixture();
    expect(changedFromPatch(s, s, { ops: [{ op: 'clear_bars', layer: 'kick', bars: { start: 13, end: 16 } }] })).toEqual({ kick: [13, 16] });
    expect(changedFromPatch(s, s, { ops: [{ op: 'update_layer', id: 'kick', update: { gain_db: -6 } }] })).toEqual({ kick: [9, 16] });
    expect(changedFromPatch(s, s, { ops: [{ op: 'update_layer', id: 'kick', update: { name: 'K' } }] })).toEqual({});
    expect(changedFromPatch(s, s, { ops: [{ op: 'set_tempo', bpm: 100 }] })).toEqual({ piano: [1, 16], kick: [9, 16] });
  });
  it('untouched text names the bars outside the change', () => {
    const s = fixture();
    expect(untouchedText(s, s, { piano: [5, 8] }, 2, false)).toBe('Bars 1–4 and 9–16 identical note for note to take 2. Soft kick unchanged.');
  });
  it('identicalOutside ignores the changed window and tails, and catches other differences', () => {
    const s = fixture();
    const sr = 100;
    const n = 30 * sr;
    const a = [new Float32Array(n), new Float32Array(n)];
    const b = [new Float32Array(n), new Float32Array(n)];
    b[0][Math.round(28 * sr)] = 0.5; // inside bar 15
    expect(identicalOutside(a, b, sr, s, { kick: [15, 15] })).toBe(true);
    b[0][Math.round(5 * sr)] = 0.5; // bar 3
    expect(identicalOutside(a, b, sr, s, { kick: [15, 15] })).toBe(false);
  });
});
