// The browser adapter: the same engine as the desktop, compiled to WebAssembly
// (crates/praxis-wasm). A request in words goes through the offline planner, every take is
// rendered by the real offline renderer and checked by the real verifier in a worker, and the
// transport plays the live score through the real-time engine in an AudioWorklet.
//
// - Takes (ADR-0011) are kept in the browser (IndexedDB, store.ts): a sound's history, its live
//   score and every take's audio, so a reload opens the same sounds.
// - After a new sound's first render the applied master gain is frozen in the score through a
//   `system` patch (as `praxis ask` does), so a later scoped edit leaves untouched bars of the
//   master sample-identical.
// - Sampled instruments load their pack over HTTP from the served `packs/` folder (packs.ts);
//   without it they play a synth stand-in and the take says so.
// - There is no API key in the browser: requests use the offline planner and edit rules. There
//   is no MCP server either, so no agents connect.

import type { BarRange, NScore, Score, ScorePatch } from '../score/types';
import { clone, deepEqual, formatRange, normalizeScore } from '../score/types';
import { restorePatch } from '../score/patchBuilder';
import { isUnpitchedClass, keyLabel } from '../score/theory';
import { barPosToSeconds, scoreSeconds, secondsToBarPos } from '../score/timeline';
import type {
  AgentInfo,
  ApplyResult,
  AskProgress,
  AskRequest,
  AskResult,
  BlankSound,
  EngineAdapter,
  ExportRequest,
  ExportResult,
  LoadResult,
  MakeTakeOptions,
  Meters,
  OpenViewEvent,
  PatchError,
  PresetInfo,
  RenderOptions,
  RenderReport,
  RenderResult,
  StartResult,
  Take,
  TakeResult,
  TakeScope,
  TakeState,
  TransportPosition,
  Unsubscribe,
  VerifyReport,
  ViewName,
  Who,
} from '../engine/EngineAdapter';
import { patchErrorMessage } from '../engine/EngineAdapter';
import { changedFromPatch, layerLabel, untouchedText } from '../engine/takes';
import { identicalOutside } from '../engine/MockEngine';
import { blankScore, DRAFT_ASK } from '../engine/blank';
import { presetsFromPayload } from '../engine/presets';
import { decodeWav, encodeWav } from '../engine/wav';
import type { CoreClient, RenderPayload } from './core';
import type { PackLoader } from './packs';
import type { LoadInfo, PlayerStatus, WorkletPlayer } from './player';
import type { SoundStore, SoundSummary, StoredSound, StoredTake } from './store';

/** Sample rate of take renders (the engine's default, as on the desktop). */
export const TAKE_RATE = 48_000;

const SILENT: Meters = { peakL: -Infinity, peakR: -Infinity, rmsL: -Infinity, rmsR: -Infinity };

interface Entry {
  kind: 'take' | 'restore' | 'edit';
  patch: ScorePatch;
  inverse: ScorePatch;
  before: number;
  after: number;
}

interface Words {
  who: Who;
  ask: string;
  title: string;
  say: string;
  opens?: ViewName;
  scope?: TakeScope;
}

/** How the browser build is playing, for the playback line. */
export interface BrowserPlayback {
  mode: 'worklet' | 'none';
  /** Plain words when the worklet cannot play (no Web Audio, the engine failed to start). */
  message: string | null;
  sampleRate: number;
}

export interface WasmEngineDeps {
  core: CoreClient;
  packs: PackLoader;
  store: SoundStore;
  /** Starts real-time playback (an AudioContext with the engine worklet); null when it cannot. */
  player?: () => Promise<WorkletPlayer>;
}

