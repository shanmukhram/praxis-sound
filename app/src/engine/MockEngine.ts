// MockEngine: the EngineAdapter used in browser dev mode and tests until the Tauri (desktop) and
// wasm (browser) adapters exist. Patch rules follow docs/contracts/patch.md (see mock/apply.ts);
// requests go through a keyword mock planner (mock/planner.ts); audio is a clearly labelled
// placeholder preview (mock/synth.ts) and checks are partly measured on it (mock/verify.ts).

import type {
  AgentInfo,
  ApplyResult,
  AskRequest,
  TakeResult,
  BlankSound,
  EngineAdapter,
  MakeTakeOptions,
  PresetInfo,
  StartResult,
  OpenViewEvent,
  Take,
  TakeScope,
  TakeState,
  ViewName,
  Who,
  ExportRequest,
  ExportResult,
  LoadResult,
  Meters,
  RenderOptions,
  RenderResult,
  TransportPosition,
  Unsubscribe,
} from './EngineAdapter';
import type { BarRange, NScore, Origin, Score, ScorePatch } from '../score/types';
import { clone, deepEqual, formatRange, normalizeScore } from '../score/types';
import { restorePatch } from '../score/patchBuilder';
import { isUnpitchedClass, keyLabel } from '../score/theory';
import { barPosToSeconds, expandNotes, scoreSeconds, secondsToBarPos } from '../score/timeline';
import { applyPatch } from './mock/apply';
import { planEdit, planFresh } from './mock/planner';
import { synthesize } from './mock/synth';
import { validate } from './mock/validate';
import { verifyMock } from './mock/verify';
import { changedFromPatch, layerLabel, untouchedText } from './takes';
import { patchErrorMessage } from './EngineAdapter';
import { encodeWav } from './wav';
import { blankScore, DRAFT_ASK } from './blank';
import { STANDARD_PRESETS } from './presets';
import type { AnalyzeResult, ClipFile, CompareLevel, LibraryEntry, LibraryResult, ReferenceClip, SoundProfile } from './EngineAdapter';
import { clipErrorWords, clipProblem, isVideoName, profileSummary, withReference } from './reference';
import { decodeClip, mockProfile } from './mock/profile';

/** Library tags: trimmed, lower case, no repeats. */
export function cleanTags(tags: string[]): string[] {
  return [...new Set(tags.map((t) => t.trim().toLowerCase()).filter(Boolean))];
}

/** The gain an A/B trim sets on the mock's output. */
function compareGain(c: CompareLevel | null): number {
  if (!c) return 1;
  return c.muted ? 0 : 10 ** (Math.min(0, c.gainDb) / 20);
}

/** One undoable step of the session: a take, a go-back, or a plain edit (mute, solo, lock). */
interface Entry {
  kind: 'take' | 'restore' | 'edit';
  patch: ScorePatch;
  inverse: ScorePatch;
  /** Current take before and after the step. */
  before: number;
  after: number;
}

/** Sample rate of take renders (kept modest: every take is rendered and checked). */
const TAKE_RATE = 22050;

export const MOCK_AGENTS: AgentInfo[] = [
  { name: 'cut-assistant', connected: true, about: 'fits sound to the edit' },
  { name: 'trailer-bot', connected: true, about: 'drafts trailer cues' },
];

const SILENT: Meters = { peakL: -Infinity, peakR: -Infinity, rmsL: -Infinity, rmsR: -Infinity };

export class MockEngine implements EngineAdapter {
  readonly kind = 'mock' as const;
  readonly label = 'Mock engine · placeholder preview';

  /** Every export request received, newest last (the mock writes nothing). */
  readonly exports: ExportRequest[] = [];

  private score: NScore | null = null;
  /** The live score is a blank sound started by hand (no take until `makeTake`). */
  private draft = false;
  private takeList: Take[] = [];
  private currentN = 0;
  private frozenGainDb: number | null = null;
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
  private playing = false;
  private offsetS = 0;
  private startedAt = 0;
  private loop: BarRange | null = null;
  private timer: ReturnType<typeof setInterval> | null = null;

