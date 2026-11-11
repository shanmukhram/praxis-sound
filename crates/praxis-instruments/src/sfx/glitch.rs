//! `glitch`: stuttering, crushed digital fragments (class glitch, pitched one-shot).
//!
//! Granular: the sound is cut into slices on the tempo grid (a quarter, an eighth or a sixteenth
//! of a beat, 20..90 ms). Each slice is a fresh grain, a repeat of the last fresh grain (the
//! stutter, replayed from a preallocated buffer, sometimes at double speed, and a little quieter
//! each time it repeats, down to 60%) or a gap. Most fresh grains (75%) are bursts of noise
//! through a comb tuned to the note's period, each with its own loop gain, from nearly plain
//! noise (data crackle) to a clearly pitched buzz (a corrupted buffer ringing), over a quiet hum on the note; the rest are tuned
//! FM blips (carrier on the note, modulator at twice it). Each grain has its own level, shape
//! (mostly percussive, some held), sample-rate reduction (a sample-and-hold at 6..16 times the
//! tone's frequency), bit depth (3..8 bits), possible octave jump and a small pan offset, and the
//! whole glitch plays into a small room. Every choice comes from the note seed, so a note repeats
//! exactly.
//!
//! **Stays in key.** Everything pitched in it is harmonic on the note: slice lengths are whole
//! periods of the note, the comb's loop is one period (its damping filter's delay compensated) and
//! the blips' sample-and-hold rate is an even multiple of their frequency, so ringing, crushing
//! and stuttering only add harmonics of the note (double-speed repeats sound an octave up); the
//! rest is unpitched noise. There is no DC.
//!
//! Slices open and close with 1.5 ms ramps; the whole glitch ends with a 5 ms `cos^2` fade
//! exactly at the end of `length` (the last stretch is never a gap). Note-offs do not cut it.
//!
//! Macros:
//! - `length`: total duration, 0.08..2 s (exponential; 0.5 = 0.4 s).
//! - `brightness`: how much of the crushed signal is mixed over the clean grain (30..100%), the
//!   comb's damping and the output low-pass (1.5..11 kHz).
//! - `weight`: a low thump at every fresh slice, on the note's pitch class at 41..78 Hz (0 at
//!   weight 0).
//! - `pitch`: whole scale steps of the key, -7..+7 (0.5 = the note).

use super::common::{
    Macros, Phasor, dc_blocker, expmap, fall, info, low_layer, midi_hz, pan_unity, pitch_steps,
    step_in_key, to_samples,
};
use super::space::Room;
use crate::synth::common::{OneShot, Params, vel_gain};
use crate::synth::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::filter::{DcBlocker, Svf, SvfMode};
use praxis_dsp::noise::White;
use praxis_dsp::rng::{Pcg32, derive};
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, Key};
use std::f64::consts::TAU;

/// Shortest and longest glitch, seconds.
pub const LENGTH_S: (f64, f64) = (0.08, 2.0);
/// Shortest and longest slice, seconds (before rounding to whole periods).
pub const SLICE_S: (f64, f64) = (0.020, 0.090);
/// Lowest MIDI pitch the tone plays (C1); lower notes move up an octave.
const LOWEST: u8 = 24;
const RAMP_S: f64 = 0.0015;
const END_FADE_S: f64 = 0.005;
/// Sample-and-hold rates in multiples of the tone's frequency. Even multiples of a tone with
/// only odd harmonics alias onto odd harmonics: never onto DC, never off the note.
const HOLD_RATES: [f64; 4] = [6.0, 8.0, 12.0, 16.0];
/// FM index of the blip (modulator at twice the carrier: odd harmonics only).
const FM_INDEX: f64 = 1.0;
/// Comb loop gain and the level of its output.
/// Comb loop gain range: each comb grain draws its own, from nearly plain noise (data crackle)
/// to a clearly pitched buzz.
const COMB_FB: (f64, f64) = (0.75, 0.96);
const COMB_LEVEL: f64 = 1.0;
/// Probability that a fresh grain is a comb burst rather than a blip.
const COMB_P: f32 = 0.75;
/// A comb grain also carries a quiet sine on its pitch (the hum of the buffer it corrupts), so
/// every take has a steady line on the note.
const COMB_HUM: f64 = 0.35;
/// Grain levels are drawn in this range; repeats lose this factor each time.
const GRAIN_LEVEL: (f64, f64) = (0.6, 1.0);
const REPEAT_DECAY: f64 = 0.9;
const REPEAT_FLOOR: f64 = 0.6;
const LEVEL: f64 = 0.4;
const SUB_LEVEL: f64 = 0.9;
const ROOM_WET: f64 = 0.3;

