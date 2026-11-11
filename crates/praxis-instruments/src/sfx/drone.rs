//! `drone`: a held, slowly moving tone (class drone, pitched, sustaining).
//!
//! Additive harmonics on the note (unit power, so brightness changes colour, not level), two
//! slow amplitude movements (one on the even harmonics, one on the odd ones above the
//! fundamental), a pair of detuned copies of the lowest two harmonics (one left, one right)
//! that beat slowly against the centre, a breathing air layer (decorrelated pink noise through
//! band-passes 3..10 harmonics up, swelling with the odd-harmonic movement) and a sub-bass on the
//! note's pitch class.
//!
//! **Loops cleanly.** When a note has a scheduled length, every oscillator and movement is
//! locked to a whole number of cycles over that length (the tuning moves by at most 1.5 cents;
//! otherwise it runs free) and the air noise restarts its seeded sequence every note length, so
//! the drone is exactly periodic with its note. Attack (`sin^2`) and
//! release (`cos^2`) have the same length, so when a looped drone note restarts while the last
//! one releases, the two sum to the steady drone: no dip, no click, no seam.
//!
//! Macros:
//! - `length`: attack and release time, 0.25..8 s (exponential; 0.5 = 1.41 s). The drone sounds
//!   for the note's length plus this release.
//! - `brightness`: harmonic roll-off, from `h^-2.6` (dark) to `h^-0.8` (bright), up to 9 kHz,
//!   and the air band, 3 to 10 times the note frequency.
//! - `weight`: the sub-bass (41..78 Hz, the note's pitch class) level (0 at weight 0).
//! - `pitch`: whole scale steps of the key, -7..+7 (0.5 = the note).

use super::common::{
    MAX_HARMONICS, Macros, PARTIAL_CEILING_HZ, Phasor, expmap, fall, harmonic_weights, harmonics,
    info, low_layer, midi_hz, pitch_steps, rise, step_in_key, to_samples,
};
use crate::synth::common::{Params, vel_gain};
use crate::synth::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::filter::{Svf, SvfMode};
use praxis_dsp::noise::Pink;
use praxis_dsp::rng::derive;
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, Key};
use std::f64::consts::TAU;

/// Shortest and longest attack/release, seconds.
pub const LENGTH_S: (f64, f64) = (0.25, 8.0);
/// Largest retuning (cents) allowed to lock an oscillator to the note length.
pub const LOCK_CENTS: f64 = 1.5;
/// Approximate rate of the slow movements, Hz (rounded to whole cycles per note).
const MOVE_HZ: [f64; 2] = [0.11, 0.07];
/// Approximate beat rate of the detuned copies, Hz.
const BEAT_HZ: f64 = 0.13;
const MOVE_DEPTH: f64 = 0.35;
const DETUNE_LEVEL: f64 = 0.28;
const LEVEL: f64 = 0.24;
const SUB_LEVEL: f64 = 0.45;
const AIR_LEVEL: f64 = 0.45;

/// The `drone` preset.
#[derive(Clone, Debug)]
pub struct Drone {
    m: Macros,
    key: Key,
    sr: f64,
}

#[derive(Clone, Debug)]
pub struct DroneVoice {
    main: Phasor,
    detune: [Phasor; 2],
    moves: [Phasor; 2],
    move_offset: [f64; 2],
    sub: Phasor,
    w: [f64; MAX_HARMONICS],
    nw: usize,
    sub_level: f64,
    air: [Svf; 2],
    noise: [Pink; 2],
    noise_seed: [u64; 2],
    span: u64,
    fade: u64,
    i: u64,
    released_at: u64,
    release_from: f64,
    env: f64,
    gain: f64,
    done: bool,
}

impl Drone {
    fn fade_samples(&self) -> u64 {
        to_samples(expmap(self.m.length, LENGTH_S.0, LENGTH_S.1), self.sr, 1)
    }
}

/// Whole cycles over `span` samples closest to `hz` (at least one).
fn whole_cycles(hz: f64, sr: f64, span: u64) -> u64 {
    ((hz * span as f64 / sr).round() as u64).max(1)
}

impl Patch for Drone {
    type Voice = DroneVoice;

