//! `tick`: a short clicky transient, usable as a clock (class tick, unpitched one-shot).
//!
//! A short modal click, like a small hard part of a mechanism striking another: a 0.15 ms contact
//! force rings six modes of a small body (spread from 0.55 to 2.4 times the click band, the
//! upper ones shorter, 3..22 ms), with the contact's grit (the pulse plus a little noise through a
//! resonant band-pass) on top; a short ping on the key's tonic or fifth (so even the click's ring
//! sits in key), an optional low knock on the tonic and the first part of a room around it. Every click has
//! the same shape: the attack starts on the note's first sample every time; the seed varies only
//! the level (+/-0.5 dB), the click band (+/-3%) and each mode's tuning (+/-0.4%), never the
//! timing, so a stream of ticks keeps time exactly. The tick ends in exact silence at the end of
//! `length` (a `cos^2` fade over its last 30%). Note-offs do not cut it, and the note's pitch is
//! ignored (tick is unpitched).
//!
//! Macros:
//! - `length`: total duration, 10..250 ms (exponential; 0.5 = 50 ms).
//! - `brightness`: click band centre 1.5..8 kHz (exponential; the modes follow), contact grit and
//!   the ping's overtone.
//! - `weight`: the knock, a 41..78 Hz tonic body (0 at weight 0).
//! - `pitch`: the ping moves over tonic / fifth positions from the tonic in octave 5 to the
//!   tonic in octave 7 (0.5 = the tonic in octave 6); the click band and modes shift
//!   continuously, an octave down at 0 to one up at 1.

use super::common::{
    Macros, Phasor, colour_shift, dc_blocker, expmap, fall, fifth_pc, info, low_layer, midi_hz,
    pitch_steps, t60_coeff, to_samples, tonic_pc,
};
use super::space::{Contact, Mode, Room};
use crate::synth::common::{Params, vel_gain};
use crate::synth::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::filter::{DcBlocker, Svf, SvfMode};
use praxis_dsp::noise::White;
use praxis_dsp::rng::derive;
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, Key};
use std::f64::consts::TAU;

/// Shortest and longest tick, seconds.
pub const LENGTH_S: (f64, f64) = (0.010, 0.250);
const ATTACK_S: f64 = 0.0004;
const FADE_FROM: f64 = 0.7;
const LEVEL: f64 = 0.45;
const CLICK_LEVEL: f64 = 3.0;
/// The click's excitation: a Hann pulse of this length and height, plus noise.
const PULSE_S: f64 = 0.00015;
const PULSE_LEVEL: f64 = 1.0;
const CLICK_NOISE: f64 = 0.25;
const PING_LEVEL: f64 = 0.12;
const KNOCK_LEVEL: f64 = 0.2;
/// Body modes: position in the spread (ratio to the click band), level, 60 dB decay (s).
const MODES: [(f64, f64, f64); 6] = [
    (0.55, 0.7, 0.022),
    (0.8, 1.0, 0.016),
    (1.1, 0.8, 0.012),
    (1.45, 0.6, 0.008),
    (1.9, 0.4, 0.005),
    (2.4, 0.25, 0.003),
];
const MODE_LEVEL: f64 = 0.6;
/// Contact pulse driving the modes, seconds.
const CONTACT_S: f64 = 0.00015;
const ROOM_WET: f64 = 0.4;

/// The `tick` preset.
#[derive(Clone, Debug)]
pub struct Tick {
    m: Macros,
    key: Key,
    sr: f64,
}

#[derive(Clone, Debug)]
pub struct TickVoice {
    i: u64,
    n: u64,
    attack: u64,
    noise: White,
    band: Svf,
    click_amp: f64,
    click_decay: f64,
    pulse: u64,
    ping: Phasor,
    ping_amp: f64,
    ping_decay: f64,
    overtone: f64,
    knock: Phasor,
    knock_amp: f64,
    knock_decay: f64,
    knock_dc: DcBlocker,
    contact: Contact,
    modes: [Mode; 6],
    mode_dc: DcBlocker,
    mode_last: f64,
    mode_scale: f64,
    room: Room,
    gain: f64,
    done: bool,
}

impl Tick {
    fn length_samples(&self) -> u64 {
        to_samples(expmap(self.m.length, LENGTH_S.0, LENGTH_S.1), self.sr, 4)
    }

    /// MIDI pitch of the ping for a `pitch` macro value.
    pub fn ping_pitch(key: &Key, pitch_macro: f32) -> u8 {
        let tonic = 84 + tonic_pc(key) - 12 * ((tonic_pc(key) >= 6) as i32);
        let fifth = (fifth_pc(key) - tonic_pc(key)).rem_euclid(12);
        let offset = match pitch_steps(pitch_macro, 2) {
            -2 => -12,
            -1 => fifth - 12,
            0 => 0,
            1 => fifth,
            _ => 12,
        };
        (tonic + offset).clamp(0, 127) as u8
    }
}

impl Patch for Tick {
    type Voice = TickVoice;

