// The page's side of the AudioWorklet (worklet.ts): one AudioWorkletNode running the wasm
// real-time engine, its commands, and the status it reports. Also renders a score through the
// same worklet in an OfflineAudioContext (no output device, never heard), which is how the
// browser build proves the worklet plays what the offline renderer renders.

import workletUrl from './worklet.ts?worker&url';
import type { WorkletEvent, WorkletMessage } from './worklet';
import { STATUS } from './wasm-api';

/** What the engine in the worklet reports. Positions are musical samples. */
export interface PlayerStatus {
  position: number;
  playing: boolean;
  ended: boolean;
  peak: [number, number];
  rms: [number, number];
  swaps: number;
}

/** What `load` answers (crates/praxis-wasm/src/realtime.rs `Realtime::load`). */
export interface LoadInfo {
  ok: boolean;
  message?: string;
  sample_rate?: number;
  length_samples?: number;
  end_samples?: number;
  is_loop?: boolean;
  latency_samples?: number;
  master_gain_db?: number;
  warnings?: string[];
  fallbacks?: string[];
}

function statusOf(s: number[]): PlayerStatus {
  return {
    position: s[STATUS.position],
    playing: s[STATUS.playing] > 0,
    ended: s[STATUS.ended] > 0,
    peak: [s[STATUS.peakL], s[STATUS.peakR]],
    rms: [s[STATUS.rmsL], s[STATUS.rmsR]],
    swaps: s[STATUS.swaps],
  };
}

/** A message the worklet answers (its `id` is added when sent). */
type Request = { type: 'packs'; files: [string, ArrayBuffer][] } | { type: 'load'; score: string; gain: number };

const added = new WeakMap<BaseAudioContext, Promise<void>>();

/** Adds the worklet module to a context once. */
function addModule(ctx: BaseAudioContext): Promise<void> {
  let p = added.get(ctx);
  if (!p) {
    p = ctx.audioWorklet.addModule(workletUrl);
    added.set(ctx, p);
  }
  return p;
}

/** Creates the engine node: the compiled module when it can be cloned, else its bytes. */
async function createNode(ctx: BaseAudioContext, module: WebAssembly.Module, bytes?: () => Promise<ArrayBuffer>): Promise<AudioWorkletNode> {
  await addModule(ctx);
  const make = (processorOptions: Record<string, unknown>) =>
    new AudioWorkletNode(ctx, 'praxis-engine', { numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [2], processorOptions });
  let node: AudioWorkletNode;
  try {
    node = make({ module });
  } catch (e) {
    if (!bytes) throw e;
    node = make({ bytes: await bytes() });
  }
  return node;
}

export class WorkletPlayer {
  private pending = new Map<number, (v: unknown) => void>();
  private next = 1;
  private statusSubs = new Set<(s: PlayerStatus) => void>();
  last: PlayerStatus = { position: 0, playing: false, ended: false, peak: [0, 0], rms: [0, 0], swaps: 0 };

  private constructor(
    readonly ctx: BaseAudioContext,
    readonly node: AudioWorkletNode,
    readonly version: string,
    /** The A/B trim after the engine (1 = as rendered). */
    private readonly trim: GainNode,
  ) {
    node.port.onmessage = (e: MessageEvent<WorkletEvent>) => this.onEvent(e.data);
  }

  /** Starts the engine in a worklet of `ctx` (an AudioContext to play, or an offline one). */
  static async create(ctx: BaseAudioContext, module: WebAssembly.Module, bytes?: () => Promise<ArrayBuffer>): Promise<WorkletPlayer> {
    const node = await createNode(ctx, module, bytes);
    const first = await new Promise<WorkletEvent>((resolve) => {
      node.port.onmessage = (e: MessageEvent<WorkletEvent>) => resolve(e.data);
    });
    if (first.type === 'error') throw new Error(`the engine could not start in the audio worklet: ${first.message}`);
    const trim = ctx.createGain();
    node.connect(trim);
    trim.connect(ctx.destination);
    return new WorkletPlayer(ctx, node, first.type === 'ready' ? first.version : '', trim);
  }

