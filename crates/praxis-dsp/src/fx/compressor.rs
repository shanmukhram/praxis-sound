//! Feed-forward compressor with soft knee and sidechain (kind 2).
//!
//! Parameters: 0 threshold_db (dB), 1 ratio (:1), 2 attack_ms (ms), 3 release_ms (ms),
//! 4 knee_db (dB), 5 makeup_db (dB), 6 mix (parallel mix, wet fraction).
//!
//! Detector: per-channel mean square with a 10 ms time constant, stereo-linked by the louder
//! channel; the level is RMS in dBFS (a sine of peak amplitude A reads 20 log10(A / sqrt 2)).
//! The static curve is the soft-knee curve of Giannoulis, Massberg and Reiss (2012); gain
//! reduction is smoothed in dB with the attack/release time constants. The key is the signal
//! passed to [`Processor::set_sidechain`] before `process`, otherwise the input itself.

use super::ramp::{RAMP_SECONDS, Ramp, ramp_len};
use crate::traits::Processor;
use crate::util::{db_to_gain, sanitize, time_coeff};

const KIND: usize = 2;
const DETECTOR_SECONDS: f32 = 0.010;

/// Static gain computer: output level (dB) for an input level (dB).
pub fn static_curve(x_db: f32, threshold_db: f32, ratio: f32, knee_db: f32) -> f32 {
    let over = x_db - threshold_db;
    let r = ratio.max(1.0);
    if knee_db > 0.0 && 2.0 * over.abs() <= knee_db {
        let t = over + knee_db * 0.5;
        x_db + (1.0 / r - 1.0) * t * t / (2.0 * knee_db)
    } else if over > 0.0 {
        threshold_db + over / r
    } else {
        x_db
    }
}

#[derive(Clone, Debug)]
pub struct Compressor {
    sr: f32,
    params: [f32; 7],
    threshold: Ramp,
    ratio: Ramp,
    knee: Ramp,
    makeup: Ramp, // linear
    mix: Ramp,
    a_att: f32,
    a_rel: f32,
    a_det: f32,
    ms: [f32; 2],
    gr_db: f32,
    sc: [Vec<f32>; 2],
    sc_len: usize,
}

impl Compressor {
    pub fn new(sample_rate: f32, max_block: usize) -> Compressor {
        let params = super::defaults::<7>(KIND);
        let mut c = Compressor {
            sr: sample_rate,
            params,
            threshold: Ramp::new(0.0, 1),
            ratio: Ramp::new(1.0, 1),
            knee: Ramp::new(0.0, 1),
            makeup: Ramp::new(1.0, 1),
            mix: Ramp::new(1.0, 1),
            a_att: 0.0,
            a_rel: 0.0,
            a_det: 0.0,
            ms: [0.0; 2],
            gr_db: 0.0,
            sc: [Vec::new(), Vec::new()],
            sc_len: 0,
        };
        c.prepare(sample_rate, max_block);
        c
    }

    /// Current smoothed gain reduction in dB (>= 0), for meters.
    pub fn gain_reduction_db(&self) -> f32 {
        self.gr_db
    }

    fn update_times(&mut self) {
        self.a_att = time_coeff(self.params[2] * 0.001, self.sr);
        self.a_rel = time_coeff(self.params[3] * 0.001, self.sr);
        self.a_det = time_coeff(DETECTOR_SECONDS, self.sr);
    }
}

impl Processor for Compressor {
    fn prepare(&mut self, sample_rate: f32, max_block: usize) {
        self.sr = sample_rate;
        let len = ramp_len(RAMP_SECONDS, sample_rate);
        let p = self.params;
        self.threshold = Ramp::new(p[0], len);
        self.ratio = Ramp::new(p[1], len);
        self.knee = Ramp::new(p[4], len);
        self.makeup = Ramp::new(db_to_gain(p[5]), len);
        self.mix = Ramp::new(p[6], len);
        self.update_times();
        self.sc = [vec![0.0; max_block], vec![0.0; max_block]];
        self.sc_len = 0;
        self.ms = [0.0; 2];
        self.gr_db = 0.0;
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let keyed = self.sc_len;
        for i in 0..n {
            let (xl, xr) = (sanitize(left[i]), sanitize(right[i]));
            let (kl, kr) = if i < keyed {
                (self.sc[0][i], self.sc[1][i])
            } else {
                (xl, xr)
            };
            let a = self.a_det;
            self.ms[0] = sanitize(kl * kl + (self.ms[0] - kl * kl) * a);
            self.ms[1] = sanitize(kr * kr + (self.ms[1] - kr * kr) * a);
            let level = self.ms[0].max(self.ms[1]);
            let x_db = 10.0 * level.max(1e-20).log10();
            let thr = self.threshold.next();
            let ratio = self.ratio.next();
            let knee = self.knee.next();
            let target = (x_db - static_curve(x_db, thr, ratio, knee)).max(0.0);
            let coeff = if target > self.gr_db {
                self.a_att
            } else {
                self.a_rel
            };
            self.gr_db = sanitize(target + (self.gr_db - target) * coeff);
            let g = db_to_gain(-self.gr_db) * self.makeup.next();
            let mix = self.mix.next();
            let dry = 1.0 - mix;
            left[i] = sanitize(xl * (dry + mix * g));
            right[i] = sanitize(xr * (dry + mix * g));
        }
        self.sc_len = 0;
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(v) = super::clamp_param(KIND, index, value) else {
            return;
        };
        self.params[index] = v;
        match index {
            0 => self.threshold.set(v),
            1 => self.ratio.set(v),
            2 | 3 => self.update_times(),
            4 => self.knee.set(v),
            5 => self.makeup.set(db_to_gain(v)),
            6 => self.mix.set(v),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.threshold.snap();
        self.ratio.snap();
        self.knee.snap();
        self.makeup.snap();
        self.mix.snap();
        self.ms = [0.0; 2];
        self.gr_db = 0.0;
        self.sc_len = 0;
    }

    fn set_sidechain(&mut self, left: &[f32], right: &[f32]) {
        let n = left.len().min(right.len()).min(self.sc[0].len());
        self.sc[0][..n].copy_from_slice(&left[..n]);
        self.sc[1][..n].copy_from_slice(&right[..n]);
        self.sc_len = n;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knee_is_continuous() {
        let (t, r, k) = (-20.0, 4.0, 6.0);
        let below = static_curve(t - k / 2.0 - 1e-3, t, r, k);
        let edge = static_curve(t - k / 2.0 + 1e-3, t, r, k);
        assert!((below - edge).abs() < 1e-2);
        let hi = static_curve(t + k / 2.0 - 1e-3, t, r, k);
        let hi2 = static_curve(t + k / 2.0 + 1e-3, t, r, k);
        assert!((hi - hi2).abs() < 1e-2);
        assert!((static_curve(0.0, t, r, k) - (t + 20.0 / 4.0)).abs() < 1e-5);
    }
}
