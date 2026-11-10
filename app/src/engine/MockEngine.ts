// MockEngine: the EngineAdapter used in browser dev mode and tests until the Tauri (desktop) and
// wasm (browser) adapters exist. Patch rules follow docs/contracts/patch.md (see mock/apply.ts);
// audio is a clearly labelled placeholder preview (see mock/synth.ts).

import type {
  ApplyResult,
  EngineAdapter,
  ExportRequest,
  ExportResult,
  LoadResult,
  Meters,
  RenderOptions,
  RenderResult,
  TransportPosition,
  Unsubscribe,
} from './EngineAdapter';
import type { BarRange, NScore, Score, ScorePatch } from '../score/types';
import { clone, normalizeScore } from '../score/types';
import { barPosToSeconds, scoreSeconds, secondsToBarPos } from '../score/timeline';
import { applyPatch } from './mock/apply';
import { synthesize } from './mock/synth';
import { validate } from './mock/validate';
import { encodeWav } from './wav';

const SILENT: Meters = { peakL: -Infinity, peakR: -Infinity, rmsL: -Infinity, rmsR: -Infinity };

export class MockEngine implements EngineAdapter {
  readonly kind = 'mock' as const;
  readonly label = 'Mock engine · placeholder preview';

  /** Every export request received, newest last (the mock writes nothing). */
  readonly exports: ExportRequest[] = [];

  private score: NScore | null = null;
  private posSubs = new Set<(p: TransportPosition) => void>();
  private meterSubs = new Set<(m: Meters) => void>();
  private playing = false;
  private offsetS = 0;
  private startedAt = 0;
  private loop: BarRange | null = null;
  private timer: ReturnType<typeof setInterval> | null = null;
  private lastUrl: string | null = null;

  private ctx: AudioContext | null = null;
  private source: AudioBufferSourceNode | null = null;
  private analysers: [AnalyserNode, AnalyserNode] | null = null;
  private ctxStartTime = 0;

  async load(input: Score): Promise<LoadResult> {
    await this.stop();
    const score = normalizeScore(input);
    this.score = score;
    this.offsetS = 0;
    this.loop = null;
    const report = validate(score);
    this.emitPosition();
    return { score: clone(score), errors: report.errors, warnings: report.warnings };
  }

  /** Current score (a copy), or null before load. */
  current(): NScore | null {
    return this.score ? clone(this.score) : null;
  }

  async applyPatch(patch: ScorePatch): Promise<ApplyResult> {
    if (!this.score) return { ok: false, error: { kind: 'invalid', message: 'no score loaded' } };
    const r = applyPatch(this.score, patch);
    if (!r.ok) return r;
    this.score = r.score;
    if (this.playing) this.restartAudio(this.positionSeconds());
    return { ok: true, score: clone(r.score), inverse: r.inverse };
  }

  async render(options: RenderOptions = {}): Promise<RenderResult> {
    if (!this.score) throw new Error('no score loaded');
    const out = synthesize(this.score, { sampleRate: options.sampleRate ?? 44100, layers: options.layers });
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

  dispose(): void {
    void this.stop();
    if (this.lastUrl && typeof URL.revokeObjectURL === 'function') URL.revokeObjectURL(this.lastUrl);
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
      src.connect(ctx.destination);
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
    this.analysers = null;
  }

  private makeUrl(wav: ArrayBuffer): string {
    if (typeof Blob !== 'undefined' && typeof URL.createObjectURL === 'function') {
      if (this.lastUrl) URL.revokeObjectURL(this.lastUrl);
      this.lastUrl = URL.createObjectURL(new Blob([wav], { type: 'audio/wav' }));
      return this.lastUrl;
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
