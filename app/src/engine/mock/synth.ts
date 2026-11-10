// MOCK PREVIEW SYNTH. Sine tones for pitched layers and simple clicks/thumps for drums, so the
// transport, playhead, waveform and spectrogram have something to show. This is NOT the Praxis
// engine and does not follow its sound (audio-quality rule 7: real instruments use samples).

import type { RenderReport } from '../EngineAdapter';
import type { NScore } from '../../score/types';
import { isUnpitchedClass, midiToHz } from '../../score/theory';
import { expandNotes, scoreSeconds, tailSeconds } from '../../score/timeline';

export interface SynthOutput {
  channels: [Float32Array, Float32Array];
  sampleRate: number;
  report: RenderReport;
}

/** Layers that would sound: not muted, not blocked; if any is solo, only solo layers. */
export function audibleLayers(score: NScore, only?: string[]): { audible: string[]; blocked: string[] } {
  const avoid = score.constraints.avoid ?? [];
  const blocked = score.layers.filter((l) => avoid.includes(l.class)).map((l) => l.id);
  const anySolo = score.layers.some((l) => l.solo);
  const audible = score.layers
    .filter((l) => !blocked.includes(l.id) && !l.muted && (!anySolo || l.solo))
    .filter((l) => !only || only.includes(l.id))
    .map((l) => l.id);
  return { audible, blocked };
}

