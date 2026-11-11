// The desktop adapter: every call goes over Tauri IPC to crates/praxis-app, whose commands wrap
// the shared command layer (praxis_cli::ops). A sound is a folder; its takes live in the folder's
// takes.json, which agents write too (MCP server), so the app re-reads the history whenever the
// file's revision moves and shows their takes live.
//
// Playback goes through the real-time engine in crates/praxis-app (audio.rs): every change of the
// live score (a take, a restore, a hand edit, undo, redo, a mute) is sent with `transport_sync`
// and swapped in while playing; position and meters come back as `praxis://transport` events.
// When no output device opens (or the sound cannot be built for playback), the transport falls
// back to an <audio> element on the current take's rendered file and says why.

import type { BarRange, NScore, Score, ScorePatch } from '../../score/types';
import { normalizeScore } from '../../score/types';
import { isUnpitchedClass } from '../../score/theory';
import type {
  AgentInfo,
  ApplyResult,
  AskRequest,
  AskResult,
  TakeResult,
  BlankSound,
  EngineAdapter,
  MakeTakeOptions,
  PresetInfo,
  StartResult,
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
  AGENT_EVENT,
  type AgentEventPayload,
  agentName,
  type AppliedPayload,
  type AskPayload,
  type LayerLive,
  type LivePayload,
  type NewSoundPayload,
  type RenderPayload,
  type SoundInfo,
  type SteppedPayload,
  type TakeRecord,
  type TakesPayload,
  toRenderReport,
  toTake,
  type TransportEventPayload,
  type TransportStatusPayload,
  type ViewRequest,
  METER_FLOOR_DB,
} from './protocol';
import { blankProblem, DRAFT_ASK } from '../blank';
import type { AnalyzeResult, ClipFile, CompareLevel, LibraryEntry, LibraryResult, ReferenceClip, SoundProfile } from '../EngineAdapter';
import { clipErrorWords, clipProblem, withReference } from '../reference';
import { type ClipPayload, type LibraryEntryPayload, toLibraryEntry, toProfile } from './protocol';
import { presetsFromPayload } from '../presets';

/** Event crates/praxis-app sends when the open sound's takes.json gets a new revision. */
export const TAKES_EVENT = 'praxis://takes';

/** Event crates/praxis-app's audio service sends with position, meters and device counters. */
export const TRANSPORT_EVENT = 'praxis://transport';

/** How the app is playing: the real-time engine, or the <audio> fallback (and why). */
export interface PlaybackInfo {
  mode: 'engine' | 'audio';
  /** Plain words for the person when the engine cannot play; null when it can. */
  message: string | null;
  device: string | null;
  sampleRate: number;
  xruns: number;
  lateWakeups: number;
  deviceErrors: number;
  /** Callback sizes the device used, in frames ([0, 0] before the first callback). */
  callbackFrames: [number, number];
  preparing: boolean;
  stalled: boolean;
}

/** An agent that made a take this recently counts as connected. */
const AGENT_ACTIVE_MS = 15 * 60 * 1000;

/** Minimal <audio> surface, so tests can run without a DOM. */
export interface AudioLike {
  src: string;
  loop: boolean;
  currentTime: number;
  readonly duration: number;
  readonly paused: boolean;
  /** 0..1; the A/B turns the take down with it in the fallback. */
  volume: number;
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

/** The words of a reply without a take. */
function replyOf(p: AskPayload): string {
  return (p.reply ?? p.agent?.reply ?? '').trim() || 'Nothing was changed.';
}

export class TauriEngine implements EngineAdapter {
  readonly kind = 'tauri' as const;
  readonly label = 'Praxis engine';

  /** Folder of the open sound; null for a new sound before its first take. */
  private sound: string | null = null;
  /** The folder a new sound's first request was asked in when it made no take yet (a question). */
  private unborn: string | null = null;
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
  private playbackSubs = new Set<(p: PlaybackInfo) => void>();

  /** 'engine' unless the real-time engine said it cannot play ('audio'), or has no commands ('none'). */
  private rtMode: 'engine' | 'audio' | 'none' = 'engine';
  private rtMessage: string | null = null;
  private rt: TransportEventPayload | null = null;
  /** Sound and live score last sent to the engine ('' = nothing open). */
  private syncedKey = '';
  /** Play was sent to the engine and no stop since. */
  private enginePlaying = false;
  private unlistenTransport: (() => void) | null = null;

