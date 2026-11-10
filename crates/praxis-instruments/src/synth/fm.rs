//! FM keys.
//!
//! - `soft_keys` (melody, sustaining): an FM electric piano. A 1:1 modulator/carrier pair gives
//!   the body; its index starts higher with velocity and tone and relaxes over ~0.35 s, so the
//!   note blooms and then mellows. A second carrier modulated at 14x the note gives the short
//!   metallic tine at the strike (faded out where it would alias). The note decays like an
//!   electric piano (60 dB in `decay_s x sqrt(261.6 Hz / f)`).
//!   Parameters: `tone` 0..1 (0.4), `decay_s` 1..10 s (3.5), `release_s` 0.05..3 s (0.45).
//! - `fm_keys` (harmony, held): two carriers for chords, the note itself (modulated at 2:1) and a
//!   quieter octave 3 cents sharp (1:1), slightly apart in the stereo field; the index settles
//!   to a warm sustain.
//!   Parameters: `tone` 0..1 (0.4), `sustain` 0..1 (0.55), `release_s` 0.05..4 s (0.6).
//!
//! Carriers and modulators start at phase zero, so the onset is silent before the attack ramp;
//! the release fade comes last, so notes end in exact silence.

use super::common::{
    Params, Release, Swell, cents, key_decay, param, samples, t60_coeff, tone_amount, vel_gain,
};
use super::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::osc::FmOperator;
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, InstrumentKind};

const SOURCE: &str = "praxis synth (procedural)";
/// Voices are freed below this level (about -100 dBFS) once the attack is over.
const FREE_LEVEL: f64 = 1e-5;
const TINE_RATIO: f64 = 14.0;

fn op(sr: f64) -> FmOperator {
    FmOperator::new(sr as f32)
}

// ---------------------------------------------------------------------------------------------

/// The `soft_keys` preset.
#[derive(Clone, Debug)]
pub struct SoftKeys {
    tone: f32,
    decay_s: f32,
    release_s: f32,
    sr: f64,
}

#[derive(Clone, Debug)]
pub struct SoftKeysVoice {
    carrier: FmOperator,
    modulator: FmOperator,
    tine: FmOperator,
    tine_mod: FmOperator,
    index_peak: f32,
    index_floor: f32,
    index_env: f32,
    index_coeff: f32,
    tine_index: f32,
    tine_env: f32,
    tine_coeff: f32,
    tine_level: f32,
    amp: f64,
    amp_coeff: f64,
    gain: f64,
    env: Swell,
    release: Release,
    level: f64,
    done: bool,
}

impl Patch for SoftKeys {
    type Voice = SoftKeysVoice;

