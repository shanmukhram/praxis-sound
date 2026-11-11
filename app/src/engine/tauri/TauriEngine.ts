// The desktop adapter: every call goes over Tauri IPC to crates/praxis-app, whose commands wrap
// the shared command layer (praxis_cli::ops). A sound is a folder; its takes live in the folder's
// takes.json, which agents write too (MCP server), so the app re-reads the history whenever the
// file's revision moves and shows their takes live. Playback is an <audio> element on the take's
// rendered file until the real-time engine (WP-R) takes over the transport.

import type { BarRange, NScore, Score, ScorePatch } from '../../score/types';
import { normalizeScore } from '../../score/types';
import { isUnpitchedClass } from '../../score/theory';
import type {
  AgentInfo,
  ApplyResult,
  AskRequest,
  AskResult,
  EngineAdapter,
  ExportRequest,
  ExportResult,
  LoadResult,
  Meters,
  OpenViewEvent,
  PatchError,
  RenderOptions,
  RenderResult,
  Take,
  TakeScope,
  TakeState,
  TransportPosition,
  Unsubscribe,
  ViewName,
} from '../EngineAdapter';
import { type Bridge, isCmdError } from './bridge';
import {
  agentName,
  type AppliedPayload,
  type AskPayload,
  type LivePayload,
  type RenderPayload,
  type SoundInfo,
  type SteppedPayload,
  type TakeRecord,
  type TakesPayload,
  toRenderReport,
  toTake,
  type ViewRequest,
} from './protocol';

/** Event crates/praxis-app sends when the open sound's takes.json gets a new revision. */
export const TAKES_EVENT = 'praxis://takes';

/** An agent that made a take this recently counts as connected. */
const AGENT_ACTIVE_MS = 15 * 60 * 1000;

/** Minimal <audio> surface, so tests can run without a DOM. */
export interface AudioLike {
  src: string;
  loop: boolean;
  currentTime: number;
  readonly duration: number;
  readonly paused: boolean;
  play(): Promise<void>;
  pause(): void;
}

export interface TauriEngineOptions {
  /** Makes the <audio> element for playback (default: `new Audio()` when there is a DOM). */
  makeAudio?: () => AudioLike | null;
  /** Clock for "recently active" agents (tests). */
  now?: () => number;
}

function message(e: unknown): string {
  if (isCmdError(e)) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}

function patchError(e: unknown): PatchError | undefined {
  return isCmdError(e) && e.patch_error ? (e.patch_error as PatchError) : undefined;
}

export class TauriEngine implements EngineAdapter {
  readonly kind = 'tauri' as const;
  readonly label = 'Praxis engine';

  /** Folder of the open sound; null for a new sound before its first take. */
  private sound: string | null = null;
  private revision = 0;
  private takeList: Take[] = [];
  private currentN = 0;
  private score: NScore | null = null;
  private canUndo = false;
  private canRedo = false;
  private seenViews = 0;
  private agentSeen = new Map<string, number>();

  private chain: Promise<unknown> = Promise.resolve();
  private unlisten: (() => void) | null = null;
  private listening: Promise<void> | null = null;
  private takeSubs = new Set<(t: Take) => void>();
  private stateSubs = new Set<(s: TakeState) => void>();
  private viewSubs = new Set<(e: OpenViewEvent) => void>();
  private soundSubs = new Set<(s: SoundInfo) => void>();
  private posSubs = new Set<(p: TransportPosition) => void>();
  private meterSubs = new Set<(m: Meters) => void>();

  private audio: AudioLike | null = null;
  private loopRange: BarRange | null = null;
  private timer: ReturnType<typeof setInterval> | null = null;
  private readonly makeAudio: () => AudioLike | null;
  private readonly now: () => number;

  constructor(
    private readonly bridge: Bridge,
    options: TauriEngineOptions = {},
  ) {
    this.makeAudio = options.makeAudio ?? (() => (typeof Audio === 'undefined' ? null : (new Audio() as AudioLike)));
    this.now = options.now ?? (() => Date.now());
  }

  // ------------------------------------------------------------ sounds

  /** Every sound folder, newest first. */
  listSounds(): Promise<SoundInfo[]> {
    return this.bridge.invoke<SoundInfo[]>('list_sounds');
  }

  /** The folder of the open sound (null for a new sound before its first take). */
  soundPath(): string | null {
    return this.sound;
  }

  /** Called once a new sound gets its folder (its first take); the shell lists it then. */
  onSoundCreated(cb: (s: SoundInfo) => void): Unsubscribe {
    this.soundSubs.add(cb);
    return () => this.soundSubs.delete(cb);
  }

