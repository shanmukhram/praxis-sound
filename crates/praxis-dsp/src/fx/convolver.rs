//! Convolution reverb (kind 6): uniformly partitioned FFT convolution, zero latency.
//!
//! Parameters: 0 predelay_ms (ms), 1 decay_scale (IR envelope time-stretch), 2 hp_hz (wet
//! high-pass, 12 dB/oct), 3 mix (wet fraction).
//!
//! The wet input is the mono sum `(l + r) / 2`, predelayed, convolved with the left and right
//! channels of the impulse response. The IR is cut into partitions of [`PART`] samples. The
//! first partition runs as a direct-form FIR, so there is no latency; partitions 1.. run as
//! overlap-save FFT convolution (FFT size 2 x PART) with a frequency-domain delay line. The FFT
//! work happens once per PART samples.
//!
//! `decay_scale` reshapes the IR envelope: the IR's own RT60 `T` is estimated (Schroeder T20)
//! when it is set, and sample `t` is multiplied by `10^(3 t / T (1 - 1/s))` (capped at +30 dB),
//! which turns an exponential decay of RT60 `T` into one of `s T`. A change is rebuilt
//! incrementally on the audio thread (a few partitions per PART samples, no allocation) into a
//! second spectrum set, then crossfaded in over one partition.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

use super::ramp::{Biquad64, RAMP_SECONDS, Ramp, ramp_len, read_frac};
use crate::biquad::{BiquadShape, Coeffs};
use crate::buffer::StereoBuffer;
use crate::delayline::DelayLine;
use crate::traits::Processor;
use crate::util::sanitize;

const KIND: usize = 6;
/// Partition length in samples.
pub const PART: usize = 512;
const FFT: usize = 2 * PART;
const BINS: usize = PART + 1;
const MAX_PREDELAY_MS: f32 = 200.0;
/// Longest accepted impulse response in seconds; longer ones are truncated.
pub const MAX_IR_SECONDS: f32 = 30.0;
/// Partitions rebuilt per PART samples while a decay_scale change is pending.
const REBUILD_PER_BLOCK: usize = 8;
const MAX_RESHAPE_DB: f64 = 30.0;

#[derive(Clone)]
struct IrSet {
    head: [Vec<f32>; 2],
    spec: [Vec<Complex<f32>>; 2],
}

impl IrSet {
    fn new(parts: usize) -> IrSet {
        IrSet {
            head: [vec![0.0; PART], vec![0.0; PART]],
            spec: [
                vec![Complex::new(0.0, 0.0); parts * BINS],
                vec![Complex::new(0.0, 0.0); parts * BINS],
            ],
        }
    }
}

pub struct Convolver {
    sr: f32,
    params: [f32; 4],
    predelay: Ramp, // samples
    hp_hz: Ramp,    // log2 Hz
    mix: Ramp,
    // impulse response
    raw: [Vec<f32>; 2],
    ir_len: usize,
    ir_t60: f64,
    parts: usize,
    sets: [IrSet; 2],
    active: usize,
    active_scale: f32,
    // incremental rebuild of the inactive set: Some(next partition) while running
    rebuild: Option<usize>,
    rebuild_scale: f32,
    ready: bool,
    xfade: bool,
    // FFT
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Arc<dyn ComplexToReal<f32>>,
    time_buf: Vec<f32>,
    freq_buf: Vec<Complex<f32>>,
    acc: Vec<Complex<f32>>,
    scratch_f: Vec<Complex<f32>>,
    scratch_i: Vec<Complex<f32>>,
    // input
    pre: DelayLine,
    fdl: Vec<Complex<f32>>,
    fdl_pos: usize,
    prev_block: Vec<f32>,
    cur_block: Vec<f32>,
    pos: usize,
    hist: Vec<f32>,
    hist_pos: usize,
    tail: [[Vec<f32>; 2]; 2],
    hp: [Biquad64; 2],
}

impl std::fmt::Debug for Convolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Convolver")
            .field("sr", &self.sr)
            .field("params", &self.params)
            .field("ir_len", &self.ir_len)
            .field("parts", &self.parts)
            .finish()
    }
}

