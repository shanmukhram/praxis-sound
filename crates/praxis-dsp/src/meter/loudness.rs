//! ITU-R BS.1770-4 / EBU R128 loudness meter (stereo).
//!
//! K-weighting is the two-stage filter of BS.1770 with coefficients derived from its analog
//! prototype at any sample rate (the libebur128 method), run in f64. Audio is accumulated in
//! 100 ms sub-blocks (`(rate + 5) / 10` samples, as libebur128):
//! - momentary: the last 4 sub-blocks (400 ms), updated every 100 ms;
//! - short-term: the last 30 sub-blocks (3 s), updated every 100 ms;
//! - integrated: 400 ms gating blocks every 100 ms, absolute gate -70 LUFS, relative gate
//!   -10 LU;
//! - loudness range (EBU Tech 3342): 3 s blocks every 1 s, absolute gate -70 LUFS, relative
//!   gate -20 LU, 10th to 95th percentile.
//!
//! Gated blocks are kept in fixed histograms with 0.01 LU bins (exact energy sums per bin), so
//! `process` never allocates and memory does not grow with duration. Silence reads `-inf`.

const BIN_LU: f64 = 0.01;
const LOW_LUFS: f64 = -70.0;
const HIGH_LUFS: f64 = 10.0;
const NBINS: usize = ((HIGH_LUFS - LOW_LUFS) / BIN_LU) as usize;
const SHORT_SUBS: usize = 30;