  private audio: AudioLike | null = null;
  /** The A/B trim on the take while comparing (also applied to the <audio> fallback). */
  private compare: CompareLevel | null = null;
  /** blob: URLs of dropped clips, released on dispose. */
  private clipUrls: string[] = [];
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
    this.unborn = null;
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
    this.listening ??= Promise.all([
      this.bridge
        .listen<{ sound: string; revision: number }>(TAKES_EVENT, (e) => {
          if (e.sound !== this.sound || e.revision === this.revision) return;
          void this.exclusive(() => this.refresh());
        })
        .then((u) => {
          this.unlisten = u;
        }),
      this.bridge
        .listen<TransportEventPayload>(TRANSPORT_EVENT, (e) => this.onTransport(e))
        .then((u) => {
          this.unlistenTransport = u;
        })
        .catch(() => undefined),
    ]).then(() => undefined);
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
      if (this.isDraft()) return { ok: false, message: DRAFT_ASK } as AskResult;
      let sound = this.sound;
      if (!sound) {
        // A new sound whose first request got a question keeps its folder (and the assistant's
        // conversation) for the answer.
        sound = this.unborn ?? (await this.bridge.invoke<string>('new_sound_path', { text: request.text }));
      }
      const progress = request.onProgress;
      const stop = progress
        ? await this.bridge
            .listen<AgentEventPayload>(AGENT_EVENT, (e) => {
              if (e.sound !== sound) return;
              if (e.event === 'step') progress({ kind: 'step', n: e.n, words: e.words });
              else if (e.event === 'step_done') progress({ kind: 'step_done', n: e.n, ok: e.ok, words: e.words });
              else if (e.event === 'said') progress({ kind: 'said', text: e.text });
            })
            .catch(() => null)
        : null;
      try {
        // A reference's profile travels in the request's words (the planner reads them; the
        // assistant measures the clip's file in full with its analyze_clip tool).
        const payload = await this.bridge.invoke<AskPayload>('ask', {
          req: { sound, text: withReference(request.text, request.reference), who: 'You', scope: request.scope ?? null },
        });
        const record = payload.take;
        if (!record) {
          // A reply without a take: a question, a refusal, an undo, a view.
          if (this.sound === sound) await this.refresh();
          else this.unborn = sound;
          return { ok: true, take: null, reply: replyOf(payload), noTake: payload.no_take ?? 'nothing_to_change' } as AskResult;
        }
        this.unborn = null;
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
      } finally {
        stop?.();
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
        const payload = await this.bridge.invoke<AskPayload>('ask', {
          req: { sound: this.sound, text: ask, patch, who: 'You', scope: scope ?? null },
        });
        const record = payload.take;
        if (!record) {
          // A lock, mute or solo: the live score changed, no take was made.
          await this.refresh();
          return { ok: true, take: null, reply: replyOf(payload), noTake: payload.no_take ?? 'live' } as AskResult;
        }
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

  // ------------------------------------------------------------ building by hand

  /** A sound folder with a project and no take yet. */
  private isDraft(): boolean {
    return !!this.sound && !!this.score && this.takeList.length === 0;
  }

  async startBlank(settings: BlankSound): Promise<StartResult> {
    const problem = blankProblem(settings);
    if (problem) return { ok: false, message: problem };
    let created: SoundInfo | null = null;
    const r = await this.exclusive(async (): Promise<StartResult> => {
      try {
        const args: Record<string, unknown> = {
          title: settings.title?.trim() || null,
          key: settings.key,
          time_signature: settings.timeSignature,
          bars: settings.bars,
          playback: settings.loop ? { mode: 'loop' } : { mode: 'one_shot', tail_seconds: 4 },
        };
        if (settings.bpm !== undefined) args.bpm = settings.bpm;
        else args.seconds = settings.seconds;
        const made = await this.bridge.invoke<NewSoundPayload>('new_sound', { args });
        await this.stopAudio();
        this.reset();
        this.revision = await this.bridge.invoke<number>('watch_sound', { sound: made.sound });
        this.sound = made.sound;
        await this.refresh(true);
        created = await this.soundInfo(made.sound);
        return { ok: true };
      } catch (e) {
        return { ok: false, message: `Could not start the sound: ${message(e)}` };
      }
    });
    if (created) {
      const info: SoundInfo = created;
      this.soundSubs.forEach((cb) => cb(info));
    }
    return r;
  }

  private presetCache: PresetInfo[] | null = null;

  async presets(): Promise<PresetInfo[]> {
    if (this.presetCache) return this.presetCache.map((p) => ({ ...p }));
    const p = await this.bridge.invoke<Parameters<typeof presetsFromPayload>[0]>('presets');
    this.presetCache = presetsFromPayload(p);
    return this.presetCache.map((x) => ({ ...x }));
  }

  async makeTake(ask: string, options: MakeTakeOptions = {}): Promise<TakeResult> {
    return this.exclusive(async () => {
      if (!this.sound) return { ok: false, message: 'Start a sound first.' } as TakeResult;
      try {
        const { take: record } = await this.bridge.invoke<AskPayload & { take: TakeRecord }>('make_take', {
          req: { sound: this.sound, ask, who: 'You', title: options.title ?? null, say: options.say ?? null, scope: options.scope ?? null },
        });
        await this.refresh(true);
        const take = this.takeList.find((t) => t.n === record.n);
        if (!take) return { ok: false, message: `Take ${record.n} was made but could not be read back.` } as TakeResult;
        this.takeSubs.forEach((cb) => cb(take));
        return { ok: true, take } as TakeResult;
      } catch (e) {
        return { ok: false, message: message(e), error: patchError(e) } as TakeResult;
      }
    });
  }

  state(): TakeState {
    return {
      takes: [...this.takeList],
      current: this.currentN,
      score: this.score,
      canUndo: this.canUndo,
      canRedo: this.canRedo,
      draft: this.isDraft(),
    };
  }

  async restore(n: number): Promise<TakeResult> {
    return this.exclusive(async () => {
      const take = this.takeList.find((t) => t.n === n);
      if (!take || !this.sound) return { ok: false, message: `There is no take ${n}.` } as TakeResult;
      try {
        await this.bridge.invoke('restore', { req: { sound: this.sound, n } });
        await this.refresh(true);
        return { ok: true, take: this.takeList.find((t) => t.n === n) ?? take } as TakeResult;
      } catch (e) {
        return { ok: false, message: `Could not go back to take ${n}: ${message(e)}`, error: patchError(e) } as TakeResult;
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
    this.syncTransport();
  }

  /**
   * Sends the live score to the real-time engine when it changed (a new take, a restore, a hand
   * edit, undo, redo, a mute): the engine swaps it in while playing, or applies a mute / solo /
   * gain change live. The engine reads the score from the sound folder itself.
   */
  private syncTransport(): void {
    const key = this.sound && this.score ? `${this.sound}\n${JSON.stringify(this.score)}` : '';
    if (key === this.syncedKey || this.rtMode === 'none') return;
    this.syncedKey = key;
    this.bridge
      .invoke<TransportStatusPayload>('transport_sync', { sound: key ? this.sound : null })
      .then((s) => this.onRtStatus(s))
      .catch((e: unknown) => this.rtUnavailable(e));
  }

  async render(_options?: RenderOptions): Promise<RenderResult> {
    if (!this.sound) throw new Error('make a first take before rendering');
    const r = await this.bridge.invoke<RenderPayload>('render_live', { sound: this.sound });
    return { audioUrl: this.bridge.fileUrl(r.output ?? ''), report: toRenderReport(r) };
  }

  // ------------------------------------------------------------ transport

  private currentTake(): Take | null {
    return this.takeList.find((t) => t.n === this.currentN) ?? null;
  }

  private barSeconds(): number {
    const s = this.score;
    if (!s) return 2;
    return (60 / s.tempo.bpm) * s.time_signature.numerator * (4 / s.time_signature.denominator);
  }

  /** How the app plays right now, and anything the person should know about it. */
  playback(): PlaybackInfo {
    const e = this.rt;
    return {
      mode: this.rtMode === 'engine' ? 'engine' : 'audio',
      message: this.rtMessage,
      device: e?.device ?? null,
      sampleRate: e?.sample_rate ?? 0,
      xruns: e?.xruns ?? 0,
      lateWakeups: e?.late_wakeups ?? 0,
      deviceErrors: e?.device_errors ?? 0,
      callbackFrames: [e?.min_callback_frames ?? 0, e?.max_callback_frames ?? 0],
      preparing: e?.preparing ?? false,
      stalled: e?.stalled ?? false,
    };
  }

  /** Playback mode, device and counters: now, on every change and with every transport event. */
  onPlayback(cb: (p: PlaybackInfo) => void): Unsubscribe {
    this.playbackSubs.add(cb);
    cb(this.playback());
    return () => this.playbackSubs.delete(cb);
  }

  private emitPlayback(): void {
    const p = this.playback();
    this.playbackSubs.forEach((cb) => cb(p));
  }

  /** A status from the engine: it plays, or why it cannot (then <audio> plays the take's file). */
  private onRtStatus(s: TransportEventPayload): void {
    this.rt = { ...s };
    if (s.available && !s.error) {
      this.rtMode = 'engine';
      this.rtMessage = null;
    } else {
      this.rtMode = 'audio';
      this.rtMessage = `${(s.error ?? 'No audio output.').replace(/\.?$/, '.')} Playing the take's own file instead.`;
    }
    this.emitPlayback();
  }

  /** The commands are missing (an older shell, or a test bridge): <audio> for good. */
  private rtUnavailable(e: unknown): void {
    if (isCmdError(e) && e.message.startsWith('unknown command')) {
      this.rtMode = 'none';
      this.rtMessage = null;
    } else {
      this.rtMode = 'audio';
      this.rtMessage = `Real-time playback is unavailable (${message(e)}). Playing the take's own file instead.`;
    }
    this.emitPlayback();
  }

  private onTransport(e: TransportEventPayload): void {
    if (this.rtMode === 'none') return;
    const ok = e.available && !e.error;
    if (ok !== (this.rtMode === 'engine')) this.onRtStatus(e);
    else this.rt = e;
    if (e.ended) this.enginePlaying = false;
    if (this.rtMode === 'engine') {
      this.emitPosition();
      const db = (v: number) => (v <= METER_FLOOR_DB ? -Infinity : v);
      const m: Meters = { peakL: db(e.peak_db[0]), peakR: db(e.peak_db[1]), rmsL: db(e.rms_db[0]), rmsR: db(e.rms_db[1]) };
      this.meterSubs.forEach((cb) => cb(m));
    }
    this.emitPlayback();
  }

  private engineOn(): boolean {
    return this.rtMode === 'engine';
  }

  async play(): Promise<void> {
    const take = this.currentTake();
    // A draft has no take (no file for <audio>), but the real-time engine plays its live score.
    if (!take && !this.isDraft()) return;
    if (this.rtMode !== 'none') {
      try {
        // Opens the device again if it was missing, and plays the live score.
        const s = await this.bridge.invoke<TransportStatusPayload>('transport_play');
        this.onRtStatus(s);
        if (this.engineOn()) {
          this.enginePlaying = true;
          this.pauseAudio();
          this.emitPosition();
          return;
        }
      } catch (e) {
        this.rtUnavailable(e);
      }
    }
    if (!take) return;
    this.audio ??= this.makeAudio();
    const a = this.audio;
    if (!a) return;
    this.applyCompareToAudio();
    if (a.src !== take.render.audioUrl) a.src = take.render.audioUrl;
    a.loop = take.render.report.is_loop && !this.loopRange;
    await a.play();
    this.startTimer();
  }

  async stop(): Promise<void> {
    await this.stopAudio();
  }

  private pauseAudio(): void {
    this.audio?.pause();
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }

  /** Stops the engine (5 ms fade) and the <audio> fallback. */
  private async stopAudio(): Promise<void> {
    this.pauseAudio();
    if (this.rtMode !== 'none' && (this.enginePlaying || this.rt?.playing)) {
      this.enginePlaying = false;
      await this.bridge.invoke('transport_stop').catch(() => undefined);
      if (this.rt) this.rt = { ...this.rt, playing: false };
    }
    this.emitPosition();
  }

  async seek(seconds: number): Promise<void> {
    const at = Math.max(0, seconds);
    if (this.engineOn()) await this.bridge.invoke('transport_seek', { seconds: at }).catch(() => undefined);
    if (this.audio) this.audio.currentTime = at;
    this.emitPosition();
  }

  async setLoop(range: BarRange | null): Promise<void> {
    this.loopRange = range;
    if (this.rtMode !== 'none') await this.bridge.invoke('transport_set_loop', { range }).catch(() => undefined);
    if (this.audio) this.audio.loop = !range && (this.currentTake()?.render.report.is_loop ?? false);
    this.emitPosition();
  }

  /** A live layer override in the engine (trim dB, mute, solo): heard at once, not saved. */
  async setLayerLive(layer: string, live: LayerLive): Promise<void> {
    if (this.rtMode === 'none') return;
    await this.bridge.invoke('transport_layer', { layer, live }).catch(() => undefined);
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

  /** The engine's position while it plays; the <audio> element's in the fallback. */
  private position(): TransportPosition {
    const a = this.audio;
    const audioOn = !!a && !a.paused;
    const e = this.engineOn() && !audioOn ? this.rt : null;
    const seconds = e ? e.seconds : (a?.currentTime ?? 0);
    const playing = e ? e.playing : audioOn;
    return { playing, seconds, bar: 1 + seconds / this.barSeconds(), loop: this.loopRange };
  }

  private emitPosition(): void {
    const p = this.position();
    this.posSubs.forEach((cb) => cb(p));
  }

  onPosition(cb: (p: TransportPosition) => void): Unsubscribe {
    this.posSubs.add(cb);
    return () => this.posSubs.delete(cb);
  }

  /** Output meters from the real-time engine (none in the <audio> fallback). */
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

  // ------------------------------------------------------------ reference clips

  /**
   * Sends the clip's bytes to the shell, which keeps a copy (for the assistant), decodes it
   * (ffmpeg for video) and returns its Sound Profile. The WebView plays the dropped file itself
   * for A/B, so the shell never grants the WebView access to the copy.
   */
  async analyzeClip(file: ClipFile): Promise<AnalyzeResult> {
    const problem = clipProblem(file);
    if (problem) return { ok: false, message: problem };
    if (!this.bridge.invokeBytes) return { ok: false, message: 'This build of the app cannot read dropped clips.' };
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const r = await this.bridge.invokeBytes<ClipPayload>('clip_analyze', bytes, { 'x-clip-name': encodeURIComponent(file.name) });
      const profile = toProfile(r.profile);
      let audioUrl: string | null = null;
      if (typeof Blob !== 'undefined' && file instanceof Blob && typeof URL.createObjectURL === 'function') {
        audioUrl = URL.createObjectURL(file);
        this.clipUrls.push(audioUrl);
      }
      return { ok: true, clip: { name: file.name, profile, audioUrl, path: r.path, libraryId: null } };
    } catch (e) {
      return { ok: false, message: clipErrorWords(file.name, isCmdError(e) ? e.code : 'error', message(e)) };
    }
  }

  async library(): Promise<LibraryEntry[]> {
    try {
      const list = await this.bridge.invoke<LibraryEntryPayload[]>('library_list');
      return list.flatMap((e) => {
        try {
          return [toLibraryEntry(e)];
        } catch {
          return [];
        }
      });
    } catch {
      return [];
    }
  }

  async libraryProfile(id: string): Promise<SoundProfile | null> {
    try {
      const e = await this.bridge.invoke<LibraryEntryPayload>('library_show', { id });
      return e.profile ? toProfile(e.profile) : null;
    } catch {
      return null;
    }
  }

  /**
   * Saves the profile to the library: the shell adds the kept clip's own cached profile (exact),
   * or the profile as the window has it when the clip is no longer kept.
   */
  async saveReference(ref: ReferenceClip, name: string, tags: string[]): Promise<LibraryResult> {
    if (!name.trim()) return { ok: false, message: 'Give the reference a name.' };
    try {
      const e = await this.bridge.invoke<LibraryEntryPayload>('library_add', { name: name.trim(), tags, clip: ref.path, profile: ref.profile });
      return { ok: true, entry: toLibraryEntry(e) };
    } catch (e) {
      return { ok: false, message: `Could not save it to the library: ${message(e)}` };
    }
  }

  async tagReference(id: string, tags: string[]): Promise<LibraryResult> {
    try {
      const e = await this.bridge.invoke<LibraryEntryPayload>('library_tag', { id, tags });
      return { ok: true, entry: toLibraryEntry(e) };
    } catch (e) {
      return { ok: false, message: `Could not change the tags: ${message(e)}` };
    }
  }

  async removeReference(id: string): Promise<boolean> {
    try {
      await this.bridge.invoke('library_remove', { id });
      return true;
    } catch {
      return false;
    }
  }

  /** The take's side of the A/B: a master trim in the real-time engine (or the <audio> fallback's volume). */
  async setCompare(level: CompareLevel | null): Promise<void> {
    this.compare = level ? { ...level } : null;
    this.applyCompareToAudio();
    if (this.rtMode === 'none') return;
    await this.bridge.invoke('transport_compare', { level: level ? { gain_db: level.gainDb, muted: level.muted } : null }).catch(() => undefined);
  }

  private applyCompareToAudio(): void {
    const a = this.audio;
    if (!a) return;
    const c = this.compare;
    a.volume = !c ? 1 : c.muted ? 0 : Math.min(1, 10 ** (Math.min(0, c.gainDb) / 20));
  }

  dispose(): void {
    if (typeof URL !== 'undefined' && typeof URL.revokeObjectURL === 'function') this.clipUrls.forEach((u) => URL.revokeObjectURL(u));
    this.clipUrls = [];
    void this.stopAudio();
    this.unlisten?.();
    this.unlisten = null;
    this.unlistenTransport?.();
    this.unlistenTransport = null;
    this.listening = null;
    this.takeSubs.clear();
    this.stateSubs.clear();
    this.viewSubs.clear();
    this.soundSubs.clear();
    this.posSubs.clear();
    this.meterSubs.clear();
    this.playbackSubs.clear();
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
