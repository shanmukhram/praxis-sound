//! `riser`: tension that builds into a hit point (class riser, pitched).
//!
//! The riser is timed backwards from the end of the note: it builds over `length` and its last
//! sound is on the sample where the note's `length_samples` ends, so a riser note that ends on a
//! downbeat lands exactly there. A note longer than `length` waits in silence first; a shorter
//! note compresses the build into the note.
//!
//! Inside, built like a recorded riser rather than a sweep: evolving air (pink and white noise,
//! decorrelated left and right, through a low-pass / high-pass pair whose band climbs from
//! ~220 Hz to the brightness ceiling and narrows from two octaves to one as it climbs), stirred
//! by slow gusts (a smooth 1/f random control) and an accelerating gate (2 to 14 Hz, opening
//! fully over the last 80 ms); a tonal bed of two three-voice unisons, one holding the note,
//! one gliding from the note up to the nearest tonic or fifth at least a fourth above
//! (accelerating, landing on it at the end), whose voices slowly drift apart (2 to 14 cents,
//! spread across the field), lightly saturated, with a faint full-band air floor under it all;
//! and a sub-bass swell on the key's tonic; the glide fades in as it climbs. All of it plays
//! into a room (1 s decay). The level rises as `x^2`; over the last 40 ms everything
//! swells 6 dB more and the noise another 12 dB (the final pull into the hit, like a reversed
//! cymbal), and a 2 ms `cos^2` fade closes it, so the loudness peaks within a few ms of the end
//! and nothing clicks.
//!
//! Macros:
//! - `length`: build time in **beats** of the score tempo, 1..16 (exponential; 0.5 = 4 beats,
//!   one 4/4 bar).
//! - `brightness`: the noise band's ceiling, 2..9 kHz (exponential), the tones' harmonics and
//!   the room's damping.
//! - `weight`: the sub-bass swell (0 at weight 0).
//! - `pitch`: the start moves by whole scale steps of the key (-7..+7, 0.5 = the note; the
//!   landing follows); the noise band shifts continuously, an octave down at 0 to one up at 1.

use super::common::{
    Macros, NOISE_CEILING_HZ, Phasor, Tone, UPDATE, colour_shift, expmap, fall, info, landing,
    low_layer, midi_hz, pan_unity, pitch_steps, step_in_key, to_samples, tonic_pc,
};
use super::space::{Room, Wander, soft_clip};
use crate::synth::common::{Params, vel_gain};
use crate::synth::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::filter::{Svf, SvfMode};
use praxis_dsp::noise::{Pink, White};
use praxis_dsp::rng::derive;
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, Key};
use std::f64::consts::TAU;