#[inline]
fn energy_to_lufs(e: f64) -> f64 {
    if e > 0.0 {
        -0.691 + 10.0 * e.log10()
    } else {
        f64::NEG_INFINITY
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Bq64 {
    b: [f64; 3],
    a: [f64; 3],
    z1: f64,
    z2: f64,
}

impl Bq64 {
    #[inline]
    fn tick(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z1;
        self.z1 = self.b[1] * x - self.a[1] * y + self.z2;
        self.z2 = self.b[2] * x - self.a[2] * y;
        if self.z1.abs() < 1e-30 {
            self.z1 = 0.0;
        }
        if self.z2.abs() < 1e-30 {
            self.z2 = 0.0;
        }
        y
    }
}

/// The two K-weighting stages (high shelf, RLB high-pass) for `rate`.
fn k_weighting(rate: f64) -> [Bq64; 2] {
    use std::f64::consts::PI;
    let f0 = 1681.974450955533;
    let g = 3.999843853973347;
    let q = 0.7071752369554196;
    let k = (PI * f0 / rate).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Bq64 {
        b: [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
        ],
        a: [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        z1: 0.0,
        z2: 0.0,
    };
    let f0 = 38.13547087602444;
    let q = 0.5003270373238773;
    let k = (PI * f0 / rate).tan();
    let d = 1.0 + k / q + k * k;
    let hp = Bq64 {
        b: [1.0, -2.0, 1.0],
        a: [1.0, 2.0 * (k * k - 1.0) / d, (1.0 - k / q + k * k) / d],
        z1: 0.0,
        z2: 0.0,
    };
    [shelf, hp]
}

/// Fixed-resolution histogram of block energies.
#[derive(Clone, Debug)]
struct Hist {
    count: Vec<u64>,
    energy: Vec<f64>,
    total_count: u64,
    total_energy: f64,
}

impl Hist {
    fn new() -> Hist {
        Hist {
            count: vec![0; NBINS],
            energy: vec![0.0; NBINS],
            total_count: 0,
            total_energy: 0.0,
        }
    }

    fn clear(&mut self) {
        self.count.iter_mut().for_each(|c| *c = 0);
        self.energy.iter_mut().for_each(|e| *e = 0.0);
        self.total_count = 0;
        self.total_energy = 0.0;
    }

    fn bin(lufs: f64) -> usize {
        (((lufs - LOW_LUFS) / BIN_LU).floor().max(0.0) as usize).min(NBINS - 1)
    }

    fn bin_center(i: usize) -> f64 {
        LOW_LUFS + (i as f64 + 0.5) * BIN_LU
    }

    /// Adds a block if it passes the absolute gate.
    fn add(&mut self, e: f64) {
        let l = energy_to_lufs(e);
        if l.is_finite() && l >= LOW_LUFS {
            let i = Self::bin(l);
            self.count[i] += 1;
            self.energy[i] += e;
            self.total_count += 1;
            self.total_energy += e;
        }
    }

    /// First bin at or above the relative threshold `gate_lu` below the mean.
    fn relative_start(&self, gate_lu: f64) -> Option<usize> {
        if self.total_count == 0 {
            return None;
        }
        let mean = self.total_energy / self.total_count as f64;
        let thr = mean * 10f64.powf(gate_lu / 10.0);
        let tl = energy_to_lufs(thr);
        let mut i = Self::bin(tl);
        // the threshold's own bin is included when its blocks average above the threshold
        if self.count[i] > 0 && self.energy[i] / (self.count[i] as f64) < thr {
            i += 1;
        }
        Some(i)
    }

    fn gated_mean(&self, gate_lu: f64) -> f64 {
        let Some(start) = self.relative_start(gate_lu) else {
            return f64::NEG_INFINITY;
        };
        let (mut n, mut e) = (0u64, 0.0f64);
        for i in start..NBINS {
            n += self.count[i];
            e += self.energy[i];
        }
        if n == 0 {
            f64::NEG_INFINITY
        } else {
            energy_to_lufs(e / n as f64)
        }
    }

    /// Loudness of the `k`-th (0-based, ascending) gated block from `start`.
    fn kth(&self, start: usize, k: u64) -> f64 {
        let mut seen = 0u64;
        for i in start..NBINS {
            seen += self.count[i];
            if seen > k {
                return Self::bin_center(i);
            }
        }
        f64::NEG_INFINITY
    }

    fn range(&self) -> f64 {
        let Some(start) = self.relative_start(-20.0) else {
            return 0.0;
        };
        let n: u64 = self.count[start..].iter().sum();
        if n == 0 {
            return 0.0;
        }
        let size = (n - 1) as f64;
        let hi = self.kth(start, (size * 0.95 + 0.5) as u64);
        let lo = self.kth(start, (size * 0.1 + 0.5) as u64);
        (hi - lo).max(0.0)
    }
}

/// Streaming stereo loudness meter. Allocates only in `new`.
#[derive(Clone, Debug)]
pub struct LoudnessMeter {
    rate: f32,
    sub_len: usize,
    sub_fill: usize,
    sub_acc: f64,
    kw: [[Bq64; 2]; 2],
    subs: [f64; SHORT_SUBS],
    sub_pos: usize,
    subs_done: u64,
    st_counter: usize,
    integrated: Hist,
    lra: Hist,
    max_m: f64,
    max_s: f64,
}

impl LoudnessMeter {
    pub fn new(sample_rate: f32) -> LoudnessMeter {
        let rate = sample_rate.max(1.0);
        let kw = k_weighting(rate as f64);
        LoudnessMeter {
            rate,
            sub_len: ((rate.round() as usize) + 5) / 10,
            sub_fill: 0,
            sub_acc: 0.0,
            kw: [kw, kw],
            subs: [0.0; SHORT_SUBS],
            sub_pos: 0,
            subs_done: 0,
            st_counter: 0,
            integrated: Hist::new(),
            lra: Hist::new(),
            max_m: f64::NEG_INFINITY,
            max_s: f64::NEG_INFINITY,
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.rate
    }

    pub fn reset(&mut self) {
        let kw = k_weighting(self.rate as f64);
        self.kw = [kw, kw];
        self.integrated.clear();
        self.lra.clear();
        self.sub_fill = 0;
        self.sub_acc = 0.0;
        self.subs = [0.0; SHORT_SUBS];
        self.sub_pos = 0;
        self.subs_done = 0;
        self.st_counter = 0;
        self.max_m = f64::NEG_INFINITY;
        self.max_s = f64::NEG_INFINITY;
    }

    /// Feeds stereo audio (equal-length channels; extra samples of the longer one are ignored).
    pub fn process(&mut self, left: &[f32], right: &[f32]) {
        for (&l, &r) in left.iter().zip(right.iter()) {
            let yl = self.kw[0][0].tick(l as f64);
            let zl = self.kw[0][1].tick(yl);
            let yr = self.kw[1][0].tick(r as f64);
            let zr = self.kw[1][1].tick(yr);
            self.sub_acc += zl * zl + zr * zr;
            self.sub_fill += 1;
            if self.sub_fill == self.sub_len {
                self.end_sub_block();
            }
        }
    }

    fn mean_of_last(&self, n: usize) -> f64 {
        let n = n.min(self.subs_done as usize).min(SHORT_SUBS);
        if n == 0 {
            return 0.0;
        }
        let mut s = 0.0;
        for k in 0..n {
            s += self.subs[(self.sub_pos + SHORT_SUBS - 1 - k) % SHORT_SUBS];
        }
        s / (n * self.sub_len) as f64
    }

    fn end_sub_block(&mut self) {
        self.subs[self.sub_pos] = self.sub_acc;
        self.sub_pos = (self.sub_pos + 1) % SHORT_SUBS;
        self.subs_done += 1;
        self.sub_acc = 0.0;
        self.sub_fill = 0;
        if self.subs_done >= 4 {
            let e = self.mean_of_last(4);
            self.integrated.add(e);
            self.max_m = self.max_m.max(energy_to_lufs(e));
        }
        if self.subs_done >= SHORT_SUBS as u64 {
            self.max_s = self
                .max_s
                .max(energy_to_lufs(self.mean_of_last(SHORT_SUBS)));
        }
        self.st_counter += 1;
        if self.st_counter == SHORT_SUBS {
            self.lra.add(self.mean_of_last(SHORT_SUBS));
            self.st_counter = 20;
        }
    }

    /// Momentary loudness (last 400 ms) in LUFS.
    pub fn momentary_lufs(&self) -> f64 {
        energy_to_lufs(self.mean_of_last(4))
    }

    /// Short-term loudness (last 3 s, or what has been fed so far) in LUFS.
    pub fn short_term_lufs(&self) -> f64 {
        energy_to_lufs(self.mean_of_last(SHORT_SUBS))
    }

    /// Gated integrated loudness in LUFS since construction or `reset`.
    pub fn integrated_lufs(&self) -> f64 {
        self.integrated.gated_mean(-10.0)
    }

    /// Loudness range in LU (EBU Tech 3342).
    pub fn loudness_range_lu(&self) -> f64 {
        self.lra.range()
    }

    /// Highest momentary loudness seen, in LUFS.
    pub fn max_momentary_lufs(&self) -> f64 {
        self.max_m
    }

    /// Highest short-term loudness seen (after the first 3 s), in LUFS.
    pub fn max_short_term_lufs(&self) -> f64 {
        self.max_s
    }
}

/// Integrated loudness of a whole stereo signal, in LUFS.
pub fn integrated_lufs(left: &[f32], right: &[f32], sample_rate: f32) -> f64 {
    let mut m = LoudnessMeter::new(sample_rate);
    m.process(left, right);
    m.integrated_lufs()
}