    fn info() -> PresetInfo {
        info("drone", GeneratorClass::Drone, true, true, (24, 84))
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> Drone {
        Drone {
            m: Macros::from_params(p),
            key: ctx.key,
            sr: context_rate(ctx),
        }
    }

    fn polyphony(&self) -> usize {
        4
    }

    fn new_voice(&self) -> DroneVoice {
        let sr = self.sr as f32;
        let bp = || {
            let mut f = Svf::new(sr);
            f.set_mode(SvfMode::BandPass);
            f
        };
        DroneVoice {
            main: Phasor::default(),
            detune: [Phasor::default(); 2],
            moves: [Phasor::default(); 2],
            move_offset: [0.0; 2],
            sub: Phasor::default(),
            w: [0.0; MAX_HARMONICS],
            nw: 0,
            sub_level: 0.0,
            air: [bp(), bp()],
            noise: [Pink::new(sr, 1), Pink::new(sr, 2)],
            noise_seed: [1, 2],
            span: 0,
            fade: 1,
            i: 0,
            released_at: u64::MAX,
            release_from: 0.0,
            env: 0.0,
            gain: 0.0,
            done: true,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        self.m.set(index, value);
    }

    fn tail_samples(&self) -> usize {
        self.fade_samples() as usize + 1
    }
}

impl Voice<Drone> for DroneVoice {
    fn start(&mut self, patch: &Drone, note: &Note) {
        let sr = note.sample_rate;
        let m = patch.m;
        let bright = m.note_brightness(note.brightness) as f64;
        let span = note.length_samples;
        let mut rng = note.rng(0xD0_0001);

        let pitch = step_in_key(&patch.key, note.pitch, pitch_steps(m.pitch, 7));
        let hz = midi_hz(pitch as f64);
        self.main = Phasor::new(hz, sr, span, LOCK_CENTS);
        if self.main.is_locked() {
            let n = whole_cycles(hz, sr, span);
            let beats = whole_cycles(BEAT_HZ, sr, span);
            self.detune = [
                Phasor::cycles(n + beats, span),
                Phasor::cycles(n.saturating_sub(beats).max(1), span),
            ];
            self.moves = [
                Phasor::cycles(whole_cycles(MOVE_HZ[0], sr, span), span),
                Phasor::cycles(whole_cycles(MOVE_HZ[1], sr, span), span),
            ];
        } else {
            let beat = BEAT_HZ;
            self.detune = [
                Phasor::new(hz + beat, sr, 0, 0.0),
                Phasor::new((hz - beat).max(1.0), sr, 0, 0.0),
            ];
            self.moves = [
                Phasor::new(MOVE_HZ[0], sr, 0, 0.0),
                Phasor::new(MOVE_HZ[1], sr, 0, 0.0),
            ];
        }
        self.move_offset = [rng.next_f32() as f64, rng.next_f32() as f64];
        let sub_hz = midi_hz(low_layer(pitch as i32 % 12) as f64);
        self.sub = Phasor::new(sub_hz, sr, span, LOCK_CENTS);
        self.sub_level = SUB_LEVEL * m.weight as f64;

        let rolloff = 2.6 - 1.8 * bright;
        let (w, nw) = harmonic_weights(rolloff, hz, PARTIAL_CEILING_HZ.min(0.45 * sr), 12);
        self.w = w;
        self.nw = nw;

        self.span = if self.main.is_locked() { span } else { 0 };
        self.noise_seed = [derive(note.seed, 0xD0_0002), derive(note.seed, 0xD0_0003)];
        let air_hz = (hz * expmap(bright as f32, 3.0, 10.0)).min(8_000.0);
        for f in self.air.iter_mut() {
            f.reset();
            f.set_cutoff_q(air_hz as f32, 1.2);
        }
        self.fade = patch.fade_samples();
        self.i = 0;
        self.released_at = u64::MAX;
        self.release_from = 0.0;
        self.env = 0.0;
        self.gain = LEVEL * vel_gain(note.velocity) as f64;
        self.done = false;
    }

    fn release(&mut self, _patch: &Drone) {
        if self.released_at == u64::MAX {
            self.released_at = self.i;
            self.release_from = self.env;
        }
    }

    #[inline]
    fn tick(&mut self, _patch: &Drone) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        let env = if self.released_at == u64::MAX {
            rise(self.i as f64 / self.fade as f64)
        } else {
            let k = self.i - self.released_at;
            if k >= self.fade {
                self.done = true;
                return (0.0, 0.0);
            }
            self.release_from * fall(k as f64 / self.fade as f64)
        };
        self.env = env;

        let theta = TAU * self.main.phase();
        let m0 = 1.0 + MOVE_DEPTH * (TAU * (self.moves[0].phase() + self.move_offset[0])).sin();
        let m1 = 1.0 + MOVE_DEPTH * (TAU * (self.moves[1].phase() + self.move_offset[1])).sin();
        // Harmonics with the slow movements: even harmonics follow m0, odd ones above the
        // fundamental follow m1.
        let mut w = self.w;
        for (h, wh) in w.iter_mut().enumerate().take(self.nw).skip(1) {
            *wh *= if (h + 1) % 2 == 0 { m0 } else { m1 };
        }
        let centre = harmonics(theta, &w[..self.nw]);
        let lo2 = &self.w[..self.nw.min(2)];
        let dl = harmonics(TAU * self.detune[0].phase(), lo2);
        let dr = harmonics(TAU * self.detune[1].phase(), lo2);
        let sub = self.sub_level * (TAU * self.sub.phase()).sin();
        // The air restarts its sequence every note length (locked notes), so it loops exactly.
        if self.i == 0 || (self.span > 0 && self.i.is_multiple_of(self.span)) {
            self.noise[0].reseed(self.noise_seed[0]);
            self.noise[1].reseed(self.noise_seed[1]);
        }
        let breath = AIR_LEVEL * (0.55 + 0.45 * (m1 - 1.0) / MOVE_DEPTH);
        let al = breath * self.air[0].tick(self.noise[0].next()) as f64;
        let ar = breath * self.air[1].tick(self.noise[1].next()) as f64;
        self.main.advance();
        self.detune[0].advance();
        self.detune[1].advance();
        self.moves[0].advance();
        self.moves[1].advance();
        self.sub.advance();
        self.i += 1;
        let g = self.gain * env;
        (
            ((centre + DETUNE_LEVEL * dl + al + sub) * g) as f32,
            ((centre + DETUNE_LEVEL * dr + ar + sub) * g) as f32,
        )
    }

    fn level(&self) -> f32 {
        if self.done {
            0.0
        } else {
            (self.gain * self.env.max(0.05)) as f32
        }
    }

    fn is_done(&self) -> bool {
        self.done
    }
}
