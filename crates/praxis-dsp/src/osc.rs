//! Oscillators: sine, band-limited saw / pulse / triangle, mip-mapped wavetable and FM operators.
//!
//! Common interface: `new(sample_rate)`, `set_freq(hz)`, `next() -> f32`, `reset(phase)`.
//! - Frequencies are in Hz. Non-finite values are treated as 0 Hz; values are clamped to
//!   just below Nyquist.
//! - Phase is in cycles, `0.0..1.0`, accumulated in `f64` so pitch stays accurate over long
//!   notes (well under 0.01 cent of drift).
//! - Output is nominally in `-1.0..=1.0`. Band-limited waveforms overshoot slightly at their
//!   edges (Gibbs ripple, about 9 % for the saw), as a real band-limited signal does.
//! - `reset(phase)` restarts the oscillator for a new note. It does not allocate.
//!
//! [`PolyBlepSaw`], [`PolyBlepSquare`] and [`Triangle`] correct their discontinuities with a
//! 16-tap windowed-sinc BLEP/BLAMP (a higher-order successor to the 2-point polyBLEP), which
//! delays their output by [`BLEP_LATENCY`] samples. [`Sine`], [`Wavetable`] and the FM types have
//! no latency.

// `next()` returning a bare sample is the crate-wide oscillator contract (docs/contracts/dsp.md),
// so these types deliberately do not implement `Iterator`.
#![allow(clippy::should_implement_trait)]

mod blep;
mod fm;
mod wavetable;

pub use fm::{Fm4, FmAlgorithm, FmOperator, FmPair};
pub use wavetable::Wavetable;

use blep::Blep;
use std::f64::consts::TAU;

/// Latency in samples of the BLEP-corrected oscillators ([`PolyBlepSaw`], [`PolyBlepSquare`],
/// [`Triangle`]).
pub const BLEP_LATENCY: usize = blep::HALF;

/// Highest phase increment (cycles per sample) the BLEP oscillators accept.
const MAX_INC: f64 = 0.499;

#[inline]
fn sanitize_sr(sample_rate: f32) -> f64 {
    if sample_rate.is_finite() && sample_rate > 0.0 {
        sample_rate as f64
    } else {
        48_000.0
    }
}

#[inline]
fn inc_for(hz: f32, sr: f64) -> f64 {
    if hz.is_finite() {
        (hz as f64 / sr).clamp(0.0, MAX_INC)
    } else {
        0.0
    }
}

#[inline]
fn wrap_phase(phase: f32) -> f64 {
    if phase.is_finite() {
        (phase as f64).rem_euclid(1.0)
    } else {
        0.0
    }
}

/// Pure sine, computed in `f64` (no table, no latency). `sin(2 pi phase)`, so phase 0 starts at
/// zero and rises.
#[derive(Clone, Debug)]
pub struct Sine {
    sr: f64,
    phase: f64,
    inc: f64,
}

impl Sine {
    pub fn new(sample_rate: f32) -> Sine {
        Sine {
            sr: sanitize_sr(sample_rate),
            phase: 0.0,
            inc: 0.0,
        }
    }

    /// Frequency in Hz. Negative values run the phase backwards (through-zero FM); the
    /// magnitude is clamped below Nyquist.
    pub fn set_freq(&mut self, hz: f32) {
        self.inc = if hz.is_finite() {
            (hz as f64 / self.sr).clamp(-0.5, 0.5)
        } else {
            0.0
        };
    }

    /// Restarts at `phase` cycles.
    pub fn reset(&mut self, phase: f32) {
        self.phase = wrap_phase(phase);
    }

    /// Current phase in cycles, `0.0..1.0`.
    pub fn phase(&self) -> f32 {
        self.phase as f32
    }

    /// The next sample.
    #[inline]
    pub fn next(&mut self) -> f32 {
        let y = (self.phase * TAU).sin() as f32;
        self.phase += self.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        } else if self.phase < 0.0 {
            self.phase += 1.0;
        }
        y
    }
}

/// Rising sawtooth (`-1` at phase 0, `+1` just before the wrap), band-limited with BLEP.
/// Output is delayed by [`BLEP_LATENCY`] samples.
#[derive(Clone, Debug)]
pub struct PolyBlepSaw {
    sr: f64,
    phase: f64,
    inc: f64,
    blep: Blep,
}