function dbfs(v: number): number {
  return v > 0 ? 20 * Math.log10(v) : -Infinity;
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** "A minor · 0:30 loop". */
export function soundSub(s: NScore): string {
  const secs = scoreSeconds(s);
  return `${keyLabel(s.key)} · ${Math.floor(secs / 60)}:${String(Math.round(secs % 60)).padStart(2, '0')}${s.playback.mode === 'loop' ? ' loop' : ''}`;
}

/** The patch that freezes the master gain after a sound's first render (`praxis ask`). */
export function freezePatch(score: NScore, gainDb: number): ScorePatch {
  return {
    description: `freeze the master gain at ${gainDb.toFixed(2)} dB after the first render, so later edits leave untouched bars sample-identical`,
    origin: 'system',
    scope: {},
    ops: [{ op: 'set_master', master: { ...clone(score.master), frozen_gain_db: gainDb } }],
  } as ScorePatch;
}

export class WasmEngine implements EngineAdapter {
  readonly kind = 'wasm' as const;
  readonly label = 'Browser engine (WebAssembly)';

  private soundId: string | null = null;
  private score: NScore | null = null;
  private draft = false;
  private takeList: Take[] = [];
  private currentN = 0;
  private takeAudio = new Map<number, Float32Array[]>();
  private undoStack: Entry[] = [];
  private redoStack: Entry[] = [];
  private urls: string[] = [];
  private chain: Promise<unknown> = Promise.resolve();
  private takeSubs = new Set<(t: Take) => void>();
  private stateSubs = new Set<(s: TakeState) => void>();
  private viewSubs = new Set<(e: OpenViewEvent) => void>();
  private posSubs = new Set<(p: TransportPosition) => void>();
  private meterSubs = new Set<(m: Meters) => void>();
  private soundSubs = new Set<(s: SoundSummary) => void>();
  private playbackSubs = new Set<(p: BrowserPlayback) => void>();

  // transport
  private player: WorkletPlayer | null = null;
  private starting: Promise<WorkletPlayer | null> | null = null;
  private playerScore: NScore | null = null;
  private statusUnsub: (() => void) | null = null;
  private playing = false;
  private offsetS = 0;
  private positionS = 0;
  private loop: BarRange | null = null;
  /** Applied gain of the last take render (the player's master gain when none is frozen). */
  private lastGainDb: number | null = null;
  playback: BrowserPlayback = { mode: 'none', message: null, sampleRate: 0 };
  /** Plain words about the last pack load (null when every pack loaded). */
  packNotice: string | null = null;

  constructor(private readonly deps: WasmEngineDeps) {}

  // ------------------------------------------------------------ sounds

  /** Sounds kept in this browser, newest first. */
  listSounds(): Promise<SoundSummary[]> {
    return this.deps.store.list();
  }

  /** The open sound's id (null for a new sound before its first take). */
  currentSound(): string | null {
    return this.soundId;
  }

  /** Called once a new sound gets its first take (and so a place in the list). */
  onSoundCreated(cb: (s: SoundSummary) => void): Unsubscribe {
    this.soundSubs.add(cb);
    return () => this.soundSubs.delete(cb);
  }

  onPlayback(cb: (p: BrowserPlayback) => void): Unsubscribe {
    this.playbackSubs.add(cb);
    cb(this.playback);
    return () => this.playbackSubs.delete(cb);
  }

  /** Opens a kept sound by id, or a new, empty sound (null). */
  async openSound(id: string | null): Promise<void> {
    return this.exclusive(async () => {
      // The sound just made (the shell opens a new sound once it is listed): nothing to reload.
      if (id !== null && id === this.soundId) {
        this.emitState();
        return;
      }
      await this.stopTransport();
      this.resetSession();
      this.soundId = null;
      this.score = null;
      this.draft = false;
      this.offsetS = this.positionS = 0;
      this.loop = null;
      if (id) {
        const s = await this.deps.store.load(id);
        if (!s) throw new Error(`There is no sound \`${id}\` in this browser.`);
        this.soundId = s.id;
        this.score = s.score ? normalizeScore(s.score) : null;
        this.draft = s.draft;
        this.currentN = s.current;
        for (const t of s.takes) {
          const wav = await this.deps.store.loadAudio(s.id, t.n);
          const audioUrl = wav ? this.makeUrl(wav) : '';
          this.takeList.push({ ...(t as Omit<Take, 'render'>), render: { audioUrl, report: t.render.report } });
        }
      }
      this.emitPosition();
      this.emitState();
    });
  }

  private async persist(newSound = false): Promise<void> {
    if (!this.soundId) return;
    const first = this.takeList[0];
    const s: StoredSound = {
      id: this.soundId,
      name: first?.title || 'Untitled sound',
      sub: this.score ? soundSub(this.score) : '',
      updated: Date.now(),
      takes: this.takeList.map((t) => ({ ...t, render: { report: t.render.report } }) as StoredTake),
      current: this.currentN,
      score: this.score ? clone(this.score) : null,
      draft: this.draft,
    };
    try {
      await this.deps.store.save(s);
    } catch (e) {
      console.warn(`Could not keep the sound in this browser: ${message(e)}`);
    }
    if (newSound) this.soundSubs.forEach((cb) => cb({ id: s.id, name: s.name, sub: s.sub, updated: s.updated }));
  }

  // ------------------------------------------------------------ score

  async load(input: Score): Promise<LoadResult> {
    await this.stop();
    const score = normalizeScore(input);
    const report = await this.deps.core.validate(score);
    await this.exclusive(async () => {
      this.resetSession();
      this.soundId = null;
      this.score = score;
      this.draft = false;
      this.offsetS = this.positionS = 0;
      this.loop = null;
      this.emitPosition();
      this.emitState();
    });
    const issues = report.ok ? report : { errors: [{ code: 'invalid', path: '', message: report.message }], warnings: [] };
    return { score: clone(score), errors: issues.errors as LoadResult['errors'], warnings: issues.warnings as LoadResult['warnings'] };
  }

  current(): NScore | null {
    return this.score ? clone(this.score) : null;
  }

  async applyPatch(patch: ScorePatch): Promise<ApplyResult> {
    return this.exclusive(async () => {
      if (!this.score) return { ok: false, error: { kind: 'invalid', message: 'no score loaded' } } as ApplyResult;
      const r = await this.deps.core.applyPatch(this.score, patch);
      if (!r.ok) return { ok: false, error: (r.error as PatchError) ?? { kind: 'invalid', message: r.message } } as ApplyResult;
      this.score = normalizeScore(r.score as Score);
      this.undoStack.push({ kind: 'edit', patch: clone(patch), inverse: r.inverse as ScorePatch, before: this.currentN, after: this.currentN });
      this.redoStack = [];
      await this.scoreChanged();
      return { ok: true, score: clone(this.score), inverse: r.inverse as ScorePatch } as ApplyResult;
    });
  }

  // ------------------------------------------------------------ takes

  async ask(request: AskRequest): Promise<AskResult> {
    return this.askAs('You', request);
  }

  private askAs(who: Who, request: AskRequest): Promise<AskResult> {
    return this.exclusive(async (): Promise<AskResult> => {
      const text = request.text.trim();
      const progress = request.onProgress;
      if (request.clipFile) return this.readClip(request.clipFile, text);
      if (this.draft && this.takeList.length === 0) return { ok: false, message: DRAFT_ASK };
      try {
        if (!this.score || this.takeList.length === 0) return await this.newSound(who, text, progress);
        return await this.editInWords(who, text, request.scope, progress);
      } catch (e) {
        return { ok: false, message: `The engine stopped: ${message(e)}` };
      }
    });
  }

  private async newSound(who: Who, text: string, progress?: (p: AskProgress) => void): Promise<TakeResult> {
    if (!text) return { ok: false, message: 'Describe a sound to start from.' };
    progress?.({ kind: 'step', n: 1, words: 'Planning the sound from your words' });
    const plan = await this.deps.core.planBrief(text, 1);
    if (!plan.ok) {
      progress?.({ kind: 'step_done', n: 1, ok: false, words: plan.message });
      return { ok: false, message: plan.message };
    }
    progress?.({ kind: 'step_done', n: 1, ok: true, words: 'Planned it' });
    await this.stopTransport();
    this.resetSession();
    this.soundId = null;
    this.score = normalizeScore(plan.score as Score);
    this.draft = false;
    return this.renderTake(this.score, null, { who, ask: text, title: plan.title, say: plan.summary }, { progress, step: 2 });
  }

  private async editInWords(who: Who, text: string, scope: TakeScope | undefined, progress?: (p: AskProgress) => void): Promise<AskResult> {
    if (!text) return { ok: false, message: 'Say what to change.' };
    const before = this.score!;
    const askText = scope ? `${text} (${scopeLabel(before, scope)})` : text;
    progress?.({ kind: 'step', n: 1, words: 'Working out the change' });
    const e = await this.deps.core.planEdit(before, text, scope ?? null, this.takeList.length + 1);
    if (!e.ok) {
      progress?.({ kind: 'step_done', n: 1, ok: false, words: e.message });
      return { ok: false, message: e.message };
    }
    if (!e.changed) {
      progress?.({ kind: 'step_done', n: 1, ok: true, words: 'Nothing to change' });
      return { ok: true, take: null, reply: e.summary, noTake: 'nothing_to_change' };
    }
    progress?.({ kind: 'step_done', n: 1, ok: true, words: e.summary });
    const patch = e.patch as ScorePatch;
    const prevN = this.currentN;
    const after = normalizeScore(e.score as Score);
    this.undoStack.push({ kind: 'take', patch: clone(patch), inverse: e.inverse as ScorePatch, before: prevN, after: this.takeList.length + 1 });
    this.redoStack = [];
    this.score = after;
    const r = await this.renderTake(after, patch, { who, ask: askText, title: e.title, say: e.summary, scope }, { before, prevN, progress, step: 2 });
    if (!r.ok) this.rollback(before);
    return r;
  }

  /** Undoes a commit whose take could not be made. */
  private rollback(before: NScore): void {
    this.undoStack.pop();
    this.score = before;
    this.emitState();
  }

  async edit(patch: ScorePatch, ask: string, scope?: TakeScope): Promise<AskResult> {
    return this.exclusive(async (): Promise<AskResult> => {
      if (!this.score || this.takeList.length === 0) return { ok: false, message: 'Make a sound first.' };
      const before = this.score;
      const r = await this.deps.core.applyPatch(before, patch);
      if (!r.ok) {
        const error = (r.error as PatchError) ?? { kind: 'invalid', message: r.message };
        return { ok: false, message: `Could not make that take: ${patchErrorMessage(error)}.`, error };
      }
      const prevN = this.currentN;
      this.undoStack.push({ kind: 'take', patch: clone(patch), inverse: r.inverse as ScorePatch, before: prevN, after: this.takeList.length + 1 });
      this.redoStack = [];
      this.score = normalizeScore(r.score as Score);
      const title = scope ? `Hand edit, ${formatRange(scope.bars)}` : 'Hand edit';
      const say = patch.description ? `Changed by hand: ${patch.description.replace(/\.$/, '')}.` : 'Changed by hand.';
      const t = await this.renderTake(this.score, patch, { who: 'You', ask, title, say, scope }, { before, prevN });
      if (!t.ok) this.rollback(before);
      return t;
    });
  }

  async takes(): Promise<Take[]> {
    return [...this.takeList];
  }

  // ------------------------------------------------------------ building by hand

  async startBlank(settings: BlankSound): Promise<StartResult> {
    let score: NScore;
    try {
      score = blankScore(settings);
    } catch (e) {
      return { ok: false, message: message(e) };
    }
    await this.exclusive(async () => {
      await this.stopTransport();
      this.resetSession();
      this.soundId = null;
      this.score = score;
      this.draft = true;
      this.offsetS = this.positionS = 0;
      this.loop = null;
      this.emitPosition();
      this.emitState();
    });
    return { ok: true };
  }

  async presets(): Promise<PresetInfo[]> {
    const p = await this.deps.core.presets();
    if (!p.ok) return [];
    return presetsFromPayload(p as Parameters<typeof presetsFromPayload>[0]).map((x) =>
      // A sampled preset is available once its pack is fetched; the browser fetches it on use.
      x.available || !x.reason?.includes('not on this server') ? x : { ...x, available: true, reason: undefined },
    );
  }

  async makeTake(ask: string, options: MakeTakeOptions = {}): Promise<TakeResult> {
    return this.exclusive(async (): Promise<TakeResult> => {
      const score = this.score;
      if (!score) return { ok: false, message: 'Start a sound first.' };
      if (!score.layers.length) return { ok: false, message: 'Add a layer first: a sound with no layers has nothing to play.' };
      const words = (title: string, say: string): Words => ({ who: 'You', ask, title: options.title ?? title, say: options.say ?? say, scope: options.scope });
      if (!this.takeList.length) {
        const names = score.layers.map((l) => (l.name || l.id).toLowerCase()).join(', ');
        return this.renderTake(score, null, words('Built by hand', `Built by hand: ${names}, ${score.bars} bars at ${score.tempo.bpm} BPM in ${keyLabel(score.key)}.`));
      }
      const prev = this.takeList.find((t) => t.n === this.currentN) ?? this.takeList.at(-1)!;
      if (deepEqual(prev.score, score)) return { ok: false, message: `Nothing changed since take ${prev.n}: edit the sound, then make a take.` };
      const patch: ScorePatch = { ...restorePatch(prev.score, score, `take ${prev.n}`), description: ask };
      const changed = changedFromPatch(prev.score, score, patch);
      const ranges = Object.values(changed);
      const lo = ranges.length ? Math.min(...ranges.map((r) => r[0])) : 1;
      const hi = ranges.length ? Math.max(...ranges.map((r) => r[1])) : score.bars;
      const title = lo > 1 || hi < score.bars ? `Hand edit, ${formatRange({ start: lo, end: hi })}` : 'Hand edit';
      const say = `Changed by hand: ${ask.replace(/^By hand: /, '').replace(/\.$/, '')}.`;
      return this.renderTake(score, patch, words(title, say), { before: prev.score, prevN: prev.n });
    });
  }

  /** The take whose score the live score equals: the current one, else the newest match. */
  private matchingTake(): number | null {
    const s = this.score;
    if (!s) return null;
    const cur = this.takeList.find((t) => t.n === this.currentN);
    if (cur && deepEqual(cur.score, s)) return cur.n;
    return [...this.takeList].reverse().find((t) => deepEqual(t.score, s))?.n ?? null;
  }

  state(): TakeState {
    return {
      takes: [...this.takeList],
      current: this.currentN,
      score: this.score ? clone(this.score) : null,
      canUndo: this.undoStack.length > 0,
      canRedo: this.redoStack.length > 0,
      draft: this.draft && this.takeList.length === 0,
    };
  }

  async restore(n: number): Promise<TakeResult> {
    return this.exclusive(async (): Promise<TakeResult> => {
      const take = this.takeList.find((t) => t.n === n);
      if (!take || !this.score) return { ok: false, message: `There is no take ${n}.` };
      const patch = { ...restorePatch(this.score, take.score, `take ${n}`), description: `Go back to take ${n}` };
      const r = await this.deps.core.applyPatch(this.score, patch);
      if (!r.ok) {
        const error = (r.error as PatchError) ?? { kind: 'invalid', message: r.message };
        return { ok: false, message: `Could not go back to take ${n}: ${patchErrorMessage(error)}`, error };
      }
      this.undoStack.push({ kind: 'restore', patch, inverse: r.inverse as ScorePatch, before: this.currentN, after: n });
      this.redoStack = [];
      this.score = normalizeScore(r.score as Score);
      this.currentN = n;
      await this.scoreChanged();
      return { ok: true, take };
    });
  }

  async undo(): Promise<boolean> {
    return this.exclusive(async () => {
      const e = this.undoStack.at(-1);
      if (!e || !this.score) return false;
      const r = await this.deps.core.applyPatch(this.score, e.inverse);
      if (!r.ok) return false;
      this.undoStack.pop();
      this.redoStack.push(e);
      this.score = normalizeScore(r.score as Score);
      this.currentN = e.kind === 'edit' ? (this.matchingTake() ?? this.currentN) : e.before;
      await this.scoreChanged();
      return true;
    });
  }

  async redo(): Promise<boolean> {
    return this.exclusive(async () => {
      const e = this.redoStack.at(-1);
      if (!e || !this.score) return false;
      const r = await this.deps.core.applyPatch(this.score, e.patch);
      if (!r.ok) return false;
      this.redoStack.pop();
      this.undoStack.push({ ...e, inverse: r.inverse as ScorePatch });
      this.score = normalizeScore(r.score as Score);
      this.currentN = e.kind === 'edit' ? (this.matchingTake() ?? this.currentN) : e.after;
      await this.scoreChanged();
      return true;
    });
  }

  async suggestions(): Promise<string[]> {
    const s = this.score;
    if (!s || this.takeList.length === 0) {
      return ['A calm 30-second felt piano loop in A minor', 'A warm logo sting in D major, 4 seconds', 'A tense 20-second loop in E minor'];
    }
    const out: string[] = [];
    if (s.layers.some((l) => !isUnpitchedClass(l.class))) out.push('Make it a little darker');
    if (s.bars >= 8) out.push('Make the last four bars softer');
    if (s.layers.some((l) => isUnpitchedClass(l.class))) out.push('Make the kick quieter');
    if (!s.layers.some((l) => l.id === 'melody') && s.bars >= 9) out.push('Add a quiet melody from bar 9');
    return out.slice(0, 4);
  }

  async agents(): Promise<AgentInfo[]> {
    return [];
  }

  onTake(cb: (take: Take) => void): Unsubscribe {
    this.takeSubs.add(cb);
    return () => this.takeSubs.delete(cb);
  }

  onState(cb: (state: TakeState) => void): Unsubscribe {
    this.stateSubs.add(cb);
    cb(this.state());
    return () => this.stateSubs.delete(cb);
  }

  onOpenView(cb: (e: OpenViewEvent) => void): Unsubscribe {
    this.viewSubs.add(cb);
    return () => this.viewSubs.delete(cb);
  }

  openView(view: ViewName, by = 'Praxis'): void {
    this.viewSubs.forEach((cb) => cb({ view, by }));
  }

  // ------------------------------------------------------------ take internals

  private exclusive<T>(fn: () => Promise<T>): Promise<T> {
    const run = this.chain.then(fn, fn);
    this.chain = run.catch(() => undefined);
    return run;
  }

  private resetSession(): void {
    this.takeList = [];
    this.currentN = 0;
    this.takeAudio.clear();
    this.undoStack = [];
    this.redoStack = [];
    this.lastGainDb = null;
  }

  /** Loads the packs `score` needs; says so in plain words when one is missing. True when it showed a step. */
  private async ensurePacks(score: NScore, progress?: (p: AskProgress) => void, step = 0): Promise<boolean> {
    let announced = false;
    const states = await this.deps.packs.ensure(score, (pack, files) => {
      if (announced || !progress) return;
      announced = true;
      progress({ kind: 'step', n: step, words: `Loading the ${pack} samples (${files} files)` });
    });
    const missing = states.filter((s) => !s.ready);
    this.packNotice = missing.length ? missing.map((s) => s.message).join(' ') : null;
    if (announced) progress?.({ kind: 'step_done', n: step, ok: !missing.length, words: this.packNotice ?? 'Samples loaded' });
    return announced;
  }

  /** Previous take's channels: in memory, else decoded from the kept WAV. */
  private async audioOf(n: number): Promise<Float32Array[] | null> {
    const have = this.takeAudio.get(n);
    if (have) return have;
    if (!this.soundId) return null;
    const wav = await this.deps.store.loadAudio(this.soundId, n);
    if (!wav) return null;
    const d = decodeWav(wav);
    this.takeAudio.set(n, d.channels);
    return d.channels;
  }

  /** Renders and checks `score` as the next take, makes it current, keeps it and announces it. */
  private async renderTake(
    score: NScore,
    patch: ScorePatch | null,
    words: Words,
    at: { before?: NScore; prevN?: number; progress?: (p: AskProgress) => void; step?: number } = {},
  ): Promise<TakeResult> {
    const progress = at.progress;
    let step = at.step ?? 1;
    if (await this.ensurePacks(score, progress, step)) step++;
    const secs = Math.round(scoreSeconds(score));
    progress?.({ kind: 'step', n: step, words: `Rendering ${score.bars} bars (${secs} s)` });
    const r = await this.deps.core.render(score, { sample_rate: TAKE_RATE });
    if (!r.ok) {
      progress?.({ kind: 'step_done', n: step, ok: false, words: r.message });
      return { ok: false, message: `Could not render the take: ${r.message}` };
    }
    const rendered = r as { ok: true } & RenderPayload;
    progress?.({ kind: 'step_done', n: step, ok: true, words: `Rendered in ${(rendered.ms / 1000).toFixed(1)} s` });
    const report = rendered.report as unknown as RenderReport;
    const checks = rendered.checks as unknown as VerifyReport;
    progress?.({ kind: 'step', n: step + 1, words: 'Checking tempo, key, grid, loudness, peaks and the loop seam' });
    progress?.({ kind: 'step_done', n: step + 1, ok: checks.passed, words: String((rendered.checks as { summary?: string }).summary ?? '') });
    this.lastGainDb = report.applied_gain_db;
    // A new sound's first render fixes the master gain for every later take.
    let final = score;
    if (!this.takeList.length && score.master.frozen_gain_db == null) {
      const f = await this.deps.core.applyPatch(score, freezePatch(score, report.applied_gain_db));
      if (f.ok) final = normalizeScore(f.score as Score);
    }
    const n = this.takeList.length + 1;
    const channels = [rendered.left, rendered.right];
    const changed = patch && at.before ? changedFromPatch(at.before, final, patch) : {};
    let untouched = '';
    if (patch && at.before && at.prevN) {
      const prevAudio = await this.audioOf(at.prevN);
      const same = prevAudio ? identicalOutside(prevAudio, channels, TAKE_RATE, final, changed) : false;
      untouched = untouchedText(at.before, final, changed, at.prevN, same);
    }
    const wav = encodeWav(channels, TAKE_RATE, 'wav32f');
    const take: Take = {
      n,
      who: words.who,
      ask: words.ask,
      title: words.title,
      say: this.packNotice ? `${words.say} ${this.packNotice}` : words.say,
      score: clone(final),
      patch: patch ? clone(patch) : null,
      changed,
      untouched,
      render: { audioUrl: this.makeUrl(wav), report },
      checks,
      opens: words.opens,
      scope: words.scope,
    };
    const isNew = !this.soundId;
    if (isNew) this.soundId = `s-${Date.now().toString(36)}-${Math.floor(Math.random() * 1e6).toString(36)}`;
    this.score = final;
    this.takeAudio.set(n, channels);
    this.takeList.push(take);
    this.currentN = n;
    try {
      await this.deps.store.saveAudio(this.soundId!, n, wav);
    } catch (e) {
      console.warn(`Could not keep take ${n}'s audio in this browser: ${message(e)}`);
    }
    await this.persist(isNew);
    this.takeSubs.forEach((cb) => cb(take));
    await this.scoreChanged(false);
    if (take.opens) this.openView(take.opens, take.who === 'You' ? 'Praxis' : take.who.replace('Agent · ', ''));
    return { ok: true, take };
  }

  /** After any change of the live score: keep it, announce it, and play the new version. */
  private async scoreChanged(save = true): Promise<void> {
    if (save) await this.persist();
    this.emitState();
    if (this.playing) await this.syncPlayer();
  }

  private emitState(): void {
    const st = this.state();
    this.stateSubs.forEach((cb) => cb(st));
  }

  async render(options: RenderOptions = {}): Promise<RenderResult> {
    if (!this.score) throw new Error('no score loaded');
    const r = await this.renderScore(this.score, options.sampleRate ?? TAKE_RATE, options.layers);
    const wav = encodeWav([r.left, r.right], options.sampleRate ?? TAKE_RATE, options.format ?? 'wav16');
    return { audioUrl: this.makeUrl(wav), report: r.report as unknown as RenderReport };
  }

  private async renderScore(score: NScore, sampleRate: number, layers?: string[]): Promise<RenderPayload> {
    await this.ensurePacks(score);
    const r = await this.deps.core.render(score, { sample_rate: sampleRate, layers });
    if (!r.ok) throw new Error(r.message);
    return r;
  }

  // ------------------------------------------------------------ clips

  private async readClip(file: File | { name: string }, text: string): Promise<AskResult> {
    if (!(file instanceof Blob) || typeof AudioContext === 'undefined') {
      return { ok: false, message: `Could not read "${file.name}" here: this browser cannot decode audio.` };
    }
    let audio: AudioBuffer;
    try {
      const ctx = new OfflineAudioContext(2, 1, TAKE_RATE);
      audio = await ctx.decodeAudioData(await file.arrayBuffer());
    } catch (e) {
      return { ok: false, message: `Could not decode "${file.name}": ${message(e)}.` };
    }
    const left = audio.getChannelData(0);
    const right = audio.numberOfChannels > 1 ? audio.getChannelData(1) : left;
    const a = await this.deps.core.analyse(left.slice(), right.slice(), audio.sampleRate, 4);
    if (!a.ok) return { ok: false, message: `Could not measure "${file.name}": ${a.message}.` };
    const tempo = (a.tempo as { bpm?: number } | null)?.bpm;
    const key = (a.key as { name?: string } | null)?.name;
    const lufs = (a.loudness as { integrated_lufs?: number } | null)?.integrated_lufs;
    const parts = [
      `"${file.name}" is ${audio.duration.toFixed(1)} s long`,
      tempo ? `around ${Math.round(tempo)} BPM` : "with no clear tempo",
      key ? `in ${key}` : "with no clear key",
      lufs !== undefined && Number.isFinite(lufs) ? `at ${lufs.toFixed(1)} LUFS` : '',
    ].filter(Boolean);
    const next = text ? ` Describe the sound you want with those numbers ("${text}") and I'll make it.` : ' Describe the sound you want with those numbers and I will make it.';
    return { ok: true, take: null, reply: `${parts.join(', ')}.${next}`, noTake: 'question' };
  }

  // ------------------------------------------------------------ transport

  private setPlayback(p: BrowserPlayback): void {
    this.playback = p;
    this.playbackSubs.forEach((cb) => cb(p));
  }

  /** The real-time engine, started on first use (a user gesture lets the AudioContext run). */
  private async ensurePlayer(): Promise<WorkletPlayer | null> {
    if (this.player) return this.player;
    if (!this.deps.player) {
      this.setPlayback({ mode: 'none', message: 'This browser has no Web Audio, so the sound cannot play here.', sampleRate: 0 });
      return null;
    }
    this.starting ??= this.deps
      .player()
      .then((p) => {
        this.player = p;
        this.statusUnsub = p.onStatus((s) => this.onStatus(s));
        this.setPlayback({ mode: 'worklet', message: null, sampleRate: p.sampleRate });
        return p;
      })
      .catch((e: unknown) => {
        this.setPlayback({ mode: 'none', message: `The audio engine could not start: ${message(e)}`, sampleRate: 0 });
        this.starting = null;
        return null;
      });
    return this.starting;
  }

  /** Sends the live score (and any new pack files) to the worklet when it changed. */
  private async syncPlayer(): Promise<LoadInfo | null> {
    const p = this.player;
    const score = this.score;
    if (!p || !score) return null;
    if (this.playerScore && deepEqual(this.playerScore, score)) return null;
    await this.ensurePacks(score);
    await p.addPackFiles(this.deps.packs.takeUnsent());
    const gain = score.master.frozen_gain_db ?? this.lastGainDb;
    const info = await p.load(score, gain ?? null);
    if (info.ok) {
      this.playerScore = clone(score);
      this.applyLoop();
    } else {
      this.setPlayback({ mode: 'worklet', message: `Could not play this version: ${info.message}`, sampleRate: p.sampleRate });
    }
    return info;
  }

  private applyLoop(): void {
    const p = this.player;
    const s = this.score;
    if (!p || !s) return;
    if (!this.loop) {
      p.setLoop(null);
      return;
    }
    const sr = p.sampleRate;
    p.setLoop([barPosToSeconds(s, this.loop.start) * sr, barPosToSeconds(s, this.loop.end + 1) * sr]);
  }

  private onStatus(s: PlayerStatus): void {
    const p = this.player;
    if (!p || !this.score) return;
    this.positionS = s.position / p.sampleRate;
    if (this.playing && s.ended && !s.playing) {
      void this.stop().then(() => this.seek(0));
      return;
    }
    this.emitPosition();
    if (this.playing) this.meterSubs.forEach((cb) => cb({ peakL: dbfs(s.peak[0]), peakR: dbfs(s.peak[1]), rmsL: dbfs(s.rms[0]), rmsR: dbfs(s.rms[1]) }));
  }

  async play(): Promise<void> {
    if (!this.score || this.playing) return;
    const p = await this.ensurePlayer();
    if (!p) return;
    if (typeof AudioContext !== 'undefined' && p.ctx instanceof AudioContext && p.ctx.state !== 'running') await p.ctx.resume();
    const info = await this.syncPlayer();
    if (info && !info.ok) return;
    p.seek(this.offsetS * p.sampleRate);
    p.play();
    this.playing = true;
    this.emitPosition();
  }

  async stop(): Promise<void> {
    if (!this.playing) return;
    this.playing = false;
    this.player?.stop();
    this.offsetS = this.positionS;
    this.emitPosition();
    this.meterSubs.forEach((cb) => cb(SILENT));
  }

  private async stopTransport(): Promise<void> {
    await this.stop();
    this.playerScore = null;
  }

  async seek(seconds: number): Promise<void> {
    const len = this.score ? scoreSeconds(this.score) : 0;
    this.offsetS = this.positionS = Math.max(0, Math.min(seconds, len));
    this.player?.seek(this.offsetS * this.player.sampleRate);
    this.emitPosition();
  }

  async setLoop(range: BarRange | null): Promise<void> {
    this.loop = range ? { ...range } : null;
    this.applyLoop();
    this.emitPosition();
  }

  onPosition(cb: (p: TransportPosition) => void): Unsubscribe {
    this.posSubs.add(cb);
    cb(this.position());
    return () => this.posSubs.delete(cb);
  }

  onMeters(cb: (m: Meters) => void): Unsubscribe {
    this.meterSubs.add(cb);
    cb(SILENT);
    return () => this.meterSubs.delete(cb);
  }

  private position(): TransportPosition {
    const seconds = this.playing ? this.positionS : this.offsetS;
    return { playing: this.playing, seconds, bar: this.score ? secondsToBarPos(this.score, seconds) : 1, loop: this.loop ? { ...this.loop } : null };
  }

  private emitPosition(): void {
    const p = this.position();
    this.posSubs.forEach((cb) => cb(p));
  }

  // ------------------------------------------------------------ export

  async export(request: ExportRequest): Promise<ExportResult> {
    const score = this.score;
    if (!score) return { ok: false, files: [], message: 'Make a sound first.' };
    const base = request.baseName || 'praxis-sound';
    const files: ExportResult['files'] = [];
    const skipped: string[] = [];
    try {
      let master: RenderPayload | null = null;
      for (const kind of request.kinds) {
        if (kind === 'wav16' || kind === 'wav24' || kind === 'wav32f') {
          master ??= await this.renderScore(score, TAKE_RATE);
          const name = `${base}${{ wav16: '.16bit.wav', wav24: '.24bit.wav', wav32f: '.32f.wav' }[kind]}`;
          download(name, encodeWav([master.left, master.right], TAKE_RATE, kind), 'audio/wav');
          files.push({ kind, path: name });
        } else if (kind === 'stems') {
          for (const l of score.layers) {
            const s = await this.renderScore(score, TAKE_RATE, [l.id]);
            const name = `${base}.stem-${l.id}.wav`;
            download(name, encodeWav([s.left, s.right], TAKE_RATE, 'wav24'), 'audio/wav');
            files.push({ kind, path: name });
          }
        } else if (kind === 'project') {
          const name = `${base}.score.json`;
          download(name, new TextEncoder().encode(JSON.stringify(score, null, 2)).buffer as ArrayBuffer, 'application/json');
          files.push({ kind, path: name });
        } else {
          skipped.push(kind === 'flac' ? 'FLAC' : 'MIDI');
        }
      }
    } catch (e) {
      return { ok: false, files, message: `Export stopped: ${message(e)}` };
    }
    const saved = files.length ? `Saved ${files.length} ${files.length === 1 ? 'file' : 'files'} to your downloads.` : '';
    const missing = skipped.length ? ` ${skipped.join(' and ')} export is in the desktop app only.` : '';
    return { ok: files.length > 0, files, message: (saved + missing).trim() };
  }

  dispose(): void {
    void this.stop();
    this.statusUnsub?.();
    this.player?.dispose();
    this.player = null;
    if (typeof URL.revokeObjectURL === 'function') this.urls.forEach((u) => URL.revokeObjectURL(u));
    this.urls = [];
  }

  // ------------------------------------------------------------ internals

  private makeUrl(wav: ArrayBuffer): string {
    if (typeof Blob !== 'undefined' && typeof URL.createObjectURL === 'function') {
      const url = URL.createObjectURL(new Blob([wav], { type: 'audio/wav' }));
      this.urls.push(url);
      return url;
    }
    return '';
  }

  /** Test hook: the channels of take n (null when not rendered in this session). */
  takeChannels(n: number): Float32Array[] | null {
    return this.takeAudio.get(n) ?? null;
  }

  /** Test hook: the master gain the player uses for the live score. */
  playerGainDb(): number | null {
    return this.score?.master.frozen_gain_db ?? this.lastGainDb;
  }
}

/** "Bars 13–16 · Felt piano". */
function scopeLabel(score: NScore, scope: TakeScope): string {
  const { start, end } = scope.bars;
  return `${start === end ? `Bar ${start}` : `Bars ${start}–${end}`} · ${layerLabel(score, scope.layer)}`;
}

function download(name: string, data: ArrayBuffer, type: string): void {
  if (typeof document === 'undefined') return;
  const url = URL.createObjectURL(new Blob([data], { type }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.style.display = 'none';
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}
