// TauriEngine against a fake Tauri bridge that answers like crates/praxis-app: the same
// command names, argument shapes and payloads (take records as in docs/contracts/cli.md,
// "The take record"), with an in-memory sound folder standing in for takes.json.

import { describe, expect, it } from 'vitest';
import { TauriEngine, TAKES_EVENT, TRANSPORT_EVENT, type AudioLike, type PlaybackInfo } from '../../src/engine/tauri/TauriEngine';
import type { Bridge } from '../../src/engine/tauri/bridge';
import type { AskPayload, SoundInfo, TakeRecord, TakesPayload, TransportEventPayload, TransportStatusPayload, ViewRequest } from '../../src/engine/tauri/protocol';
import { toTake, toWho } from '../../src/engine/tauri/protocol';
import type { Meters, Take, TakeState, TransportPosition } from '../../src/engine/EngineAdapter';
import { applyPatch } from '../../src/engine/mock/apply';
import { muteLayer } from '../../src/score/patchBuilder';
import type { NScore, ScorePatch } from '../../src/score/types';
import { fixture } from './fixtures';
import { blankScore, DRAFT_ASK } from '../../src/engine/blank';
import { addLayerPatch } from '../../src/ui/handEdits';
import { STANDARD_PRESETS } from '../../src/engine/presets';

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
  /** The audio service (crates/praxis-app/src/audio.rs); null answers like a shell without it. */
  transport: { available: boolean; error: string | null } | null = null;
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
      case 'new_sound': {
        // crates/praxis-app/src/hand.rs `new_sound`: a folder with a blank project, no take.
        const a = args!.args as { title: string | null; key: NScore['key']; bpm?: number; seconds?: number; time_signature: NScore['time_signature']; bars: number; playback: NScore['playback'] };
        const slug = (a.title ?? 'untitled sound').toLowerCase().replace(/[^a-z0-9]+/g, '-');
        const s = this.sound(`${this.root}\\${slug}`);
        s.live = blankScore({ title: a.title ?? undefined, key: a.key, bpm: a.bpm, seconds: a.seconds, timeSignature: a.time_signature, bars: a.bars, loop: a.playback.mode === 'loop' });
        s.revision = 1;
        return { sound: s.path, revision: 1, live: { score: s.live, can_undo: false, can_redo: false, current: 0 } } as T;
      }
      case 'make_take': {
        const s = this.sound(req.sound as string);
        if (!s.live?.layers.length) throw { code: 'error', message: 'add a layer first: a sound with no layers has nothing to play' };
        const prev = s.takes.find((t) => t.n === s.current);
        if (prev && JSON.stringify(prev.score) === JSON.stringify(s.live)) throw { code: 'patch_rejected', message: `nothing changed since take ${s.current}: edit the sound, then make a take` };
        const patch = s.takes.length ? ({ description: req.ask, origin: 'user', scope: { layers: [] }, ops: [{ op: 'set_tempo', bpm: s.live.tempo.bpm }] } as ScorePatch) : null;
        const rec = s.addTake('You', req.ask as string, s.live, patch, patch ? { felt_piano: [1, 16] } : {});
        rec.title = patch ? 'Hand edit' : 'Built by hand';
        rec.provider = 'hand';
        if (req.scope) rec.scope = req.scope as TakeRecord['scope'];
        return { sound: s.path, revision: s.revision, current: s.current, take: rec, can_undo: true, can_redo: false } as T;
      }
      case 'presets':
        return {
          presets: STANDARD_PRESETS.map((p) => ({ ...p, available: p.name !== 'upright_piano', reason: p.name === 'upright_piano' ? 'pack missing' : null })),
          engine_only: [{ name: 'tape_hiss', kind: 'synth', class: 'texture', source: 'synth' }],
        } as T;
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
      case 'transport_sync':
      case 'transport_play':
      case 'transport_status':
        if (!this.transport) break;
        return this.status() as T;
      case 'transport_stop':
      case 'transport_seek':
      case 'transport_set_loop':
      case 'transport_layer':
        if (!this.transport) break;
        return undefined as T;
    }
    throw { code: 'error', message: `unknown command ${cmd}` };
  }

  /** What the audio service sends (`TransportEvent`), with overrides. */
  event(over: Partial<TransportEventPayload> = {}): TransportEventPayload {
    const t = this.transport ?? { available: false, error: 'no service' };
    return {
      available: t.available,
      device: t.available ? 'Speakers (Realtek(R) Audio)' : null,
      sample_rate: t.available ? 48000 : 0,
      error: t.error,
      playing: false,
      ended: false,
      position: 0,
      seconds: 0,
      peak_db: [-120, -120],
      rms_db: [-120, -120],
      xruns: 0,
      xruns_total: 0,
      late_wakeups: 0,
      device_errors: 0,
      min_callback_frames: 480,
      max_callback_frames: 480,
      callbacks: 10,
      swaps: 1,
      preparing: false,
      stalled: false,
      ...over,
    };
  }

  status(): TransportStatusPayload {
    return { ...this.event(), requested_buffer_frames: 256, loaded: true, prepare_ms: 12, warnings: [], max_render_ms: 0.4, p99_render_ms: 0.2, layers: {}, loop_range: null };
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

  it('plays through the real-time engine: transport commands, position and meters from its events', async () => {
    const { bridge, engine, audio } = setup();
    bridge.transport = { available: true, error: null };
    const s = bridge.sound(`${bridge.root}\\calm`);
    s.addTake('You', 'calm loop', fixture(), null);
    await engine.openSound(s.path);
    await tick();
    expect(bridge.calls.filter((c) => c.cmd === 'transport_sync').map((c) => c.args)).toEqual([{ sound: s.path }]);
    const positions: TransportPosition[] = [];
    const meters: Meters[] = [];
    engine.onPosition((p) => positions.push(p));
    engine.onMeters((m) => meters.push(m));

    await engine.play();
    expect(bridge.calls.at(-1)!.cmd).toBe('transport_play');
    expect(audio.paused).toBe(true); // no <audio>: the engine plays
    expect(engine.playback()).toMatchObject({ mode: 'engine', message: null, device: 'Speakers (Realtek(R) Audio)', callbackFrames: [480, 480] });

    // 128 BPM in 4/4: a bar is 1.875 s, so 3.75 s is the start of bar 3.
    bridge.emit(TRANSPORT_EVENT, bridge.event({ playing: true, seconds: 3.75, position: 180000, peak_db: [-6, -7.5], rms_db: [-18, -120] }));
    expect(positions.at(-1)).toEqual({ playing: true, seconds: 3.75, bar: 3, loop: null });
    expect(meters.at(-1)).toEqual({ peakL: -6, peakR: -7.5, rmsL: -18, rmsR: -Infinity });

    await engine.seek(7.5);
    await engine.setLoop({ start: 5, end: 6 });
    expect(bridge.calls.slice(-2)).toEqual([
      { cmd: 'transport_seek', args: { seconds: 7.5 } },
      { cmd: 'transport_set_loop', args: { range: { start: 5, end: 6 } } },
    ]);
    await engine.setLayerLive('piano', { gain_db: -3 });
    expect(bridge.calls.at(-1)).toEqual({ cmd: 'transport_layer', args: { layer: 'piano', live: { gain_db: -3 } } });
    await engine.stop();
    expect(bridge.calls.at(-1)!.cmd).toBe('transport_stop');
    expect(positions.at(-1)!.playing).toBe(false);
    expect(audio.src).toBe('');
  });

  it('sends every change of the live score to the engine (take, mute, undo, go back), once each', async () => {
    const { bridge, engine } = setup();
    bridge.transport = { available: true, error: null };
    const s = bridge.sound(`${bridge.root}\\calm`);
    s.addTake('You', 'calm loop', fixture(), null);
    await engine.openSound(s.path);
    await tick();
    const syncs = () => bridge.calls.filter((c) => c.cmd === 'transport_sync').length;
    expect(syncs()).toBe(1);

    await engine.ask({ text: 'a little faster' }); // a new take while playing
    await tick();
    expect(syncs()).toBe(2);
    const mute = await engine.applyPatch(muteLayer(engine.state().score!, 'piano', true));
    expect(mute.ok).toBe(true);
    await tick();
    expect(syncs()).toBe(3);
    await engine.undo();
    await tick();
    expect(syncs()).toBe(4);
    await engine.restore(1);
    await tick();
    expect(syncs()).toBe(5);
    // Re-reading an unchanged history sends nothing.
    s.revision += 1;
    bridge.touch(s);
    await tick();
    await tick();
    expect(syncs()).toBe(5);
    // A take by an agent is swapped in too.
    bridge.calls.length = 0;
    await bridge.invoke<AskPayload>('ask', { req: { sound: s.path, text: 'lift the ending', who: 'cut-assistant' } });
    bridge.touch(s);
    await tick();
    await tick();
    expect(engine.state().current).toBe(s.current);
    expect(bridge.calls.filter((c) => c.cmd === 'transport_sync')).toHaveLength(1);
    // Closing the sound clears the engine.
    await engine.openSound(null);
    await tick();
    expect(bridge.calls.at(-1)).toEqual({ cmd: 'transport_sync', args: { sound: null } });
  });

  it("falls back to the take's file and says so when no output device opens", async () => {
    const { bridge, engine, audio } = setup();
    bridge.transport = { available: false, error: 'No audio output: no default output device' };
    const info: PlaybackInfo[] = [];
    engine.onPlayback((p) => info.push(p));
    const s = bridge.sound(`${bridge.root}\\calm`);
    s.addTake('You', 'calm loop', fixture(), null);
    await engine.openSound(s.path);
    await engine.play();
    expect(audio.paused).toBe(false);
    expect(audio.src).toBe(engine.state().takes[0].render.audioUrl);
    expect(info.at(-1)).toMatchObject({ mode: 'audio', message: "No audio output: no default output device. Playing the take's own file instead." });
    // The device comes back: the next play goes through the engine.
    bridge.transport = { available: true, error: null };
    await engine.stop();
    await engine.play();
    expect(audio.paused).toBe(true);
    expect(info.at(-1)).toMatchObject({ mode: 'engine', message: null });
  });

  it('shows device trouble: xruns, errors and a stalled device in the playback info', async () => {
    const { bridge, engine } = setup();
    bridge.transport = { available: true, error: null };
    const s = bridge.sound(`${bridge.root}\\calm`);
    s.addTake('You', 'calm loop', fixture(), null);
    await engine.openSound(s.path);
    const info: PlaybackInfo[] = [];
    engine.onPlayback((p) => info.push(p));
    bridge.emit(TRANSPORT_EVENT, bridge.event({ playing: true, xruns: 2, late_wakeups: 5, device_errors: 1, stalled: true }));
    expect(info.at(-1)).toMatchObject({ mode: 'engine', xruns: 2, lateWakeups: 5, deviceErrors: 1, stalled: true });
    // The sound cannot be built for playback: the take's file plays instead, and the reason shows.
    bridge.emit(TRANSPORT_EVENT, bridge.event({ error: 'Could not build the sound to play: missing pack' }));
    expect(info.at(-1)).toMatchObject({ mode: 'audio', message: "Could not build the sound to play: missing pack. Playing the take's own file instead." });
  });
});

