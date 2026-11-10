//! True-peak limiter with lookahead (kind 9).
//!
//! Parameters: 0 ceiling_dbtp (dBTP), 1 release_ms (ms), 2 lookahead_ms (ms, adds latency).
//!
//! Detection uses the 4x true-peak detector of [`crate::meter::true_peak`] on both channels
//! (stereo-linked). For every segment the required gain `ceiling / peak` is held for a window
//! wide enough to cover the interpolator's reach (+-6 samples), released exponentially, then
//! smoothed by a box filter as long as the lookahead. Because the box average of held minima
//! never rises above the minimum it covers, the gain at every sample within 6 samples of a
//! peak is at or below that peak's required gain, so the output's true peak stays at the
//! ceiling. `latency_samples()` equals the lookahead (at least 12 samples).
//!
//! Changing `lookahead_ms` changes the latency and restarts the gain computer; it is meant to be
//! set once, not automated.

use super::ramp::{RAMP_SECONDS, Ramp, ramp_len};
use crate::meter::true_peak::{DELAY as DET_DELAY, TruePeakDetector};
use crate::traits::Processor;
use crate::util::{db_to_gain, sanitize, time_coeff};

const KIND: usize = 9;
/// Interpolator reach on either side of a segment, in samples.
const REACH: usize = 6;
/// Latency beyond the box filter: detector delay + reach - 1.
const EXTRA: usize = DET_DELAY + REACH - 1;
const MIN_LATENCY: usize = EXTRA + 2;
const MAX_LOOKAHEAD_MS: f32 = 10.0;
/// Internal safety margin below the ceiling (dB).
const MARGIN_DB: f32 = 0.02;

#[derive(Clone, Debug)]
pub struct TruePeakLimiter {
    sr: f32,
    params: [f32; 3],
    ceiling: Ramp, // linear, margin applied
    rel_a: f32,
    det: [TruePeakDetector; 2],
    latency: usize,
    box_len: usize,
    win: usize,
    // signal delay
    delay: [Vec<f32>; 2],
    dpos: usize,
    // sliding minimum (monotonic deque in a ring)
    dq_val: Vec<f32>,
    dq_idx: Vec<u64>,
    dq_head: usize,
    dq_len: usize,
    count: u64,
    // release and box
    rel: f32,
    boxbuf: Vec<f32>,
    box_pos: usize,
    box_sum: f64,
    gain: f32,
}

impl TruePeakLimiter {
    pub fn new(sample_rate: f32, max_block: usize) -> TruePeakLimiter {
        let params = super::defaults::<3>(KIND);
        let mut l = TruePeakLimiter {
            sr: sample_rate,
            params,
            ceiling: Ramp::new(1.0, 1),
            rel_a: 0.0,
            det: [TruePeakDetector::new(), TruePeakDetector::new()],
            latency: MIN_LATENCY,
            box_len: 2,
            win: 2,
            delay: [Vec::new(), Vec::new()],
            dpos: 0,
            dq_val: Vec::new(),
            dq_idx: Vec::new(),
            dq_head: 0,
            dq_len: 0,
            count: 0,
            rel: 1.0,
            boxbuf: Vec::new(),
            box_pos: 0,
            box_sum: 0.0,
            gain: 1.0,
        };
        l.prepare(sample_rate, max_block);
        l
    }

    /// The gain applied to the most recent output sample (linear, <= 1), for meters.
    pub fn gain(&self) -> f32 {
        self.gain
    }

    fn ceiling_lin(db: f32) -> f32 {
        db_to_gain(db - MARGIN_DB)
    }

    fn configure(&mut self) {
        let la = (self.params[2] * 0.001 * self.sr).round() as usize;
        self.latency = la.max(MIN_LATENCY);
        self.box_len = self.latency - EXTRA;
        self.win = self.box_len + 2 * REACH;
        self.clear_state();
    }

