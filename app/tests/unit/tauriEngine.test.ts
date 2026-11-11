// TauriEngine against a fake Tauri bridge that answers like crates/praxis-app: the same
// command names, argument shapes and payloads (take records as in docs/contracts/cli.md,
// "The take record"), with an in-memory sound folder standing in for takes.json.

import { describe, expect, it } from 'vitest';
import { TauriEngine, TAKES_EVENT, type AudioLike } from '../../src/engine/tauri/TauriEngine';
import type { Bridge } from '../../src/engine/tauri/bridge';
import type { AskPayload, SoundInfo, TakeRecord, TakesPayload, ViewRequest } from '../../src/engine/tauri/protocol';
import { toTake, toWho } from '../../src/engine/tauri/protocol';
import type { Take, TakeState } from '../../src/engine/EngineAdapter';
import { applyPatch } from '../../src/engine/mock/apply';
import { muteLayer } from '../../src/score/patchBuilder';
import type { NScore, ScorePatch } from '../../src/score/types';
import { fixture } from './fixtures';

const clone = <T>(v: T): T => (v === undefined ? v : (JSON.parse(JSON.stringify(v)) as T));

const CHECKS = {
  checks: [
    { name: 'key', passed: true, measured: 'a minor', expected: 'a minor' },
    { name: 'loudness', passed: true, measured: '-16.0 LUFS', expected: '-16 ± 1 LUFS' },
  ],
  tempo_bpm: 128,
  key: 'a minor',
  grid_deviation_ms: 1.7,
  onsets_on_grid: 0.91,
  integrated_lufs: -16,
  true_peak_dbtp: -1,
  seam_jump_ratio: 1,
  seam_excess_db: -20,
  passed: true,
  failed: [],
  layers_checked: ['piano'],
};

/** One sound folder, kept the way the take ops keep it. */
class FakeSound {
  revision = 0;
  current = 0;
  takes: TakeRecord[] = [];
  views: ViewRequest[] = [];
  live: NScore | null = null;
  undo: { patch: ScorePatch; inverse: ScorePatch; current: [number, number] }[] = [];
  redo: { patch: ScorePatch; inverse: ScorePatch; current: [number, number] }[] = [];

  constructor(readonly path: string) {}

  addTake(who: string, ask: string, score: NScore, patch: ScorePatch | null, changed: Record<string, [number, number]> = {}): TakeRecord {
    const n = this.takes.length + 1;
    const rec: TakeRecord = {
      n,
      who: toWho(who),
      ask,
      title: n === 1 ? 'Calm piano loop' : `Take ${n}`,
      say: `Take ${n} said.`,
      patch,
      changed,
      untouched: n === 1 ? '' : `Bars 1–12 sample-identical to take ${n - 1}.`,
      render: {
        path: `${this.path}\\takes\\${n}\\mix.wav`,
        report: {
          sample_rate: 48000,
          length_samples: 1440000,
          frames: 1440000,
          tail_samples: 0,
          is_loop: true,
          loudness_lufs: -16,
          loudness_before_lufs: -17,
          true_peak_dbtp: -1,
          applied_gain_db: 1,
          warmup_cycles: 1,
          notes_rendered: 176,
          blocked_layers: [],
          render_seconds: 5.9,
          warnings: [],
        },
      },
      checks: clone(CHECKS),
      version: `take-${n}`,
      from: n - 1,
      provider: 'offline-rules',
      seed: n,
      score: clone(score),
    };
    if (patch && this.live) this.undo.push({ patch, inverse: { ops: [] } as unknown as ScorePatch, current: [this.current, n] });
    this.redo = [];
    this.takes.push(rec);
    this.current = n;
    this.live = clone(score);
    this.revision += 1;
    return rec;
  }

  payload(): TakesPayload {
    return {
      sound: this.path,
      revision: this.revision,
      current: this.current,
      takes: clone(this.takes),
      score: this.live ? clone(this.live) : null,
      can_undo: this.undo.length > 0,
      can_redo: this.redo.length > 0,
      views: clone(this.views),
    };
  }
}

/** Answers every command TauriEngine sends, as crates/praxis-app/src/shell.rs does. */
class FakeBridge implements Bridge {
  sounds = new Map<string, FakeSound>();
  calls: { cmd: string; args?: Record<string, unknown> }[] = [];
  watched: string | null = null;
  private listeners = new Map<string, ((p: unknown) => void)[]>();
  readonly root = 'C:\\Users\\me\\Documents\\Praxis Sound\\sounds';