impl PolyBlepSaw {
    pub fn new(sample_rate: f32) -> PolyBlepSaw {
        let mut s = PolyBlepSaw {
            sr: sanitize_sr(sample_rate),
            phase: 0.0,
            inc: 0.0,
            blep: Blep::new(),
        };
        s.reset(0.0);
        s
    }

    /// Frequency in Hz, `0..Nyquist`.
    pub fn set_freq(&mut self, hz: f32) {
        self.inc = inc_for(hz, self.sr);
    }

    /// Restarts at `phase` cycles. The onset (from silence to the waveform) is band-limited too.
    pub fn reset(&mut self, phase: f32) {
        self.phase = wrap_phase(phase);
        self.blep.clear();
        self.blep.add_step(0.0, Self::naive(self.phase));
    }

    /// Output latency in samples.
    pub fn latency_samples(&self) -> usize {
        BLEP_LATENCY
    }

    #[inline]
    fn naive(p: f64) -> f32 {
        (2.0 * p - 1.0) as f32
    }

    /// The next sample.
    #[inline]
    pub fn next(&mut self) -> f32 {
        let out = self.blep.push(Self::naive(self.phase));
        let p1 = self.phase + self.inc;
        if p1 >= 1.0 {
            let p = p1 - 1.0;
            self.phase = p;
            self.blep.add_step((p / self.inc) as f32, -2.0);
        } else {
            self.phase = p1;
        }
        out
    }
}

/// Pulse wave with variable pulse width (`+1` for the first `width` of each cycle, then `-1`),
/// band-limited with BLEP. A width of 0.5 is a square. Width changes take effect immediately and
/// are band-limited as well. The output has a DC offset of `2 * width - 1`.
/// Output is delayed by [`BLEP_LATENCY`] samples.
#[derive(Clone, Debug)]
pub struct PolyBlepSquare {
    sr: f64,
    phase: f64,
    inc: f64,
    width: f64,
    /// Width used for the last sample pushed.
    last_width: f64,
    /// Naive value of the last sample pushed (0 after a reset: the onset comes from silence).
    last: f32,
    /// False until the first sample after a reset, which plays `phase` itself.
    started: bool,
    blep: Blep,
}

impl PolyBlepSquare {
    pub fn new(sample_rate: f32) -> PolyBlepSquare {
        let mut s = PolyBlepSquare {
            sr: sanitize_sr(sample_rate),
            phase: 0.0,
            inc: 0.0,
            width: 0.5,
            last_width: 0.5,
            last: 0.0,
            started: false,
            blep: Blep::new(),
        };
        s.reset(0.0);
        s
    }

    /// Frequency in Hz, `0..Nyquist`.
    pub fn set_freq(&mut self, hz: f32) {
        self.inc = inc_for(hz, self.sr);
    }

    /// Pulse width as a fraction of the cycle, clamped to `0.01..=0.99`.
    pub fn set_pulse_width(&mut self, width: f32) {
        self.width = if width.is_finite() {
            (width as f64).clamp(0.01, 0.99)
        } else {
            0.5
        };
    }

    /// Restarts at `phase` cycles. The onset is band-limited.
    pub fn reset(&mut self, phase: f32) {
        self.phase = wrap_phase(phase);
        self.blep.clear();
        self.last = 0.0;
        self.started = false;
    }

    /// Output latency in samples.
    pub fn latency_samples(&self) -> usize {
        BLEP_LATENCY
    }

    #[inline]
    fn naive(&self, p: f64) -> f32 {
        if p < self.width { 1.0 } else { -1.0 }
    }

    /// The next sample.
    ///
    /// Between two samples both the phase and the width are taken to move linearly, and every
    /// sign change of `phase - width` (and of `phase - 1 - width` after a wrap) becomes a
    /// band-limited edge at its exact fractional time. A width that sweeps past the phase
    /// therefore makes one clean edge, never a glitch.
    #[inline]
    pub fn next(&mut self) -> f32 {
        let mut jumps = 0.0f32;
        let w1 = self.width;
        if self.started {
            let (p0, w0, dt) = (self.phase, self.last_width, self.inc);
            let p1 = p0 + dt;
            for c in [0.0, 1.0] {
                let g0 = p0 - w0 - c;
                let g1 = p1 - w1 - c;
                if (g0 < 0.0) != (g1 < 0.0) {
                    // Falling edge when the phase passes the width, rising when the width
                    // overtakes the phase.
                    let h = if g1 >= 0.0 { -2.0 } else { 2.0 };
                    self.blep.add_step((g1 / (g1 - g0)) as f32, h);
                    jumps += h;
                }
            }
            if p1 >= 1.0 {
                self.blep.add_step(((p1 - 1.0) / dt) as f32, 2.0);
                jumps += 2.0;
            }
            self.phase = if p1 >= 1.0 { p1 - 1.0 } else { p1 };
        }
        self.last_width = w1;
        let v = self.naive(self.phase);
        let unexplained = v - (self.last + jumps);
        if unexplained.abs() > 0.5 {
            // The width jumped across the phase (or this is the first sample after a reset, coming
            // from silence): band-limit that jump too.
            let frac = if self.started { 0.5 } else { 0.0 };
            self.blep.add_step(frac, unexplained);
        }
        self.started = true;
        self.last = v;
        self.blep.push(v)
    }
}

