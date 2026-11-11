//! `drone`: a held, slowly moving tone (class drone, pitched, sustaining).
//!
//! Built like a recorded drone (air moving through a bowed or blown body in a large room), not a
//! static hum. Most of it is breath: decorrelated pink noise between ~0.7 and 2.5 times the note,
//! notched at the fundamental and the octave so the pitch stays exact, swelling with the slow
//! drift. Inside it sit the harmonics of the note (unit power, so brightness changes colour, not
//! level): the fundamental and a share of each harmonic as steady sines whose levels drift on
//! their own slow cycles (two per harmonic, 0.04..0.3 Hz, seeded phases, deeper for the upper
//! harmonics), detuned pairs on the lowest four harmonics (one left, one right) each beating at
//! its own slow rate, and harmonics 2..7 mostly as bowed resonances (the same noise through
//! narrow band-passes on them, drifting with them). Air higher up, a sub-bass on the note's pitch
//! class and a room with a long (3 s), soft decay complete it.
//!
//! **Loops cleanly.** When a note has a scheduled length, every oscillator, beat and drift is
//! locked to a whole number of cycles over that length (the tuning moves by at most 1.5 cents;
//! otherwise it runs free) and the noise restarts its seeded sequence every note length, so the
//! drone's source is exactly periodic with its note. Attack (`sin^2`) and release (`cos^2`) have
//! the same length and shape the room's output too, so when a looped drone note restarts while
//! the last one releases, the two sum to the steady drone: no dip, no click, no seam.
//!
//! Macros:
//! - `length`: attack and release time, 0.25..8 s (exponential; 0.5 = 1.41 s). The drone sounds
//!   for the note's length plus this release.
//! - `brightness`: harmonic roll-off, from `h^-3` (dark) to `h^-1` (bright), up to 9 kHz, the
//!   bowed share of the upper harmonics (80..95%), the air band (3 to 10 times the note
//!   frequency) and the room's damping.
//! - `weight`: the sub-bass (41..78 Hz, the note's pitch class) level (0 at weight 0).
//! - `pitch`: whole scale steps of the key, -7..+7 (0.5 = the note).

use super::common::{
    MAX_HARMONICS, Macros, PARTIAL_CEILING_HZ, Phasor, UPDATE, expmap, fall, harmonic_weights,
    harmonics, info, low_layer, midi_hz, pitch_steps, rise, step_in_key, to_samples,
};
use super::space::Room;
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
/// Drift rates (Hz, rounded to whole cycles per note) range.
const DRIFT_HZ: (f64, f64) = (0.04, 0.3);
/// Drift depth of the fundamental and of the highest harmonic (linear in between).
const DRIFT_DEPTH: (f64, f64) = (0.15, 0.6);
/// Detuned pairs: beat rate (Hz) of harmonics 1..4 and the pairs' level relative to the
/// harmonic they double.
const BEAT_HZ: [f64; PAIRS] = [0.13, 0.21, 0.29, 0.37];
const PAIRS: usize = 4;
const PAIR_LEVEL: f64 = 0.45;
/// Bowed resonances: harmonics 2..=7 of the note also sound as noise through narrow
/// band-passes (Q below), carrying this share of each harmonic's level (the rest stays a
/// steady sine); the fundamental stays a pure sine so the pitch is exact.
const BOWED: usize = 6;
const BOWED_Q: f64 = 14.0;
const BOWED_SHARE: (f64, f64) = (0.8, 0.95);
const LEVEL: f64 = 0.24;
const SUB_LEVEL: f64 = 0.45;
const AIR_LEVEL: f64 = 1.2;
const ROOM_WET: f64 = 0.8;
const ROOM_RT60: f64 = 3.0;
/// Breath: pink noise between ~0.7 and 2.5 times the note (at least 150..300 Hz), with
/// notches on the fundamental and the octave so the pitch stays exact, swelling with the slow
/// drift. It carries most of the drone; the harmonics sit inside it at this share.
const BREATH_LEVEL: f64 = 4.0;
const TONAL_SHARE: f64 = 0.45;

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
    pairs: [[Phasor; 2]; PAIRS],
    drift: [[Phasor; 2]; MAX_HARMONICS],
    drift_offset: [[f64; 2]; MAX_HARMONICS],
    drift_depth: [f64; MAX_HARMONICS],
    sub: Phasor,
    w: [f64; MAX_HARMONICS],
    wm: [f64; MAX_HARMONICS],
    nw: usize,
    sub_level: f64,
    bowed: [[Svf; BOWED]; 2],
    bowed_gain: [f64; BOWED],
    bowed_share: f64,
    air: [Svf; 2],
    breath: [[Svf; 6]; 2],
    noise: [Pink; 2],
    noise_seed: [u64; 2],
    room: Room,
    span: u64,
    fade: u64,
    i: u64,
    count: u32,
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

