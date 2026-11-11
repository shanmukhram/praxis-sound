//! `impact`: a tuned hit (class impact, pitched one-shot).
//!
//! Modelled as a strike on a large tuned body in a hall. A raised-cosine contact force (a hard,
//! short pulse when bright, a softer, longer one when dark) drives: the body's fundamental on the
//! key's tonic (the tonic nearest the note, kept between C1 and B4, so the note only picks the
//! register), which starts 5 semitones sharp and relaxes into pitch within ~50 ms, with an octave
//! overtone and saturation that grow with brightness; the body's upper modes at near-harmonic
//! ratios (like a tuned drum: 1.5, 1.98, 2.44, 2.9, 3.4 times the fundamental, the upper ones
//! decaying faster); and a cloud of short shell modes (the material's knock), struck again by
//! debris, small seeded fragments landing over 40..400 ms. A crack of noise marks the contact, a
//! low slap of band noise the air pushed out, a decorrelated low-passed rumble the rest; a
//! sub-bass on the tonic drops into pitch with the body, and a gusting low roll holds, then dies
//! away. The fundamental and sub stay dry (their pitch drop must not ring on); everything else
//! plays into a hall (above 200 Hz) whose decay follows the length. Small differences between
//! takes (mode tuning and levels, the shell cloud, the debris, the roll's gusts) come from the
//! note seed. A 1 ms ramp starts it; a `cos^2` fade over the last 25% of `length` ends it in
//! exact silence. Note-offs do not cut it.
//!
//! Macros:
//! - `length`: total duration, 0.5..8 s (exponential; 0.5 = 2 s); the body, roll and hall decays
//!   scale with it.
//! - `brightness`: contact hardness (pulse 4..0.8 ms), body overtone and saturation, crack band
//!   1.2..6 kHz, upper-mode and shell levels, slap and roll colour, debris low-pass
//!   300..6000 Hz, hall damping.
//! - `weight`: body level (0.15..1 x) and length (25..90% of `length`), the sub-bass and the low
//!   roll (0 at weight 0).
//! - `pitch`: the body moves between tonic and fifth: an octave down, a fourth down (the fifth
//!   below), the tonic, a fifth up, an octave up (0.5 = the tonic); the slap and shell colour
//!   follow by up to half an octave.

use super::common::{
    Macros, colour_shift, dc_blocker, expmap, fall, fifth_pc, info, low_layer, midi_hz,
    pitch_steps, t60_coeff, to_samples, tonic_near, tonic_pc,
};
use super::space::{Contact, Mode, Room, Wander, soft_clip};
use crate::synth::common::{Params, vel_gain};
use crate::synth::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::filter::{DcBlocker, Svf, SvfMode};
use praxis_dsp::noise::White;
use praxis_dsp::rng::{Pcg32, derive};
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, Key};
use std::f64::consts::TAU;

