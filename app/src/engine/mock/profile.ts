// The mock engine's Sound Profile: rough measurements in the browser (RMS envelope, onsets, a
// zero-crossing brightness, stereo width), labelled `mock: true`. The real profile is the
// analyzer's (crates/praxis-analyze, `praxis analyze`); this only lets the UI be built and tested
// without the desktop shell.

import type { DecodedAudio } from '../wav';
import { decodeWav } from '../wav';
import type { ClipKind, SoundProfile } from '../reference';

const HOP_S = 0.01;
const FLOOR_DB = -100;

const db = (x: number) => (x > 0 ? Math.max(FLOOR_DB, 20 * Math.log10(x)) : FLOOR_DB);

/** Decodes a WAV in plain TypeScript, anything else through WebAudio when the page has it. */
export async function decodeClip(bytes: ArrayBuffer): Promise<DecodedAudio | null> {
  try {
    return decodeWav(bytes);
  } catch {
    /* not a WAV */
  }
  const Ctx = (globalThis as { OfflineAudioContext?: typeof OfflineAudioContext }).OfflineAudioContext;
  if (!Ctx) return null;
  try {
    const ctx = new Ctx(2, 1, 44100);
    const buf = await ctx.decodeAudioData(bytes.slice(0));
    return { sampleRate: buf.sampleRate, channels: Array.from({ length: buf.numberOfChannels }, (_, c) => buf.getChannelData(c)) };
  } catch {
    return null;
  }
}

/** Words in a file name that say what the clip is, when the measurements cannot. */
function kindFromName(name: string): ClipKind | null {
  const n = name.toLowerCase();
  if (/(rain|wind|ambien|room ?tone|forest|crowd|traffic|drone|bed|atmos)/.test(n)) return 'ambience';
  if (/(hit|whoosh|impact|boom|click|swoosh|sting|riser|sfx|door|step)/.test(n)) return 'sfx';
  if (/(loop|music|song|beat|piano|groove|track)/.test(n)) return 'music';
  return null;
}

function percentile(xs: number[], q: number): number {
  if (!xs.length) return FLOOR_DB;
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.max(0, Math.floor(q * (s.length - 1))))];
}

