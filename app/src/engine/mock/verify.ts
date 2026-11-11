// Mock render verification, shaped like `verify_render` (contracts/sound-profile.md). Loudness,
// peak, clipping, loop seam and silence are measured on the mock render; tempo, key and grid come
// from the score (the mock does not detect them from audio) and say so in `measured`.

import type { RenderReport, VerifyCheck, VerifyReport } from '../EngineAdapter';
import type { NScore } from '../../score/types';
import { isUnpitchedClass, inKey, keyLabel, semitone } from '../../score/theory';
import { expandNotes, stepSeconds } from '../../score/timeline';
import { audibleLayers } from './synth';

const db = (x: number) => (x > 0 ? 20 * Math.log10(x) : -Infinity);

export function verifyMock(score: NScore, channels: Float32Array[], sr: number, report: RenderReport): VerifyReport {
  const checks: VerifyCheck[] = [];
  const add = (name: string, passed: boolean, measured: string, expected: string, inconclusive = false) =>
    checks.push({ name, passed, measured, expected, inconclusive });
  const { audible } = audibleLayers(score);
  const events = expandNotes(score).filter((e) => audible.includes(e.layer));

  // Tempo (from the score in the mock).
  add('tempo', true, `${score.tempo.bpm.toFixed(1)} BPM (from the score; the mock does not detect tempo)`, `${score.tempo.bpm.toFixed(1)} ± 2 BPM`);

  // Key: every pitched note in key (chromatic and unpitched scores skip).
  const pitchedIds = new Set(score.layers.filter((l) => !isUnpitchedClass(l.class)).map((l) => l.id));
  const pitched = events.filter((e) => pitchedIds.has(e.layer));
  const keyName = keyLabel(score.key).toLowerCase();
  if (score.key.mode !== 'chromatic' && pitched.length) {
    const off = pitched.filter((e) => !inKey(score.key, e.pitch)).length;
    // As verify_render: with no third (or minor sixth) over the tonic, major vs minor can't be heard.
    const tonic = semitone(score.key.tonic);
    const pcs = new Set(pitched.map((e) => (((e.pitch - tonic) % 12) + 12) % 12));
    const t = score.key.tonic;
    if (off === 0 && ![3, 4, 8].some((i) => pcs.has(i))) {
      add('key', false, `${keyName} (from the score); can't tell: no third, so ${t} minor vs ${t} major can't be heard`, keyName, true);
    } else {
      add('key', off === 0, off === 0 ? `${keyName} (all notes in key; from the score)` : `${off} notes outside ${keyName}`, keyName);
    }
  }

  // Grid: share of notes within 10 ms of a step, and the median deviation.
  const devs = events.map((e) => {
    const step = stepSeconds(score, e.grid);
    const pos = e.startS / step;
    return Math.abs(pos - Math.round(pos)) * step * 1000;
  });
  const onGrid = devs.length ? devs.filter((d) => d <= 10).length / devs.length : 1;
  const sorted = [...devs].sort((a, b) => a - b);
  const median = sorted.length ? sorted[Math.floor(sorted.length / 2)] : 0;
  add('grid', onGrid >= 0.9, `${Math.round(onGrid * 100)}% of notes within 10 ms (from the score)`, '>= 90% within 10 ms');

  // Loudness and peak (mock estimates on the render).
  const target = score.master.loudness?.target_lufs ?? -16;
  const ceiling = score.master.loudness?.true_peak_dbtp ?? -1;
  const lufs = report.loudness_lufs;
  add('loudness', Number.isFinite(lufs) && Math.abs(lufs - target) <= 0.5, `${fmt(lufs)} LUFS (mock RMS estimate)`, `${target.toFixed(1)} ± 0.5 LUFS`);
  let peak = 0;
  let clipped = 0;
  for (const ch of channels) {
    for (const x of ch) {
      const a = Math.abs(x);
      if (a > peak) peak = a;
      if (a >= 1) clipped++;
    }
  }
  const peakDb = db(peak);
  add('true_peak', peakDb <= ceiling + 0.05, `${fmt(peakDb)} dB (mock sample peak)`, `<= ${ceiling.toFixed(1)} dBTP`);
  add('clipping', clipped === 0, clipped ? `${clipped} samples at full scale` : 'no sample at full scale', 'no sample at full scale');

  // Loop seam: the jump from the last sample to the first against ordinary jumps near the seam.
  let seamRatio: number | null = null;
  if (score.playback.mode === 'loop' && channels[0]?.length > 2) {
    const n = channels[0].length;
    const win = Math.max(8, Math.round(sr * 0.05));
    let worst = 0;
    let ordinary = 1e-9;
    for (const ch of channels) {
      worst = Math.max(worst, Math.abs(ch[0] - ch[n - 1]));
      for (let i = 1; i < win; i++) {
        ordinary = Math.max(ordinary, Math.abs(ch[i] - ch[i - 1]), Math.abs(ch[n - i] - ch[n - i - 1]));
      }
    }
    seamRatio = worst / ordinary;
    const secs = n / sr;
    add('loop_seam', seamRatio <= 1, seamRatio <= 1 ? `no click at ${clock(secs)}` : `jump ${seamRatio.toFixed(2)}x the ordinary at ${clock(secs)}`, 'no larger than ordinary jumps');
  }

  // Silence: sound where the score has notes.
  let sum = 0;
  let count = 0;
  for (const ch of channels) {
    for (let i = 0; i < ch.length; i += 4) {
      sum += ch[i] * ch[i];
      count++;
    }
  }
  const rmsDb = db(Math.sqrt(sum / Math.max(1, count)));
  if (events.length) add('silence', rmsDb > -70, rmsDb > -70 ? 'sound where the score has notes' : 'silent render', 'not silent where notes play');

  return {
    checks,
    tempo_bpm: score.tempo.bpm,
    key: score.key.mode === 'chromatic' ? null : keyName,
    grid_deviation_ms: median,
    onsets_on_grid: onGrid,
    integrated_lufs: Number.isFinite(lufs) ? lufs : null,
    true_peak_dbtp: Number.isFinite(peakDb) ? peakDb : null,
    seam_jump_ratio: seamRatio,
    seam_excess_db: seamRatio === null ? null : db(seamRatio),
    passed: checks.every((c) => c.passed || c.inconclusive),
  };
}

function fmt(v: number): string {
  return Number.isFinite(v) ? v.toFixed(1) : '-inf';
}

export function clock(s: number): string {
  const m = Math.floor(s / 60);
  return `${m}:${String(Math.round(s - m * 60)).padStart(2, '0')}`;
}