/// Shortest and longest build, beats.
pub const LENGTH_BEATS: (f64, f64) = (1.0, 16.0);
/// Closing fade, seconds.
pub const FADE_S: f64 = 0.002;
/// Final pull: over this many seconds before the end (`s` = 0..1) everything rises `1 + s^2`
/// times (6 dB) and the noise another `1 + 3 s^4` times (12 dB, steepest at the end), so the
/// riser peaks at its end.
pub const PULL_S: f64 = 0.040;
const NOISE_PULL: f64 = 3.0;
const PULL: f64 = 1.0;
/// Fade applied when a note-off arrives before the riser has finished (transport stop).
const STOP_S: f64 = 0.010;
/// The glide lands at least this many semitones above the start.
const MIN_GLIDE: i32 = 5;
const NOISE_START_HZ: f64 = 220.0;
const CEILING_HZ: (f64, f64) = (2000.0, 9000.0);
/// Noise band width in octaves at the start and the end of the build.
const BAND_OCT: (f64, f64) = (2.0, 1.0);
/// Unison: voice offsets (times the detune) and pans; the centre voice is the loudest, so the
/// tuning stays on the note.
const UNISON: [(f64, f64, f64); 3] = [(0.0, 1.0, 0.0), (-1.0, 0.6, -0.6), (1.0, 0.6, 0.6)];
const DETUNE_CENTS: (f64, f64) = (2.0, 14.0);
/// Accelerating gate: rate (Hz) at the start and end, depth, and the ungated final stretch.
const GATE_HZ: (f64, f64) = (2.0, 14.0);
const GATE_DEPTH: f64 = 0.35;
const GATE_OPEN_S: f64 = 0.080;
const GUST_HZ: f64 = 4.0;
const GUST_DEPTH: f64 = 0.25;
const LEVEL: f64 = 0.15;
const NOISE_LEVEL: f64 = 2.2;
const HOLD_LEVEL: f64 = 0.12;
/// The glide fades in as it climbs (`x` times this), so it is heard mostly near its landing.
const GLIDE_LEVEL: f64 = 0.16;
/// Share of the tonal bed the room hears (the noise goes in whole).
const TONE_SEND: f64 = 1.0;
/// A faint full-band air floor under the band (a recording's own air), relative to the band.
const FLOOR_LEVEL: f64 = 0.06;
const TONE_DRIVE: f64 = 1.5;
const SUB_LEVEL: f64 = 0.5;
const ROOM_WET: f64 = 0.9;

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
    open: u64,
    stop_at: u64,
    stop_len: u64,
    pink: [Pink; 2],
    white: [White; 2],
    lp: [[Svf; 2]; 2],
    hp: [[Svf; 2]; 2],
    noise_lo: f64,
    noise_hi: f64,
    gust: Wander,
    gate_phase: f64,
    hold: [Tone; 3],
    glide: [Tone; 3],
    pans: [(f64, f64); 3],
    start_hz: f64,
    glide_st: f64,
    sub: Phasor,
    sub_level: f64,
    room: Room,
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
        let filter = |mode| {
            let mut f = Svf::new(sr);
            f.set_mode(mode);
            f
        };
        let pair = |mode| [filter(mode), filter(mode)];
        RiserVoice {
            i: 0,
            start: 0,
            n: 0,
            fade: 1,
            pull: 1,
            open: 1,
            stop_at: u64::MAX,
            stop_len: 1,
            pink: [Pink::new(sr, 1), Pink::new(sr, 2)],
            white: [White::new(3), White::new(4)],
            lp: [pair(SvfMode::LowPass), pair(SvfMode::LowPass)],
            hp: [pair(SvfMode::HighPass), pair(SvfMode::HighPass)],
            noise_lo: NOISE_START_HZ,
            noise_hi: CEILING_HZ.0,
            gust: Wander::default(),
            gate_phase: 0.0,
            hold: [Tone::default(); 3],
            glide: [Tone::default(); 3],
            pans: [(1.0, 1.0); 3],
            start_hz: 0.0,
            glide_st: 0.0,
            sub: Phasor::default(),
            sub_level: 0.0,
            room: Room::new(self.sr),
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
    fn update(&mut self, x: f64, gust: f64) {
        let c = self.noise_lo
            * (self.noise_hi / self.noise_lo).powf(x.powf(1.5))
            * 2f64.powf(0.15 * gust);
        let half = 0.5 * (BAND_OCT.0 + (BAND_OCT.1 - BAND_OCT.0) * x);
        let top = (c * 2f64.powf(half)).min(NOISE_CEILING_HZ);
        let bottom = (c / 2f64.powf(half)).max(30.0);
        for ch in 0..2 {
            self.lp[ch][0].set_cutoff_q(top as f32, 0.54);
            self.lp[ch][1].set_cutoff_q(top as f32, 1.31);
            self.hp[ch][0].set_cutoff_q(bottom as f32, 0.54);
            self.hp[ch][1].set_cutoff_q(bottom as f32, 1.31);
        }
    }
}