  /** The reference library (profiles only), newest first. */
  private lib: LibraryEntry[] = [];
  private libSeq = 0;
  /** The A/B trim on the take while comparing. */
  private compare: CompareLevel | null = null;
  /** The last request's words as the planner got them (tests). */
  lastRequest = '';

  private ctx: AudioContext | null = null;
  private gainNode: GainNode | null = null;
  private source: AudioBufferSourceNode | null = null;
  private analysers: [AnalyserNode, AnalyserNode] | null = null;
  private ctxStartTime = 0;

  /** Loads a score as the live score of a new, empty session (no takes yet). */
  async load(input: Score): Promise<LoadResult> {
    await this.stop();
    const score = normalizeScore(input);
    this.resetSession();
    this.score = score;
    this.draft = false;
    this.offsetS = 0;
    this.loop = null;
    const report = validate(score);
    this.emitPosition();
    this.emitState();
    return { score: clone(score), errors: report.errors, warnings: report.warnings };
  }

  /** Current live score (a copy), or null before load. */
  current(): NScore | null {
    return this.score ? clone(this.score) : null;
  }

  async applyPatch(patch: ScorePatch): Promise<ApplyResult> {
    return this.exclusive(async () => {
      if (!this.score) return { ok: false, error: { kind: 'invalid', message: 'no score loaded' } } as ApplyResult;
      const r = applyPatch(this.score, patch);
      if (!r.ok) return r;
      this.score = r.score;
      this.undoStack.push({ kind: 'edit', patch: clone(patch), inverse: r.inverse, before: this.currentN, after: this.currentN });
      this.redoStack = [];
      if (this.playing) this.restartAudio(this.positionSeconds());
      this.emitState();
      return { ok: true, score: clone(r.score), inverse: r.inverse } as ApplyResult;
    });
  }

  // ------------------------------------------------------------ takes

  /** Starts a sound from a score: take 1 (a new sound). Mock and test entry point. */
  async startSound(input: Score, meta: { ask: string; title: string; say: string; who?: Who }): Promise<Take> {
    return this.exclusive(async () => {
      await this.stop();
      this.resetSession();
      this.score = normalizeScore(input);
      this.draft = false;
      return this.renderTake(this.score, null, { who: meta.who ?? 'You', ask: meta.ask, title: meta.title, say: meta.say });
    });
  }

  /**
   * The acceptance-1 demo conversation: take 1 the piano alone, take 2 adds the kick from bar 9,
   * take 3 an agent's lift in bars 13-16. The piano is unlocked so pointed edits on it work.
   */
  async seedDemo(acceptance1: Score): Promise<void> {
    const full = normalizeScore(acceptance1);
    const kick = full.layers.find((l) => l.id === 'kick');
    const kickPattern = full.patterns.find((p) => p.id === 'kick_soft');
    const pianoOnly: NScore = {
      ...clone(full),
      layers: full.layers.filter((l) => l.id !== 'kick').map((l) => ({ ...l, locked: false })),
      patterns: full.patterns.filter((p) => p.id !== 'kick_soft'),
    };
    await this.startSound(pianoOnly, {
      ask: 'A calm 30-second felt piano loop in A minor. Rhythmic, notes ringing into each other. No risers or pads.',
      title: 'Felt piano loop',
      say: 'One felt piano playing a two-bar motif on an eighth-note grid, strong on the first beat and bright on beat 3, each note ringing into the next. 16 bars at 128 BPM, so it loops at exactly 30 seconds.',
    });
    if (kick && kickPattern) {
      await this.edit(
        {
          description: 'Add a soft kick from bar 9',
          origin: 'model',
          scope: { layers: ['kick'] },
          ops: [
            { op: 'add_pattern', pattern: kickPattern },
            { op: 'add_layer', layer: kick },
          ],
        },
        'Add a soft kick from bar 9.',
        undefined,
        { title: 'Felt piano + soft kick', say: 'Kept the piano exactly as it was and added a soft, muted kick on beats 1 and 3 under bars 9 to 16. No new melody, no fills.' },
      );
    }
    await this.agentAsk('cut-assistant', 'Give the last four bars a gentle lift so the loop can end on the cut at 0:30.', { layer: 'piano', bars: { start: 13, end: 16 } }, {
      title: 'Lift in bars 13–16',
      askText: 'Give the last four bars a gentle lift so the loop can end on the cut at 0:30.',
    });
  }

