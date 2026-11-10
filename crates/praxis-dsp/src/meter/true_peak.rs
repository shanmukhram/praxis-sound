//! 4x oversampled true-peak detection (ITU-R BS.1770-4 Annex 2).
//!
//! The interpolator is the 48-tap Hann-windowed sinc used by libebur128 / the `ebur128` crate
//! (12 taps per phase). Phase 0 passes the samples unchanged, so the detected value is never
//! below the sample peak. The same detector drives [`crate::fx::TruePeakLimiter`], so the
//! limiter and the meter agree exactly.

/// Taps per polyphase branch.
pub const TAPS: usize = 12;
/// Delay of the detector in samples: `push(x[n])` reports the segment that starts at `x[n - 6]`.
pub const DELAY: usize = TAPS / 2;

fn coeff(offset: f64) -> f64 {
    // Hann window over +-6 samples times sinc.
    let w = 0.5 * (1.0 + (std::f64::consts::PI * offset / 6.0).cos());
    if offset.abs() < 1e-12 {
        w
    } else {
        let x = std::f64::consts::PI * offset;
        w * x.sin() / x
    }
}

/// Polyphase coefficients for phases 1..=3: `PHASES[f-1][i]` multiplies `x[n - i]` and yields
/// the value at time `n - 6 + f / 4`.
fn phases() -> [[f32; TAPS]; 3] {
    let mut p = [[0.0f32; TAPS]; 3];
    for (fi, row) in p.iter_mut().enumerate() {
        let f = (fi + 1) as f64;
        for (i, c) in row.iter_mut().enumerate() {
            *c = coeff(i as f64 - 6.0 + f / 4.0) as f32;
        }
    }
    p
}

/// Streaming single-channel true-peak detector. Allocation-free.
#[derive(Clone, Debug)]
pub struct TruePeakDetector {
    h: [[f32; TAPS]; 3],
    ring: [f32; 2 * TAPS],
    pos: usize,
}

impl Default for TruePeakDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl TruePeakDetector {
    pub fn new() -> TruePeakDetector {
        TruePeakDetector {
            h: phases(),
            ring: [0.0; 2 * TAPS],
            pos: 0,
        }
    }

    pub fn reset(&mut self) {
        self.ring = [0.0; 2 * TAPS];
        self.pos = 0;
    }

    /// Pushes `x[n]` and returns the absolute peak of the segment `[n - 6, n - 5)`: the sample
    /// `x[n - 6]` and the three interpolated points after it.
    #[inline]
    pub fn push(&mut self, x: f32) -> f32 {
        self.pos = if self.pos == 0 {
            TAPS - 1
        } else {
            self.pos - 1
        };
        self.ring[self.pos] = x;
        self.ring[self.pos + TAPS] = x;
        let hist = &self.ring[self.pos..self.pos + TAPS];
        let mut peak = hist[DELAY].abs();
        for row in self.h.iter() {
            let mut acc = 0.0f32;
            for (c, v) in row.iter().zip(hist.iter()) {
                acc += c * v;
            }
            peak = peak.max(acc.abs());
        }
        peak
    }
}

/// Linear true peak of one channel (zero-padded at both ends).
pub fn true_peak_linear(x: &[f32]) -> f32 {
    let mut d = TruePeakDetector::new();
    let mut peak = 0.0f32;
    for &v in x.iter().chain(std::iter::repeat_n(&0.0, DELAY)) {
        peak = peak.max(d.push(v));
    }
    peak
}

/// True peak of a stereo signal in dBTP (4x oversampling, the limiter's detector).
/// `-inf` for silence. `sample_rate` is accepted for the contract; 4x is used at every rate.
pub fn true_peak_dbtp(left: &[f32], right: &[f32], _sample_rate: f32) -> f64 {
    let p = true_peak_linear(left).max(true_peak_linear(right)) as f64;
    if p > 0.0 {
        20.0 * p.log10()
    } else {
        f64::NEG_INFINITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_samples_and_finds_intersample_peaks() {
        // phase 0 is exact
        let mut x = vec![0.0f32; 64];
        x[20] = 0.5;
        assert!((true_peak_linear(&x) - 0.5).abs() < 1e-6);
        // sine at fs/4 with 45 degree offset: samples at 0.707, true peak 1
        let s: Vec<f32> = (0..4800)
            .map(|n| (std::f32::consts::FRAC_PI_2 * n as f32 + std::f32::consts::FRAC_PI_4).sin())
            .collect();
        let sp = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let tp = true_peak_linear(&s);
        assert!(sp < 0.71);
        assert!(tp > 0.98, "{tp}");
    }
}
