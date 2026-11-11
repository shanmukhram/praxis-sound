//! `glitch`: stuttering, crushed digital fragments (class glitch, pitched one-shot).
//!
//! A tuned FM blip (carrier on the note, modulator at twice it) cut into slices on the tempo
//! grid (a quarter, an eighth or a sixteenth of a beat, 20..90 ms). Each slice is fresh sound,
//! a repeat of the last fresh slice (the stutter, replayed from a preallocated buffer) or a gap,
//! and fresh slices may jump an octave. Each slice has its own sample-rate reduction (a
//! sample-and-hold at 6..16 times the tone's frequency) and bit depth (3..8 bits), and a small
//! pan offset. Every choice comes from the note seed, so a note repeats exactly.
//!
//! **Stays in key.** Slice lengths are whole periods of the note and the sample-and-hold rate is
//! an even multiple of the tone's frequency, so the crushing and the stutter only add (odd)
//! harmonics of the note: the glitch is harmonic, never inharmonic noise, and has no DC.
//!
//! Slices open and close with 1.5 ms ramps; the whole glitch ends with a 5 ms `cos^2` fade
//! exactly at the end of `length` (the last stretch is never a gap). Note-offs do not cut it.
//!
//! Macros:
//! - `length`: total duration, 0.08..2 s (exponential; 0.5 = 0.4 s).
//! - `brightness`: how much of the crushed signal is mixed over the clean blip (30..100%) and
//!   the output low-pass (1.5..11 kHz).
//! - `weight`: a low thump at every fresh slice, on the note's pitch class at 41..78 Hz (0 at
//!   weight 0).
//! - `pitch`: whole scale steps of the key, -7..+7 (0.5 = the note).

use super::common::{
    Macros, Phasor, dc_blocker, expmap, fall, info, low_layer, midi_hz, pan_unity, pitch_steps,
    step_in_key, to_samples,
};
use crate::synth::common::{OneShot, Params, vel_gain};
use crate::synth::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::filter::{DcBlocker, Svf, SvfMode};
use praxis_dsp::rng::Pcg32;
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
const LEVEL: f64 = 0.4;
const SUB_LEVEL: f64 = 0.9;

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
    pan: (f64, f64),
    lp: Svf,
    thump: OneShot,
    thump_dc: DcBlocker,
    tone_dc: DcBlocker,
    sub: Phasor,
    sub_level: f64,
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
        // Longest slice plus one period of the lowest note.
        let cap = (SLICE_S.1 * sr + sr / midi_hz(LOWEST as f64)).ceil() as usize + 4;
        let mut lp = Svf::new(sr as f32);
        lp.set_mode(SvfMode::LowPass);
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
            thump_dc: dc_blocker(sr as f32),
            tone_dc: dc_blocker(sr as f32),
            pan: (1.0, 1.0),
            lp,
            thump: OneShot::default(),
            sub: Phasor::default(),
            sub_level: 0.0,
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
            self.thump.trigger(0.002, 0.045, self.sr);
        }
        self.pan = pan_unity(pan);
        self.last_kind = kind;
    }

    #[inline]
    fn source(&self) -> f64 {
        let th = TAU * self.phase * self.octave;
        (th + FM_INDEX * (2.0 * th).sin()).sin()
    }
}

impl Voice<Glitch> for GlitchVoice {
    fn start(&mut self, patch: &Glitch, note: &Note) {
        let sr = note.sample_rate;
        let m = patch.m;
        let bright = m.note_brightness(note.brightness);
        self.sr = sr;
        self.rng = note.rng(0x611_0001);
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
        self.lp.reset();
        self.lp
            .set_cutoff_q(expmap(bright, 1500.0, 11_000.0) as f32, 0.7);
        self.thump_dc.reset();
        self.tone_dc.reset();
        let sub_hz = midi_hz(low_layer(pitch as i32 % 12) as f64);
        self.sub = Phasor::new(sub_hz, sr, 0, 0.0);
        self.sub_level = SUB_LEVEL * m.weight as f64;
        self.thump.clear();
        self.gain = LEVEL * vel_gain(note.velocity) as f64;
        self.phase = 0.0;
        self.octave = 1.0;
        self.i = 0;
        self.slice_len = 0;
        self.slice_start = 0;
        self.buf_len = 0;
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
        let s = match self.kind {
            FRESH => {
                let clean = self.source();
                let inc = self.octave * self.hz / self.sr;
                self.hold_phase += self.hold_rate * inc;
                if self.hold_phase >= 1.0 {
                    self.hold_phase -= 1.0;
                    self.held = clean;
                }
                self.phase = (self.phase + self.hz / self.sr).fract();
                let crushed = (self.held * self.levels).round() / self.levels;
                let v = clean + self.crush * (crushed - clean);
                if let Some(b) = self.buf.get_mut(pos as usize) {
                    *b = v as f32;
                }
                v
            }
            REPEAT => self.buf.get(pos as usize).copied().unwrap_or(0.0) as f64,
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
        let g = self.gain * env;
        let (pl, pr) = self.pan;
        self.i += 1;
        (
            ((tone * pl + sub) * g) as f32,
            ((tone * pr + sub) * g) as f32,
        )
    }

    fn level(&self) -> f32 {
        if self.done { 0.0 } else { self.gain as f32 }
    }

    fn is_done(&self) -> bool {
        self.done
    }
}