    fn clear_state(&mut self) {
        for d in self.delay.iter_mut() {
            d.iter_mut().for_each(|v| *v = 0.0);
        }
        self.dpos = 0;
        self.dq_head = 0;
        self.dq_len = 0;
        self.count = 0;
        self.rel = 1.0;
        self.boxbuf.iter_mut().for_each(|v| *v = 1.0);
        self.box_pos = 0;
        self.box_sum = self.box_len as f64;
        self.gain = 1.0;
        for d in self.det.iter_mut() {
            d.reset();
        }
    }

    #[inline]
    fn sliding_min(&mut self, r: f32) -> f32 {
        let cap = self.dq_val.len();
        // pop back while >= r
        while self.dq_len > 0 {
            let back = (self.dq_head + self.dq_len - 1) % cap;
            if self.dq_val[back] >= r {
                self.dq_len -= 1;
            } else {
                break;
            }
        }
        let slot = (self.dq_head + self.dq_len) % cap;
        self.dq_val[slot] = r;
        self.dq_idx[slot] = self.count;
        self.dq_len += 1;
        // pop front outside the window
        let oldest = self.count + 1 - self.win.min(self.count as usize + 1) as u64;
        while self.dq_idx[self.dq_head] < oldest {
            self.dq_head = (self.dq_head + 1) % cap;
            self.dq_len -= 1;
        }
        self.count += 1;
        self.dq_val[self.dq_head]
    }
}

impl Processor for TruePeakLimiter {
    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sr = sample_rate;
        let max_latency =
            ((MAX_LOOKAHEAD_MS * 0.001 * sample_rate).round() as usize).max(MIN_LATENCY) + 1;
        let max_box = max_latency - EXTRA;
        let max_win = max_box + 2 * REACH;
        self.delay = [vec![0.0; max_latency + 1], vec![0.0; max_latency + 1]];
        self.dq_val = vec![1.0; max_win + 2];
        self.dq_idx = vec![0; max_win + 2];
        self.boxbuf = vec![1.0; max_box + 1];
        self.ceiling = Ramp::new(
            Self::ceiling_lin(self.params[0]),
            ramp_len(RAMP_SECONDS, sample_rate),
        );
        self.rel_a = time_coeff(self.params[1] * 0.001, sample_rate);
        self.configure();
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let dlen = self.latency + 1;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let xl = sanitize(*l);
            let xr = sanitize(*r);
            let peak = self.det[0].push(xl).max(self.det[1].push(xr));
            let c = self.ceiling.next();
            let need = if peak > c { c / peak } else { 1.0 };
            let m = self.sliding_min(need);
            self.rel = if m < self.rel {
                m
            } else {
                m + (self.rel - m) * self.rel_a
            };
            let old = self.boxbuf[self.box_pos];
            self.boxbuf[self.box_pos] = self.rel;
            self.box_pos += 1;
            if self.box_pos == self.box_len {
                self.box_pos = 0;
            }
            self.box_sum += self.rel as f64 - old as f64;
            let g = ((self.box_sum / self.box_len as f64) as f32).min(1.0);
            self.gain = g;
            // delay line: write at dpos, read the sample `latency` samples old
            self.delay[0][self.dpos] = xl;
            self.delay[1][self.dpos] = xr;
            let rd = (self.dpos + 1) % dlen;
            *l = self.delay[0][rd] * g;
            *r = self.delay[1][rd] * g;
            self.dpos = rd;
        }
        // keep the running sum exact over long runs
        if self.count % 65_536 < left.len() as u64 {
            self.box_sum = self.boxbuf[..self.box_len].iter().map(|&v| v as f64).sum();
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(v) = super::clamp_param(KIND, index, value) else {
            return;
        };
        self.params[index] = v;
        match index {
            0 => self.ceiling.set(Self::ceiling_lin(v)),
            1 => self.rel_a = time_coeff(v * 0.001, self.sr),
            2 => {
                let la = ((v * 0.001 * self.sr).round() as usize).max(MIN_LATENCY);
                if la != self.latency {
                    self.configure();
                }
            }
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.ceiling.snap();
        self.clear_state();
    }

    fn latency_samples(&self) -> usize {
        self.latency
    }

    /// No tail beyond the latency (which `latency_samples` reports separately).
    fn tail_samples(&self) -> usize {
        0
    }
}
