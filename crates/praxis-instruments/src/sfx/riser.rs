//! `riser`: tension that builds into a hit point (class riser, pitched).
//!
//! The riser is timed backwards from the end of the note: it builds over `length` and its last
//! sound is on the sample where the note's `length_samples` ends, so a riser note that ends on a
//! downbeat lands exactly there. A note longer than `length` waits in silence first; a shorter
//! note compresses the build into the note.
//!
//! Inside: decorrelated left/right noise through a band-pass whose centre climbs from ~220 Hz
//! to the brightness ceiling (and whose Q narrows as it climbs), a tone holding the note, a
//! second tone gliding from the note up to the nearest tonic or fifth at least a fourth above
//! (accelerating, landing on it at the end) and a sub-bass swell on the key's tonic. The level
//! rises as `x^2`; over the last 40 ms everything swells 6 dB more and the noise another 16 dB
//! (the final pull into the hit, like a reversed cymbal), and a 2 ms `cos^2` fade closes it, so
//! the loudness peaks within a few ms of the end and nothing clicks.
//!
//! Macros:
//! - `length`: build time in **beats** of the score tempo, 1..16 (exponential; 0.5 = 4 beats,
//!   one 4/4 bar).
//! - `brightness`: the noise band's ceiling, 2..9 kHz (exponential), and the tones' harmonics.
//! - `weight`: the sub-bass swell (0 at weight 0).
//! - `pitch`: the start moves by whole scale steps of the key (-7..+7, 0.5 = the note; the
//!   landing follows); the noise band shifts continuously, an octave down at 0 to one up at 1.

use super::common::{
    Macros, Phasor, Tone, UPDATE, colour_shift, expmap, fall, info, landing, low_layer, midi_hz,
    pitch_steps, step_in_key, to_samples, tonic_pc,
};
use crate::synth::common::{Params, vel_gain};
use crate::synth::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::filter::{Svf, SvfMode};
use praxis_dsp::noise::White;
use praxis_dsp::rng::derive;
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, Key};
use std::f64::consts::TAU;

/// Shortest and longest build, beats.
pub const LENGTH_BEATS: (f64, f64) = (1.0, 16.0);
/// Closing fade, seconds.
pub const FADE_S: f64 = 0.002;
/// Final pull: over this many seconds before the end (`s` = 0..1) everything rises `1 + s^2`
/// times (6 dB) and the noise another `1 + 5 s^4` times (16 dB, steepest at the end), so the
/// riser peaks at its end.
pub const PULL_S: f64 = 0.040;
const NOISE_PULL: f64 = 5.0;
const PULL: f64 = 1.0;
/// Fade applied when a note-off arrives before the riser has finished (transport stop).
const STOP_S: f64 = 0.010;
/// The glide lands at least this many semitones above the start.
const MIN_GLIDE: i32 = 5;
const NOISE_START_HZ: f64 = 220.0;
const CEILING_HZ: (f64, f64) = (2000.0, 9000.0);
const LEVEL: f64 = 0.15;
const NOISE_LEVEL: f64 = 0.9;
const HOLD_LEVEL: f64 = 0.28;
const GLIDE_LEVEL: f64 = 0.36;
const SUB_LEVEL: f64 = 0.5;

/// The `riser` preset.
#[derive(Clone, Debug)]
pub struct Riser {
    m: Macros,
    key: Key,
    bpm: f64,
    sr: f64,
}

#[derive(Clone, Debug)]
pub struct RiserVoice {
    i: u64,
    /// First sample of the build.
    start: u64,
    /// Build length in samples (the sound ends at `start + n`).
    n: u64,
    fade: u64,
    pull: u64,
    stop_at: u64,
    stop_len: u64,
    noise: [White; 2],
    bands: [Svf; 2],
    noise_lo: f64,
    noise_hi: f64,
    hold: Tone,
    glide: Tone,
    start_hz: f64,
    glide_st: f64,
    sub: Phasor,
    sub_level: f64,
    gain: f64,
    env: f64,
    count: u32,
    sr: f64,
    done: bool,
}

impl Riser {
    /// Build length in samples at the current `length` macro and tempo.
    pub fn build_samples(&self) -> u64 {
        let beats = expmap(self.m.length, LENGTH_BEATS.0, LENGTH_BEATS.1);
        to_samples(beats * 60.0 / self.bpm, self.sr, 2)
    }
}

impl Patch for Riser {
    type Voice = RiserVoice;