/// Shortest and longest impact, seconds.
pub const LENGTH_S: (f64, f64) = (0.5, 8.0);
/// Lowest and highest MIDI pitch of the body (C1..B4).
pub const BODY_MIDI: (i32, i32) = (24, 71);
/// The body starts this many semitones sharp and relaxes into tune.
const DROP_ST: f64 = 5.0;
const DROP_TAU_S: f64 = 0.015;
const ATTACK_S: f64 = 0.001;
/// Contact pulse length at brightness 1 and 0, seconds.
const CONTACT_S: (f64, f64) = (0.0008, 0.004);
/// The final fade covers the last quarter of the length.
const FADE_FROM: f64 = 0.75;
/// Upper body modes: ratio to the fundamental, level, decay relative to the body's.
const MODES: [(f64, f64, f64); 5] = [
    (1.504, 0.55, 0.45),
    (1.98, 0.4, 0.35),
    (2.44, 0.28, 0.28),
    (2.9, 0.2, 0.22),
    (3.4, 0.14, 0.18),
];
/// Shell modes: how many, their range (Hz, at brightness 0 and 1 for the top) and decays.
const SHELL: usize = 8;
const SHELL_LO_HZ: f64 = 280.0;
const SHELL_HI_HZ: (f64, f64) = (900.0, 3200.0);
const SHELL_T60_S: (f64, f64) = (0.18, 0.05);
const LEVEL: f64 = 0.5;
/// Body saturation drive at brightness 0 and 1 (a harder hit drives the body harder).
const BODY_DRIVE: (f64, f64) = (0.6, 3.0);
/// The room hears only what is above this, so the body and slap stay dry and in tune.
const ROOM_LOWCUT_HZ: f64 = 200.0;
const MODE_LEVEL: f64 = 0.9;
const SHELL_LEVEL: f64 = 0.35;
const CRACK_LEVEL: f64 = 0.5;
const SLAP_LEVEL: f64 = 0.9;
const TAIL_LEVEL: f64 = 0.10;
const SUB_LEVEL: f64 = 0.45;
const SUB_DROP_ST: f64 = 7.0;
const ROOM_WET: f64 = 0.9;
/// Octave overtone of the body at brightness 0 and 1 (rising as the square root of brightness),
/// and its decay relative to the body's.
const OVERTONE: (f64, f64) = (0.0, 0.8);
const OVERTONE_DECAY: f64 = 0.5;
/// The low roll (weight): level, low-pass, hold (fraction of the length) and decay, gusting
/// at about this rate.
const ROLL_LEVEL: f64 = 1.6;
const ROLL_HZ: f64 = 140.0;
const ROLL_HOLD: f64 = 0.3;
const ROLL_T60: f64 = 0.55;
const ROLL_GUST_HZ: f64 = 0.7;
const ROLL_DEPTH: f64 = 0.45;
/// Debris: small fragments landing from 40 ms after the hit, densest at first (events per
/// second, e-folding time) and their force.
const DEBRIS_FROM_S: f64 = 0.04;
const DEBRIS_TO_S: f64 = 0.4;
const DEBRIS_RATE: f64 = 140.0;
const DEBRIS_TAU_S: f64 = 0.09;
const DEBRIS_FORCE: f64 = 0.35;

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
    contact: Contact,
    body_phase: f64,
    body_hz: f64,
    body_amp: f64,
    body_decay: f64,
    body_rise: f64,
    body_rise_coeff: f64,
    drive: f64,
    overtone: f64,
    overtone_env: f64,
    overtone_decay: f64,
    roll_noise: [White; 2],
    roll_lp: [Svf; 2],
    roll_hp: [Svf; 2],
    roll_gust: Wander,
    roll_amp: f64,
    roll_hold: u64,
    roll_decay: f64,
    roll_rise: f64,
    roll_rise_coeff: f64,
    debris_rng: Pcg32,
    debris_from: u64,
    debris_to: u64,
    debris_k: f64,
    debris_force: f64,
    drop: f64,
    drop_coeff: f64,
    modes: [Mode; 5],
    shell: [Mode; SHELL],
    mode_gain: f64,
    shell_gain: f64,
    sub_phase: f64,
    sub_hz: f64,
    sub_amp: f64,
    sub_decay: f64,
    sub_drop: f64,
    body_dc: DcBlocker,
    shell_dc: DcBlocker,
    noise: White,
    tail_noise: [White; 2],
    crack: Svf,
    crack_hp: Svf,
    crack_amp: f64,
    crack_decay: f64,
    slap: Svf,
    slap_amp: f64,
    slap_decay: f64,
    tail: [Svf; 2],
    tail_amp: f64,
    tail_decay: f64,
    room: Room,
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
            contact: Contact::default(),
            body_phase: 0.0,
            body_hz: 0.0,
            body_amp: 0.0,
            body_decay: 0.0,
            body_rise: 0.0,
            body_rise_coeff: 0.0,
            drive: 1.0,
            overtone: 0.0,
            overtone_env: 0.0,
            overtone_decay: 0.0,
            roll_noise: [White::new(4), White::new(5)],
            roll_lp: [filter(SvfMode::LowPass), filter(SvfMode::LowPass)],
            roll_hp: [filter(SvfMode::HighPass), filter(SvfMode::HighPass)],
            roll_gust: Wander::default(),
            roll_amp: 0.0,
            roll_hold: 0,
            roll_decay: 0.0,
            roll_rise: 0.0,
            roll_rise_coeff: 0.0,
            debris_rng: Pcg32::new(0),
            debris_from: 0,
            debris_to: 0,
            debris_k: 0.0,
            debris_force: 0.0,
            drop: 0.0,
            drop_coeff: 0.0,
            modes: [Mode::default(); 5],
            shell: [Mode::default(); SHELL],
            mode_gain: 0.0,
            shell_gain: 0.0,
            sub_phase: 0.0,
            sub_hz: 0.0,
            sub_amp: 0.0,
            sub_decay: 0.0,
            sub_drop: 0.0,
            body_dc: dc_blocker(sr),
            shell_dc: dc_blocker(sr),
            noise: White::new(1),
            tail_noise: [White::new(2), White::new(3)],
            crack: filter(SvfMode::BandPass),
            crack_hp: filter(SvfMode::HighPass),
            crack_amp: 0.0,
            crack_decay: 0.0,
            slap: filter(SvfMode::BandPass),
            slap_amp: 0.0,
            slap_decay: 0.0,
            tail: [filter(SvfMode::LowPass), filter(SvfMode::LowPass)],
            tail_amp: 0.0,
            tail_decay: 0.0,
            room: Room::new(self.sr),
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
        let seconds = n / sr;
        let colour = colour_shift(m.pitch).sqrt();

        let contact_s = CONTACT_S.1 * (CONTACT_S.0 / CONTACT_S.1).powf(bright);
        self.contact.strike(to_samples(contact_s, sr, 2));

        let body = Impact::body_pitch(&patch.key, note.pitch, m.pitch);
        self.body_hz = midi_hz(body as f64);
        self.body_phase = 0.0;
        self.body_amp = 0.15 + 0.85 * w;
        let body_t60 = (0.25 + 0.65 * w) * n;
        self.body_decay = t60_coeff(body_t60);
        self.body_rise = 0.0;
        self.body_rise_coeff = (-1.0 / (0.5 * contact_s * sr).max(1.0)).exp();
        self.drive = BODY_DRIVE.0 + (BODY_DRIVE.1 - BODY_DRIVE.0) * bright;
        self.overtone = OVERTONE.0 + (OVERTONE.1 - OVERTONE.0) * bright.sqrt();
        self.overtone_env = 1.0;
        self.overtone_decay = t60_coeff(OVERTONE_DECAY * body_t60);
        self.drop = DROP_ST;

        // Low roll: a gusting band of low rumble that holds, then dies away (weight).
        self.roll_noise[0].reseed(derive(note.seed, 0x1A_0005));
        self.roll_noise[1].reseed(derive(note.seed, 0x1A_0006));
        let roll_hz = ROLL_HZ * 2f64.powf(3.0 * (bright - 0.5));
        for f in self.roll_lp.iter_mut() {
            f.reset();
            f.set_cutoff_q(roll_hz as f32, 0.7);
        }
        for f in self.roll_hp.iter_mut() {
            f.reset();
            f.set_cutoff_q(30.0, 0.7);
        }
        self.roll_gust
            .start(derive(note.seed, 0x1A_0007), ROLL_GUST_HZ, sr);
        self.roll_amp = ROLL_LEVEL * w;
        self.roll_hold = (ROLL_HOLD * n) as u64;
        self.roll_decay = t60_coeff(ROLL_T60 * n);
        self.roll_rise = 0.0;
        self.roll_rise_coeff = (-1.0 / (0.03 * sr)).exp();

        // Debris: seeded fragments, each a small force on the shell.
        self.debris_rng = note.rng(0x1A_0008);
        self.debris_from = to_samples(DEBRIS_FROM_S, sr, 1).min(self.n);
        self.debris_to = to_samples(DEBRIS_TO_S, sr, 1).min(self.n);
        self.debris_k = (-1.0 / (DEBRIS_TAU_S * sr)).exp();
        self.debris_force = DEBRIS_RATE / sr;
        self.drop_coeff = t60_coeff(DROP_TAU_S * 6.9 * sr);

        // Upper modes: near-harmonic, seeded small detunings and level differences.
        self.mode_gain = MODE_LEVEL * (0.15 + 0.85 * bright) * (0.4 + 0.6 * w);
        for (md, (ratio, amp, dec)) in self.modes.iter_mut().zip(MODES) {
            let detune = 1.0 + 0.006 * rng.next_bipolar() as f64;
            let a = amp * (0.75 + 0.5 * rng.next_f32() as f64);
            md.set(self.body_hz * ratio * detune, body_t60 / sr * dec, a, sr);
        }
        // Shell: a log-spread cloud of short modes (the material's own knock).
        let hi = expmap(bright as f32, SHELL_HI_HZ.0, SHELL_HI_HZ.1) * colour;
        let lo = SHELL_LO_HZ * colour;
        self.shell_gain = SHELL_LEVEL * (0.1 + 0.9 * bright);
        for (k, md) in self.shell.iter_mut().enumerate() {
            let u = (k as f64 + rng.next_f32() as f64) / SHELL as f64;
            let f = lo * (hi / lo).powf(u);
            let t60 = SHELL_T60_S.0 * (SHELL_T60_S.1 / SHELL_T60_S.0).powf(u);
            let a = (0.35 + 0.65 * rng.next_f32() as f64) * 2f64.powf(-1.5 * u);
            md.set(f, t60.min(0.5 * seconds), a, sr);
        }

        self.sub_hz = midi_hz(low_layer(tonic_pc(&patch.key)) as f64);
        self.sub_phase = 0.0;
        self.sub_amp = SUB_LEVEL * w;
        self.sub_decay = t60_coeff(0.5 * n);
        self.sub_drop = SUB_DROP_ST;
        self.body_dc.reset();
        self.shell_dc.reset();

        let centre = expmap(bright as f32, 1200.0, 6000.0) * (0.97 + 0.06 * rng.next_f32() as f64);
        self.crack.reset();
        self.crack.set_cutoff_q(centre as f32, 0.8);
        self.crack_hp.reset();
        self.crack_hp.set_cutoff_q((0.5 * centre) as f32, 0.7);
        self.crack_amp = CRACK_LEVEL * (0.15 + 1.1 * bright);
        self.crack_decay = t60_coeff(0.025 * sr);
        self.slap.reset();
        self.slap
            .set_cutoff_q((240.0 * colour * 2f64.powf(bright - 0.5)) as f32, 1.6);
        self.slap_amp = SLAP_LEVEL;
        self.slap_decay = t60_coeff(0.09 * sr);
        let tail_hz = expmap(bright as f32, 300.0, 6000.0);
        for f in self.tail.iter_mut() {
            f.reset();
            f.set_cutoff_q(tail_hz as f32, 0.6);
        }
        self.tail_amp = TAIL_LEVEL;
        self.tail_decay = t60_coeff(0.8 * n);

        let rt60 = (0.5 * seconds).clamp(0.6, 3.0);
        self.room
            .set(rt60, 0.85, expmap(bright as f32, 1500.0, 7000.0), sr);
        self.room.set_lowcut(ROOM_LOWCUT_HZ, sr);
        self.room.clear();

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
        let force = self.contact.next();
        // Debris: the chance of a fragment falls away exponentially after `debris_from`.
        let mut debris = 0.0;
        if self.i >= self.debris_from && self.i < self.debris_to {
            let p = self.debris_force;
            self.debris_force *= self.debris_k;
            let u = self.debris_rng.next_f32() as f64;
            let a = self.debris_rng.next_f32() as f64;
            if u < p {
                debris = DEBRIS_FORCE * (0.2 + 0.8 * a * a);
            }
        }

        // Body fundamental: rises with the contact, relaxes into pitch, a loud body saturates.
        self.body_rise = 1.0 - (1.0 - self.body_rise) * self.body_rise_coeff;
        let ratio = 2f64.powf(self.drop / 12.0);
        self.drop *= self.drop_coeff;
        let th = TAU * self.body_phase;
        let fund = self.body_amp
            * self.body_rise
            * (th.sin() + self.overtone * self.overtone_env * (2.0 * th).sin());
        self.overtone_env *= self.overtone_decay;
        self.body_phase = (self.body_phase + self.body_hz * ratio / self.sr).fract();
        self.body_amp *= self.body_decay;
        let fund = soft_clip(fund, self.drive);
        let mut upper = 0.0;
        for md in self.modes.iter_mut() {
            upper += md.tick(force);
        }
        let mut shell = 0.0;
        for md in self.shell.iter_mut() {
            shell += md.tick(force + debris);
        }
        let sub_ratio = 2f64.powf(self.sub_drop / 12.0);
        self.sub_drop *= self.drop_coeff;
        let sub = self.sub_amp * self.body_rise * (TAU * self.sub_phase).sin();
        self.sub_phase = (self.sub_phase + self.sub_hz * sub_ratio / self.sr).fract();
        self.sub_amp *= self.sub_decay;
        // The fundamental and sub stay dry (their early pitch drop must not ring on in the room);
        // the upper modes and the shell are what the room hears of the body.
        let low = self.body_dc.tick((fund + sub) as f32) as f64;
        let ring =
            self.shell_dc
                .tick((self.mode_gain * upper + self.shell_gain * shell) as f32) as f64;

        let wn = self.noise.next();
        let crack = self.crack_amp * self.crack_hp.tick(self.crack.tick(wn)) as f64;
        self.crack_amp *= self.crack_decay;
        let slap = self.slap_amp * self.slap.tick(wn) as f64;
        self.slap_amp *= self.slap_decay;
        let tl = self.tail_amp * self.tail[0].tick(self.tail_noise[0].next()) as f64;
        let tr = self.tail_amp * self.tail[1].tick(self.tail_noise[1].next()) as f64;
        self.tail_amp *= self.tail_decay;

        // The roll: rises over ~30 ms, holds, then decays; decorrelated left and right.
        self.roll_rise = 1.0 - (1.0 - self.roll_rise) * self.roll_rise_coeff;
        if self.i >= self.roll_hold {
            self.roll_amp *= self.roll_decay;
        }
        let gust = (ROLL_DEPTH * self.roll_gust.next() - 0.5 * ROLL_DEPTH * ROLL_DEPTH).exp();
        let ra = self.roll_amp * self.roll_rise * gust;
        let mut roll = [0.0; 2];
        for (c, v) in roll.iter_mut().enumerate() {
            let x = self.roll_lp[c].tick(self.roll_noise[c].next());
            *v = ra * self.roll_hp[c].tick(x) as f64;
        }

        let dl = ring + crack + slap + tl + roll[0];
        let dr = ring + crack + slap + tr + roll[1];
        let (wl, wr) = self.room.tick(dl, dr);
        let (dl, dr) = (dl + low, dr + low);
        let g = self.gain * env;
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
            (self.gain
                * (self.body_amp + self.tail_amp + self.sub_amp + self.slap_amp + self.roll_amp))
                as f32
        }
    }

    fn is_done(&self) -> bool {
        self.done
    }
}