/// Triangle wave (0 at phase 0, `+1` at 0.25, `-1` at 0.75), band-limited by correcting its
/// corners with BLAMP. Output is delayed by [`BLEP_LATENCY`] samples.
#[derive(Clone, Debug)]
pub struct Triangle {
    sr: f64,
    phase: f64,
    inc: f64,
    fresh: bool,
    blep: Blep,
}

impl Triangle {
    pub fn new(sample_rate: f32) -> Triangle {
        let mut s = Triangle {
            sr: sanitize_sr(sample_rate),
            phase: 0.0,
            inc: 0.0,
            fresh: true,
            blep: Blep::new(),
        };
        s.reset(0.0);
        s
    }

    /// Frequency in Hz, `0..Nyquist`.
    pub fn set_freq(&mut self, hz: f32) {
        self.inc = inc_for(hz, self.sr);
    }

    /// Restarts at `phase` cycles. The onset is band-limited.
    pub fn reset(&mut self, phase: f32) {
        self.phase = wrap_phase(phase);
        self.blep.clear();
        self.fresh = true;
    }

    /// Output latency in samples.
    pub fn latency_samples(&self) -> usize {
        BLEP_LATENCY
    }

    #[inline]
    fn naive(p: f64) -> f32 {
        let v = if p < 0.25 {
            4.0 * p
        } else if p < 0.75 {
            2.0 - 4.0 * p
        } else {
            4.0 * p - 4.0
        };
        v as f32
    }

    /// The next sample.
    #[inline]
    pub fn next(&mut self) -> f32 {
        let p0 = self.phase;
        let dt = self.inc;
        if self.fresh {
            // Onset from silence: a step to the start value and a slope change to the start slope.
            self.fresh = false;
            let slope = if (0.25..0.75).contains(&p0) {
                -4.0 * dt
            } else {
                4.0 * dt
            };
            self.blep.add_step(0.0, Self::naive(p0));
            self.blep.add_ramp(0.0, slope as f32);
        }
        let out = self.blep.push(Self::naive(p0));
        let p1 = p0 + dt;
        if dt > 0.0 {
            // Corners at 0.25 (slope +4 -> -4 cycles) and 0.75 (-4 -> +4), unwrapped.
            for (corner, sign) in [(0.25, -1.0), (0.75, 1.0), (1.25, -1.0), (1.75, 1.0)] {
                if p0 < corner && p1 >= corner {
                    let frac = (p1 - corner) / dt;
                    self.blep.add_ramp(frac as f32, (sign * 8.0 * dt) as f32);
                }
            }
        }
        self.phase = if p1 >= 1.0 { p1 - 1.0 } else { p1 };
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_starts_at_zero_and_rises() {
        let mut s = Sine::new(48_000.0);
        s.set_freq(1000.0);
        assert_eq!(s.next(), 0.0);
        assert!(s.next() > 0.0);
    }

    #[test]
    fn blep_oscillators_are_bounded_and_finite() {
        let sr = 48_000.0;
        let mut saw = PolyBlepSaw::new(sr);
        let mut sq = PolyBlepSquare::new(sr);
        let mut tri = Triangle::new(sr);
        for (i, hz) in [20.0f32, 440.0, 5000.0, 15_000.0, 23_900.0, f32::NAN]
            .into_iter()
            .enumerate()
        {
            saw.set_freq(hz);
            sq.set_freq(hz);
            sq.set_pulse_width(0.1 + 0.15 * i as f32);
            tri.set_freq(hz);
            for _ in 0..4000 {
                for v in [saw.next(), sq.next(), tri.next()] {
                    assert!(v.is_finite() && v.abs() < 1.6, "{hz} {v}");
                }
            }
        }
    }
}