/// Schroeder-integration RT60 estimate (T20, -5..-25 dB) of a mono energy signal, in samples.
fn estimate_t60(energy: &[f64]) -> Option<f64> {
    let total: f64 = energy.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let mut rem = total;
    let (mut t5, mut t25) = (None, None);
    for (i, e) in energy.iter().enumerate() {
        let db = 10.0 * (rem / total).max(1e-30).log10();
        if t5.is_none() && db <= -5.0 {
            t5 = Some(i);
        }
        if t25.is_none() && db <= -25.0 {
            t25 = Some(i);
            break;
        }
        rem -= e;
    }
    match (t5, t25) {
        (Some(a), Some(b)) if b > a => Some(3.0 * (b - a) as f64),
        _ => None,
    }
}

impl Convolver {
    pub fn new(sample_rate: f32, max_block: usize) -> Convolver {
        let params = super::defaults::<4>(KIND);
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(FFT);
        let inv = planner.plan_fft_inverse(FFT);
        let scratch_f = fwd.make_scratch_vec();
        let scratch_i = inv.make_scratch_vec();
        let mut c = Convolver {
            sr: sample_rate,
            params,
            predelay: Ramp::new(0.0, 1),
            hp_hz: Ramp::new(0.0, 1),
            mix: Ramp::new(0.0, 1),
            raw: [Vec::new(), Vec::new()],
            ir_len: 0,
            ir_t60: 1.0,
            parts: 0,
            sets: [IrSet::new(0), IrSet::new(0)],
            active: 0,
            active_scale: 1.0,
            rebuild: None,
            rebuild_scale: 1.0,
            ready: false,
            xfade: false,
            fwd,
            inv,
            time_buf: vec![0.0; FFT],
            freq_buf: vec![Complex::new(0.0, 0.0); BINS],
            acc: vec![Complex::new(0.0, 0.0); BINS],
            scratch_f,
            scratch_i,
            pre: DelayLine::new(4),
            fdl: Vec::new(),
            fdl_pos: 0,
            prev_block: vec![0.0; PART],
            cur_block: vec![0.0; PART],
            pos: 0,
            hist: vec![0.0; 2 * PART],
            hist_pos: 0,
            tail: std::array::from_fn(|_| [vec![0.0; PART], vec![0.0; PART]]),
            hp: [Biquad64::new(&Coeffs::IDENTITY); 2],
        };
        c.prepare(sample_rate, max_block);
        c
    }

    /// Sets the impulse response (at the processing sample rate). Allocates; call off the audio
    /// thread. Resets the convolution state. A mono IR may leave `right` empty.
    pub fn set_impulse(&mut self, ir: &StereoBuffer) {
        let max = (MAX_IR_SECONDS * self.sr) as usize;
        let n = ir.left.len().min(max);
        let right = if ir.right.len() >= n {
            &ir.right[..n]
        } else {
            &ir.left[..n]
        };
        self.raw = [
            ir.left[..n].iter().map(|&v| sanitize(v)).collect(),
            right.iter().map(|&v| sanitize(v)).collect(),
        ];
        self.ir_len = n;
        let energy: Vec<f64> = (0..n)
            .map(|i| {
                let (l, r) = (self.raw[0][i] as f64, self.raw[1][i] as f64);
                l * l + r * r
            })
            .collect();
        self.ir_t60 = estimate_t60(&energy).unwrap_or(n.max(1) as f64);
        self.parts = n.saturating_sub(PART).div_ceil(PART);
        self.sets = [IrSet::new(self.parts), IrSet::new(self.parts)];
        self.fdl = vec![Complex::new(0.0, 0.0); self.parts.max(1) * BINS];
        let scale = self.params[1];
        self.active = 0;
        self.build_head(0, scale);
        for k in 0..self.parts {
            self.build_part(0, k, scale);
        }
        self.active_scale = scale;
        self.rebuild = None;
        self.ready = false;
        self.xfade = false;
        self.clear_state();
    }

    /// The IR RT60 estimate in seconds (0 when no IR is set).
    pub fn impulse_rt60_seconds(&self) -> f32 {
        if self.ir_len == 0 {
            0.0
        } else {
            (self.ir_t60 / self.sr as f64) as f32
        }
    }

    #[inline]
    fn reshape(&self, t: usize, scale: f32) -> f32 {
        if scale == 1.0 {
            return 1.0;
        }
        let k = 3.0 * std::f64::consts::LN_10 / self.ir_t60 * (1.0 - 1.0 / scale as f64);
        let lim = MAX_RESHAPE_DB / 20.0 * std::f64::consts::LN_10;
        (k * t as f64).min(lim).exp() as f32
    }

