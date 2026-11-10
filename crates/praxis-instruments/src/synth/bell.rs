//! `bell`: a struck bell from inharmonic modal partials (class motif; the Phase 1 test tone).
//!
//! Partials, as ratios of the note frequency (the **reference partial is ratio 1.0**, tuned
//! exactly to the MIDI pitch; tests measure it):
//!
//! | ratio | role | level | ring (x reference) |
//! | --- | --- | --- | --- |
//! | 0.5 | hum (skipped below 30 Hz) | 0.20 | 1.25 |
//! | 1.0 | reference / strike note | 1.00 | 1.00 |
//! | 2.0 (+ beating twin) | octave | 0.42 | 0.60 |
//! | 2.756 (+ beating twin) | free-bar mode | 0.30 | 0.45 |
//! | 4.07, 5.40, 6.80 | upper modes | 0.16, 0.10, 0.06 | 0.32, 0.24, 0.18 |
//! | 8.93, 11.34 | strike shimmer | 0.08, 0.05 | 40 ms, 25 ms |
//!
//! The octave and 2.756 modes each have a quiet twin 0.6..1.4 Hz away (seeded per note), so the
//! tone beats slowly like a real bell. Partial phases are seeded per note. `tone`, velocity and
//! bright accents scale the upper modes and the shimmer; partials are faded out above 12 kHz and
//! dropped above 0.45 x the sample rate. The reference partial rings 60 dB in
//! `decay_s x sqrt(261.6 Hz / f)` (so high bells are shorter); a 1.5 ms attack ramp and the
//! release fade keep the strike and the end click-free.
//!
//! Parameters: `tone` 0..1 (0.5), `decay_s` 0.5..12 s (4.0), `release_s` 0.05..6 s (1.6).

use super::common::{
    MAX_PARAMS, Params, Partial, Ramp, Release, key_decay, param, samples, tone_amount, vel_gain,
};
use super::poly::{Note, Patch, Voice, context_rate};
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, InstrumentKind};

const POLYPHONY: usize = 32;
const ATTACK_S: f32 = 0.0015;
/// Summed partial amplitude at full velocity (the waveform peak is lower: phases are spread).
const PEAK: f64 = 0.6;
/// Voice freed below this summed partial amplitude (about -100 dBFS).
const FREE_LEVEL: f64 = 1e-5;

/// (ratio, level, ring factor, tone weight). Ring factor multiplies the reference T60.
const MODES: [(f64, f64, f64, f64); 7] = [
    (0.5, 0.20, 1.25, 0.0),
    (1.0, 1.00, 1.00, 0.0),
    (2.0, 0.42, 0.60, 0.6),
    (2.756, 0.30, 0.45, 1.0),
    (4.07, 0.16, 0.32, 1.0),
    (5.40, 0.10, 0.24, 1.0),
    (6.80, 0.06, 0.18, 1.0),
];
/// (ratio, level, T60 seconds) of the strike shimmer.
const SHIMMER: [(f64, f64, f64); 2] = [(8.93, 0.08, 0.04), (11.34, 0.05, 0.025)];
const PARTIALS: usize = MODES.len() + 2 + SHIMMER.len();

/// The `bell` preset.
#[derive(Clone, Debug)]
pub struct Bell {
    tone: f32,
    decay_s: f32,
    release_s: f32,
    sr: f64,
}

#[derive(Clone, Debug)]
pub struct BellVoice {
    parts: [Partial; PARTIALS],
    gain: f64,
    ramp: Ramp,
    release: Release,
    level: f64,
    done: bool,
}

impl Patch for Bell {
    type Voice = BellVoice;

