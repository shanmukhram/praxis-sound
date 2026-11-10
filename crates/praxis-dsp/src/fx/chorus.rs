//! Stereo chorus (kind 7): one modulated delay per channel, LFOs in quadrature.
//!
//! Parameters: 0 rate_hz (Hz), 1 depth_ms (ms, peak-to-peak sweep), 2 delay_ms (ms, shortest
//! delay), 3 mix (wet fraction). The delay sweeps `delay_ms .. delay_ms + depth_ms`. The LFO
//! phase restarts at 0 on `reset`, so renders are deterministic.

use super::ramp::{RAMP_SECONDS, Ramp, ramp_len};
use crate::delayline::DelayLine;
use crate::traits::Processor;
use crate::util::sanitize;

const KIND: usize = 7;
const MAX_MS: f32 = 61.0;

#[derive(Clone, Debug)]
pub struct Chorus {
    sr: f32,
    params: [f32; 4],
    rate: Ramp,
    depth: Ramp, // samples
    delay: Ramp, // samples
    mix: Ramp,
    phase: f32, // 0..1
    lines: [DelayLine; 2],
}

impl Chorus {
    pub fn new(sample_rate: f32, max_block: usize) -> Chorus {
        let params = super::defaults::<4>(KIND);
        let mut c = Chorus {
            sr: sample_rate,
            params,
            rate: Ramp::new(0.0, 1),
            depth: Ramp::new(0.0, 1),
            delay: Ramp::new(0.0, 1),
            mix: Ramp::new(0.0, 1),
            phase: 0.0,
            lines: [DelayLine::new(4), DelayLine::new(4)],
        };
        c.prepare(sample_rate, max_block);
        c
    }
}

impl Processor for Chorus {
    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sr = sample_rate;
        let len = ramp_len(RAMP_SECONDS, sample_rate);
        let ms = sample_rate * 0.001;
        let p = self.params;
        self.rate = Ramp::new(p[0], len);
        self.depth = Ramp::new(p[1] * ms, len);
        self.delay = Ramp::new(p[2] * ms, len);
        self.mix = Ramp::new(p[3], len);
        let cap = (MAX_MS * ms).ceil() as usize + 8;
        self.lines = [DelayLine::new(cap), DelayLine::new(cap)];
        self.phase = 0.0;
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let tau = std::f32::consts::TAU;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let rate = self.rate.next();
            let depth = self.depth.next();
            let base = self.delay.next();
            let mix = self.mix.next();
            let (s, c) = (self.phase * tau).sin_cos();
            self.phase += rate / self.sr;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            *l = sanitize(*l);
            *r = sanitize(*r);
            self.lines[0].push(*l);
            self.lines[1].push(*r);
            let wl = self.lines[0].read(base + depth * 0.5 * (1.0 + s));
            let wr = self.lines[1].read(base + depth * 0.5 * (1.0 + c));
            *l = sanitize(*l + (wl - *l) * mix);
            *r = sanitize(*r + (wr - *r) * mix);
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(v) = super::clamp_param(KIND, index, value) else {
            return;
        };
        self.params[index] = v;
        let ms = self.sr * 0.001;
        match index {
            0 => self.rate.set(v),
            1 => self.depth.set(v * ms),
            2 => self.delay.set(v * ms),
            3 => self.mix.set(v),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.rate.snap();
        self.depth.snap();
        self.delay.snap();
        self.mix.snap();
        self.phase = 0.0;
        for l in self.lines.iter_mut() {
            l.reset();
        }
    }

    fn tail_samples(&self) -> usize {
        let d = self.delay.value().max(self.delay.target());
        let w = self.depth.value().max(self.depth.target());
        (d + w).ceil() as usize + 4
    }
}