    fn build_head(&mut self, set: usize, scale: f32) {
        for ch in 0..2 {
            for i in 0..PART {
                let v = if i < self.ir_len {
                    self.raw[ch][i] * self.reshape(i, scale)
                } else {
                    0.0
                };
                self.sets[set].head[ch][i] = v;
            }
        }
    }

    /// Partition `k` (0-based, covering IR samples (k+1)*PART ..) of set `set`.
    fn build_part(&mut self, set: usize, k: usize, scale: f32) {
        for ch in 0..2 {
            let off = (k + 1) * PART;
            for i in 0..FFT {
                self.time_buf[i] = if i < PART && off + i < self.ir_len {
                    self.raw[ch][off + i] * self.reshape(off + i, scale)
                } else {
                    0.0
                };
            }
            let _ = self.fwd.process_with_scratch(
                &mut self.time_buf,
                &mut self.freq_buf,
                &mut self.scratch_f,
            );
            self.sets[set].spec[ch][k * BINS..(k + 1) * BINS].copy_from_slice(&self.freq_buf);
        }
    }

    fn clear_state(&mut self) {
        self.pre.reset();
        self.fdl
            .iter_mut()
            .for_each(|c| *c = Complex::new(0.0, 0.0));
        self.fdl_pos = 0;
        self.prev_block.iter_mut().for_each(|v| *v = 0.0);
        self.cur_block.iter_mut().for_each(|v| *v = 0.0);
        self.pos = 0;
        self.hist.iter_mut().for_each(|v| *v = 0.0);
        self.hist_pos = 0;
        for t in self.tail.iter_mut() {
            for c in t.iter_mut() {
                c.iter_mut().for_each(|v| *v = 0.0);
            }
        }
        self.hp[0].reset();
        self.hp[1].reset();
    }

    fn update_hp(&mut self) {
        let c = Coeffs::design(
            BiquadShape::HighPass,
            self.hp_hz.value().exp2() as f64,
            std::f64::consts::FRAC_1_SQRT_2,
            self.sr as f64,
        );
        self.hp[0].set(&c);
        self.hp[1].set(&c);
    }

    /// Partitions 1.. of `set`, for the output block after the one just completed, into
    /// `tail[slot]`.
    fn tail_for(&mut self, set: usize, slot: usize) {
        let p = self.parts;
        for ch in 0..2 {
            self.acc
                .iter_mut()
                .for_each(|c| *c = Complex::new(0.0, 0.0));
            for k in 0..p {
                // X_{j-k} for partition k+1 (fdl_pos holds X_j)
                let xi = (self.fdl_pos + p - k) % p;
                let x = &self.fdl[xi * BINS..(xi + 1) * BINS];
                let h = &self.sets[set].spec[ch][k * BINS..(k + 1) * BINS];
                for ((a, xv), hv) in self.acc.iter_mut().zip(x.iter()).zip(h.iter()) {
                    *a += xv * hv;
                }
            }
            let _ = self.inv.process_with_scratch(
                &mut self.acc,
                &mut self.time_buf,
                &mut self.scratch_i,
            );
            let norm = 1.0 / FFT as f32;
            for i in 0..PART {
                self.tail[slot][ch][i] = self.time_buf[PART + i] * norm;
            }
        }
    }

    fn block_boundary(&mut self) {
        // finish a crossfade that ran over the block just completed
        if self.xfade {
            self.xfade = false;
            self.active ^= 1;
            self.tail.swap(0, 1);
            self.active_scale = self.rebuild_scale;
        }
        if self.parts > 0 {
            self.time_buf[..PART].copy_from_slice(&self.prev_block);
            self.time_buf[PART..].copy_from_slice(&self.cur_block);
            self.fdl_pos = (self.fdl_pos + 1) % self.parts;
            let _ = self.fwd.process_with_scratch(
                &mut self.time_buf,
                &mut self.freq_buf,
                &mut self.scratch_f,
            );
            let at = self.fdl_pos * BINS;
            self.fdl[at..at + BINS].copy_from_slice(&self.freq_buf);
            self.tail_for(self.active, 0);
        }
        std::mem::swap(&mut self.prev_block, &mut self.cur_block);
        // incremental rebuild of the inactive set
        let target = self.params[1];
        if self.ir_len > 0 && self.rebuild.is_none() && !self.ready && target != self.active_scale {
            self.rebuild = Some(0);
            self.rebuild_scale = target;
            let other = self.active ^ 1;
            self.build_head(other, target);
        }
        if let Some(mut k) = self.rebuild {
            if self.rebuild_scale != target {
                // the target moved again: restart
                k = 0;
                self.rebuild_scale = target;
                let other = self.active ^ 1;
                self.build_head(other, target);
            }
            let other = self.active ^ 1;
            let end = (k + REBUILD_PER_BLOCK).min(self.parts);
            for part in k..end {
                self.build_part(other, part, target);
            }
            if end >= self.parts {
                self.rebuild = None;
                self.ready = true;
            } else {
                self.rebuild = Some(end);
            }
        } else if self.ready {
            // start the crossfade over the next block
            self.ready = false;
            if self.rebuild_scale == target {
                self.xfade = true;
                if self.parts > 0 {
                    self.tail_for(self.active ^ 1, 1);
                }
            }
        }
    }