const FRESH: u8 = 0;
const REPEAT: u8 = 1;
const GAP: u8 = 2;

/// The `glitch` preset.
#[derive(Clone, Debug)]
pub struct Glitch {
    m: Macros,
    key: Key,
    bpm: f64,
    sr: f64,
}

#[derive(Clone, Debug)]
pub struct GlitchVoice {
    i: u64,
    n: u64,
    rng: Pcg32,
    hz: f64,
    period: f64,
    beat: f64,
    slice_start: u64,
    slice_len: u64,
    kind: u8,
    last_kind: u8,
    buf: Vec<f32>,
    buf_len: u64,
    phase: f64,
    octave: f64,
    hold_phase: f64,
    hold_rate: f64,
    held: f64,
    levels: f64,
    crush: f64,
    comb_grain: bool,
    grain_level: f64,
    grain_env: f64,
    grain_decay: f64,
    double_speed: bool,
    repeat_gain: f64,
    comb: Vec<f32>,
    comb_pos: usize,
    comb_delay: f64,
    comb_damp: f64,
    comb_fb: f64,
    comb_lp: f64,
    noise: White,
    exc_hp: [Svf; 2],
    comb_hp: [Svf; 2],
    pan: (f64, f64),
    lp: Svf,
    thump: OneShot,
    thump_dc: DcBlocker,
    tone_dc: DcBlocker,
    sub: Phasor,
    sub_level: f64,
    room: Room,
    ramp: u64,
    end_fade: u64,
    gain: f64,
    sr: f64,
    done: bool,
}

impl Glitch {
    fn length_samples(&self) -> u64 {
        to_samples(expmap(self.m.length, LENGTH_S.0, LENGTH_S.1), self.sr, 2)
    }
}

impl Patch for Glitch {
    type Voice = GlitchVoice;

