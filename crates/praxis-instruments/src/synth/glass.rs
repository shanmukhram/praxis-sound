//! `glass`: a morphing wavetable tone (class motif, sustaining).
//!
//! A three-frame wavetable (glassy: strong odd harmonics; softer; nearly a sine) morphs from
//! bright to dark as the note rings, like struck glass losing its upper partials. A quiet sine
//! an octave up, 0.6..1.2 Hz sharp (seeded), beats slowly against the second harmonic. The table
//! is mip-mapped, so nothing aliases. `tone`, velocity and bright accents start the morph
//! brighter. The note decays 60 dB in `decay_s x sqrt(261.6 Hz / f)`.
//!
//! Parameters: `tone` 0..1 (0.45), `decay_s` 0.5..10 s (3.0), `release_s` 0.05..5 s (0.9).

use super::common::{
    Params, Partial, Ramp, Release, key_decay, param, samples, t60_coeff, tone_amount, vel_gain,
};
use super::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::osc::Wavetable;
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, InstrumentKind};

const FREE_LEVEL: f64 = 1e-5;
const MORPH_S: f64 = 0.6;

/// The `glass` preset.
#[derive(Clone, Debug)]
pub struct Glass {
    tone: f32,
    decay_s: f32,
    release_s: f32,
    sr: f64,
    table: Wavetable,
}

#[derive(Clone, Debug)]
pub struct GlassVoice {
    osc: Wavetable,
    octave: Partial,
    pos: f64,
    pos_coeff: f64,
    amp: f64,
    amp_coeff: f64,
    gain: f64,
    ramp: Ramp,
    release: Release,
    level: f64,
    done: bool,
}

fn frames() -> [Vec<f32>; 3] {
    let bright: Vec<f32> = (1..=24)
        .map(|k| {
            let odd = if k % 2 == 1 { 1.0 } else { 0.55 };
            odd * 0.82f32.powi(k - 1)
        })
        .collect();
    let soft: Vec<f32> = (1..=12).map(|k| 0.5f32.powi(k - 1)).collect();
    let dark = vec![1.0, 0.18, 0.05];
    [bright, soft, dark]
}

impl Patch for Glass {
    type Voice = GlassVoice;

    fn info() -> PresetInfo {
        PresetInfo {
            kind: InstrumentKind::Synth,
            name: "glass".into(),
            class: GeneratorClass::Motif,
            pitched: true,
            sustaining: true,
            range: (48, 108),
            params: vec![
                param("tone", 0.0, 1.0, 0.45, ""),
                param("decay_s", 0.5, 10.0, 3.0, "s"),
                param("release_s", 0.05, 5.0, 0.9, "s"),
            ],
            source: "praxis synth (procedural)".into(),
        }
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> Glass {
        let sr = context_rate(ctx);
        Glass {
            tone: p[0],
            decay_s: p[1],
            release_s: p[2],
            sr,
            table: Wavetable::from_frames(sr as f32, &frames()),
        }
    }

    fn polyphony(&self) -> usize {
        24
    }

    fn new_voice(&self) -> GlassVoice {
        GlassVoice {
            osc: self.table.clone(),
            octave: Partial::default(),
            pos: 1.0,
            pos_coeff: 0.0,
            amp: 0.0,
            amp_coeff: 0.0,
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

impl Voice<Glass> for GlassVoice {
    fn start(&mut self, patch: &Glass, note: &Note) {
        let sr = note.sample_rate;
        let t = tone_amount(patch.tone, note.velocity, note.brightness) as f64;
        let mut rng = note.rng(0x6A55);
        self.osc.set_freq(note.hz as f32);
        self.osc.reset(0.0);
        // Distance still to travel towards the dark frame (position 1).
        self.pos = 1.0 - 0.6 * (1.0 - t);
        self.pos_coeff = (-1.0 / (MORPH_S * sr)).exp();
        let t60 = patch.decay_s as f64 * key_decay(note.hz);
        let beat = rng.range(0.6, 1.2) as f64;
        self.octave
            .start(2.0 * note.hz + beat, sr, 0.0, 0.16 * (0.5 + t), 0.7 * t60);
        self.amp = 1.0;
        self.amp_coeff = t60_coeff(t60, sr);
        self.gain = 0.45 * vel_gain(note.velocity) as f64;
        self.ramp.start(0.003, sr);
        self.release.reset();
        self.level = self.gain;
        self.done = false;
    }

    fn release(&mut self, patch: &Glass) {
        self.release.start(patch.release_s, patch.sr);
    }

    #[inline]
    fn tick(&mut self, _patch: &Glass) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        self.osc.set_position((1.0 - self.pos) as f32);
        self.pos *= self.pos_coeff;
        let s = self.osc.next() as f64 * self.amp + self.octave.next();
        self.amp *= self.amp_coeff;
        let g = self.gain * self.ramp.next() as f64 * self.release.next() as f64;
        self.level = self.gain * self.amp * self.release.gain() as f64;
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