    fn info() -> PresetInfo {
        info("tick", GeneratorClass::Tick, false, false, (24, 96))
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> Tick {
        Tick {
            m: Macros::from_params(p),
            key: ctx.key,
            sr: context_rate(ctx),
        }
    }

    fn polyphony(&self) -> usize {
        8
    }

    fn new_voice(&self) -> TickVoice {
        let mut band = Svf::new(self.sr as f32);
        band.set_mode(SvfMode::BandPass);
        TickVoice {
            i: 0,
            n: 0,
            attack: 1,
            noise: White::new(1),
            band,
            click_amp: 0.0,
            click_decay: 0.0,
            pulse: 1,
            ping: Phasor::default(),
            ping_amp: 0.0,
            ping_decay: 0.0,
            overtone: 0.0,
            knock: Phasor::default(),
            knock_amp: 0.0,
            knock_decay: 0.0,
            knock_dc: dc_blocker(self.sr as f32),
            contact: Contact::default(),
            modes: [Mode::default(); 6],
            mode_dc: dc_blocker(self.sr as f32),
            mode_last: 0.0,
            mode_scale: 1.0,
            room: Room::new(self.sr),
            gain: 0.0,
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

impl Voice<Tick> for TickVoice {
    fn start(&mut self, patch: &Tick, note: &Note) {
        let sr = note.sample_rate;
        let m = patch.m;
        let bright = m.note_brightness(note.brightness);
        let mut rng = note.rng(0x71C_0001);
        self.noise.reseed(derive(note.seed, 0x71C_0002));
        self.n = patch.length_samples();
        let n = self.n as f64;
        self.attack = to_samples(ATTACK_S, sr, 1);

        let centre = expmap(bright, 1500.0, 8000.0)
            * colour_shift(m.pitch)
            * (0.97 + 0.06 * rng.next_f32() as f64);
        self.band.reset();
        self.band
            .set_cutoff_q(centre.min(super::common::NOISE_CEILING_HZ) as f32, 1.5);
        self.click_amp = CLICK_LEVEL;
        self.pulse = to_samples(PULSE_S, sr, 2);
        self.click_decay = t60_coeff((0.012 * sr).min(0.5 * n));

        let ping_hz = midi_hz(Tick::ping_pitch(&patch.key, m.pitch) as f64);
        self.ping = Phasor::new(ping_hz, sr, 0, 0.0);
        self.ping_amp = PING_LEVEL;
        self.ping_decay = t60_coeff(0.6 * n);
        self.overtone = 0.35 * bright as f64;

        let knock_hz = midi_hz(low_layer(tonic_pc(&patch.key)) as f64);
        self.knock = Phasor::new(knock_hz, sr, 0, 0.0);
        self.knock_amp = KNOCK_LEVEL * m.weight as f64;
        self.knock_decay = t60_coeff((0.025 * sr).min(0.8 * n));
        self.knock_dc.reset();

        self.contact.strike(to_samples(CONTACT_S, sr, 2));
        let limit = super::common::NOISE_CEILING_HZ.min(0.45 * sr);
        for (md, (ratio, amp, t60)) in self.modes.iter_mut().zip(MODES) {
            let f = centre * ratio * (1.0 + 0.004 * rng.next_bipolar() as f64);
            let t60 = t60.min(0.6 * n / sr);
            let a = if f < limit { amp } else { 0.0 };
            md.set(f.min(limit), t60, a, sr);
        }
        self.mode_dc.reset();
        self.mode_last = 0.0;
        self.mode_scale = sr / (std::f64::consts::TAU * centre);
        self.room
            .set(1.0, 0.5, expmap(bright, 4000.0, 10_000.0), sr);
        self.room.clear();

        let jitter = 2f64.powf(rng.next_bipolar() as f64 * 0.5 / 6.02);
        self.gain = LEVEL * vel_gain(note.velocity) as f64 * jitter;
        self.i = 0;
        self.done = false;
    }

    fn release(&mut self, _patch: &Tick) {
        // One-shot.
    }

    #[inline]
    fn tick(&mut self, _patch: &Tick) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        if self.i >= self.n {
            self.done = true;
            return (0.0, 0.0);
        }
        let x = self.i as f64 / self.n as f64;
        let env = if self.i < self.attack {
            (self.i + 1) as f64 / self.attack as f64
        } else {
            1.0
        } * fall((x - FADE_FROM) / (1.0 - FADE_FROM));
        let pulse = if self.i < self.pulse {
            let s = (std::f64::consts::PI * (self.i as f64 + 0.5) / self.pulse as f64).sin();
            PULSE_LEVEL * s * s
        } else {
            0.0
        };
        // The noise swells in over 2 ms after the pulse, so the peak is always the pulse's.
        let swell = (self.i as f64 / (8 * self.pulse) as f64).min(1.0);
        let excite = pulse + CLICK_NOISE * swell * self.click_amp * self.noise.next() as f64;
        let click = CLICK_LEVEL * self.band.tick(excite as f32) as f64;
        self.click_amp *= self.click_decay;
        let th = TAU * self.ping.phase();
        let ping = self.ping_amp * (th.sin() + self.overtone * (2.0 * th).sin());
        self.ping.advance();
        self.ping_amp *= self.ping_decay;
        let knock = self.knock_amp * (TAU * self.knock.phase()).sin();
        let knock = self.knock_dc.tick(knock as f32) as f64;
        self.knock.advance();
        self.knock_amp *= self.knock_decay;
        let force = self.contact.next();
        let mut body = 0.0;
        for md in self.modes.iter_mut() {
            body += md.tick(force);
        }
        // The body radiates its velocity (the displacement's first difference, scaled back to
        // the click band): no net push, so no DC.
        let vel = (body - self.mode_last) * self.mode_scale;
        self.mode_last = body;
        let body = MODE_LEVEL * self.mode_dc.tick(vel as f32) as f64;
        self.i += 1;
        let dry = click + body + ping + knock;
        let (wl, wr) = self.room.tick(dry, dry);
        let g = self.gain * env;
        (
            ((dry + ROOM_WET * wl) * g) as f32,
            ((dry + ROOM_WET * wr) * g) as f32,
        )
    }

    fn level(&self) -> f32 {
        if self.done {
            0.0
        } else {
            (self.gain * (self.click_amp + self.ping_amp + self.knock_amp)) as f32
        }
    }

    fn is_done(&self) -> bool {
        self.done
    }
}