impl Voice<Riser> for RiserVoice {
    fn start(&mut self, patch: &Riser, note: &Note) {
        let sr = note.sample_rate;
        let m = patch.m;
        let bright = m.note_brightness(note.brightness);
        self.sr = sr;
        self.pink[0].reseed(derive(note.seed, 0x21_0001));
        self.pink[1].reseed(derive(note.seed, 0x21_0002));
        self.white[0].reseed(derive(note.seed, 0x21_0003));
        self.white[1].reseed(derive(note.seed, 0x21_0004));
        self.gust.start(derive(note.seed, 0x21_0005), GUST_HZ, sr);
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
        self.open = to_samples(GATE_OPEN_S, sr, 1).min(self.n / 2).max(1);
        self.stop_len = to_samples(STOP_S, sr, 1);
        self.gate_phase = 0.0;

        let shift = colour_shift(m.pitch);
        self.noise_hi = (expmap(bright, CEILING_HZ.0, CEILING_HZ.1) * shift).min(NOISE_CEILING_HZ);
        self.noise_lo = NOISE_START_HZ * shift;
        for f in self.lp.iter_mut().chain(self.hp.iter_mut()).flatten() {
            f.reset();
        }

        let from = step_in_key(&patch.key, note.pitch, pitch_steps(m.pitch, 7));
        let to = landing(&patch.key, from, MIN_GLIDE);
        self.start_hz = midi_hz(from as f64);
        self.glide_st = to as f64 - from as f64;
        let rolloff = 1.8 - 1.0 * bright as f64;
        let wide = 2f64.powf(DETUNE_CENTS.1 / 1200.0);
        for k in 0..3 {
            self.hold[k].start(rolloff, self.start_hz * wide, sr, 8);
            self.glide[k].start(rolloff, midi_hz(to as f64) * wide, sr, 8);
            self.pans[k] = pan_unity(UNISON[k].2);
        }

        let sub_hz = midi_hz(low_layer(tonic_pc(&patch.key)) as f64);
        self.sub = Phasor::new(sub_hz, sr, 0, 0.0);
        self.sub_level = SUB_LEVEL * m.weight as f64;

        self.room.set(1.0, 0.7, expmap(bright, 2500.0, 9000.0), sr);
        self.room.clear();

        self.gain = LEVEL * vel_gain(note.velocity) as f64;
        self.env = 0.0;
        self.count = 0;
        self.update(0.0, 0.0);
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
        let gust = self.gust.next();
        self.count += 1;
        if self.count >= UPDATE {
            self.count = 0;
            self.update(x, gust);
        }
        let build = x * x;
        let pull_from = self.n - self.pull;
        let (mut noise_pull, mut pull) = (1.0, 1.0);
        if j >= pull_from {
            let s = (j - pull_from) as f64 / self.pull as f64;
            let s2 = s * s;
            noise_pull += NOISE_PULL * s2 * s2;
            pull += PULL * s2;
        }
        let mut close = 1.0;
        let fade_from = self.n - self.fade;
        if j >= fade_from {
            close *= fall((j - fade_from) as f64 / self.fade as f64);
        }
        if self.stop_at != u64::MAX {
            close *= fall((self.i - self.stop_at) as f64 / self.stop_len as f64);
        }
        self.env = build * close;

        // Gate: accelerating, fading out over the last stretch so the end is fully open.
        let rate = GATE_HZ.0 + (GATE_HZ.1 - GATE_HZ.0) * x * x;
        self.gate_phase = (self.gate_phase + rate / self.sr).fract();
        let open_from = self.n - self.open;
        let depth = if j >= open_from {
            GATE_DEPTH * x * fall((j - open_from) as f64 / self.open as f64)
        } else {
            GATE_DEPTH * x
        };
        let gate = 1.0 - depth * (0.5 + 0.5 * (TAU * self.gate_phase).cos());
        let flutter = (GUST_DEPTH * gust - 0.5 * GUST_DEPTH * GUST_DEPTH).exp();

        let mut nz = [0.0; 2];
        for (ch, v) in nz.iter_mut().enumerate() {
            let p = self.pink[ch].next();
            let src = 0.6 * p + 0.25 * self.white[ch].next();
            let a = self.lp[ch][0].tick(src);
            let a = self.lp[ch][1].tick(a);
            let b = self.hp[ch][0].tick(a);
            *v = self.hp[ch][1].tick(b) as f64 + FLOOR_LEVEL * p as f64;
        }

        let cents = DETUNE_CENTS.0 + (DETUNE_CENTS.1 - DETUNE_CENTS.0) * x;
        let glide_hz = self.start_hz * 2f64.powf(self.glide_st * x * x * x / 12.0);
        let (mut tl, mut tr) = (0.0, 0.0);
        for k in 0..3 {
            let (off, amp, _) = UNISON[k];
            let d = 2f64.powf(off * cents / 1200.0);
            let v = HOLD_LEVEL * self.hold[k].next(self.start_hz * d, self.sr)
                + GLIDE_LEVEL * x * self.glide[k].next(glide_hz * d, self.sr);
            let (pl, pr) = self.pans[k];
            tl += amp * v * pl;
            tr += amp * v * pr;
        }
        let sub = self.sub_level * (TAU * self.sub.phase()).sin();
        self.sub.advance();

        let nk = NOISE_LEVEL * flutter * gate;
        let (tl, tr) = (soft_clip(tl, TONE_DRIVE), soft_clip(tr, TONE_DRIVE));
        let (nl, nr) = (nk * noise_pull * nz[0], nk * noise_pull * nz[1]);
        let (wl, wr) = self
            .room
            .tick((TONE_SEND * tl + nl) * build, (TONE_SEND * tr + nr) * build);
        let dl = (tl + sub + nl) * build;
        let dr = (tr + sub + nr) * build;
        let g = self.gain * pull * close;
        self.i += 1;
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