  fileUrl(path: string): string {
    return `http://asset.localhost/${encodeURIComponent(path)}`;
  }

  async listen<T>(event: string, cb: (payload: T) => void): Promise<() => void> {
    const list = this.listeners.get(event) ?? [];
    list.push(cb as (p: unknown) => void);
    this.listeners.set(event, list);
    return () => this.listeners.set(event, (this.listeners.get(event) ?? []).filter((x) => x !== cb));
  }

  emit(event: string, payload: unknown): void {
    (this.listeners.get(event) ?? []).forEach((cb) => cb(payload));
  }

  /** What the watcher does when takes.json gets a new revision. */
  touch(sound: FakeSound): void {
    if (this.watched === sound.path) this.emit(TAKES_EVENT, { sound: sound.path, revision: sound.revision });
  }

  sound(path: string): FakeSound {
    let s = this.sounds.get(path);
    if (!s) {
      s = new FakeSound(path);
      this.sounds.set(path, s);
    }
    return s;
  }

  async invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    this.calls.push({ cmd, args: clone(args) });
    const req = (args?.req ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case 'list_sounds':
        return [...this.sounds.values()]
          .filter((s) => s.takes.length)
          .map((s): SoundInfo => ({ id: s.path.split('\\').at(-1)!, name: s.takes[0].title, path: s.path, sub: 'A minor · 0:30 loop', takes: s.takes.length, modified_ms: 1 })) as T;
      case 'new_sound_path':
        return `${this.root}\\calm-30-second-felt-piano-loop` as T;
      case 'watch_sound': {
        // Like commands::audio_dirs: only a folder with a take history is a sound.
        const path = args!.sound as string;
        if (!this.sounds.get(path)?.revision) throw { code: 'error', message: `${path} is not a Praxis sound folder (it has no takes.json of format praxis-takes)` };
        this.watched = path;
        return this.sound(path).revision as T;
      }
      case 'takes':
        return this.sound(args!.sound as string).payload() as T;
      case 'ask': {
        const s = this.sound(req.sound as string);
        const text = req.text as string;
        if (text.includes('impossible')) throw { code: 'patch_rejected', message: 'layer `piano` is locked', patch_error: { kind: 'locked_layer', layer: 'piano' } };
        let score: NScore;
        let patch: ScorePatch | null = null;
        if (!s.live) score = fixture();
        else if (req.patch) {
          patch = req.patch as ScorePatch;
          const r = applyPatch(s.live, patch);
          if (!r.ok) throw { code: 'patch_rejected', message: 'refused', patch_error: r.error };
          score = r.score;
        } else {
          score = clone(s.live);
          score.tempo.bpm += 1;
          patch = { description: text, origin: 'model', ops: [] } as unknown as ScorePatch;
        }
        const rec = s.addTake((req.who as string) ?? 'You', text, score, patch, patch ? { piano: [13, 16] } : {});
        const out: AskPayload = { sound: s.path, revision: s.revision, current: s.current, take: rec, can_undo: s.undo.length > 0, can_redo: false };
        return out as T;
      }
      case 'restore': {
        const s = this.sound(req.sound as string);
        const t = s.takes.find((x) => x.n === req.n);
        if (!t) throw { code: 'error', message: `there is no take ${req.n}` };
        s.undo.push({ patch: { ops: [] } as unknown as ScorePatch, inverse: { ops: [] } as unknown as ScorePatch, current: [s.current, t.n] });
        s.current = t.n;
        s.live = clone(t.score!) as NScore;
        s.revision += 1;
        return { current: s.current, restored: true } as T;
      }
      case 'apply_patch': {
        const s = this.sound(args!.sound as string);
        const r = applyPatch(s.live!, args!.patch as ScorePatch);
        if (!r.ok) throw { code: 'patch_rejected', message: 'refused', patch_error: r.error };
        s.undo.push({ patch: args!.patch as ScorePatch, inverse: r.inverse, current: [s.current, s.current] });
        s.redo = [];
        s.live = r.score;
        return { live: { score: s.live, can_undo: true, can_redo: false, current: s.current }, inverse: r.inverse } as T;
      }
      case 'undo':
      case 'redo': {
        const s = this.sound(args!.sound as string);
        const from = cmd === 'undo' ? s.undo : s.redo;
        const to = cmd === 'undo' ? s.redo : s.undo;
        const e = from.pop();
        if (e) {
          to.push(e);
          const n = cmd === 'undo' ? e.current[0] : e.current[1];
          if (e.inverse.ops.length || e.patch.ops.length) {
            const r = applyPatch(s.live!, cmd === 'undo' ? e.inverse : e.patch);
            if (r.ok) s.live = r.score;
          } else s.live = clone(s.takes.find((t) => t.n === n)!.score!) as NScore;
          if (n !== s.current) {
            s.current = n;
            s.revision += 1;
          }
        }
        return { moved: !!e, live: { score: s.live, can_undo: s.undo.length > 0, can_redo: s.redo.length > 0, current: s.current } } as T;
      }
      case 'render_live':
        return { output: `${args!.sound}\\live\\mix.wav`, sample_rate: 48000, frames: 960000, is_loop: true, loudness_lufs: -16.2, true_peak_dbtp: -1.2, check: CHECKS } as T;
      case 'export': {
        const a = args!.args as { sound: string; take: number; kinds: string[]; destination: string; base_name: string };
        return { ok: true, files: a.kinds.map((k) => ({ kind: k, path: `${a.sound}\\${a.destination}\\${a.base_name}.${k}` })), message: `Wrote ${a.kinds.length} files.` } as T;
      }
      default:
        throw { code: 'error', message: `unknown command ${cmd}` };
    }
  }
}

