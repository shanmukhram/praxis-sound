// A full Sound Profile as `praxis analyze` returns it, for the reference-clip tests.

import type { SoundProfile } from '../../../src/engine/reference';

/** A full profile as `praxis analyze` returns it (contracts/sound-profile.md). */
export function profile(over: Partial<SoundProfile> = {}): SoundProfile {
  return {
    version: 1,
    source: { file_name: 'bell loop.wav', duration_s: 30, sample_rate: 48000, channels: 2, pcm_hash: 'abc' },
    class: { kind: 'music', confidence: 0.86, scores: [0.86, 0.06, 0.08] },
    tempo: { bpm: 127.99, confidence: 0.62, beats_s: [0, 0.47, 0.94], downbeat_s: 0, beats_per_bar: 4 },
    key: { key: { tonic: 'A', mode: 'minor' }, confidence: 0.83, chroma: new Array(12).fill(0.5) },
    loudness: { integrated_lufs: -16.02, range_lu: 4.1, true_peak_dbtp: -1.1, sample_peak_dbfs: -1.3, rms_dbfs: -18.4, crest_db: 17.1 },
    envelope: { attack_s: 0.004, decay_s: 0.3, sustain: 0.45, release_s: 1.9, frames_db: [-60, -20, -14, -18, -30], hop_s: 0.01 },
    spectral: { centroid_hz: 1180, rolloff_hz: 3400, flatness: 0.08, bands_db: [-40, -30, -18, -10, -6, -5, -9, -16, -26, -38], tilt_db_per_oct: -4.2, centroid_track_hz: [1100, 1200] },
    groove: { grid: 'sixteenth', steps_per_bar: 16, step_weights: [1, 0.1, 0.3, 0.1, 0.5, 0.1, 0.3, 0.1, 0.9, 0.1, 0.3, 0.1, 0.5, 0.1, 0.3, 0.1], swing: 0.04, onsets_per_bar: 8 },
    onsets_s: [0, 0.47, 0.94, 1.41],
    pitch: { hop_s: 0.01, midi: [69, 69, null, 72] },
    tonality: 0.82,
    stereo: { width: 0.3, correlation: 0.7 },
    reverb: { rt60_s: 1.4, confidence: 0.7 },
    sections: [
      { start_s: 0, end_s: 15, label: 'A' },
      { start_s: 15, end_s: 30, label: 'B' },
    ],
    features: [],
    ...over,
  };
}