  /** Opens a sound folder, or a new sound (null), and follows its takes. */
  async openSound(path: string | null): Promise<void> {
    await this.ensureListening();
    return this.exclusive(async () => {
      await this.stopAudio();
      if (path === this.sound && path !== null) {
        await this.refresh();
        return;
      }
      this.reset();
      if (path) {
        try {
          // Refused unless the folder is a sound; only then may the WebView load its audio.
          this.revision = await this.bridge.invoke<number>('watch_sound', { sound: path });
        } catch (e) {
          this.emitState();
          throw new Error(`Could not open ${path}: ${message(e)}`);
        }
        this.sound = path;
        await this.refresh(true);
      } else {
        this.emitState();
      }
    });
  }

  private reset(): void {
    this.sound = null;
    this.revision = 0;
    this.takeList = [];
    this.currentN = 0;
    this.score = null;
    this.canUndo = false;
    this.canRedo = false;
    this.seenViews = 0;
    this.agentSeen.clear();
  }

  private ensureListening(): Promise<void> {
    this.listening ??= this.bridge
      .listen<{ sound: string; revision: number }>(TAKES_EVENT, (e) => {
        if (e.sound !== this.sound || e.revision === this.revision) return;
        void this.exclusive(() => this.refresh());
      })
      .then((u) => {
        this.unlisten = u;
      });
    return this.listening;
  }

  /**
   * Re-reads the sound's take history and live score. New takes are announced through `onTake`
   * (and their `opens` view through `onOpenView`), whoever made them. `initial` takes the
   * history as already seen, so opening a sound does not replay it.
   */
  private async refresh(initial = false): Promise<void> {
    const sound = this.sound;
    if (!sound) return;
    const payload = await this.bridge.invoke<TakesPayload>('takes', { sound });
    if (sound !== this.sound) return;
    this.revision = Math.max(this.revision, payload.revision ?? 0);
    const liveScore = payload.score ? normalizeScore(payload.score) : null;
    const known = new Set(this.takeList.map((t) => t.n));
    const takes = payload.takes.map((r: TakeRecord) => toTake(r, (p) => this.bridge.fileUrl(p), liveScore));
    this.takeList = takes;
    this.currentN = payload.current ?? takes.at(-1)?.n ?? 0;
    this.score = liveScore;
    this.canUndo = payload.can_undo ?? false;
    this.canRedo = payload.can_redo ?? false;
    const fresh = initial ? [] : takes.filter((t) => !known.has(t.n));
    for (const t of initial ? takes : fresh) {
      const a = agentName(t.who);
      // On open, agents that worked on this sound are listed; their newer takes refresh them.
      if (a) this.agentSeen.set(a, initial ? Math.max(this.agentSeen.get(a) ?? 0, 0) : this.now());
    }
    const views = payload.views ?? [];
    const newViews = initial ? [] : views.filter((v) => v.revision > this.seenViews);
    this.seenViews = Math.max(this.seenViews, ...views.map((v) => v.revision));
    this.emitState();
    for (const t of fresh) {
      this.takeSubs.forEach((cb) => cb(t));
      if (t.opens && !newViews.some((v) => v.take === t.n && v.view === t.opens)) this.emitView(t.opens, agentName(t.who) ?? 'Praxis');
    }
    for (const v of newViews) this.emitView(v.view, viewBy(v));
  }

  // ------------------------------------------------------------ score

  async load(input: Score): Promise<LoadResult> {
    const score = normalizeScore(input);
    await this.exclusive(async () => {
      await this.stopAudio();
      this.reset();
      this.score = score;
      this.emitState();
    });
    return { score, errors: [], warnings: [] };
  }

  async applyPatch(patch: ScorePatch): Promise<ApplyResult> {
    return this.exclusive(async () => {
      if (!this.sound) return { ok: false, error: { kind: 'invalid', message: 'make a first take before editing' } } as ApplyResult;
      try {
        const r = await this.bridge.invoke<AppliedPayload>('apply_patch', { sound: this.sound, patch });
        this.setLive(r.live);
        this.emitState();
        return { ok: true, score: this.score!, inverse: r.inverse } as ApplyResult;
      } catch (e) {
        return { ok: false, error: patchError(e) ?? { kind: 'invalid', message: message(e) } } as ApplyResult;
      }
    });
  }

  private setLive(live: LivePayload): void {
    this.score = normalizeScore(live.score);
    this.canUndo = live.can_undo;
    this.canRedo = live.can_redo;
  }

  // ------------------------------------------------------------ takes

