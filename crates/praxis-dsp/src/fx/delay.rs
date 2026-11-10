//! Stereo feedback delay with tempo sync, ping-pong and damped feedback (kind 4).
//!
//! Parameters: 0 time_beats (quarter-note beats; > 0 = tempo-synced, needs [`Delay::set_tempo`]),
//! 1 time_ms (ms, used when time_beats = 0), 2 feedback, 3 ping_pong (bool, >= 0.5 = on),
//! 4 damp_hz (one-pole low-pass in the feedback path), 5 mix (wet fraction).
//!
//! The first echo arrives exactly `round(time * sample_rate)` samples after the input. Time
//! changes glide over 50 ms (tape-style) instead of jumping. Ping-pong feeds the mono input to
//! the left line and cross-couples the feedback, with a 20 ms crossfade when it is toggled.

use super::ramp::{OnePole, RAMP_SECONDS, Ramp, one_pole_coeff, ramp_len};
use crate::delayline::DelayLine;
use crate::traits::Processor;
use crate::util::sanitize;

const KIND: usize = 4;
const TIME_GLIDE_SECONDS: f32 = 0.05;
/// Longest synced delay: 4 beats at 20 BPM.
const MAX_SECONDS: f32 = 12.0;
const DEFAULT_BPM: f32 = 120.0;

#[derive(Clone, Debug)]
pub struct Delay {
    sr: f32,
    params: [f32; 6],
    bpm: f32,
    time: Ramp, // samples
    feedback: Ramp,
    pp: Ramp,
    mix: Ramp,
    damp_a: f32,
    lines: [DelayLine; 2],
    damp: [OnePole; 2],
}

impl Delay {
    pub fn new(sample_rate: f32, max_block: usize) -> Delay {
        let params = super::defaults::<6>(KIND);
        let mut d = Delay {
            sr: sample_rate,
            params,
            bpm: DEFAULT_BPM,
            time: Ramp::new(1.0, 1),
            feedback: Ramp::new(0.0, 1),
            pp: Ramp::new(0.0, 1),
            mix: Ramp::new(0.0, 1),
            damp_a: 0.0,
            lines: [DelayLine::new(4), DelayLine::new(4)],
            damp: [OnePole::default(); 2],
        };
        d.prepare(sample_rate, max_block);
        d
    }

    /// Sets the tempo (quarter-note BPM) for tempo-synced times. Clamped to 20..=400.
    pub fn set_tempo(&mut self, bpm: f32) {
        if bpm.is_finite() {
            self.bpm = bpm.clamp(20.0, 400.0);
            self.time.set(self.target_samples());
        }
    }

    /// The delay time in samples the current parameters ask for (integer).
    pub fn target_samples(&self) -> f32 {
        let secs = if self.params[0] > 0.0 {
            self.params[0] * 60.0 / self.bpm
        } else {
            self.params[1] * 0.001
        };
        let max = (self.lines[0].capacity() - 4) as f32;
        (secs * self.sr).round().clamp(2.0, max)
    }
}

impl Processor for Delay {
    fn set_tempo(&mut self, bpm: f32) {
        Delay::set_tempo(self, bpm);
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sr = sample_rate;
        let cap = (MAX_SECONDS * sample_rate).ceil() as usize + 8;
        self.lines = [DelayLine::new(cap), DelayLine::new(cap)];
        let len = ramp_len(RAMP_SECONDS, sample_rate);
        let p = self.params;
        self.time = Ramp::new(0.0, ramp_len(TIME_GLIDE_SECONDS, sample_rate));
        self.time.set_now(self.target_samples());
        self.feedback = Ramp::new(p[2], len);
        self.pp = Ramp::new(if p[3] >= 0.5 { 1.0 } else { 0.0 }, len);
        self.mix = Ramp::new(p[5], len);
        self.damp_a = one_pole_coeff(p[4], sample_rate);
        self.damp = [OnePole::default(); 2];
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let t = self.time.next();
            let fb = self.feedback.next();
            let pp = self.pp.next();
            let mix = self.mix.next();
            // Read before writing: the sample pushed t - 1 pushes ago is t samples old now.
            let dl = self.lines[0].read(t - 1.0);
            let dr = self.lines[1].read(t - 1.0);
            let fl = self.damp[0].lp(dl, self.damp_a) * fb;
            let fr = self.damp[1].lp(dr, self.damp_a) * fb;
            let (xl, xr) = (sanitize(*l), sanitize(*r));
            let mono = 0.5 * (xl + xr);
            let in_l = xl + (mono - xl) * pp;
            let in_r = xr * (1.0 - pp);
            let wl = in_l + fl + (fr - fl) * pp;
            let wr = in_r + fr + (fl - fr) * pp;
            self.lines[0].push(sanitize(wl));
            self.lines[1].push(sanitize(wr));
            *l = sanitize(xl + (dl - xl) * mix);
            *r = sanitize(xr + (dr - xr) * mix);
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(v) = super::clamp_param(KIND, index, value) else {
            return;
        };
        self.params[index] = v;
        match index {
            0 | 1 => self.time.set(self.target_samples()),
            2 => self.feedback.set(v),
            3 => self.pp.set(if v >= 0.5 { 1.0 } else { 0.0 }),
            4 => self.damp_a = one_pole_coeff(v, self.sr),
            5 => self.mix.set(v),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.time.snap();
        self.feedback.snap();
        self.pp.snap();
        self.mix.snap();
        for l in self.lines.iter_mut() {
            l.reset();
        }
        self.damp = [OnePole::default(); 2];
    }

    /// Until the echoes fall below -90 dB: `(repeats + 1) * delay`, where `feedback^repeats`
    /// reaches -90 dB (the damping filter only shortens it). Not capped.
    fn tail_samples(&self) -> usize {
        let t = self.time.value().max(self.time.target()) as f64;
        let fb = self.feedback.value().max(self.feedback.target()) as f64;
        let repeats = if fb <= 1e-6 {
            0.0
        } else {
            (-90.0 / 20.0 * std::f64::consts::LN_10 / fb.ln()).ceil()
        };
        ((repeats + 1.0) * t).ceil() as usize + 1
    }
}
