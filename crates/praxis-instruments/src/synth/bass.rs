//! Bass presets (class bass, held notes).
//!
//! - `sub_bass`: a sine with a slight 2nd harmonic (and a touch of 3rd as the tone opens).
//!   Parameters: `tone` 0..1 (0.3), `release_s` 0.02..2 s (0.12).
//! - `warm_bass`: two band-limited saws 3 cents either side of the note (same start phase) into a gently driven
//!   4-pole ladder low-pass; the cutoff starts open and settles over a quarter second.
//!   Parameters: `tone` 0..1 (0.4), `resonance` 0..0.8 (0.15), `release_s` 0.02..2 s (0.15).
//!
//! Both start at phase zero under a short attack ramp and end with the release fade, applied
//! after every filter, so they reach exact silence.

use super::common::{Params, Release, Swell, cents, param, samples, tone_amount, vel_gain};
use super::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::filter::Ladder;
use praxis_dsp::osc::PolyBlepSaw;
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, InstrumentKind};
use std::f64::consts::TAU;

const SOURCE: &str = "praxis synth (procedural)";

// ---------------------------------------------------------------------------------------------

/// The `sub_bass` preset.
#[derive(Clone, Debug)]
pub struct SubBass {
    tone: f32,
    release_s: f32,
    sr: f64,
}

#[derive(Clone, Debug, Default)]
pub struct SubBassVoice {
    phase: f64,
    inc: f64,
    h2: f64,
    h3: f64,
    gain: f64,
    env: Swell,
    release: Release,
    done: bool,
}

impl Patch for SubBass {
    type Voice = SubBassVoice;

    fn info() -> PresetInfo {
        PresetInfo {
            kind: InstrumentKind::Synth,
            name: "sub_bass".into(),
            class: GeneratorClass::Bass,
            pitched: true,
            sustaining: false,
            range: (24, 60),
            params: vec![
                param("tone", 0.0, 1.0, 0.3, ""),
                param("release_s", 0.02, 2.0, 0.12, "s"),
            ],
            source: SOURCE.into(),
        }
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> SubBass {
        SubBass {
            tone: p[0],
            release_s: p[1],
            sr: context_rate(ctx),
        }
    }

    fn polyphony(&self) -> usize {
        8
    }

    fn new_voice(&self) -> SubBassVoice {
        SubBassVoice {
            done: true,
            ..SubBassVoice::default()
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.tone = value,
            1 => self.release_s = value,
            _ => {}
        }
    }

    fn tail_samples(&self) -> usize {
        samples(self.release_s, self.sr, 1) as usize + 1
    }
}

impl Voice<SubBass> for SubBassVoice {
    fn start(&mut self, patch: &SubBass, note: &Note) {
        let t = tone_amount(patch.tone, note.velocity, note.brightness) as f64;
        self.phase = 0.0;
        self.inc = note.hz / note.sample_rate;
        self.h2 = 0.08 + 0.22 * t;
        self.h3 = 0.08 * t * t;
        self.gain = 0.7 * vel_gain(note.velocity) as f64 / (1.0 + 0.5 * (self.h2 + self.h3));
        self.env.start(0.006, 1.0, 1.0, note.sample_rate);
        self.release.reset();
        self.done = false;
    }

    fn release(&mut self, patch: &SubBass) {
        self.release.start(patch.release_s, patch.sr);
    }

    #[inline]
    fn tick(&mut self, _patch: &SubBass) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        let p = self.phase * TAU;
        // Cosine-phase upper harmonics would start at a non-zero value; sine phase keeps the
        // onset at zero.
        let y = p.sin() + self.h2 * (2.0 * p).sin() + self.h3 * (3.0 * p).sin();
        self.phase = (self.phase + self.inc).fract();
        let g = self.gain * self.env.next() as f64 * self.release.next() as f64;
        if self.release.is_done() {
            self.done = true;
        }
        let y = (y * g) as f32;
        (y, y)
    }

    fn level(&self) -> f32 {
        (self.gain * self.release.gain() as f64) as f32
    }

    fn is_done(&self) -> bool {
        self.done
    }
}