/// A slow phasor near `hz`: locked to whole cycles over `span` when `locked`, free otherwise.
fn slow(hz: f64, sr: f64, span: u64, locked: bool) -> Phasor {
    if locked {
        Phasor::cycles(whole_cycles(hz, sr, span), span)
    } else {
        Phasor::new(hz, sr, 0, 0.0)
    }
}

/// Breath filters: two high-passes, two low-passes and two notches (set per note).
fn breath_chain(sr: f32) -> [Svf; 6] {
    let mk = |mode| {
        let mut f = Svf::new(sr);
        f.set_mode(mode);
        f
    };
    [
        mk(SvfMode::HighPass),
        mk(SvfMode::HighPass),
        mk(SvfMode::LowPass),
        mk(SvfMode::LowPass),
        mk(SvfMode::Notch),
        mk(SvfMode::Notch),
    ]
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
            pairs: [[Phasor::default(); 2]; PAIRS],
            drift: [[Phasor::default(); 2]; MAX_HARMONICS],
            drift_offset: [[0.0; 2]; MAX_HARMONICS],
            drift_depth: [0.0; MAX_HARMONICS],
            sub: Phasor::default(),
            w: [0.0; MAX_HARMONICS],
            wm: [0.0; MAX_HARMONICS],
            nw: 0,
            sub_level: 0.0,
            bowed: [
                [bp(), bp(), bp(), bp(), bp(), bp()],
                [bp(), bp(), bp(), bp(), bp(), bp()],
            ],
            bowed_gain: [0.0; BOWED],
            bowed_share: 0.0,
            air: [bp(), bp()],
            breath: [breath_chain(sr), breath_chain(sr)],
            noise: [Pink::new(sr, 1), Pink::new(sr, 2)],
            noise_seed: [1, 2],
            room: Room::new(self.sr),
            span: 0,
            fade: 1,
            i: 0,
            count: 0,
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

impl DroneVoice {
    /// Drift multiplier of harmonic `h` (index) now: `1 + depth * mean of two slow sines`.
    #[inline]
    fn drift_of(&self, h: usize) -> f64 {
        let [a, b] = self.drift[h];
        let [oa, ob] = self.drift_offset[h];
        let s = (TAU * (a.phase() + oa)).sin() + (TAU * (b.phase() + ob)).sin();
        1.0 + self.drift_depth[h] * 0.5 * s
    }

    fn update(&mut self) {
        let keep = 1.0 - self.bowed_share;
        for h in 0..self.nw {
            let tonal = if (1..=BOWED).contains(&h) { keep } else { 1.0 };
            self.wm[h] = self.w[h] * self.drift_of(h) * tonal;
        }
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
        let locked = self.main.is_locked();
        let n = whole_cycles(hz, sr, span);
        for (h, pair) in self.pairs.iter_mut().enumerate() {
            let k = (h + 1) as u64;
            if locked {
                let beats = whole_cycles(BEAT_HZ[h], sr, span);
                *pair = [
                    Phasor::cycles(k * n + beats, span),
                    Phasor::cycles((k * n).saturating_sub(beats).max(1), span),
                ];
            } else {
                let f = hz * k as f64;
                *pair = [
                    Phasor::new(f + BEAT_HZ[h], sr, 0, 0.0),
                    Phasor::new((f - BEAT_HZ[h]).max(1.0), sr, 0, 0.0),
                ];
            }
        }
        for h in 0..MAX_HARMONICS {
            for j in 0..2 {
                let r = DRIFT_HZ.0 * (DRIFT_HZ.1 / DRIFT_HZ.0).powf(rng.next_f32() as f64);
                self.drift[h][j] = slow(r, sr, span, locked);
                self.drift_offset[h][j] = rng.next_f32() as f64;
            }
            let u = h as f64 / (MAX_HARMONICS - 1) as f64;
            self.drift_depth[h] = DRIFT_DEPTH.0 + (DRIFT_DEPTH.1 - DRIFT_DEPTH.0) * u;
        }
        let sub_hz = midi_hz(low_layer(pitch as i32 % 12) as f64);
        self.sub = Phasor::new(sub_hz, sr, span, LOCK_CENTS);
        self.sub_level = SUB_LEVEL * m.weight as f64;

        let rolloff = 3.0 - 2.0 * bright;
        let (w, nw) = harmonic_weights(rolloff, hz, PARTIAL_CEILING_HZ.min(0.45 * sr), 12);
        self.w = w;
        self.nw = nw;

        self.span = if locked { span } else { 0 };
        self.noise_seed = [derive(note.seed, 0xD0_0002), derive(note.seed, 0xD0_0003)];
        // Pink noise (RMS ~0.25, equal power per octave over ~11 octaves) through a band-pass of
        // Q gives about 0.25 / sqrt(7.6 Q): scale each band to the harmonic it stands in for.
        let band_rms = 0.25 / (7.6 * BOWED_Q).sqrt();
        for ch in self.bowed.iter_mut() {
            for (k, f) in ch.iter_mut().enumerate() {
                f.reset();
                let fh = (hz * (k + 2) as f64).min(PARTIAL_CEILING_HZ);
                f.set_cutoff_q(fh as f32, BOWED_Q as f32);
            }
        }
        self.bowed_share = BOWED_SHARE.0 + (BOWED_SHARE.1 - BOWED_SHARE.0) * bright;
        for k in 0..BOWED {
            let wh = if k + 1 < nw { w[k + 1] } else { 0.0 };
            self.bowed_gain[k] = self.bowed_share * wh * std::f64::consts::FRAC_1_SQRT_2 / band_rms;
        }
        let air_hz = (hz * expmap(bright as f32, 3.0, 10.0)).min(8_000.0);
        for f in self.air.iter_mut() {
            f.reset();
            f.set_cutoff_q(air_hz as f32, 1.2);
        }
        let lo = (0.7 * hz).max(150.0);
        let hi = (2.5 * hz).max(300.0);
        for ch in self.breath.iter_mut() {
            for f in ch.iter_mut() {
                f.reset();
            }
            ch[0].set_cutoff_q(lo as f32, 0.54);
            ch[1].set_cutoff_q(lo as f32, 1.31);
            ch[2].set_cutoff_q(hi as f32, 0.54);
            ch[3].set_cutoff_q(hi as f32, 1.31);
            ch[4].set_cutoff_q(hz as f32, 3.0);
            ch[5].set_cutoff_q((2.0 * hz) as f32, 3.0);
        }
        self.room
            .set(ROOM_RT60, 0.95, expmap(bright as f32, 1800.0, 7000.0), sr);
        self.room.clear();
        self.fade = patch.fade_samples();
        self.i = 0;
        self.count = 0;
        self.released_at = u64::MAX;
        self.release_from = 0.0;
        self.env = 0.0;
        self.gain = LEVEL * vel_gain(note.velocity) as f64;
        self.update();
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
        self.count += 1;
        if self.count >= UPDATE {
            self.count = 0;
            self.update();
        }

        let centre = harmonics(TAU * self.main.phase(), &self.wm[..self.nw]);
        let (mut pl, mut pr) = (0.0, 0.0);
        for h in 0..PAIRS.min(self.nw) {
            let a = PAIR_LEVEL * self.wm[h];
            pl += a * (TAU * self.pairs[h][0].phase()).sin();
            pr += a * (TAU * self.pairs[h][1].phase()).sin();
        }
        let sub = self.sub_level * (TAU * self.sub.phase()).sin();
        // The noise restarts its sequence every note length (locked notes), so it loops exactly.
        if self.i == 0 || (self.span > 0 && self.i.is_multiple_of(self.span)) {
            self.noise[0].reseed(self.noise_seed[0]);
            self.noise[1].reseed(self.noise_seed[1]);
        }
        let swell = self.drift_of(2).max(0.0);
        let mut drifts = [0.0; BOWED];
        for (k, d) in drifts.iter_mut().enumerate() {
            *d = self.drift_of(k + 1);
        }
        let mut side = [0.0; 2];
        for (ch, s) in side.iter_mut().enumerate() {
            let nz = self.noise[ch].next();
            let mut bowed = 0.0;
            for (k, f) in self.bowed[ch].iter_mut().enumerate() {
                bowed += self.bowed_gain[k] * drifts[k] * f.tick(nz) as f64;
            }
            let air = AIR_LEVEL * self.air[ch].tick(nz) as f64 * swell;
            let mut b = nz;
            for f in self.breath[ch].iter_mut() {
                b = f.tick(b);
            }
            let breath = BREATH_LEVEL * b as f64 * (0.6 + 0.4 * swell);
            *s = TONAL_SHARE * (bowed + air) + breath;
        }
        self.main.advance();
        for p in self.pairs.iter_mut().flatten() {
            p.advance();
        }
        for p in self.drift.iter_mut().flatten() {
            p.advance();
        }
        self.sub.advance();
        self.i += 1;
        let dl = TONAL_SHARE * (centre + pl) + side[0] + sub;
        let dr = TONAL_SHARE * (centre + pr) + side[1] + sub;
        let (wl, wr) = self.room.tick(dl, dr);
        let g = self.gain * env;
        (
            ((dl + ROOM_WET * wl) * g) as f32,
            ((dr + ROOM_WET * wr) * g) as f32,
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
