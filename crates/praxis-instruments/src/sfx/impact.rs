//! `impact`: a tuned hit (class impact, pitched one-shot).
//!
//! A low body on the key's tonic (the tonic nearest the note, kept between C1 and B4, so the
//! note only picks the register) that drops into pitch from a fourth above over ~10 ms, a short
//! band-passed noise transient, a low-passed noise tail (decorrelated left and right) and a
//! sub-bass on the tonic. Everything starts with a 1 ms ramp and ends in exact silence at the
//! end of `length` (a `cos^2` fade over the last 25%). Note-offs do not cut it.
//!
//! Macros:
//! - `length`: total duration, 0.5..8 s (exponential; 0.5 = 2 s).
//! - `brightness`: transient band 1.2..6 kHz, tail low-pass 300..6000 Hz, body overtone level.
//! - `weight`: body level (0.15..1 x) and body length (25..90% of `length`), and the sub-bass
//!   level (0 at weight 0).
//! - `pitch`: the body moves between tonic and fifth: an octave down, a fourth down (the fifth
//!   below), the tonic, a fifth up, an octave up (0.5 = the tonic).

use super::common::{
    Macros, Phasor, dc_blocker, expmap, fall, fifth_pc, info, low_layer, midi_hz, pitch_steps,
    t60_coeff, to_samples, tonic_near, tonic_pc,
};
use crate::synth::common::{Params, vel_gain};
use crate::synth::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::filter::{DcBlocker, Svf, SvfMode};
use praxis_dsp::noise::White;
use praxis_dsp::rng::derive;
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, Key};
use std::f64::consts::TAU;

/// Shortest and longest impact, seconds.
pub const LENGTH_S: (f64, f64) = (0.5, 8.0);
/// Lowest and highest MIDI pitch of the body (C1..B4).
pub const BODY_MIDI: (i32, i32) = (24, 71);
/// The body starts this many semitones sharp and falls into tune.
const DROP_ST: f64 = 5.0;
const DROP_TAU_S: f64 = 0.010;
const ATTACK_S: f64 = 0.001;
/// The final fade covers the last quarter of the length.
const FADE_FROM: f64 = 0.75;
const LEVEL: f64 = 0.5;
const TRANSIENT_LEVEL: f64 = 0.55;
const TAIL_LEVEL: f64 = 0.10;
const SUB_LEVEL: f64 = 0.45;

/// The `impact` preset.
#[derive(Clone, Debug)]
pub struct Impact {
    m: Macros,
    key: Key,
    sr: f64,
}

#[derive(Clone, Debug)]
pub struct ImpactVoice {
    i: u64,
    n: u64,
    attack: u64,
    body_phase: f64,
    body_hz: f64,
    body_amp: f64,
    body_decay: f64,
    overtone: f64,
    sub: Phasor,
    sub_amp: f64,
    sub_decay: f64,
    low_dc: DcBlocker,
    drop_coeff: f64,
    drop: f64,
    noise: White,
    tail_noise: [White; 2],
    transient: Svf,
    transient_amp: f64,
    transient_decay: f64,
    tail: [Svf; 2],
    tail_amp: f64,
    tail_decay: f64,
    gain: f64,
    sr: f64,
    done: bool,
}

impl Impact {
    fn length_samples(&self) -> u64 {
        to_samples(expmap(self.m.length, LENGTH_S.0, LENGTH_S.1), self.sr, 2)
    }

    /// MIDI pitch of the body for a note: the nearest tonic, moved by the `pitch` macro along
    /// tonic / fifth positions, folded into [`BODY_MIDI`].
    pub fn body_pitch(key: &Key, pitch: u8, pitch_macro: f32) -> u8 {
        let tonic = tonic_near(key, pitch) as i32;
        let fifth = (fifth_pc(key) - tonic_pc(key)).rem_euclid(12);
        let offset = match pitch_steps(pitch_macro, 2) {
            -2 => -12,
            -1 => fifth - 12,
            0 => 0,
            1 => fifth,
            _ => 12,
        };
        let mut p = tonic + offset;
        while p > BODY_MIDI.1 {
            p -= 12;
        }
        while p < BODY_MIDI.0 {
            p += 12;
        }
        p as u8
    }
}

impl Patch for Impact {
    type Voice = ImpactVoice;