describe('TauriEngine: building by hand (WP-U5)', () => {
  it('starts a blank sound in its own folder, edits it live and makes its first take', async () => {
    const { bridge, engine, takes } = setup();
    bridge.transport = { available: true, error: null };
    await engine.openSound(null);
    const created: SoundInfo[] = [];
    engine.onSoundCreated((s) => created.push(s));

    expect(await engine.startBlank({ key: { tonic: 'A', mode: 'minor' }, bpm: 1000, timeSignature: { numerator: 4, denominator: 4 }, bars: 16, loop: true })).toEqual({
      ok: false,
      message: 'That makes 1000 BPM; the tempo must be between 20 and 400.',
    });
    expect(bridge.calls.some((c) => c.cmd === 'new_sound')).toBe(false);

    const r = await engine.startBlank({ title: 'Calm piano by hand', key: { tonic: 'A', mode: 'minor' }, seconds: 30, timeSignature: { numerator: 4, denominator: 4 }, bars: 16, loop: true });
    expect(r).toEqual({ ok: true });
    const call = bridge.calls.find((c) => c.cmd === 'new_sound')!;
    expect(call.args).toEqual({
      args: { title: 'Calm piano by hand', key: { tonic: 'A', mode: 'minor' }, seconds: 30, time_signature: { numerator: 4, denominator: 4 }, bars: 16, playback: { mode: 'loop' } },
    });
    const path = `${bridge.root}\\calm-piano-by-hand`;
    expect(engine.soundPath()).toBe(path);
    expect(bridge.watched).toBe(path);
    expect(created.map((s) => s.path)).toEqual([path]);
    let st = engine.state();
    expect(st.draft).toBe(true);
    expect(st.takes).toEqual([]);
    expect(st.score!.tempo.bpm).toBe(128);

    // The live score is synced to the real-time engine before any take, and plays.
    await tick();
    expect(bridge.calls.some((c) => c.cmd === 'transport_sync' && c.args?.sound === path)).toBe(true);
    await engine.play();
    expect(bridge.calls.at(-1)!.cmd).toBe('transport_play');

    // A request in words waits for the first take; it never reaches the planner.
    const asked = await engine.ask({ text: 'make it darker' });
    expect(asked).toEqual({ ok: false, message: DRAFT_ASK });
    expect(bridge.calls.some((c) => c.cmd === 'ask')).toBe(false);

    // Hand edits are plain patches (no take); then one take for the batch.
    const piano = (await engine.presets()).find((p) => p.name === 'felt_piano')!;
    const applied = await engine.applyPatch(addLayerPatch(st.score!, piano));
    expect(applied.ok).toBe(true);
    expect(engine.state().score!.layers.map((l) => l.id)).toEqual(['felt_piano']);
    const t = await engine.makeTake('By hand: added Felt piano', { scope: { layer: 'felt_piano', bars: { start: 1, end: 16 } } });
    expect(t.ok).toBe(true);
    if (!t.ok) return;
    expect(t.take.n).toBe(1);
    expect(t.take.patch).toBeNull();
    expect(bridge.calls.find((c) => c.cmd === 'make_take')!.args).toEqual({
      req: { sound: path, ask: 'By hand: added Felt piano', who: 'You', title: null, say: null, scope: { layer: 'felt_piano', bars: { start: 1, end: 16 } } },
    });
    st = engine.state();
    expect(st.draft).toBe(false);
    expect(st.current).toBe(1);
    expect(takes.map((x) => x.n)).toEqual([1]);

    // Nothing changed since take 1: refused in words, with no take.
    const again = await engine.makeTake('again');
    expect(again).toMatchObject({ ok: false, message: 'nothing changed since take 1: edit the sound, then make a take' });
    engine.dispose();
  });

  it('reads the presets once, with what is not installed and engine-only presets', async () => {
    const { bridge, engine } = setup();
    const a = await engine.presets();
    const b = await engine.presets();
    expect(bridge.calls.filter((c) => c.cmd === 'presets')).toHaveLength(1);
    expect(a).toEqual(b);
    expect(a.find((p) => p.name === 'upright_piano')).toMatchObject({ available: false, reason: 'pack missing' });
    expect(a.find((p) => p.name === 'felt_piano')).toMatchObject({ available: true, kind: 'sampler' });
    expect(a.at(-1)).toEqual({ name: 'tape_hiss', kind: 'synth', class: 'texture', pitched: true, description: '', available: true });
  });
});
