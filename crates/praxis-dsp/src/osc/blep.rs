//! Band-limited step (BLEP) and ramp (BLAMP) correction shared by the classic-waveform
//! oscillators.
//!
//! The correction kernel is a Kaiser-windowed sinc spanning [`HALF`] samples either side of a
//! discontinuity (16 taps), integrated once for steps and twice for slope changes. Compared with
//! the textbook 2-point polyBLEP it keeps the passband flat to about 15 kHz at 48 kHz and pushes
//! aliasing in the audible band down by more than 60 dB, at the cost of [`HALF`] samples of
//! fixed latency. The tables are built once per process (no allocation, stored in a static).

use std::sync::OnceLock;

/// Half-width of the correction kernel in samples; also the oscillator latency in samples.
pub const HALF: usize = 8;
/// Number of taps (output samples touched by one discontinuity).
const LEN: usize = 2 * HALF;
/// Table points per sample.
const RES: usize = 256;
const POINTS: usize = LEN * RES + 1;
/// Kernel cutoff in cycles per sample (0.45 = 21.6 kHz at 48 kHz).
const CUTOFF: f64 = 0.45;
/// Kaiser window shape.
const BETA: f64 = 8.5;
/// Largest `f32` below 1.
const BELOW_ONE: f32 = 1.0 - f32::EPSILON / 2.0;

struct Tables {
    /// Integrated kernel I(t), t in [-HALF, HALF]; rises smoothly from 0 to 1.
    step: [f32; POINTS],
    /// Ramp residual R(t) = integral of (I - u) from -HALF to t; zero at both ends.
    ramp: [f32; POINTS],
}

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let half = x * 0.5;
    for k in 1..64 {
        let f = half / k as f64;
        term *= f * f;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

fn kernel(x: f64) -> f64 {
    let r = x / HALF as f64;
    if r.abs() >= 1.0 {
        return 0.0;
    }
    let w = bessel_i0(BETA * (1.0 - r * r).sqrt()) / bessel_i0(BETA);
    let a = 2.0 * std::f64::consts::PI * CUTOFF * x;
    let sinc = if a.abs() < 1e-12 { 1.0 } else { a.sin() / a };
    2.0 * CUTOFF * sinc * w
}

fn build() -> Tables {
    // Integrate on a grid 8x finer than the stored table (trapezoid rule).
    const SUB: usize = 8;
    let fine = (LEN * RES * SUB) as f64;
    let h = LEN as f64 / fine;
    let mut step = [0f64; POINTS];
    let mut ramp_raw = [0f64; POINTS];
    let mut i_acc = 0.0f64;
    let mut ii_acc = 0.0f64;
    let mut prev_k = kernel(-(HALF as f64));
    let mut prev_i = 0.0f64;
    for f in 1..=(LEN * RES * SUB) {
        let x = -(HALF as f64) + f as f64 * h;
        let k = kernel(x);
        i_acc += 0.5 * (prev_k + k) * h;
        ii_acc += 0.5 * (prev_i + i_acc) * h;
        prev_k = k;
        prev_i = i_acc;
        if f % SUB == 0 {
            step[f / SUB] = i_acc;
            ramp_raw[f / SUB] = ii_acc;
        }
    }
    let total = i_acc;
    let mut t = Tables {
        step: [0.0; POINTS],
        ramp: [0.0; POINTS],
    };
    for p in 0..POINTS {
        let x = -(HALF as f64) + p as f64 / RES as f64;
        let i = step[p] / total;
        let ii = ramp_raw[p] / total;
        t.step[p] = i as f32;
        t.ramp[p] = (ii - x.max(0.0)) as f32;
    }
    t.step[POINTS - 1] = 1.0;
    t.ramp[POINTS - 1] = 0.0;
    t
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(build)
}

#[inline]
fn lookup(table: &[f32; POINTS], tau: f32) -> f32 {
    let p = ((tau + HALF as f32) * RES as f32).clamp(0.0, (POINTS - 1) as f32);
    let i = (p as usize).min(POINTS - 2);
    let fr = p - i as f32;
    table[i] + (table[i + 1] - table[i]) * fr
}

/// Correction accumulator. Discontinuities are added for the upcoming sample, then the naive
/// value of that sample is pushed and the fully corrected sample from [`HALF`] samples ago pops
/// out.
#[derive(Clone)]
pub struct Blep {
    buf: [f32; LEN],
    head: usize,
    tab: &'static Tables,
}

impl std::fmt::Debug for Blep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Blep").field("head", &self.head).finish()
    }
}

impl Blep {
    pub fn new() -> Blep {
        Blep {
            buf: [0.0; LEN],
            head: 0,
            tab: tables(),
        }
    }

    pub fn clear(&mut self) {
        self.buf = [0.0; LEN];
        self.head = 0;
    }

    /// A step of `height` that happened `frac` samples (`0..1`) before the upcoming sample.
    ///
    /// `frac` is kept strictly below 1: a step exactly one sample back would sit on the
    /// previous sample, whose naive value was taken before the step, so it must count as
    /// "just after" that sample (rounding to 1.0 in `f32` would otherwise double the edge).
    #[inline]
    pub fn add_step(&mut self, frac: f32, height: f32) {
        let frac = frac.clamp(0.0, BELOW_ONE);
        for j in 0..LEN {
            let tau = j as f32 - HALF as f32 + frac;
            let i = lookup(&self.tab.step, tau);
            let r = if tau >= 0.0 { i - 1.0 } else { i };
            self.buf[(self.head + j) % LEN] += height * r;
        }
    }

    /// A change of slope by `delta` (output units per sample) `frac` samples before the upcoming
    /// sample.
    #[inline]
    pub fn add_ramp(&mut self, frac: f32, delta: f32) {
        let frac = frac.clamp(0.0, 1.0);
        for j in 0..LEN {
            let tau = j as f32 - HALF as f32 + frac;
            self.buf[(self.head + j) % LEN] += delta * lookup(&self.tab.ramp, tau);
        }
    }

    /// Pushes the naive value of the upcoming sample and returns the corrected sample from
    /// [`HALF`] samples ago.
    #[inline]
    pub fn push(&mut self, naive: f32) -> f32 {
        self.buf[(self.head + HALF) % LEN] += naive;
        let out = self.buf[self.head];
        self.buf[self.head] = 0.0;
        self.head = (self.head + 1) % LEN;
        crate::util::sanitize(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sane() {
        let t = tables();
        assert_eq!(t.step[0], 0.0);
        assert_eq!(t.step[POINTS - 1], 1.0);
        assert!((t.step[POINTS / 2] - 0.5).abs() < 1e-6);
        // Antisymmetric step residual, ramp residual vanishing at the ends.
        for p in 0..POINTS {
            assert!((t.step[p] + t.step[POINTS - 1 - p] - 1.0).abs() < 1e-5);
        }
        assert!(t.ramp[0].abs() < 1e-6);
        assert!(t.ramp[POINTS - 2].abs() < 1e-4);
    }

    #[test]
    fn step_reconstructs_band_limited_edge() {
        // A unit step at the upcoming sample: corrected output is 0 long before, 1 long after
        // and 0.5 exactly at the step.
        let mut b = Blep::new();
        b.add_step(0.0, 1.0);
        let mut out = Vec::new();
        for _ in 0..(3 * HALF) {
            out.push(b.push(1.0));
        }
        // out[HALF] is the step sample.
        assert!(out[0].abs() < 1e-6);
        assert!((out[HALF] - 0.5).abs() < 1e-5);
        assert!((out[3 * HALF - 1] - 1.0).abs() < 1e-6);
    }
}