  async ask(request: AskRequest): Promise<TakeResult> {
    return this.askAs('You', request);
  }

  /** Mock only: a connected agent makes a request (stands in for the MCP server, WP-X). */
  async agentAsk(name: string, text: string, scope?: TakeScope, opts: { title?: string; askText?: string } = {}): Promise<TakeResult> {
    return this.askAs(`Agent · ${name}`, { text, scope }, opts);
  }

  private askAs(who: Who, request: AskRequest, opts: { title?: string; askText?: string } = {}): Promise<TakeResult> {
    return this.exclusive(async () => {
      const typed = request.text.trim();
      const askText = opts.askText ?? (request.scope ? `${typed} (${scopeLabel(this.score, request.scope)})` : typed);
      // The planner reads the reference's profile in words, as the real planner does.
      const text = withReference(typed, request.reference);
      const reference = request.reference?.name;
      this.lastRequest = text;
      if (this.draft && this.takeList.length === 0) {
        return { ok: false, message: DRAFT_ASK } as TakeResult;
      }
      if (!this.score || this.takeList.length === 0) {
        if (!text) return { ok: false, message: 'Describe a sound to start from.' } as TakeResult;
        const fresh = planFresh(text);
        this.resetSession();
        this.score = fresh.score;
        this.draft = false;
        const take = await this.renderTake(fresh.score, null, { who, ask: askText, title: fresh.title, say: fresh.say, reference });
        return { ok: true, take } as TakeResult;
      }
      const origin: Origin = 'model';
      const plan = planEdit(this.score, typed, request.scope, origin);
      if (!plan.ok) return { ok: false, message: reference ? `${plan.message} (The mock planner cannot match a reference; the desktop app's assistant can.)` : plan.message } as TakeResult;
      return this.commitTake(plan.patch, { who, ask: askText, title: opts.title ?? plan.title, say: plan.say, opens: plan.opens, scope: request.scope, reference });
    });
  }