/** A rough profile of decoded audio (or of the name alone when it could not be decoded). */
export function mockProfile(name: string, audio: DecodedAudio | null, bytes: number): SoundProfile {
  const sr = audio?.sampleRate ?? 48000;
  const chans = audio?.channels.length ? audio.channels : [new Float32Array(0)];
  const left = chans[0];
  const right = chans[1] ?? chans[0];
  const n = left.length;
  const duration = audio ? n / sr : Math.max(1, bytes / (sr * 4));
  const hop = Math.max(1, Math.round(HOP_S * sr));
  const frames: number[] = [];
  let peak = 0;
  let sumSq = 0;
  let zc = 0;
  let side = 0;
  let mid = 0;
  let lr = 0;
  let ll = 0;
  let rr = 0;
  for (let f = 0; f * hop < n; f++) {
    let e = 0;
    const end = Math.min(n, (f + 1) * hop);
    for (let i = f * hop; i < end; i++) {
      const m = 0.5 * (left[i] + right[i]);
      e += m * m;
      const a = Math.abs(left[i]) > Math.abs(right[i]) ? Math.abs(left[i]) : Math.abs(right[i]);
      if (a > peak) peak = a;
      if (i > 0 && (left[i] >= 0) !== (left[i - 1] >= 0)) zc++;
      const s = 0.5 * (left[i] - right[i]);
      side += s * s;
      mid += m * m;
      lr += left[i] * right[i];
      ll += left[i] * left[i];
      rr += right[i] * right[i];
    }
    sumSq += e;
    frames.push(db(Math.sqrt(e / Math.max(1, end - f * hop))));
  }
  const rms = n ? Math.sqrt(sumSq / n) : 0;
  const rmsDb = db(rms);
  const peakDb = db(peak);
  // Onsets: the envelope jumps 9 dB above its level 50 ms earlier, at most one per 80 ms.
  const onsets: number[] = [];
  let last = -1;
  for (let f = 5; f < frames.length; f++) {
    const before = Math.min(...frames.slice(f - 5, f));
    if (frames[f] > -60 && frames[f] - before >= 9 && (last < 0 || f - last >= 8)) {
      onsets.push((f * hop) / sr);
      last = f;
    }
  }
  const zcr = n ? zc / n : 0;
  const centroid = audio ? Math.min(sr / 2, Math.max(60, zcr * sr * 0.5)) : 1200;
  const loud = frames.filter((x) => x > -70);
  const range = loud.length ? percentile(loud, 0.95) - percentile(loud, 0.1) : 0;
  // Tempo from regular onsets.
  const gaps = onsets.slice(1).map((t, i) => t - onsets[i]);
  const med = gaps.length ? [...gaps].sort((a, b) => a - b)[Math.floor(gaps.length / 2)] : 0;
  const cv = gaps.length > 1 && med > 0 ? Math.sqrt(gaps.reduce((s, g) => s + (g - med) ** 2, 0) / gaps.length) / med : 1;
  let bpm = med > 0 ? 60 / med : 0;
  while (bpm > 0 && bpm < 60) bpm *= 2;
  while (bpm > 200) bpm /= 2;
  const steady = onsets.length >= 6 && cv < 0.25;
  const named = kindFromName(name);
  const measured: ClipKind = duration < 4 && onsets.length <= 3 ? 'sfx' : range < 8 && !steady ? 'ambience' : 'music';
  const kind = named ?? measured;
  const scores: [number, number, number] = kind === 'music' ? [0.7, 0.15, 0.15] : kind === 'sfx' ? [0.15, 0.7, 0.15] : [0.15, 0.15, 0.7];
  // A crude decay: from the loudest frame, how long the envelope takes to fall 20 dB, times 3.
  let top = 0;
  for (let f = 1; f < frames.length; f++) if (frames[f] > frames[top]) top = f;
  let fall = frames.length - 1;
  for (let f = top; f < frames.length; f++) {
    if (frames[f] <= frames[top] - 20) {
      fall = f;
      break;
    }
  }
  const hopS = hop / sr;
  const rt60 = Math.min(8, Math.max(0.05, (fall - top) * hopS * 3));
  const step = Math.max(1, Math.ceil(frames.length / 2000));
  const env = frames.filter((_, i) => i % step === 0);
  const bands = [31.5, 63, 125, 250, 500, 1000, 2000, 4000, 8000, 16000].map((fc) => -8 - 5 * Math.abs(Math.log2(fc / centroid)));
  const stepsPerBar = 16;
  const weights = new Array(stepsPerBar).fill(0);
  if (steady) {
    const stepS = 60 / bpm / 4;
    for (const t of onsets) weights[Math.round((t - onsets[0]) / stepS) % stepsPerBar] += 1;
  }
  const wmax = Math.max(1, ...weights);
  return {
    version: 1,
    mock: true,
    source: { file_name: name, duration_s: duration, sample_rate: sr, channels: chans.length, pcm_hash: `mock-${n}-${Math.round(rms * 1e6)}` },
    class: { kind, confidence: named ? 0.75 : 0.55, scores },
    tempo: steady ? { bpm, confidence: 0.6, beats_s: onsets, downbeat_s: onsets[0] ?? 0, beats_per_bar: 4 } : null,
    key: null,
    loudness: { integrated_lufs: rmsDb - 0.7, range_lu: range, true_peak_dbtp: peakDb, sample_peak_dbfs: peakDb, rms_dbfs: rmsDb, crest_db: peakDb - rmsDb },
    envelope: { attack_s: Math.max(0.001, top * hopS - (onsets[0] ?? 0)), decay_s: 0.2, sustain: range < 8 ? 0.8 : 0.3, release_s: rt60 / 3, frames_db: env, hop_s: hopS * step },
    spectral: { centroid_hz: centroid, rolloff_hz: Math.min(sr / 2, centroid * 2.5), flatness: Math.min(1, zcr * 4), bands_db: bands, tilt_db_per_oct: -3, centroid_track_hz: env.map(() => centroid) },
    groove: steady ? { grid: 'sixteenth', steps_per_bar: stepsPerBar, step_weights: weights.map((w) => w / wmax), swing: 0, onsets_per_bar: (onsets.length / Math.max(1, duration)) * ((60 / bpm) * 4) } : null,
    onsets_s: onsets,
    pitch: { hop_s: HOP_S, midi: [] },
    tonality: Math.max(0, Math.min(1, 1 - zcr * 6)),
    stereo: { width: mid > 0 ? side / mid : 0, correlation: ll > 0 && rr > 0 ? lr / Math.sqrt(ll * rr) : 1 },
    reverb: { rt60_s: rt60, confidence: 0.4 },
    sections: [{ start_s: 0, end_s: duration, label: 'A' }],
    features: [],
  };
}