    fn info() -> PresetInfo {
        PresetInfo {
            kind: InstrumentKind::Synth,
            name: "bell".into(),
            class: GeneratorClass::Motif,
            pitched: true,
            sustaining: true,
            range: (48, 108),
            params: vec![
                param("tone", 0.0, 1.0, 0.5, ""),
                param("decay_s", 0.5, 12.0, 4.0, "s"),
                param("release_s", 0.05, 6.0, 1.6, "s"),
            ],
            source: "praxis synth (procedural)".into(),
        }
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> Bell {
        Bell {
            tone: p[0],
            decay_s: p[1],
            release_s: p[2],
            sr: context_rate(ctx),
        }
    }

    fn polyphony(&self) -> usize {
        POLYPHONY
    }

    fn new_voice(&self) -> BellVoice {
        BellVoice {
            parts: [Partial::default(); PARTIALS],
            gain: 0.0,
            ramp: Ramp::default(),
            release: Release::default(),
            level: 0.0,
            done: true,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.tone = value,
            1 => self.decay_s = value,
            2 => self.release_s = value,
            _ => {}
        }
    }

    fn tail_samples(&self) -> usize {
        samples(self.release_s, self.sr, 1) as usize + 1
    }
}

const _: () = assert!(3 <= MAX_PARAMS);

/// Gentle fade of partials towards the top of the audible range.
fn top_fade(hz: f64) -> f64 {
    ((16_000.0 - hz) / 4_000.0).clamp(0.0, 1.0)
}

impl Voice<Bell> for BellVoice {
    fn start(&mut self, patch: &Bell, note: &Note) {
        let sr = note.sample_rate;
        let f0 = note.hz;
        let t = tone_amount(patch.tone, note.velocity, note.brightness) as f64;
        let upper = 0.25 + 1.5 * t;
        let t60 = patch.decay_s as f64 * key_decay(f0);
        let mut rng = note.rng(0xBE11);
        let mut k = 0;
        let mut total = 0.0;
        for &(ratio, lvl, ring, weight) in MODES.iter() {
            let hz = f0 * ratio;
            let amp = if hz < 30.0 {
                0.0
            } else {
                lvl * (1.0 + weight * (upper - 1.0)) * top_fade(hz)
            };
            let phase = rng.next_f32() as f64;
            self.parts[k].start(hz, sr, phase, amp, t60 * ring);
            total += self.parts[k].amp();
            k += 1;
            if ratio == 2.0 || ratio == 2.756 {
                // A quiet twin a little sharp of the mode: slow beating, as in a real bell.
                let beat = rng.range(0.6, 1.4) as f64;
                let phase = rng.next_f32() as f64;
                self.parts[k].start(hz + beat, sr, phase, 0.35 * amp, t60 * ring * 0.9);
                total += self.parts[k].amp();
                k += 1;
            }
        }
        let strike = (0.3 + 0.7 * t) * (0.4 + 0.6 * note.velocity as f64);
        for &(ratio, lvl, t60s) in SHIMMER.iter() {
            let hz = f0 * ratio;
            let phase = rng.next_f32() as f64;
            self.parts[k].start(hz, sr, phase, lvl * strike * top_fade(hz), t60s);
            total += self.parts[k].amp();
            k += 1;
        }
        // Normalise by the summed partial levels so loudness follows velocity, not tone.
        self.gain = PEAK * vel_gain(note.velocity) as f64 / total.max(1e-9);
        self.ramp.start(ATTACK_S, sr);
        self.release.reset();
        self.level = total * self.gain;
        self.done = false;
    }

    fn release(&mut self, patch: &Bell) {
        self.release.start(patch.release_s, patch.sr);
    }

    #[inline]
    fn tick(&mut self, _patch: &Bell) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        let mut s = 0.0;
        let mut lvl = 0.0;
        for p in self.parts.iter_mut() {
            s += p.next();
            lvl += p.amp();
        }
        let g = self.gain * self.ramp.next() as f64 * self.release.next() as f64;
        self.level = lvl * self.gain * self.release.gain() as f64;
        if self.release.is_done() || (self.ramp.is_done() && self.level < FREE_LEVEL) {
            self.done = true;
        }
        let y = (s * g) as f32;
        (y, y)
    }

    fn level(&self) -> f32 {
        self.level as f32
    }

    fn is_done(&self) -> bool {
        self.done
    }
}