  async edit(patch: ScorePatch, ask: string, scope?: TakeScope, words?: { title?: string; say?: string }): Promise<TakeResult> {
    return this.exclusive(async () => {
      if (!this.score || this.takeList.length === 0) return { ok: false, message: 'Make a sound first.' } as TakeResult;
      const title = words?.title ?? (scope ? `Hand edit, ${formatRange(scope.bars)}` : 'Hand edit');
      return this.commitTake(patch, {
        who: 'You',
        ask,
        title,
        say: words?.say ?? `Changed by hand: ${patch.description || 'an edit'}.`,
        scope,
      });
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
      return { ok: false, message: e instanceof Error ? e.message : String(e) };
    }
    await this.exclusive(async () => {
      await this.stop();
      this.resetSession();
      this.score = score;
      this.draft = true;
      this.offsetS = 0;
      this.loop = null;
      this.emitPosition();
      this.emitState();
    });
    return { ok: true };
  }

  async presets(): Promise<PresetInfo[]> {
    return clone(STANDARD_PRESETS);
  }

  async makeTake(ask: string, options: MakeTakeOptions = {}): Promise<TakeResult> {
    return this.exclusive(async () => {
      const score = this.score;
      if (!score) return { ok: false, message: 'Start a sound first.' } as TakeResult;
      if (!score.layers.length) return { ok: false, message: 'Add a layer first: a sound with no layers has nothing to play.' } as TakeResult;
      const words = (title: string, say: string) => ({ who: 'You' as Who, ask, title: options.title ?? title, say: options.say ?? say, scope: options.scope });
      if (!this.takeList.length) {
        const names = score.layers.map((l) => (l.name || l.id).toLowerCase()).join(', ');
        const take = await this.renderTake(score, null, words('Built by hand', `Built by hand: ${names}, ${score.bars} bars at ${score.tempo.bpm} BPM in ${keyLabel(score.key)}.`));
        return { ok: true, take } as TakeResult;
      }
      const prev = this.takeList.find((t) => t.n === this.currentN) ?? this.takeList.at(-1)!;
      if (deepEqual(prev.score, score)) return { ok: false, message: `Nothing changed since take ${prev.n}: edit the sound, then make a take.` } as TakeResult;
      // The take's patch is the exact diff from the current take (crates/praxis-app/src/hand.rs).
      const patch: ScorePatch = { ...restorePatch(prev.score, score, `take ${prev.n}`), description: ask };
      const changed = changedFromPatch(prev.score, score, patch);
      const ranges = Object.values(changed);
      const lo = ranges.length ? Math.min(...ranges.map((r) => r[0])) : 1;
      const hi = ranges.length ? Math.max(...ranges.map((r) => r[1])) : score.bars;
      const title = lo > 1 || hi < score.bars ? `Hand edit, ${formatRange({ start: lo, end: hi })}` : 'Hand edit';
      const say = `Changed by hand: ${ask.replace(/^By hand: /, '').replace(/\.$/, '')}.`;
      const take = await this.renderTake(score, patch, words(title, say), prev.score, prev.n, true);
      return { ok: true, take } as TakeResult;
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
    return this.exclusive(async () => {
      const take = this.takeList.find((t) => t.n === n);
      if (!take || !this.score) return { ok: false, message: `There is no take ${n}.` } as TakeResult;
      const patch = { ...restorePatch(this.score, take.score, `take ${n}`), description: `Go back to take ${n}` };
      const r = applyPatch(this.score, patch);
      if (!r.ok) return { ok: false, message: `Could not go back to take ${n}: ${patchErrorMessage(r.error)}`, error: r.error } as TakeResult;
      this.undoStack.push({ kind: 'restore', patch, inverse: r.inverse, before: this.currentN, after: n });
      this.redoStack = [];
      this.score = r.score;
      this.currentN = n;
      this.emitState();
      return { ok: true, take } as TakeResult;
    });
  }

  async undo(): Promise<boolean> {
    return this.exclusive(async () => {
      const e = this.undoStack.at(-1);
      if (!e || !this.score) return false;
      const r = applyPatch(this.score, e.inverse);
      if (!r.ok) return false;
      this.undoStack.pop();
      this.redoStack.push(e);
      this.score = r.score;
      // A plain edit (mute, a hand edit) moves the current take only onto a take it now equals.
      this.currentN = e.kind === 'edit' ? (this.matchingTake() ?? this.currentN) : e.before;
      if (this.playing) this.restartAudio(this.positionSeconds());
      this.emitState();
      return true;
    });
  }

  async redo(): Promise<boolean> {
    return this.exclusive(async () => {
      const e = this.redoStack.at(-1);
      if (!e || !this.score) return false;
      const r = applyPatch(this.score, e.patch);
      if (!r.ok) return false;
      this.redoStack.pop();
      this.undoStack.push({ ...e, inverse: r.inverse });
      this.score = r.score;
      this.currentN = e.kind === 'edit' ? (this.matchingTake() ?? this.currentN) : e.after;
      if (this.playing) this.restartAudio(this.positionSeconds());
      this.emitState();
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
    if (s.bars >= 8) out.push('Change the last four bars');
    if (s.layers.some((l) => isUnpitchedClass(l.class))) out.push('Make the kick quieter');
    if (!s.layers.some((l) => l.id === 'melody') && s.bars >= 9) out.push('Add a quiet glass melody from bar 9');
    return out.slice(0, 4);
  }

  async agents(): Promise<AgentInfo[]> {
    return MOCK_AGENTS.map((a) => ({ ...a }));
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
    this.frozenGainDb = null;
    this.takeAudio.clear();
    this.undoStack = [];
    this.redoStack = [];
  }

  private async commitTake(
    patch: ScorePatch,
    words: { who: Who; ask: string; title: string; say: string; opens?: ViewName; scope?: TakeScope; reference?: string },
  ): Promise<TakeResult> {
    const before = this.score!;
    const r = applyPatch(before, patch);
    if (!r.ok) return { ok: false, message: `Could not make that take: ${patchErrorMessage(r.error)}.`, error: r.error };
    const prevN = this.currentN;
    this.undoStack.push({ kind: 'take', patch: clone(patch), inverse: r.inverse, before: prevN, after: this.takeList.length + 1 });
    this.redoStack = [];
    this.score = r.score;
    const take = await this.renderTake(r.score, patch, words, before, prevN);
    return { ok: true, take };
  }

  /** Renders and checks `score` as the next take, makes it current and announces it. */
  private async renderTake(
    score: NScore,
    patch: ScorePatch | null,
    words: { who: Who; ask: string; title: string; say: string; opens?: ViewName; scope?: TakeScope; reference?: string },
    before?: NScore,
    prevN?: number,
    holdGain = false,
  ): Promise<Take> {
    // Bar-scoped edits (and hand takes, as after `master.frozen_gain_db`) hold the master gain so
    // untouched bars stay sample-identical; a new sound or a layer-wide request re-normalises.
    const hold = (holdGain || !!patch?.scope?.bars) && this.frozenGainDb !== null;
    const out = synthesize(score, { sampleRate: TAKE_RATE, gainDb: hold ? this.frozenGainDb! : undefined });
    if (!hold) this.frozenGainDb = out.report.applied_gain_db;
    const n = this.takeList.length + 1;
    const changed = patch && before ? changedFromPatch(before, score, patch) : {};
    let untouched = '';
    if (patch && before && prevN) {
      const prevAudio = this.takeAudio.get(prevN);
      const same = prevAudio ? identicalOutside(prevAudio, out.channels, out.sampleRate, score, changed) : false;
      untouched = untouchedText(before, score, changed, prevN, same);
    }
    const wav = encodeWav(out.channels, out.sampleRate, 'wav16');
    const take: Take = {
      n,
      who: words.who,
      ask: words.ask,
      title: words.title,
      say: words.say,
      score: clone(score),
      patch: patch ? clone(patch) : null,
      changed,
      untouched,
      render: { audioUrl: this.makeUrl(wav), report: out.report },
      checks: verifyMock(score, out.channels, out.sampleRate, out.report),
      opens: words.opens,
      scope: words.scope,
    };
    if (words.reference) take.reference = words.reference;
    this.takeAudio.set(n, out.channels);
    this.takeList.push(take);
    this.currentN = n;
    this.takeSubs.forEach((cb) => cb(take));
    this.emitState();
    if (take.opens) this.openView(take.opens, take.who === 'You' ? 'Praxis' : take.who.replace('Agent · ', ''));
    return take;
  }

  private emitState(): void {
    const st = this.state();
    this.stateSubs.forEach((cb) => cb(st));
  }

  async render(options: RenderOptions = {}): Promise<RenderResult> {
    if (!this.score) throw new Error('no score loaded');
    const out = synthesize(this.score, { sampleRate: options.sampleRate ?? 44100, layers: options.layers, gainDb: this.frozenGainDb ?? undefined });
    const wav = encodeWav(out.channels, out.sampleRate, options.format ?? 'wav16');
    return { audioUrl: this.makeUrl(wav), report: out.report };
  }

  // ------------------------------------------------------------ transport

  async play(): Promise<void> {
    if (!this.score || this.playing) return;
    this.playing = true;
    this.startedAt = now();
    this.restartAudio(this.offsetS);
    this.timer = setInterval(() => this.tick(), 33);
    this.emitPosition();
  }

  async stop(): Promise<void> {
    if (!this.playing) return;
    this.offsetS = this.positionSeconds();
    this.playing = false;
    this.stopAudio();
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    this.emitPosition();
    this.meterSubs.forEach((cb) => cb(SILENT));
  }

  async seek(seconds: number): Promise<void> {
    const len = this.score ? scoreSeconds(this.score) : 0;
    this.offsetS = Math.max(0, Math.min(seconds, len));
    this.startedAt = now();
    if (this.playing) this.restartAudio(this.offsetS);
    this.emitPosition();
  }

  async setLoop(range: BarRange | null): Promise<void> {
    this.loop = range ? { ...range } : null;
    if (this.playing) {
      const pos = this.positionSeconds();
      this.offsetS = pos;
      this.startedAt = now();
      this.restartAudio(pos);
    }
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

  async export(request: ExportRequest): Promise<ExportResult> {
    this.exports.push(clone(request));
    const base = `${request.destination.replace(/[\\/]+$/, '')}/${request.baseName}`;
    const files: ExportResult['files'] = [];
    for (const kind of request.kinds) {
      if (kind === 'stems') {
        for (const l of this.score?.layers ?? []) files.push({ kind, path: `${base}.stem-${l.id}.wav` });
      } else {
        const ext = { wav16: '.16bit.wav', wav24: '.24bit.wav', wav32f: '.32f.wav', flac: '.flac', midi: '.mid', project: '.praxis' }[kind];
        files.push({ kind, path: `${base}${ext}` });
      }
    }
    return { ok: true, files, message: 'Mock engine: request recorded; no files were written.' };
  }

  // ------------------------------------------------------------ reference clips

  async analyzeClip(file: ClipFile): Promise<AnalyzeResult> {
    const problem = clipProblem(file);
    if (problem) return { ok: false, message: problem };
    let bytes: ArrayBuffer;
    try {
      bytes = await file.arrayBuffer();
    } catch (e) {
      return { ok: false, message: `Could not read ${file.name}: ${e instanceof Error ? e.message : String(e)}.` };
    }
    const audio = await decodeClip(bytes);
    if (!audio && isVideoName(file.name)) {
      return { ok: false, message: clipErrorWords(file.name, 'ffmpeg_missing', 'video needs ffmpeg') };
    }
    const profile = mockProfile(file.name, audio, file.size);
    let audioUrl: string | null = null;
    if (typeof Blob !== 'undefined' && file instanceof Blob && typeof URL.createObjectURL === 'function') {
      audioUrl = URL.createObjectURL(file);
      this.urls.push(audioUrl);
    }
    return { ok: true, clip: { name: file.name, profile, audioUrl, path: `mock://clips/${file.name}`, libraryId: null } };
  }

  async library(): Promise<LibraryEntry[]> {
    return this.lib.map((e) => clone(e));
  }

  async saveReference(ref: ReferenceClip, name: string, tags: string[]): Promise<LibraryResult> {
    const n = name.trim();
    if (!n) return { ok: false, message: 'Give the reference a name.' };
    if (this.lib.some((e) => e.name.toLowerCase() === n.toLowerCase())) return { ok: false, message: `There is already a reference called "${n}".` };
    // Profiles only: the entry keeps the measurements, never the clip's audio or its file.
    const profile: SoundProfile = { ...clone(ref.profile), tags: cleanTags(tags), style_name: n };
    const entry: LibraryEntry = { id: `ref-${++this.libSeq}`, name: n, tags: cleanTags(tags), kind: profile.class.kind, summary: profileSummary(profile), profile };
    this.lib.unshift(entry);
    return { ok: true, entry: clone(entry) };
  }

  async tagReference(id: string, tags: string[]): Promise<LibraryResult> {
    const e = this.lib.find((x) => x.id === id);
    if (!e) return { ok: false, message: 'That reference is no longer in the library.' };
    e.tags = cleanTags(tags);
    e.profile.tags = [...e.tags];
    return { ok: true, entry: clone(e) };
  }

  async removeReference(id: string): Promise<boolean> {
    const before = this.lib.length;
    this.lib = this.lib.filter((e) => e.id !== id);
    return this.lib.length < before;
  }

  async setCompare(level: CompareLevel | null): Promise<void> {
    this.compare = level ? { ...level } : null;
    if (this.gainNode) this.gainNode.gain.value = compareGain(this.compare);
  }

  /** The A/B trim the transport plays with now (tests). */
  compareLevel(): CompareLevel | null {
    return this.compare ? { ...this.compare } : null;
  }

  dispose(): void {
    void this.stop();
    if (typeof URL.revokeObjectURL === 'function') this.urls.forEach((u) => URL.revokeObjectURL(u));
    this.urls = [];
    void this.ctx?.close();
    this.ctx = null;
  }

  // ------------------------------------------------------------ internals

  private loopSeconds(): [number, number] | null {
    if (!this.score) return null;
    if (this.loop) return [barPosToSeconds(this.score, this.loop.start), barPosToSeconds(this.score, this.loop.end + 1)];
    if (this.score.playback.mode === 'loop') return [0, scoreSeconds(this.score)];
    return null;
  }

  private positionSeconds(): number {
    if (!this.score) return 0;
    if (!this.playing) return this.offsetS;
    const elapsed = this.ctx && this.source ? this.ctx.currentTime - this.ctxStartTime : (now() - this.startedAt) / 1000;
    let t = this.offsetS + elapsed;
    const lp = this.loopSeconds();
    // Playback starts at the offset, runs to the loop end, then cycles [a, b) (as WebAudio does).
    if (lp && this.offsetS < lp[1] && t >= lp[1]) {
      const [a, b] = lp;
      t = a + ((t - b) % (b - a));
    }
    return t;
  }

  private position(): TransportPosition {
    const seconds = this.positionSeconds();
    return {
      playing: this.playing,
      seconds,
      bar: this.score ? secondsToBarPos(this.score, seconds) : 1,
      loop: this.loop ? { ...this.loop } : null,
    };
  }

  private emitPosition(): void {
    const p = this.position();
    this.posSubs.forEach((cb) => cb(p));
  }

  private tick(): void {
    if (!this.score) return;
    if (!this.loopSeconds() && this.positionSeconds() >= scoreSeconds(this.score)) {
      void this.stop().then(() => this.seek(0));
      return;
    }
    this.emitPosition();
    this.meterSubs.forEach((cb) => cb(this.readMeters()));
  }

  private readMeters(): Meters {
    if (!this.analysers) return SILENT;
    const buf = new Float32Array(this.analysers[0].fftSize);
    const read = (a: AnalyserNode) => {
      a.getFloatTimeDomainData(buf);
      let peak = 0;
      let sum = 0;
      for (const x of buf) {
        peak = Math.max(peak, Math.abs(x));
        sum += x * x;
      }
      const db = (v: number) => (v > 0 ? 20 * Math.log10(v) : -Infinity);
      return [db(peak), db(Math.sqrt(sum / buf.length))];
    };
    const [pl, rl] = read(this.analysers[0]);
    const [pr, rr] = read(this.analysers[1]);
    return { peakL: pl, peakR: pr, rmsL: rl, rmsR: rr };
  }

  /** (Re)starts WebAudio playback at `at` seconds. Without WebAudio the clock still runs. */
  private restartAudio(at: number): void {
    this.stopAudio();
    this.offsetS = at;
    this.startedAt = now();
    if (!this.score) return;
    const Ctx: typeof AudioContext | undefined = (globalThis as { AudioContext?: typeof AudioContext }).AudioContext;
    if (!Ctx) return;
    try {
      this.ctx ??= new Ctx();
      const ctx = this.ctx;
      void ctx.resume();
      const out = synthesize(this.score, { sampleRate: ctx.sampleRate });
      const buffer = ctx.createBuffer(2, out.channels[0].length, out.sampleRate);
      buffer.copyToChannel(out.channels[0] as Float32Array<ArrayBuffer>, 0);
      buffer.copyToChannel(out.channels[1] as Float32Array<ArrayBuffer>, 1);
      const src = ctx.createBufferSource();
      src.buffer = buffer;
      const lp = this.loopSeconds();
      if (lp) {
        src.loop = true;
        src.loopStart = lp[0];
        src.loopEnd = lp[1];
      }
      const splitter = ctx.createChannelSplitter(2);
      const a = ctx.createAnalyser();
      const b = ctx.createAnalyser();
      a.fftSize = b.fftSize = 1024;
      const gain = ctx.createGain();
      gain.gain.value = compareGain(this.compare);
      gain.connect(ctx.destination);
      this.gainNode = gain;
      src.connect(gain);
      src.connect(splitter);
      splitter.connect(a, 0);
      splitter.connect(b, 1);
      this.analysers = [a, b];
      this.ctxStartTime = ctx.currentTime;
      src.start(0, at);
      this.source = src;
    } catch {
      this.source = null;
      this.analysers = null;
    }
  }

  private stopAudio(): void {
    try {
      this.source?.stop();
    } catch {
      /* already stopped */
    }
    this.source?.disconnect();
    this.source = null;
    this.gainNode?.disconnect();
    this.gainNode = null;
    this.analysers = null;
  }

  private makeUrl(wav: ArrayBuffer): string {
    if (typeof Blob !== 'undefined' && typeof URL.createObjectURL === 'function') {
      // Every take keeps its own URL (all are playable), released on dispose.
      const url = URL.createObjectURL(new Blob([wav], { type: 'audio/wav' }));
      this.urls.push(url);
      return url;
    }
    // Non-browser (tests): a data URL.
    const bytes = new Uint8Array(wav);
    let bin = '';
    for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return `data:audio/wav;base64,${btoa(bin)}`;
  }
}

function now(): number {
  return typeof performance !== 'undefined' ? performance.now() : Date.now();
}

/** "Bars 13–16 · Felt piano". */
export function scopeLabel(score: NScore | null, scope: TakeScope): string {
  const { start, end } = scope.bars;
  return `${start === end ? `Bar ${start}` : `Bars ${start}–${end}`} · ${score ? layerLabel(score, scope.layer) : scope.layer}`;
}

/**
 * True when two renders are sample-identical outside the changed bars plus the changed layers'
 * tails (wrapping on loops): the contract's audio-level isolation window.
 */
export function identicalOutside(a: Float32Array[], b: Float32Array[], sr: number, score: NScore, changed: Record<string, [number, number]>): boolean {
  if (a.length !== b.length || a[0].length !== b[0].length) return false;
  const ids = Object.keys(changed);
  const n = a[0].length;
  const keep = new Uint8Array(n).fill(1);
  if (ids.length) {
    const barS = scoreSeconds(score) / score.bars;
    const from = Math.min(...ids.map((l) => changed[l][0]));
    const to = Math.max(...ids.map((l) => changed[l][1]));
    const tail = Math.max(0.35, ...expandNotes(score).filter((e) => ids.includes(e.layer)).map((e) => e.lengthS + 0.35));
    const s0 = Math.floor((from - 1) * barS * sr);
    const s1 = Math.ceil((to * barS + tail) * sr);
    const loop = score.playback.mode === 'loop';
    for (let i = s0; i < s1; i++) {
      if (i < n) keep[i] = 0;
      else if (loop) keep[i % n] = 0;
    }
  }
  for (let c = 0; c < a.length; c++) {
    for (let i = 0; i < n; i++) if (keep[i] && a[c][i] !== b[c][i]) return false;
  }
  return true;
}