    fn info() -> PresetInfo {
        info("impact", GeneratorClass::Impact, true, false, (24, 96))
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> Impact {
        Impact {
            m: Macros::from_params(p),
            key: ctx.key,
            sr: context_rate(ctx),
        }
    }

    fn polyphony(&self) -> usize {
        4
    }

    fn new_voice(&self) -> ImpactVoice {
        let sr = self.sr as f32;
        let filter = |mode| {
            let mut f = Svf::new(sr);
            f.set_mode(mode);
            f
        };
        ImpactVoice {
            i: 0,
            n: 0,
            attack: 1,
            body_phase: 0.0,
            body_hz: 0.0,
            body_amp: 0.0,
            body_decay: 0.0,
            overtone: 0.0,
            sub: Phasor::default(),
            sub_amp: 0.0,
            sub_decay: 0.0,
            low_dc: dc_blocker(sr),
            drop_coeff: 0.0,
            drop: 0.0,
            noise: White::new(1),
            tail_noise: [White::new(2), White::new(3)],
            transient: filter(SvfMode::BandPass),
            transient_amp: 0.0,
            transient_decay: 0.0,
            tail: [filter(SvfMode::LowPass), filter(SvfMode::LowPass)],
            tail_amp: 0.0,
            tail_decay: 0.0,
            gain: 0.0,
            sr: self.sr,
            done: true,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        self.m.set(index, value);
    }

    fn tail_samples(&self) -> usize {
        self.length_samples() as usize + 1
    }
}

impl Voice<Impact> for ImpactVoice {
    fn start(&mut self, patch: &Impact, note: &Note) {
        let sr = note.sample_rate;
        let m = patch.m;
        let bright = m.note_brightness(note.brightness) as f64;
        let w = m.weight as f64;
        let mut rng = note.rng(0x1A_0001);
        self.noise.reseed(derive(note.seed, 0x1A_0002));
        self.tail_noise[0].reseed(derive(note.seed, 0x1A_0003));
        self.tail_noise[1].reseed(derive(note.seed, 0x1A_0004));
        self.sr = sr;
        self.n = patch.length_samples();
        self.attack = to_samples(ATTACK_S, sr, 1);
        let n = self.n as f64;

        let body = Impact::body_pitch(&patch.key, note.pitch, m.pitch);
        self.body_hz = midi_hz(body as f64);
        self.body_phase = 0.0;
        self.body_amp = 0.15 + 0.85 * w;
        self.body_decay = t60_coeff((0.25 + 0.65 * w) * n);
        self.overtone = 0.12 + 0.5 * bright;
        self.drop = DROP_ST;
        self.drop_coeff = t60_coeff(DROP_TAU_S * 6.9 * sr);

        let sub_hz = midi_hz(low_layer(tonic_pc(&patch.key)) as f64);
        self.sub = Phasor::new(sub_hz, sr, 0, 0.0);
        self.sub_amp = SUB_LEVEL * w;
        self.sub_decay = t60_coeff(0.5 * n);
        self.low_dc.reset();

        let centre = expmap(bright as f32, 1200.0, 6000.0) * (0.97 + 0.06 * rng.next_f32() as f64);
        self.transient.reset();
        self.transient.set_cutoff_q(centre as f32, 0.9);
        self.transient_amp = TRANSIENT_LEVEL * (0.6 + 0.4 * bright);
        self.transient_decay = t60_coeff(0.045 * sr);
        let tail_hz = expmap(bright as f32, 300.0, 6000.0);
        for f in self.tail.iter_mut() {
            f.reset();
            f.set_cutoff_q(tail_hz as f32, 0.6);
        }
        self.tail_amp = TAIL_LEVEL;
        self.tail_decay = t60_coeff(0.8 * n);

        self.gain = LEVEL * vel_gain(note.velocity) as f64;
        self.i = 0;
        self.done = false;
    }

    fn release(&mut self, _patch: &Impact) {
        // One-shot: the hit always rings to its end.
    }

    #[inline]
    fn tick(&mut self, _patch: &Impact) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        if self.i >= self.n {
            self.done = true;
            return (0.0, 0.0);
        }
        let x = self.i as f64 / self.n as f64;
        let env = if self.i < self.attack {
            self.i as f64 / self.attack as f64
        } else {
            1.0
        } * fall((x - FADE_FROM) / (1.0 - FADE_FROM));

        let ratio = 2f64.powf(self.drop / 12.0);
        self.drop *= self.drop_coeff;
        let th = TAU * self.body_phase;
        let body = self.body_amp * (th.sin() + self.overtone * (2.0 * th).sin());
        self.body_phase = (self.body_phase + self.body_hz * ratio / self.sr).fract();
        self.body_amp *= self.body_decay;
        let sub = self.sub_amp * (TAU * self.sub.phase()).sin();
        self.sub.advance();
        self.sub_amp *= self.sub_decay;
        let low = self.low_dc.tick((body + sub) as f32) as f64;

        let tr = self.transient_amp * self.transient.tick(self.noise.next()) as f64;
        self.transient_amp *= self.transient_decay;
        let tl = self.tail_amp * self.tail[0].tick(self.tail_noise[0].next()) as f64;
        let tr2 = self.tail_amp * self.tail[1].tick(self.tail_noise[1].next()) as f64;
        self.tail_amp *= self.tail_decay;

        let g = self.gain * env;
        self.i += 1;
        (((low + tr + tl) * g) as f32, ((low + tr + tr2) * g) as f32)
    }

    fn level(&self) -> f32 {
        if self.done {
            0.0
        } else {
            (self.gain * (self.body_amp + self.tail_amp + self.sub_amp)) as f32
        }
    }

    fn is_done(&self) -> bool {
        self.done
    }
}