  async ask(request: AskRequest): Promise<AskResult> {
    let created: SoundInfo | null = null;
    const r = await this.exclusive(async () => {
      let sound = this.sound;
      if (!sound) {
        sound = await this.bridge.invoke<string>('new_sound_path', { text: request.text });
      }
      try {
        const { take: record } = await this.bridge.invoke<AskPayload>('ask', {
          req: { sound, text: request.text, who: 'You', scope: request.scope ?? null },
        });
        if (this.sound !== sound) {
          this.revision = await this.bridge.invoke<number>('watch_sound', { sound });
          this.sound = sound;
          created = await this.soundInfo(sound);
        }
        await this.refresh(true);
        const take = this.takeList.find((t) => t.n === record.n);
        if (!take) return { ok: false, message: `Take ${record.n} was made but could not be read back.` } as AskResult;
        if (take.opens) this.emitView(take.opens, 'Praxis');
        this.takeSubs.forEach((cb) => cb(take));
        return { ok: true, take } as AskResult;
      } catch (e) {
        return { ok: false, message: message(e), error: patchError(e) } as AskResult;
      }
    });
    if (created) {
      const info: SoundInfo = created;
      this.soundSubs.forEach((cb) => cb(info));
    }
    return r;
  }

  private async soundInfo(path: string): Promise<SoundInfo> {
    const all = await this.bridge.invoke<SoundInfo[]>('list_sounds');
    const id = path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
    return all.find((s) => s.path === path) ?? { id, name: id, path, sub: 'New', takes: 1, modified_ms: this.now() };
  }

  async edit(patch: ScorePatch, ask: string, scope?: TakeScope): Promise<AskResult> {
    return this.exclusive(async () => {
      if (!this.sound) return { ok: false, message: 'Make a first take before editing it by hand.' } as AskResult;
      try {
        // A hand edit is an `ask` carrying the explicit patch (provider "hand").
        const { take: record } = await this.bridge.invoke<AskPayload>('ask', {
          req: { sound: this.sound, text: ask, patch, who: 'You', scope: scope ?? null },
        });
        await this.refresh(true);
        const take = this.takeList.find((t) => t.n === record.n);
        if (!take) return { ok: false, message: `Take ${record.n} was made but could not be read back.` } as AskResult;
        this.takeSubs.forEach((cb) => cb(take));
        return { ok: true, take } as AskResult;
      } catch (e) {
        return { ok: false, message: message(e), error: patchError(e) } as AskResult;
      }
    });
  }

  async takes(): Promise<Take[]> {
    return [...this.takeList];
  }

  state(): TakeState {
    return {
      takes: [...this.takeList],
      current: this.currentN,
      score: this.score,
      canUndo: this.canUndo,
      canRedo: this.canRedo,
    };
  }

  async restore(n: number): Promise<AskResult> {
    return this.exclusive(async () => {
      const take = this.takeList.find((t) => t.n === n);
      if (!take || !this.sound) return { ok: false, message: `There is no take ${n}.` } as AskResult;
      try {
        await this.bridge.invoke('restore', { req: { sound: this.sound, n } });
        await this.refresh(true);
        return { ok: true, take: this.takeList.find((t) => t.n === n) ?? take } as AskResult;
      } catch (e) {
        return { ok: false, message: `Could not go back to take ${n}: ${message(e)}`, error: patchError(e) } as AskResult;
      }
    });
  }

  undo(): Promise<boolean> {
    return this.step('undo');
  }

  redo(): Promise<boolean> {
    return this.step('redo');
  }

  private step(cmd: 'undo' | 'redo'): Promise<boolean> {
    return this.exclusive(async () => {
      if (!this.sound) return false;
      try {
        const r = await this.bridge.invoke<SteppedPayload>(cmd, { sound: this.sound });
        // The step may move the current take (takes.json): read the history back.
        if (r.moved && r.live.current !== this.currentN) await this.refresh(true);
        else {
          this.setLive(r.live);
          this.emitState();
        }
        return r.moved;
      } catch {
        return false;
      }
    });
  }

  async suggestions(): Promise<string[]> {
    return suggestionsFor(this.score, this.takeList.length);
  }

  async agents(): Promise<AgentInfo[]> {
    const now = this.now();
    return [...this.agentSeen.entries()]
      .sort((a, b) => b[1] - a[1])
      .map(([name, at]) => ({ name, connected: now - at < AGENT_ACTIVE_MS, about: 'Makes takes through the Praxis MCP server.' }));
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
    this.emitView(view, by);
  }

  private emitView(view: ViewName, by: string): void {
    this.viewSubs.forEach((cb) => cb({ view, by }));
  }

  private emitState(): void {
    const s = this.state();
    this.stateSubs.forEach((cb) => cb(s));
  }

  async render(_options?: RenderOptions): Promise<RenderResult> {
    if (!this.sound) throw new Error('make a first take before rendering');
    const r = await this.bridge.invoke<RenderPayload>('render_live', { sound: this.sound });
    return { audioUrl: this.bridge.fileUrl(r.output ?? ''), report: toRenderReport(r) };
  }