  get sampleRate(): number {
    return this.ctx.sampleRate;
  }

  private onEvent(e: WorkletEvent): void {
    if (e.type === 'reply') {
      const p = this.pending.get(e.id);
      this.pending.delete(e.id);
      p?.(e.result);
    } else if (e.type === 'status') {
      this.last = statusOf(e.s);
      this.statusSubs.forEach((cb) => cb(this.last));
    }
  }

  private request(msg: Request): Promise<unknown> {
    const id = this.next++;
    return new Promise((resolve) => {
      this.pending.set(id, resolve);
      this.node.port.postMessage({ ...msg, id });
    });
  }

  private send(msg: WorkletMessage): void {
    this.node.port.postMessage(msg);
  }

  /** Hands pack files to the engine in the worklet (copied; the page keeps its own). */
  async addPackFiles(files: [string, ArrayBuffer][]): Promise<void> {
    if (!files.length) return;
    await this.request({ type: 'packs', files });
  }

  /** Prepares `score` in the worklet and swaps it in (cross-faded when playing). */
  async load(score: unknown, masterGainDb: number | null): Promise<LoadInfo> {
    const r = await this.request({ type: 'load', score: JSON.stringify(score), gain: masterGainDb ?? Number.NaN });
    try {
      return JSON.parse(String(r)) as LoadInfo;
    } catch {
      return { ok: false, message: String(r) };
    }
  }

  play(): void {
    this.send({ type: 'play' });
  }

  /** Resolves once the worklet has handled every message sent before it (they arrive in order). */
  async sync(): Promise<void> {
    await this.request({ type: 'packs', files: [] });
  }

  stop(): void {
    this.send({ type: 'stop' });
  }

  /** Musical position in samples. */
  seek(position: number): void {
    this.send({ type: 'seek', position: Math.max(0, Math.round(position)) });
  }

  /** Loop range in musical samples; null clears it. */
  setLoop(range: [number, number] | null): void {
    this.send({ type: 'loop', start: range ? Math.round(range[0]) : 0, end: range ? Math.round(range[1]) : 0 });
  }

  setMute(layer: string, on: boolean): void {
    this.send({ type: 'mute', layer, on });
  }

  setSolo(layer: string, on: boolean): void {
    this.send({ type: 'solo', layer, on });
  }

  /** A linear gain on what the engine plays (the A/B trim); 1 puts it back. */
  setTrim(gain: number): void {
    this.trim.gain.value = Math.max(0, gain);
  }

  onStatus(cb: (s: PlayerStatus) => void): () => void {
    this.statusSubs.add(cb);
    return () => this.statusSubs.delete(cb);
  }

  dispose(): void {
    this.node.port.onmessage = null;
    this.node.disconnect();
    this.trim.disconnect();
  }
}

/**
 * Renders `score` through the worklet engine in an OfflineAudioContext: `frames` frames played
 * from the start (pack files first, then load, play, render). Nothing reaches a device.
 */
export async function renderThroughWorklet(opts: {
  module: WebAssembly.Module;
  bytes?: () => Promise<ArrayBuffer>;
  score: unknown;
  masterGainDb: number | null;
  frames: number;
  sampleRate: number;
  packFiles: [string, ArrayBuffer][];
}): Promise<{ left: Float32Array; right: Float32Array; load: LoadInfo; ms: number }> {
  const ctx = new OfflineAudioContext({ numberOfChannels: 2, length: opts.frames, sampleRate: opts.sampleRate });
  const player = await WorkletPlayer.create(ctx, opts.module, opts.bytes);
  await player.addPackFiles(opts.packFiles);
  const load = await player.load(opts.score, opts.masterGainDb);
  if (!load.ok) throw new Error(load.message ?? 'the worklet could not load the score');
  player.play();
  await player.sync();
  const t0 = performance.now();
  const buf = await ctx.startRendering();
  const ms = performance.now() - t0;
  player.dispose();
  return { left: buf.getChannelData(0).slice(), right: buf.getChannelData(1).slice(), load, ms };
}