// ---------------------------------------------------------------------------------------------

/// The `warm_bass` preset.
#[derive(Clone, Debug)]
pub struct WarmBass {
    tone: f32,
    resonance: f32,
    release_s: f32,
    sr: f64,
}

#[derive(Clone, Debug)]
pub struct WarmBassVoice {
    saws: [PolyBlepSaw; 2],
    filter: Ladder,
    base_cut: f64,
    env_cut: f64,
    cut_env: f64,
    cut_coeff: f64,
    gain: f64,
    env: Swell,
    release: Release,
    done: bool,
}

impl Patch for WarmBass {
    type Voice = WarmBassVoice;

    fn info() -> PresetInfo {
        PresetInfo {
            kind: InstrumentKind::Synth,
            name: "warm_bass".into(),
            class: GeneratorClass::Bass,
            pitched: true,
            sustaining: false,
            range: (24, 60),
            params: vec![
                param("tone", 0.0, 1.0, 0.4, ""),
                param("resonance", 0.0, 0.8, 0.15, ""),
                param("release_s", 0.02, 2.0, 0.15, "s"),
            ],
            source: SOURCE.into(),
        }
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> WarmBass {
        WarmBass {
            tone: p[0],
            resonance: p[1],
            release_s: p[2],
            sr: context_rate(ctx),
        }
    }

    fn polyphony(&self) -> usize {
        8
    }

    fn new_voice(&self) -> WarmBassVoice {
        let sr = self.sr as f32;
        WarmBassVoice {
            saws: [PolyBlepSaw::new(sr), PolyBlepSaw::new(sr)],
            filter: Ladder::new(sr),
            base_cut: 200.0,
            env_cut: 0.0,
            cut_env: 0.0,
            cut_coeff: 0.0,
            gain: 0.0,
            env: Swell::default(),
            release: Release::default(),
            done: true,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.tone = value,
            1 => self.resonance = value,
            2 => self.release_s = value,
            _ => {}
        }
    }

    fn tail_samples(&self) -> usize {
        samples(self.release_s, self.sr, 1) as usize + 1
    }
}

impl Voice<WarmBass> for WarmBassVoice {
    fn start(&mut self, patch: &WarmBass, note: &Note) {
        let sr = note.sample_rate;
        let t = tone_amount(patch.tone, note.velocity, note.brightness) as f64;
        // Both saws start at the same seeded phase and drift apart slowly (an analogue-style
        // detune), so the pair never starts half-cancelled.
        let phase = note.rng(0xBA55).next_f32();
        for (saw, c) in self.saws.iter_mut().zip([-3.0, 3.0]) {
            saw.set_freq((note.hz * cents(c)) as f32);
            saw.reset(phase);
        }
        self.filter.reset();
        self.filter.set_resonance(patch.resonance);
        self.filter.set_drive(1.5);
        self.base_cut = (note.hz * (2.0 + 6.0 * t)).max(120.0);
        self.env_cut = note.hz * (2.0 + 14.0 * t);
        self.cut_env = 1.0;
        self.cut_coeff = (-1.0 / (0.25 * sr)).exp();
        self.gain = 0.9 * vel_gain(note.velocity) as f64;
        self.env.start(0.004, 0.6, 0.8, sr);
        self.release.reset();
        self.done = false;
    }

    fn release(&mut self, patch: &WarmBass) {
        self.release.start(patch.release_s, patch.sr);
    }

    #[inline]
    fn tick(&mut self, _patch: &WarmBass) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        let x = 0.5 * (self.saws[0].next() + self.saws[1].next());
        self.filter
            .set_cutoff((self.base_cut + self.env_cut * self.cut_env).min(18_000.0) as f32);
        self.cut_env *= self.cut_coeff;
        let y = self.filter.tick(x) as f64;
        let g = self.gain * self.env.next() as f64 * self.release.next() as f64;
        if self.release.is_done() {
            self.done = true;
        }
        let y = (y * g) as f32;
        (y, y)
    }

    fn level(&self) -> f32 {
        (self.gain * self.release.gain() as f64) as f32
    }

    fn is_done(&self) -> bool {
        self.done
    }
}