    fn info() -> PresetInfo {
        PresetInfo {
            kind: InstrumentKind::Synth,
            name: "soft_keys".into(),
            class: GeneratorClass::Melody,
            pitched: true,
            sustaining: true,
            range: (36, 96),
            params: vec![
                param("tone", 0.0, 1.0, 0.4, ""),
                param("decay_s", 1.0, 10.0, 3.5, "s"),
                param("release_s", 0.05, 3.0, 0.45, "s"),
            ],
            source: SOURCE.into(),
        }
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> SoftKeys {
        SoftKeys {
            tone: p[0],
            decay_s: p[1],
            release_s: p[2],
            sr: context_rate(ctx),
        }
    }

    fn polyphony(&self) -> usize {
        24
    }

    fn new_voice(&self) -> SoftKeysVoice {
        let sr = self.sr;
        SoftKeysVoice {
            carrier: op(sr),
            modulator: op(sr),
            tine: op(sr),
            tine_mod: op(sr),
            index_peak: 0.0,
            index_floor: 0.0,
            index_env: 0.0,
            index_coeff: 0.0,
            tine_index: 0.0,
            tine_env: 0.0,
            tine_coeff: 0.0,
            tine_level: 0.0,
            amp: 0.0,
            amp_coeff: 0.0,
            gain: 0.0,
            env: Swell::default(),
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

impl Voice<SoftKeys> for SoftKeysVoice {
    fn start(&mut self, patch: &SoftKeys, note: &Note) {
        let sr = note.sample_rate;
        let hz = note.hz as f32;
        let t = tone_amount(patch.tone, note.velocity, note.brightness);
        let mut rng = note.rng(0x50F7);
        for (o, ratio) in [
            (&mut self.carrier, 1.0),
            (&mut self.modulator, 1.0),
            (&mut self.tine, 1.0),
            (&mut self.tine_mod, TINE_RATIO as f32),
        ] {
            o.set_freq(hz);
            o.set_ratio(ratio);
            o.reset(0.0);
        }
        self.carrier.set_index(1.0);
        self.tine.set_index(1.0);
        self.modulator.set_feedback(0.0);
        self.index_peak = 0.35 + 1.9 * t + 0.1 * rng.next_bipolar();
        self.index_floor = 0.12 + 0.2 * t;
        self.index_env = 1.0;
        self.index_coeff = (-1.0 / (0.35 * sr)).exp() as f32;
        // The tine fades out where 14x the note would approach Nyquist.
        let room = ((0.45 * sr - TINE_RATIO * note.hz) / (0.1 * sr)).clamp(0.0, 1.0) as f32;
        self.tine_index = (0.6 + 1.6 * t) * room;
        self.tine_env = 1.0;
        self.tine_coeff = (-1.0 / (0.03 * sr)).exp() as f32;
        self.tine_level = 0.22 * (0.4 + 0.6 * t) * room;
        self.amp = 1.0;
        self.amp_coeff = t60_coeff(patch.decay_s as f64 * key_decay(note.hz), sr);
        self.gain = 0.5 * vel_gain(note.velocity) as f64 * (1.0 + 0.03 * rng.next_bipolar() as f64);
        self.env.start(0.002, 1.0, 1.0, sr);
        self.release.reset();
        self.level = self.gain;
        self.done = false;
    }

    fn release(&mut self, patch: &SoftKeys) {
        self.release.start(patch.release_s, patch.sr);
    }

    #[inline]
    fn tick(&mut self, _patch: &SoftKeys) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        self.modulator
            .set_index(self.index_floor + self.index_peak * self.index_env);
        self.index_env *= self.index_coeff;
        self.tine_mod.set_index(self.tine_index * self.tine_env);
        self.tine_env *= self.tine_coeff;
        let body = self.carrier.next_pm(self.modulator.next());
        let tine = self.tine.next_pm(self.tine_mod.next()) * self.tine_level * self.tine_env;
        let a = self.amp * self.env.next() as f64;
        self.amp *= self.amp_coeff;
        let g = self.gain * a * self.release.next() as f64;
        self.level = self.gain * self.amp * self.release.gain() as f64;
        if self.release.is_done() || (self.env.attacked() && self.level < FREE_LEVEL) {
            self.done = true;
        }
        let y = ((body + tine) as f64 * g) as f32;
        (y, y)
    }

    fn level(&self) -> f32 {
        self.level as f32
    }

    fn is_done(&self) -> bool {
        self.done
    }
}

// ---------------------------------------------------------------------------------------------

/// The `fm_keys` preset.
#[derive(Clone, Debug)]
pub struct FmKeys {
    tone: f32,
    sustain: f32,
    release_s: f32,
    sr: f64,
}

#[derive(Clone, Debug)]
pub struct FmKeysVoice {
    c1: FmOperator,
    m1: FmOperator,
    c2: FmOperator,
    m2: FmOperator,
    index1: (f32, f32),
    index2: (f32, f32),
    index_env: f32,
    index_coeff: f32,
    gain: f64,
    env: Swell,
    release: Release,
    done: bool,
}

impl Patch for FmKeys {
    type Voice = FmKeysVoice;

    fn info() -> PresetInfo {
        PresetInfo {
            kind: InstrumentKind::Synth,
            name: "fm_keys".into(),
            class: GeneratorClass::Harmony,
            pitched: true,
            sustaining: false,
            range: (36, 96),
            params: vec![
                param("tone", 0.0, 1.0, 0.4, ""),
                param("sustain", 0.0, 1.0, 0.55, ""),
                param("release_s", 0.05, 4.0, 0.6, "s"),
            ],
            source: SOURCE.into(),
        }
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> FmKeys {
        FmKeys {
            tone: p[0],
            sustain: p[1],
            release_s: p[2],
            sr: context_rate(ctx),
        }
    }

    fn polyphony(&self) -> usize {
        24
    }

    fn new_voice(&self) -> FmKeysVoice {
        let sr = self.sr;
        FmKeysVoice {
            c1: op(sr),
            m1: op(sr),
            c2: op(sr),
            m2: op(sr),
            index1: (0.0, 0.0),
            index2: (0.0, 0.0),
            index_env: 0.0,
            index_coeff: 0.0,
            gain: 0.0,
            env: Swell::default(),
            release: Release::default(),
            done: true,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.tone = value,
            1 => self.sustain = value,
            2 => self.release_s = value,
            _ => {}
        }
    }

    fn tail_samples(&self) -> usize {
        samples(self.release_s, self.sr, 1) as usize + 1
    }
}

impl Voice<FmKeys> for FmKeysVoice {
    fn start(&mut self, patch: &FmKeys, note: &Note) {
        let sr = note.sample_rate;
        let hz = note.hz as f32;
        let t = tone_amount(patch.tone, note.velocity, note.brightness);
        let octave = (2.0 * cents(3.0)) as f32;
        for (o, ratio) in [
            (&mut self.c1, 1.0),
            (&mut self.m1, 2.0),
            (&mut self.c2, octave),
            (&mut self.m2, octave),
        ] {
            o.set_freq(hz);
            o.set_ratio(ratio);
            o.reset(0.0);
        }
        self.c1.set_index(1.0);
        self.c2.set_index(0.35);
        // (attack peak, sustained) modulation indices.
        self.index1 = (0.5 + 1.3 * t, 0.25 + 0.5 * t);
        self.index2 = (0.3 + 0.8 * t, 0.15 + 0.3 * t);
        self.index_env = 1.0;
        self.index_coeff = (-1.0 / (0.5 * sr)).exp() as f32;
        self.gain = 0.45 * vel_gain(note.velocity) as f64;
        self.env.start(0.008, 1.5, patch.sustain, sr);
        self.release.reset();
        self.done = false;
    }

    fn release(&mut self, patch: &FmKeys) {
        self.release.start(patch.release_s, patch.sr);
    }

    #[inline]
    fn tick(&mut self, _patch: &FmKeys) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        let e = self.index_env;
        self.m1
            .set_index(self.index1.1 + (self.index1.0 - self.index1.1) * e);
        self.m2
            .set_index(self.index2.1 + (self.index2.0 - self.index2.1) * e);
        self.index_env *= self.index_coeff;
        let a = self.c1.next_pm(self.m1.next());
        let b = self.c2.next_pm(self.m2.next());
        let g = (self.gain * self.env.next() as f64 * self.release.next() as f64) as f32;
        if self.release.is_done() {
            self.done = true;
        }
        ((a + 0.75 * b) * g, (a + 1.25 * b) * g)
    }

    fn level(&self) -> f32 {
        (self.gain * self.release.gain() as f64) as f32
    }

    fn is_done(&self) -> bool {
        self.done
    }
}