class FakeAudio implements AudioLike {
  src = '';
  loop = false;
  currentTime = 0;
  duration = 30;
  paused = true;
  async play(): Promise<void> {
    this.paused = false;
  }
  pause(): void {
    this.paused = true;
  }
}

function setup(): { bridge: FakeBridge; engine: TauriEngine; audio: FakeAudio; states: TakeState[]; takes: Take[]; views: string[] } {
  const bridge = new FakeBridge();
  const audio = new FakeAudio();
  const engine = new TauriEngine(bridge, { makeAudio: () => audio, now: () => 1_000_000 });
  const states: TakeState[] = [];
  const takes: Take[] = [];
  const views: string[] = [];
  engine.onState((s) => states.push(s));
  engine.onTake((t) => takes.push(t));
  engine.onOpenView((e) => views.push(`${e.view} by ${e.by}`));
  return { bridge, engine, audio, states, takes, views };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe('TauriEngine over a fake Tauri bridge', () => {
  it('makes take 1 of a new sound in a new folder and announces the folder', async () => {
    const { bridge, engine, takes } = setup();
    await engine.openSound(null);
    expect(engine.state().takes).toEqual([]);
    const created: SoundInfo[] = [];
    engine.onSoundCreated((s) => created.push(s));

    const r = await engine.ask({ text: 'A calm 30-second felt piano loop in A minor' });
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.take.n).toBe(1);
    expect(r.take.who).toBe('You');
    expect(r.take.render.audioUrl).toBe(bridge.fileUrl(`${bridge.root}\\calm-30-second-felt-piano-loop\\takes\\1\\mix.wav`));
    expect(r.take.render.report.mock).toBe(false);
    expect(r.take.render.report.length_samples).toBe(1440000);
    expect(r.take.checks.passed).toBe(true);
    expect(r.take.checks.checks.map((c) => c.name)).toEqual(['key', 'loudness']);
    expect(r.take.score.key.tonic).toBe('A');
    expect(engine.soundPath()).toBe(`${bridge.root}\\calm-30-second-felt-piano-loop`);
    expect(created.map((s) => s.id)).toEqual(['calm-30-second-felt-piano-loop']);
    expect(takes.map((t) => t.n)).toEqual([1]);
    const askCall = bridge.calls.find((c) => c.cmd === 'ask')!;
    expect(askCall.args).toEqual({ req: { sound: `${bridge.root}\\calm-30-second-felt-piano-loop`, text: 'A calm 30-second felt piano loop in A minor', who: 'You', scope: null } });
    expect(bridge.watched).toBe(engine.soundPath());
    const st = engine.state();
    expect(st.current).toBe(1);
    expect(st.score?.key.mode).toBe('minor');
  });

  it('opens a sound folder without replaying its history', async () => {
    const { bridge, engine, takes, views, states } = setup();
    const s = bridge.sound(`${bridge.root}\\calm`);
    s.addTake('You', 'calm loop', fixture(), null);
    s.addTake('cut-assistant', 'add a kick', fixture(), { ops: [] } as unknown as ScorePatch, { kick: [9, 16] });
    s.views.push({ revision: 2, view: 'notes', by: 'cut-assistant', take: 2 });
    await engine.openSound(s.path);
    expect(engine.state().takes.map((t) => [t.n, t.who])).toEqual([
      [1, 'You'],
      [2, 'Agent · cut-assistant'],
    ]);
    expect(engine.state().current).toBe(2);
    expect(engine.state().canUndo).toBe(true);
    expect(takes).toEqual([]);
    expect(views).toEqual([]);
    expect(states.at(-1)?.takes.length).toBe(2);
    expect((await engine.listSounds()).map((x) => x.path)).toEqual([s.path]);
    // An agent that worked on the sound earlier is known but not counted as connected.
    expect(await engine.agents()).toEqual([{ name: 'cut-assistant', connected: false, about: 'Makes takes through the Praxis MCP server.' }]);
  });

  it("shows an agent's take live when takes.json gets a new revision", async () => {
    const { bridge, engine, takes, views } = setup();
    const s = bridge.sound(`${bridge.root}\\calm`);
    s.addTake('You', 'calm loop', fixture(), null);
    await engine.openSound(s.path);

    // The agent writes take 2 through the MCP server and opens Notes; the watcher fires.
    s.addTake('Agent · cut-assistant', 'lift bars 13-16', fixture(), { ops: [] } as unknown as ScorePatch, { piano: [13, 16] });
    s.takes[1].opens = 'notes';
    s.views.push({ revision: s.revision, view: 'notes', by: 'cut-assistant', take: 2 });
    bridge.touch(s);
    await tick();
    await tick();
    expect(engine.state().takes.map((t) => t.n)).toEqual([1, 2]);
    expect(engine.state().current).toBe(2);
    expect(takes.map((t) => [t.n, t.who])).toEqual([[2, 'Agent · cut-assistant']]);
    expect(views).toEqual(['notes by cut-assistant']);
    expect(await engine.agents()).toEqual([{ name: 'cut-assistant', connected: true, about: 'Makes takes through the Praxis MCP server.' }]);

    // The same revision again (or another sound's) changes nothing.
    bridge.touch(s);
    bridge.emit(TAKES_EVENT, { sound: 'elsewhere', revision: 99 });
    await tick();
    expect(takes.length).toBe(1);
    expect(views.length).toBe(1);
  });

  it('sends scoped asks and hand edits, and reports refusals in words', async () => {
    const { bridge, engine } = setup();
    const s = bridge.sound(`${bridge.root}\\calm`);
    s.addTake('You', 'calm loop', fixture(), null);
    await engine.openSound(s.path);

    const scope = { layer: 'piano', bars: { start: 13, end: 16 } };
    const r = await engine.ask({ text: 'softer', scope });
    expect(r.ok && r.take.n).toBe(2);
    expect(bridge.calls.filter((c) => c.cmd === 'ask').at(-1)!.args).toEqual({ req: { sound: s.path, text: 'softer', who: 'You', scope } });

    const no = await engine.ask({ text: 'something impossible' });
    expect(no).toEqual({ ok: false, message: 'layer `piano` is locked', error: { kind: 'locked_layer', layer: 'piano' } });

    const score = engine.state().score!;
    const patch = muteLayer(score, score.layers[0].id, true);
    const e = await engine.edit(patch, 'Mute the piano by hand');
    expect(e.ok && e.take.n).toBe(3);
    const call = bridge.calls.filter((c) => c.cmd === 'ask').at(-1)!.args as { req: Record<string, unknown> };
    expect(call.req.patch).toEqual(patch);
    expect(call.req.text).toBe('Mute the piano by hand');
  });

  it('applies plain patches, undoes across takes and goes back', async () => {
    const { bridge, engine } = setup();
    const s = bridge.sound(`${bridge.root}\\calm`);
    s.addTake('You', 'calm loop', fixture(), null);
    await engine.openSound(s.path);
    await engine.ask({ text: 'faster' });
    expect(engine.state().current).toBe(2);

    const score = engine.state().score!;
    const id = score.layers[0].id;
    const r = await engine.applyPatch(muteLayer(score, id, true));
    expect(r.ok).toBe(true);
    expect(engine.state().score!.layers[0].muted).toBe(true);
    expect(engine.state().takes.length).toBe(2);

    expect(await engine.undo()).toBe(true);
    expect(engine.state().score!.layers[0].muted).toBe(false);
    expect(engine.state().canRedo).toBe(true);
    // Undoing the take itself moves the current take back to 1.
    expect(await engine.undo()).toBe(true);
    expect(engine.state().current).toBe(1);
    expect(await engine.redo()).toBe(true);
    expect(engine.state().current).toBe(2);

    const back = await engine.restore(1);
    expect(back.ok && back.take.n).toBe(1);
    expect(engine.state().current).toBe(1);
    expect(bridge.calls.find((c) => c.cmd === 'restore')!.args).toEqual({ req: { sound: s.path, n: 1 } });
    const missing = await engine.restore(7);
    expect(missing.ok).toBe(false);

    const bad = await engine.applyPatch({ ops: [{ op: 'update_layer', id: 'nope', update: { muted: true } }] } as unknown as ScorePatch);
    expect(bad.ok).toBe(false);
    if (!bad.ok) expect(bad.error.kind).toBe('unknown_id');
  });

  it("plays the current take's file, renders the live score and exports", async () => {
    const { bridge, engine, audio } = setup();
    const s = bridge.sound(`${bridge.root}\\calm`);
    s.addTake('You', 'calm loop', fixture(), null);
    await engine.openSound(s.path);
    const positions: boolean[] = [];
    engine.onPosition((p) => positions.push(p.playing));

    await engine.play();
    expect(audio.src).toBe(engine.state().takes[0].render.audioUrl);
    expect(audio.loop).toBe(true);
    expect(audio.paused).toBe(false);
    await engine.seek(4);
    expect(audio.currentTime).toBe(4);
    await engine.stop();
    expect(audio.paused).toBe(true);
    expect(positions.at(-1)).toBe(false);

    const rr = await engine.render();
    expect(rr.audioUrl).toBe(bridge.fileUrl(`${s.path}\\live\\mix.wav`));
    expect(rr.report.length_samples).toBe(960000);

    const ex = await engine.export({ kinds: ['wav24', 'midi'], destination: 'exports', baseName: 'calm-take-1' });
    expect(ex.ok).toBe(true);
    expect(ex.files.map((f) => f.kind)).toEqual(['wav24', 'midi']);
    expect(bridge.calls.find((c) => c.cmd === 'export')!.args).toEqual({ args: { sound: s.path, take: 1, kinds: ['wav24', 'midi'], destination: 'exports', base_name: 'calm-take-1' } });
    expect(await engine.suggestions()).toContain('Change the last four bars');
  });

  it('refuses to open a folder that is not a sound, and stays on an empty sound', async () => {
    const { bridge, engine } = setup();
    await expect(engine.openSound('C:\\')).rejects.toThrow('Could not open C:\\: C:\\ is not a Praxis sound folder');
    expect(engine.soundPath()).toBeNull();
    expect(engine.state().takes).toEqual([]);
    expect(bridge.watched).toBeNull();
    // Nothing was asked of the folder after the refusal.
    expect(bridge.calls.map((c) => c.cmd)).toEqual(['watch_sound']);
  });

  it('maps a contract take record onto the TypeScript Take field for field', () => {
    const s = new FakeSound('D:\\s');
    const rec = s.addTake('cut-assistant', 'x', fixture(), null);
    rec.opens = 'mix';
    rec.scope = { layer: 'piano', bars: { start: 1, end: 2 } };
    const t = toTake(rec, (p) => `u:${p}`, null);
    expect(t.who).toBe('Agent · cut-assistant');
    expect(t.render.audioUrl).toBe('u:D:\\s\\takes\\1\\mix.wav');
    expect(t.opens).toBe('mix');
    expect(t.scope).toEqual(rec.scope);
    expect(t.checks.grid_deviation_ms).toBe(1.7);
    expect(Object.keys(t).sort()).toEqual(['ask', 'changed', 'checks', 'n', 'opens', 'patch', 'render', 'say', 'scope', 'score', 'title', 'untouched', 'who'].sort());
  });
});
