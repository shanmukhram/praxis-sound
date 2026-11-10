//! Mid/side stereo width with mono-below crossover (kind 8).
//!
//! Parameters: 0 width (side gain: 0 mono, 1 unchanged, 2 double side), 1 bass_mono_hz
//! (Hz; side content below it is removed with a 24 dB/oct Linkwitz-Riley high-pass; 0 = off).
//! Switching the crossover on or off crossfades over 20 ms.

use super::ramp::{Biquad64, RAMP_SECONDS, Ramp, biquad_tail, ramp_len};
use crate::biquad::{BiquadShape, Coeffs};
use crate::traits::Processor;
use crate::util::sanitize;

const KIND: usize = 8;
const CHUNK: usize = 16;
const MIN_HZ: f32 = 10.0;

#[derive(Clone, Debug)]
pub struct Width {
    sr: f32,
    params: [f32; 2],
    width: Ramp,
    amount: Ramp, // 0 = crossover off, 1 = on
    hz: Ramp,     // log2 Hz
    hp: [Biquad64; 2],
    hp_c: Coeffs,
}

impl Width {
    pub fn new(sample_rate: f32, max_block: usize) -> Width {
        let params = super::defaults::<2>(KIND);
        let mut w = Width {
            sr: sample_rate,
            params,
            width: Ramp::new(1.0, 1),
            amount: Ramp::new(0.0, 1),
            hz: Ramp::new(0.0, 1),
            hp: [Biquad64::new(&Coeffs::IDENTITY); 2],
            hp_c: Coeffs::IDENTITY,
        };
        w.prepare(sample_rate, max_block);
        w
    }

    fn update(&mut self) {
        let c = Coeffs::design(
            BiquadShape::HighPass,
            self.hz.value().exp2() as f64,
            std::f64::consts::FRAC_1_SQRT_2,
            self.sr as f64,
        );
        self.hp[0].set(&c);
        self.hp[1].set(&c);
        self.hp_c = c;
    }

    fn set_hz(&mut self, v: f32, now: bool) {
        if v > 0.0 {
            if now {
                self.hz.set_now(v.max(MIN_HZ).log2());
                self.amount.set_now(1.0);
            } else {
                self.hz.set(v.max(MIN_HZ).log2());
                self.amount.set(1.0);
            }
        } else if now {
            self.amount.set_now(0.0);
        } else {
            self.amount.set(0.0);
        }
    }
}

impl Processor for Width {
    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sr = sample_rate;
        let len = ramp_len(RAMP_SECONDS, sample_rate);
        self.width = Ramp::new(self.params[0], len);
        self.amount = Ramp::new(0.0, len);
        self.hz = Ramp::new(100f32.log2(), len);
        self.set_hz(self.params[1], true);
        self.update();
        self.hp[0].reset();
        self.hp[1].reset();
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let mut start = 0;
        while start < n {
            let len = CHUNK.min(n - start);
            if self.hz.moving() {
                self.hz.advance(len as u32);
                self.update();
            }
            for i in start..start + len {
                let w = self.width.next();
                let a = self.amount.next();
                let (xl, xr) = (sanitize(left[i]), sanitize(right[i]));
                let m = 0.5 * (xl + xr);
                let s = 0.5 * (xl - xr);
                let h0 = self.hp[0].tick64(s as f64);
                let hs = self.hp[1].tick64(h0) as f32;
                let s = (s + (hs - s) * a) * w;
                left[i] = sanitize(m + s);
                right[i] = sanitize(m - s);
            }
            start += len;
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(v) = super::clamp_param(KIND, index, value) else {
            return;
        };
        self.params[index] = v;
        match index {
            0 => self.width.set(v),
            1 => self.set_hz(v, false),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.width.snap();
        self.amount.snap();
        self.hz.snap();
        self.update();
        self.hp[0].reset();
        self.hp[1].reset();
    }

    /// Ring-out of the crossover (two cascaded sections, so twice the single-pole time).
    fn tail_samples(&self) -> usize {
        if self.amount.value() > 0.0 || self.amount.target() > 0.0 {
            2 * biquad_tail(&self.hp_c)
        } else {
            0
        }
    }
}