  // ------------------------------------------------------------ transport (<audio> on the take's file)

  private currentTake(): Take | null {
    return this.takeList.find((t) => t.n === this.currentN) ?? null;
  }

  private barSeconds(): number {
    const s = this.score;
    if (!s) return 2;
    return (60 / s.tempo.bpm) * s.time_signature.numerator * (4 / s.time_signature.denominator);
  }

  async play(): Promise<void> {
    const take = this.currentTake();
    if (!take) return;
    this.audio ??= this.makeAudio();
    const a = this.audio;
    if (!a) return;
    if (a.src !== take.render.audioUrl) a.src = take.render.audioUrl;
    a.loop = take.render.report.is_loop && !this.loopRange;
    await a.play();
    this.startTimer();
  }

  async stop(): Promise<void> {
    await this.stopAudio();
  }

  private async stopAudio(): Promise<void> {
    this.audio?.pause();
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    this.emitPosition();
  }

  async seek(seconds: number): Promise<void> {
    if (this.audio) this.audio.currentTime = Math.max(0, seconds);
    this.emitPosition();
  }

  async setLoop(range: BarRange | null): Promise<void> {
    this.loopRange = range;
    if (this.audio) this.audio.loop = !range && (this.currentTake()?.render.report.is_loop ?? false);
    this.emitPosition();
  }

  private startTimer(): void {
    if (this.timer || typeof setInterval === 'undefined') return;
    this.timer = setInterval(() => {
      const a = this.audio;
      if (a && this.loopRange && !a.paused) {
        const bar = this.barSeconds();
        const end = this.loopRange.end * bar;
        if (a.currentTime >= end) a.currentTime = (this.loopRange.start - 1) * bar;
      }
      this.emitPosition();
      if (!a || a.paused) {
        if (this.timer) clearInterval(this.timer);
        this.timer = null;
      }
    }, 50);
  }

  private emitPosition(): void {
    const a = this.audio;
    const seconds = a?.currentTime ?? 0;
    const p: TransportPosition = { playing: !!a && !a.paused, seconds, bar: 1 + seconds / this.barSeconds(), loop: this.loopRange };
    this.posSubs.forEach((cb) => cb(p));
  }

  onPosition(cb: (p: TransportPosition) => void): Unsubscribe {
    this.posSubs.add(cb);
    return () => this.posSubs.delete(cb);
  }

  /** No meters until the real-time engine (WP-R); the <audio> element has none. */
  onMeters(cb: (m: Meters) => void): Unsubscribe {
    this.meterSubs.add(cb);
    return () => this.meterSubs.delete(cb);
  }

  async export(request: ExportRequest): Promise<ExportResult> {
    if (!this.sound) return { ok: false, files: [], message: 'Make a first take before exporting.' };
    try {
      return await this.bridge.invoke<ExportResult>('export', {
        args: {
          sound: this.sound,
          take: this.currentN || null,
          kinds: request.kinds,
          destination: request.destination,
          base_name: request.baseName,
        },
      });
    } catch (e) {
      return { ok: false, files: [], message: message(e) };
    }
  }

  dispose(): void {
    void this.stopAudio();
    this.unlisten?.();
    this.unlisten = null;
    this.listening = null;
    this.takeSubs.clear();
    this.stateSubs.clear();
    this.viewSubs.clear();
    this.soundSubs.clear();
    this.posSubs.clear();
    this.meterSubs.clear();
  }

  private exclusive<T>(fn: () => Promise<T>): Promise<T> {
    const run = this.chain.then(fn, fn);
    this.chain = run.catch(() => undefined);
    return run;
  }
}

function viewBy(v: ViewRequest): string {
  const w = (v.by ?? '').trim();
  return !w || w === 'You' ? 'Praxis' : w.replace(/^Agent · /, '');
}

/** Suggestion chips from the current sound (what a person would plausibly ask next). */
export function suggestionsFor(s: NScore | null, takes: number): string[] {
  if (!s || takes === 0) {
    return ['A calm 30-second felt piano loop in A minor', 'A warm logo sting in D major, 4 seconds', 'A tense 20-second loop in E minor'];
  }
  const out: string[] = [];
  if (s.layers.some((l) => !isUnpitchedClass(l.class))) out.push('Make it a little darker');
  if (s.bars >= 8) out.push('Change the last four bars');
  if (s.layers.some((l) => isUnpitchedClass(l.class))) out.push('Make the kick quieter');
  if (!s.layers.some((l) => l.class === 'melody') && s.bars >= 9) out.push('Add a quiet melody from bar 9');
  return out.slice(0, 4);
}
