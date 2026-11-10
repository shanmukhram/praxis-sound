//! `noise_bed`: a filtered-noise ambience bed (class ambience, unpitched, held notes).
//!
//! Two decorrelated pink-noise channels (seeded from the note), each through two band-passes
//! and a low-passed body. The band-pass centres sit at 2x and 5x the note frequency (the note
//! colours the bed; it is not a pitch) and drift slowly, half an octave at `drift` 1, on seeded
//! low-frequency sines, so the bed breathes instead of hissing. `tone`, velocity and bright
//! accents open the filters. Filter coefficients update every 16 samples on the voice's own
//! clock, so output does not depend on block size.
//!
//! A slow attack and the release fade (applied last) keep it click-free; it ends in exact
//! silence.
//!
//! Parameters: `tone` 0..1 (0.35), `drift` 0..1 (0.5), `attack_s` 0.05..4 s (0.8),
//! `release_s` 0.1..8 s (1.6).

use super::common::{Params, Release, Swell, param, samples, tone_amount, vel_gain};
use super::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::filter::{Svf, SvfMode};
use praxis_dsp::noise::Pink;
use praxis_dsp::rng::derive;
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, InstrumentKind};
use std::f64::consts::TAU;

const UPDATE: u32 = 16;
const LFO_HZ: [f64; 2] = [0.071, 0.113];

/// The `noise_bed` preset.
#[derive(Clone, Debug)]
pub struct NoiseBed {
    tone: f32,
    drift: f32,
    attack_s: f32,
    release_s: f32,
    sr: f64,
}

#[derive(Clone, Debug)]
pub struct NoiseBedVoice {
    noise: [Pink; 2],
    /// Per channel: band 1, band 2, body low-pass.
    filters: [[Svf; 3]; 2],
    centres: [f64; 2],
    body_hz: f64,
    lfo: [f64; 2],
    lfo_inc: [f64; 2],
    depth: f64,
    count: u32,
    gain: f64,
    env: Swell,
    release: Release,
    done: bool,
}

impl Patch for NoiseBed {
    type Voice = NoiseBedVoice;

    fn info() -> PresetInfo {
        PresetInfo {
            kind: InstrumentKind::Synth,
            name: "noise_bed".into(),
            class: GeneratorClass::Ambience,
            pitched: false,
            sustaining: false,
            range: (24, 108),
            params: vec![
                param("tone", 0.0, 1.0, 0.35, ""),
                param("drift", 0.0, 1.0, 0.5, ""),
                param("attack_s", 0.05, 4.0, 0.8, "s"),
                param("release_s", 0.1, 8.0, 1.6, "s"),
            ],
            source: "praxis synth (procedural)".into(),
        }
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> NoiseBed {
        NoiseBed {
            tone: p[0],
            drift: p[1],
            attack_s: p[2],
            release_s: p[3],
            sr: context_rate(ctx),
        }
    }

    fn polyphony(&self) -> usize {
        6
    }

    fn new_voice(&self) -> NoiseBedVoice {
        let sr = self.sr as f32;
        let bp = || {
            let mut f = Svf::new(sr);
            f.set_mode(SvfMode::BandPass);
            f
        };
        let lp = || {
            let mut f = Svf::new(sr);
            f.set_mode(SvfMode::LowPass);
            f
        };
        NoiseBedVoice {
            noise: [Pink::new(sr, 1), Pink::new(sr, 2)],
            filters: [[bp(), bp(), lp()], [bp(), bp(), lp()]],
            centres: [0.0; 2],
            body_hz: 0.0,
            lfo: [0.0; 2],
            lfo_inc: [0.0; 2],
            depth: 0.0,
            count: 0,
            gain: 0.0,
            env: Swell::default(),
            release: Release::default(),
            done: true,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.tone = value,
            1 => self.drift = value,
            2 => self.attack_s = value,
            3 => self.release_s = value,
            _ => {}
        }
    }

    fn tail_samples(&self) -> usize {
        samples(self.release_s, self.sr, 1) as usize + 1
    }
}

impl NoiseBedVoice {
    fn update_filters(&mut self) {
        for (ch, filters) in self.filters.iter_mut().enumerate() {
            for (b, f) in filters.iter_mut().take(2).enumerate() {
                // The two channels drift in opposite directions for width.
                let sign = if ch == 0 { 1.0 } else { -1.0 };
                let oct = self.depth * (TAU * self.lfo[b]).sin() * sign;
                let hz = (self.centres[b] * 2f64.powf(oct)).clamp(60.0, 12_000.0);
                f.set_cutoff_q(hz as f32, 1.4);
            }
        }
    }
}

impl Voice<NoiseBed> for NoiseBedVoice {
    fn start(&mut self, patch: &NoiseBed, note: &Note) {
        let sr = note.sample_rate;
        let t = tone_amount(patch.tone, note.velocity, note.brightness) as f64;
        let mut rng = note.rng(0xBED0);
        for (ch, n) in self.noise.iter_mut().enumerate() {
            n.reseed(derive(note.seed, 0xBED1 + ch as u64));
        }
        let open = 1.0 + 1.5 * t;
        let base = note.hz.clamp(40.0, 1500.0);
        self.centres = [base * 2.0 * open, base * 5.0 * open];
        self.body_hz = (300.0 + 3500.0 * t * t).min(0.45 * sr);
        for filters in self.filters.iter_mut() {
            for f in filters.iter_mut() {
                f.reset();
            }
            filters[2].set_cutoff_q(self.body_hz as f32, 0.6);
        }
        for (k, lfo) in self.lfo.iter_mut().enumerate() {
            *lfo = rng.next_f32() as f64;
            self.lfo_inc[k] = LFO_HZ[k] * (0.8 + 0.4 * rng.next_f32() as f64) / sr * UPDATE as f64;
        }
        self.depth = 0.5 * patch.drift as f64;
        self.count = 0;
        self.update_filters();
        self.gain = 1.6 * vel_gain(note.velocity) as f64;
        self.env.start(patch.attack_s, 1.0, 1.0, sr);
        self.release.reset();
        self.done = false;
    }

    fn release(&mut self, patch: &NoiseBed) {
        self.release.start(patch.release_s, patch.sr);
    }

    #[inline]
    fn tick(&mut self, _patch: &NoiseBed) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        self.count += 1;
        if self.count >= UPDATE {
            self.count = 0;
            for (lfo, inc) in self.lfo.iter_mut().zip(self.lfo_inc) {
                *lfo = (*lfo + inc).fract();
            }
            self.update_filters();
        }
        let mut out = [0.0f32; 2];
        for (ch, o) in out.iter_mut().enumerate() {
            let x = self.noise[ch].next();
            let [b1, b2, body] = &mut self.filters[ch];
            *o = b1.tick(x) + 0.6 * b2.tick(x) + 0.5 * body.tick(x);
        }
        let g = (self.gain * self.env.next() as f64 * self.release.next() as f64) as f32;
        if self.release.is_done() {
            self.done = true;
        }
        (out[0] * g, out[1] * g)
    }

    fn level(&self) -> f32 {
        (self.gain * self.release.gain() as f64) as f32
    }

    fn is_done(&self) -> bool {
        self.done
    }
}