function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function hash(...parts: (string | number)[]): number {
  let h = 0x811c9dc5;
  for (const p of parts) {
    for (const ch of String(p)) {
      h ^= ch.charCodeAt(0);
      h = Math.imul(h, 0x01000193);
    }
    h ^= 0xff;
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

const dbToLin = (db: number) => Math.pow(10, db / 20);
const linToDb = (x: number) => (x > 0 ? 20 * Math.log10(x) : -Infinity);

export interface SynthOptions {
  sampleRate?: number;
  /** Render only these layers (stems). */
  layers?: string[];
  /**
   * Apply this master gain instead of normalising (like `master.frozen_gain_db`), so a scoped
   * edit cannot change the level of untouched bars.
   */
  gainDb?: number;
}

export function synthesize(score: NScore, opts: SynthOptions = {}): SynthOutput {
  const t0 = typeof performance !== 'undefined' ? performance.now() : 0;
  const sr = opts.sampleRate ?? 44100;
  const isLoop = score.playback.mode === 'loop';
  const length = Math.round(scoreSeconds(score) * sr);
  const tail = Math.round(tailSeconds(score) * sr);
  const total = Math.max(1, length + tail);
  const L = new Float32Array(total);
  const R = new Float32Array(total);
  // Each layer renders into its own buffer so its (mock) effects apply to it alone.
  const stems = new Map<string, [Float32Array, Float32Array]>();
  const { audible, blocked } = audibleLayers(score, opts.layers);
  const layerById = new Map(score.layers.map((l) => [l.id, l]));
  let notes = 0;

  for (const e of expandNotes(score)) {
    if (!audible.includes(e.layer)) continue;
    const layer = layerById.get(e.layer)!;
    notes++;
    if (!stems.has(layer.id)) stems.set(layer.id, [new Float32Array(total), new Float32Array(total)]);
    const [SL, SR] = stems.get(layer.id)!;
    const gain = dbToLin(layer.gain_db) * Math.pow(e.velocity, 1.5) * 0.25;
    const pan = Math.max(-1, Math.min(1, layer.pan));
    const gl = Math.cos(((pan + 1) * Math.PI) / 4) * gain;
    const gr = Math.sin(((pan + 1) * Math.PI) / 4) * gain;
    const start = Math.round(e.startS * sr);
    const drum = isUnpitchedClass(layer.class);
    const dur = drum ? 0.35 : e.lengthS + 0.3;
    const n = Math.round(dur * sr);
    const rand = rng(hash(score.seed, layer.seed_offset, e.layer, e.bar, e.stepInBar, e.pitch));
    const f = midiToHz(e.pitch);
    const bright = e.accent === 'bright' ? 0.35 : 0.12;
    // A seeded start phase per note, so repeated notes do not stack in phase (real strings and
    // samples never do); deterministic because it comes from the note's own seed.
    const phase0 = rand() * 2 * Math.PI;
    for (let i = 0; i < n; i++) {
      const t = i / sr;
      let x: number;
      if (drum) {
        x = drumSample(e.pitch, t, rand);
      } else {
        const att = Math.min(1, t / 0.004);
        const rel = t > e.lengthS ? Math.exp(-(t - e.lengthS) / 0.08) : 1;
        const env = att * Math.exp(-t / 1.1) * rel;
        const w = 2 * Math.PI * f * t + phase0;
        x = env * (Math.sin(w) + bright * Math.sin(2 * w) + 0.05 * Math.sin(3 * w));
      }
      let idx = start + i;
      if (isLoop) idx %= length;
      else if (idx >= total) break;
      SL[idx] += x * gl;
      SR[idx] += x * gr;
    }
  }

  // Mock layer effects: an `eq` with `lp_hz` becomes a one-pole low-pass (run twice on loops so
  // the filter state at the loop point is the steady state, as the engine's warm-up cycle does).
  for (const layer of score.layers) {
    const stem = stems.get(layer.id);
    if (!stem) continue;
    const lp = layer.effects.find((fx) => fx.kind === 'eq' && !fx.bypass && fx.params?.lp_hz !== undefined)?.params?.lp_hz;
    for (const ch of stem) {
      if (lp !== undefined) lowpass(ch, sr, lp, isLoop);
    }
    for (let i = 0; i < total; i++) {
      L[i] += stem[0][i];
      R[i] += stem[1][i];
    }
  }

  // Mock "master": an RMS loudness estimate normalised to the score's target, held under the
  // peak ceiling; or a frozen gain when given.
  let peak = 0;
  let sum = 0;
  for (let i = 0; i < total; i++) {
    peak = Math.max(peak, Math.abs(L[i]), Math.abs(R[i]));
    sum += L[i] * L[i] + R[i] * R[i];
  }
  const rms = Math.sqrt(sum / (2 * total));
  const before = linToDb(rms) - 0.691;
  const target = score.master.loudness?.target_lufs ?? -16;
  const ceiling = score.master.loudness?.true_peak_dbtp ?? -1;
  let gainDb = 0;
  if (opts.gainDb !== undefined) gainDb = opts.gainDb;
  else if (peak > 0 && score.master.loudness?.normalize !== false) {
    gainDb = target - before;
    if (linToDb(peak) + gainDb > ceiling - 0.1) gainDb = ceiling - 0.1 - linToDb(peak);
  }
  const g = dbToLin(gainDb);
  for (let i = 0; i < total; i++) {
    L[i] *= g;
    R[i] *= g;
  }
  const report: RenderReport = {
    mock: true,
    sample_rate: sr,
    length_samples: length,
    tail_samples: isLoop ? 0 : tail,
    is_loop: isLoop,
    loudness_before_lufs: before,
    loudness_lufs: peak > 0 ? before + gainDb : -Infinity,
    true_peak_dbtp: peak > 0 ? linToDb(peak) + gainDb : -Infinity,
    applied_gain_db: gainDb,
    warmup_cycles: isLoop ? 1 : 0,
    notes_rendered: notes,
    blocked_layers: blocked,
    render_seconds: (typeof performance !== 'undefined' ? performance.now() - t0 : 0) / 1000,
    warnings: [
      'mock preview: sine and click placeholders, not Praxis instruments or effects',
      'mock preview: loudness is an unweighted RMS estimate, not BS.1770 LUFS; peak is sample peak',
    ],
  };
  return { channels: [L, R], sampleRate: sr, report };
}

function lowpass(x: Float32Array, sr: number, hz: number, loop: boolean): void {
  const a = 1 - Math.exp((-2 * Math.PI * Math.max(20, hz)) / sr);
  let y = 0;
  if (loop) for (let i = 0; i < x.length; i++) y += a * (x[i] - y);
  for (let i = 0; i < x.length; i++) {
    y += a * (x[i] - y);
    x[i] = y;
  }
}

function drumSample(pitch: number, t: number, rand: () => number): number {
  if (pitch === 35 || pitch === 36) {
    // Pitch sweep 155 Hz -> 45 Hz; phase is the integral of the frequency.
    const phase = 2 * Math.PI * (45 * t + 110 * 0.03 * (1 - Math.exp(-t / 0.03)));
    return Math.sin(phase) * Math.exp(-t / 0.18) * Math.min(1, t / 0.002);
  }
  const noise = rand() * 2 - 1;
  if (pitch === 38 || pitch === 40) {
    return (0.7 * noise + 0.5 * Math.sin(2 * Math.PI * 185 * t)) * Math.exp(-t / 0.09);
  }
  if (pitch === 42 || pitch === 44) return noise * 0.5 * Math.exp(-t / 0.03);
  if (pitch === 46) return noise * 0.5 * Math.exp(-t / 0.2);
  return Math.sin(2 * Math.PI * 1800 * t) * Math.exp(-t / 0.01);
}