    fn info() -> PresetInfo {
        info("glitch", GeneratorClass::Glitch, true, false, (24, 96))
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> Glitch {
        let bpm = if ctx.bpm.is_finite() && ctx.bpm > 0.0 {
            ctx.bpm.clamp(20.0, 400.0)
        } else {
            120.0
        };
        Glitch {
            m: Macros::from_params(p),
            key: ctx.key,
            bpm,
            sr: context_rate(ctx),
        }
    }

    fn polyphony(&self) -> usize {
        2
    }

    fn new_voice(&self) -> GlitchVoice {
        let sr = self.sr;
        let longest_period = sr / midi_hz(LOWEST as f64);
        // Longest slice plus one period of the lowest note.
        let cap = (SLICE_S.1 * sr + longest_period).ceil() as usize + 4;
        let comb_cap = longest_period.ceil() as usize + 8;
        let mut lp = Svf::new(sr as f32);
        lp.set_mode(SvfMode::LowPass);
        let hp = || {
            let mut f = Svf::new(sr as f32);
            f.set_mode(SvfMode::HighPass);
            f
        };
        let exc_hp = [hp(), hp()];
        let comb_hp = [hp(), hp()];
        GlitchVoice {
            i: 0,
            n: 0,
            rng: Pcg32::new(0),
            hz: 0.0,
            period: 1.0,
            beat: 0.5,
            slice_start: 0,
            slice_len: 0,
            kind: GAP,
            last_kind: GAP,
            buf: vec![0.0; cap],
            buf_len: 0,
            phase: 0.0,
            octave: 1.0,
            hold_phase: 0.0,
            hold_rate: 4.0,
            held: 0.0,
            levels: 128.0,
            crush: 1.0,
            comb_grain: false,
            grain_level: 1.0,
            grain_env: 1.0,
            grain_decay: 1.0,
            double_speed: false,
            repeat_gain: 1.0,
            comb: vec![0.0; comb_cap],
            comb_pos: 0,
            comb_delay: 2.0,
            comb_damp: 0.3,
            comb_fb: COMB_FB.1,
            comb_lp: 0.0,
            noise: White::new(1),
            exc_hp,
            comb_hp,
            thump_dc: dc_blocker(sr as f32),
            tone_dc: dc_blocker(sr as f32),
            pan: (1.0, 1.0),
            lp,
            thump: OneShot::default(),
            sub: Phasor::default(),
            sub_level: 0.0,
            room: Room::new(sr),
            ramp: 1,
            end_fade: 1,
            gain: 0.0,
            sr,
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

impl GlitchVoice {
    /// Whole periods of the note closest to `seconds`, in samples.
    fn periods(&self, seconds: f64) -> u64 {
        let k = (seconds * self.sr / self.period).round().max(1.0);
        ((k * self.period).round() as u64).max(2 * self.ramp + 2)
    }

    /// Chooses the next slice (always draws the same number of random values).
    fn next_slice(&mut self) {
        let div = [4.0, 8.0, 16.0][(self.rng.next_u32() % 3) as usize];
        let r_kind = self.rng.next_f32();
        let r_oct = self.rng.next_f32();
        let bits = 3 + self.rng.next_u32() % 6;
        let rate = HOLD_RATES[(self.rng.next_u32() % HOLD_RATES.len() as u32) as usize];
        let pan = 0.35 * self.rng.next_bipolar() as f64;
        let r_grain = self.rng.next_f32();
        let r_level = self.rng.next_f32() as f64;
        let r_shape = self.rng.next_f32();
        let r_half = self.rng.next_f32();
        let r_fb = self.rng.next_f32() as f64;

        let first = self.slice_len == 0 && self.i == 0;
        let seconds = (self.beat / div).clamp(SLICE_S.0, SLICE_S.1);
        let fresh_len = self.periods(seconds).min(self.buf.len() as u64);
        let remaining = self.n.saturating_sub(self.i);
        let mut kind = if first || r_kind < 0.45 || self.buf_len == 0 {
            FRESH
        } else if r_kind < 0.82 {
            REPEAT
        } else {
            GAP
        };
        if kind == GAP && (self.last_kind == GAP || remaining < 3 * fresh_len) {
            kind = REPEAT;
        }
        self.kind = kind;
        self.slice_start = self.i;
        self.slice_len = if kind == REPEAT {
            self.buf_len
        } else {
            fresh_len
        };
        if kind == FRESH {
            self.octave = if !first && r_oct < 0.25 { 2.0 } else { 1.0 };
            self.levels = (1u32 << (bits - 1)) as f64;
            self.hold_rate = rate;
            self.hold_phase = (rate * self.phase * self.octave).fract();
            self.held = self.source();
            self.buf_len = self.slice_len;
            self.comb_grain = !first && r_grain < COMB_P;
            if self.comb_grain {
                self.comb_fb = COMB_FB.0 + (COMB_FB.1 - COMB_FB.0) * r_fb;
                self.held = 0.0;
            }
            self.grain_level = GRAIN_LEVEL.0 + (GRAIN_LEVEL.1 - GRAIN_LEVEL.0) * r_level;
            self.grain_env = 1.0;
            // Most grains are percussive (decaying to about -20 dB over the slice); some hold.
            self.grain_decay = if r_shape < 0.7 {
                (-2.3 / self.slice_len.max(1) as f64).exp()
            } else {
                1.0
            };
            self.repeat_gain = 1.0;
            self.thump.trigger(0.002, 0.045, self.sr);
        } else if kind == REPEAT {
            self.double_speed = r_half < 0.25;
            self.repeat_gain = (self.repeat_gain * REPEAT_DECAY).max(REPEAT_FLOOR);
        }
        self.pan = pan_unity(pan);
        self.last_kind = kind;
    }

    #[inline]
    fn source(&self) -> f64 {
        let th = TAU * self.phase * self.octave;
        (th + FM_INDEX * (2.0 * th).sin()).sin()
    }

    /// One sample of the comb (a loop of one period of the note at the current octave).
    #[inline]
    fn comb(&mut self, x: f64) -> f64 {
        let cap = self.comb.len();
        let d = (self.comb_delay / self.octave).clamp(2.0, (cap - 3) as f64);
        let k = d.floor() as usize;
        let t = d - k as f64;
        // `comb[comb_pos]` is the previous output, y[n-1]: delay j is index j - 1 back.
        let at = |j: usize| (self.comb_pos + cap - (j - 1)) % cap;
        let a = self.comb[at(k)] as f64;
        let b = self.comb[at(k + 1)] as f64;
        let delayed = a + t * (b - a);
        self.comb_lp = delayed + self.comb_damp * (self.comb_lp - delayed);
        let y = x + self.comb_fb * self.comb_lp;
        let y = if y.is_finite() && y.abs() > 1e-20 {
            y
        } else {
            0.0
        };
        self.comb_pos = (self.comb_pos + 1) % cap;
        self.comb[self.comb_pos] = y as f32;
        y
    }
}

impl Voice<Glitch> for GlitchVoice {
    fn start(&mut self, patch: &Glitch, note: &Note) {
        let sr = note.sample_rate;
        let m = patch.m;
        let bright = m.note_brightness(note.brightness);
        self.sr = sr;
        self.rng = note.rng(0x611_0001);
        self.noise.reseed(derive(note.seed, 0x611_0002));
        let mut pitch = step_in_key(&patch.key, note.pitch, pitch_steps(m.pitch, 7));
        while pitch < LOWEST {
            pitch += 12;
        }
        self.hz = midi_hz(pitch as f64);
        self.period = sr / self.hz;
        self.beat = 60.0 / patch.bpm;
        self.n = patch.length_samples();
        self.ramp = to_samples(RAMP_S, sr, 1);
        self.end_fade = to_samples(END_FADE_S, sr, 1).min(self.n / 2).max(1);
        self.crush = 0.3 + 0.7 * bright as f64;
        // The comb's loop is one period; its damping one-pole delays low frequencies by
        // a / (1 - a) samples, taken off the delay line so the ring stays on the note.
        self.comb_damp = 0.55 - 0.45 * bright as f64;
        self.comb_delay = self.period - self.comb_damp / (1.0 - self.comb_damp);
        self.comb.fill(0.0);
        self.comb_pos = 0;
        self.comb_lp = 0.0;
        // Fourth-order high-passes at half the note: flat at the note (so they never tilt a
        // broad comb peak off pitch), steep below it.
        for f in self.exc_hp.iter_mut().chain(self.comb_hp.iter_mut()) {
            f.reset();
        }
        for hp in [&mut self.exc_hp, &mut self.comb_hp] {
            hp[0].set_cutoff_q((0.5 * self.hz) as f32, 0.54);
            hp[1].set_cutoff_q((0.5 * self.hz) as f32, 1.31);
        }
        self.lp.reset();
        self.lp
            .set_cutoff_q(expmap(bright, 1500.0, 11_000.0) as f32, 0.7);
        self.thump_dc.reset();
        self.tone_dc.reset();
        let sub_hz = midi_hz(low_layer(pitch as i32 % 12) as f64);
        self.sub = Phasor::new(sub_hz, sr, 0, 0.0);
        self.sub_level = SUB_LEVEL * m.weight as f64;
        self.thump.clear();
        self.room
            .set(0.6, 0.4, expmap(bright, 3000.0, 10_000.0), sr);
        self.room.clear();
        self.gain = LEVEL * vel_gain(note.velocity) as f64;
        self.phase = 0.0;
        self.octave = 1.0;
        self.i = 0;
        self.slice_len = 0;
        self.slice_start = 0;
        self.buf_len = 0;
        self.repeat_gain = 1.0;
        self.double_speed = false;
        self.last_kind = GAP;
        self.done = false;
        self.next_slice();
    }

    fn release(&mut self, _patch: &Glitch) {
        // One-shot: the pattern always plays to its end.
    }

    #[inline]
    fn tick(&mut self, _patch: &Glitch) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        if self.i >= self.n {
            self.done = true;
            return (0.0, 0.0);
        }
        if self.i >= self.slice_start + self.slice_len {
            self.next_slice();
        }
        let pos = self.i - self.slice_start;
        // The comb rings on (and decays) whatever the slice, so a comb grain starts from it.
        let exc = if self.kind == FRESH && self.comb_grain {
            let e = self.exc_hp[0].tick(self.noise.next());
            self.exc_hp[1].tick(e) as f64 * self.grain_env
        } else {
            let e = self.exc_hp[0].tick(0.0);
            self.exc_hp[1].tick(e) as f64
        };
        let ring = self.comb(exc);
        let norm = (1.0 - self.comb_fb * self.comb_fb).sqrt() * COMB_LEVEL;
        let ring = self.comb_hp[0].tick(ring as f32);
        let ring = self.comb_hp[1].tick(ring) as f64 * norm;
        let s = match self.kind {
            FRESH => {
                let clean = if self.comb_grain {
                    let hum = (TAU * self.phase * self.octave).sin() * self.grain_env;
                    ring + COMB_HUM * hum
                } else {
                    self.source() * self.grain_env
                };
                let inc = self.octave * self.hz / self.sr;
                self.hold_phase += self.hold_rate * inc;
                if self.hold_phase >= 1.0 {
                    self.hold_phase -= 1.0;
                    self.held = clean;
                }
                let crushed = (self.held * self.levels).round() / self.levels;
                let raw = clean + self.crush * (crushed - clean);
                self.phase = (self.phase + self.hz / self.sr).fract();
                self.grain_env *= self.grain_decay;
                let v = raw * self.grain_level;
                if let Some(b) = self.buf.get_mut(pos as usize) {
                    *b = v as f32;
                }
                v
            }
            REPEAT => {
                let at = if self.double_speed {
                    (2 * pos) % self.buf_len.max(1)
                } else {
                    pos
                };
                self.buf.get(at as usize).copied().unwrap_or(0.0) as f64 * self.repeat_gain
            }
            _ => 0.0,
        };
        let r = self.ramp as f64;
        let gate = ((pos + 1) as f64 / r)
            .min((self.slice_len - pos) as f64 / r)
            .min(1.0);
        let tone = self.tone_dc.tick(self.lp.tick((s * gate) as f32)) as f64;
        let sub = self.sub_level * self.thump.next() as f64 * (TAU * self.sub.phase()).sin();
        let sub = self.thump_dc.tick(sub as f32) as f64;
        self.sub.advance();
        let fade_from = self.n - self.end_fade;
        let env = if self.i >= fade_from {
            fall((self.i - fade_from) as f64 / self.end_fade as f64)
        } else {
            1.0
        };
        let (pl, pr) = self.pan;
        let dl = tone * pl + sub;
        let dr = tone * pr + sub;
        let (wl, wr) = self.room.tick(dl, dr);
        let g = self.gain * env;
        self.i += 1;
        (
            ((dl + ROOM_WET * wl) * g) as f32,
            ((dr + ROOM_WET * wr) * g) as f32,
        )
    }

    fn level(&self) -> f32 {
        if self.done { 0.0 } else { self.gain as f32 }
    }

    fn is_done(&self) -> bool {
        self.done
    }
}