    fn info() -> PresetInfo {
        info("riser", GeneratorClass::Riser, true, false, (24, 96))
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> Riser {
        let bpm = if ctx.bpm.is_finite() && ctx.bpm > 0.0 {
            ctx.bpm.clamp(20.0, 400.0)
        } else {
            120.0
        };
        Riser {
            m: Macros::from_params(p),
            key: ctx.key,
            bpm,
            sr: context_rate(ctx),
        }
    }

    fn polyphony(&self) -> usize {
        2
    }

    fn new_voice(&self) -> RiserVoice {
        let sr = self.sr as f32;
        let bp = || {
            let mut f = Svf::new(sr);
            f.set_mode(SvfMode::BandPass);
            f
        };
        RiserVoice {
            i: 0,
            start: 0,
            n: 0,
            fade: 1,
            pull: 1,
            stop_at: u64::MAX,
            stop_len: 1,
            noise: [White::new(1), White::new(2)],
            bands: [bp(), bp()],
            noise_lo: NOISE_START_HZ,
            noise_hi: CEILING_HZ.0,
            hold: Tone::default(),
            glide: Tone::default(),
            start_hz: 0.0,
            glide_st: 0.0,
            sub: Phasor::default(),
            sub_level: 0.0,
            gain: 0.0,
            env: 0.0,
            count: 0,
            sr: self.sr,
            done: true,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        self.m.set(index, value);
    }

    fn tail_samples(&self) -> usize {
        to_samples(STOP_S, self.sr, 1) as usize + 1
    }
}

impl RiserVoice {
    fn update(&mut self, x: f64) {
        let c = self.noise_lo * (self.noise_hi / self.noise_lo).powf(x.powf(1.5));
        let q = 0.8 + 2.2 * x;
        for f in self.bands.iter_mut() {
            f.set_cutoff_q(c as f32, q as f32);
        }
    }
}

impl Voice<Riser> for RiserVoice {
    fn start(&mut self, patch: &Riser, note: &Note) {
        let sr = note.sample_rate;
        let m = patch.m;
        let bright = m.note_brightness(note.brightness);
        self.sr = sr;
        self.noise[0].reseed(derive(note.seed, 0x21_0001));
        self.noise[1].reseed(derive(note.seed, 0x21_0002));
        let total = note.length_samples;
        self.n = patch.build_samples().min(total);
        self.start = total - self.n;
        self.i = 0;
        self.stop_at = u64::MAX;
        if self.n < 2 {
            self.done = true;
            return;
        }
        self.fade = to_samples(FADE_S, sr, 1).min(self.n / 4).max(1);
        self.pull = to_samples(PULL_S, sr, 1).min(self.n / 4).max(1);
        self.stop_len = to_samples(STOP_S, sr, 1);

        let shift = colour_shift(m.pitch);
        self.noise_hi = (expmap(bright, CEILING_HZ.0, CEILING_HZ.1) * shift)
            .min(super::common::NOISE_CEILING_HZ);
        self.noise_lo = NOISE_START_HZ * shift;
        for f in self.bands.iter_mut() {
            f.reset();
        }

        let from = step_in_key(&patch.key, note.pitch, pitch_steps(m.pitch, 7));
        let to = landing(&patch.key, from, MIN_GLIDE);
        self.start_hz = midi_hz(from as f64);
        self.glide_st = to as f64 - from as f64;
        let rolloff = 2.4 - 1.6 * bright as f64;
        self.hold.start(rolloff, self.start_hz, sr, 8);
        self.glide.start(rolloff, midi_hz(to as f64), sr, 8);

        let sub_hz = midi_hz(low_layer(tonic_pc(&patch.key)) as f64);
        self.sub = Phasor::new(sub_hz, sr, 0, 0.0);
        self.sub_level = SUB_LEVEL * m.weight as f64;

        self.gain = LEVEL * vel_gain(note.velocity) as f64;
        self.env = 0.0;
        self.count = 0;
        self.update(0.0);
        self.done = false;
    }

    fn release(&mut self, _patch: &Riser) {
        // A note-off on time arrives after the riser has ended; an early one (transport stop)
        // fades it out quickly.
        if !self.done && self.stop_at == u64::MAX && self.i + self.fade < self.start + self.n {
            self.stop_at = self.i;
        }
    }

    #[inline]
    fn tick(&mut self, _patch: &Riser) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        let end = self.start + self.n;
        if self.i >= end || (self.stop_at != u64::MAX && self.i >= self.stop_at + self.stop_len) {
            self.done = true;
            return (0.0, 0.0);
        }
        if self.i < self.start {
            self.i += 1;
            return (0.0, 0.0);
        }
        let j = self.i - self.start;
        let x = j as f64 / self.n as f64;
        self.count += 1;
        if self.count >= UPDATE {
            self.count = 0;
            self.update(x);
        }
        let mut a = x * x;
        let pull_from = self.n - self.pull;
        let (mut noise_pull, mut pull) = (1.0, 1.0);
        if j >= pull_from {
            let s = (j - pull_from) as f64 / self.pull as f64;
            let s2 = s * s;
            noise_pull += NOISE_PULL * s2 * s2;
            pull += PULL * s2;
        }
        let fade_from = self.n - self.fade;
        if j >= fade_from {
            a *= fall((j - fade_from) as f64 / self.fade as f64);
        }
        if self.stop_at != u64::MAX {
            a *= fall((self.i - self.stop_at) as f64 / self.stop_len as f64);
        }
        self.env = a;

        let nl = self.bands[0].tick(self.noise[0].next()) as f64;
        let nr = self.bands[1].tick(self.noise[1].next()) as f64;
        let hold = HOLD_LEVEL * self.hold.next(self.start_hz, self.sr);
        let glide_hz = self.start_hz * 2f64.powf(self.glide_st * x * x * x / 12.0);
        let glide = GLIDE_LEVEL * self.glide.next(glide_hz, self.sr);
        let sub = self.sub_level * (TAU * self.sub.phase()).sin();
        self.sub.advance();
        let mid = (hold + glide + sub) * pull;
        let nk = NOISE_LEVEL * noise_pull * pull;
        let g = self.gain * a;
        self.i += 1;
        (((mid + nk * nl) * g) as f32, ((mid + nk * nr) * g) as f32)
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