    #[inline]
    fn head(&self, set: usize, ch: usize) -> f32 {
        let h = &self.sets[set].head[ch];
        let x = &self.hist[self.hist_pos..self.hist_pos + PART];
        let mut acc = 0.0f32;
        for (a, b) in h.iter().zip(x.iter()) {
            acc += a * b;
        }
        acc
    }
}

impl Processor for Convolver {
    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sr = sample_rate;
        let len = ramp_len(RAMP_SECONDS, sample_rate);
        let ms = sample_rate * 0.001;
        let p = self.params;
        self.predelay = Ramp::new(p[0] * ms, len);
        self.hp_hz = Ramp::new(p[2].log2(), len);
        self.mix = Ramp::new(p[3], len);
        self.pre = DelayLine::new((MAX_PREDELAY_MS * ms).ceil() as usize + 8);
        self.update_hp();
        self.clear_state();
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let active_ir = self.ir_len > 0;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let pd = self.predelay.next();
            let mix = self.mix.next();
            if self.hp_hz.moving() {
                self.hp_hz.next();
                self.update_hp();
            }
            let (xl, xr) = (sanitize(*l), sanitize(*r));
            let (mut wl, mut wr) = (0.0, 0.0);
            if active_ir {
                self.pre.push(0.5 * (xl + xr));
                let x = read_frac(&self.pre, pd);
                self.hist_pos = if self.hist_pos == 0 {
                    PART - 1
                } else {
                    self.hist_pos - 1
                };
                self.hist[self.hist_pos] = x;
                self.hist[self.hist_pos + PART] = x;
                self.cur_block[self.pos] = x;
                let a = self.active;
                wl = self.head(a, 0) + self.tail[0][0][self.pos];
                wr = self.head(a, 1) + self.tail[0][1][self.pos];
                if self.xfade {
                    let b = a ^ 1;
                    let nl = self.head(b, 0) + self.tail[1][0][self.pos];
                    let nr = self.head(b, 1) + self.tail[1][1][self.pos];
                    let w = (self.pos + 1) as f32 / PART as f32;
                    wl += (nl - wl) * w;
                    wr += (nr - wr) * w;
                }
                self.pos += 1;
                if self.pos == PART {
                    self.pos = 0;
                    self.block_boundary();
                }
            }
            let wl = self.hp[0].tick(wl);
            let wr = self.hp[1].tick(wr);
            *l = sanitize(xl + (wl - xl) * mix);
            *r = sanitize(xr + (wr - xr) * mix);
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(v) = super::clamp_param(KIND, index, value) else {
            return;
        };
        self.params[index] = v;
        match index {
            0 => self.predelay.set(v * self.sr * 0.001),
            2 => self.hp_hz.set(v.log2()),
            3 => self.mix.set(v),
            _ => {} // decay_scale: rebuilt at the next partition boundary
        }
    }

    fn reset(&mut self) {
        self.predelay.snap();
        self.hp_hz.snap();
        self.mix.snap();
        self.update_hp();
        // apply any pending decay change completely
        if self.ir_len > 0 && (self.params[1] != self.active_scale || self.xfade) {
            let s = self.params[1];
            let a = self.active;
            self.build_head(a, s);
            for k in 0..self.parts {
                self.build_part(a, k, s);
            }
            self.active_scale = s;
        }
        self.rebuild = None;
        self.ready = false;
        self.xfade = false;
        self.clear_state();
    }

    /// Predelay + IR length + ring-out of the wet high-pass.
    fn tail_samples(&self) -> usize {
        if self.ir_len == 0 {
            return 0;
        }
        let pd = self.predelay.value().max(self.predelay.target()).ceil() as usize;
        pd + self.ir_len + self.hp[0].tail()
    }
}
