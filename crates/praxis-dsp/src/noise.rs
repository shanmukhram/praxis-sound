//! Noise sources, all driven by the seeded [`crate::rng::Pcg32`]: the same seed gives
//! bit-identical output. Voices should seed from the note's stable key
//! (`rng::derive(seed, note_key)`) and call `reseed` on note start.
//!
//! Levels: [`White`] is uniform in `-1..1` (RMS about 0.58); [`Pink`] and [`Brown`] are scaled to
//! an RMS of about 0.25 so their peaks rarely pass 1.

// `next()` returning a bare sample is the crate-wide generator contract.
#![allow(clippy::should_implement_trait)]

use crate::rng::Pcg32;
use crate::util::sanitize;

/// Uniform white noise in `-1..1`.
#[derive(Clone, Debug)]
pub struct White {
    rng: Pcg32,
}

impl White {
    pub fn new(seed: u64) -> White {
        White {
            rng: Pcg32::new(seed),
        }
    }

    /// Restarts the sequence from `seed`.
    pub fn reseed(&mut self, seed: u64) {
        self.rng = Pcg32::new(seed);
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        self.rng.next_bipolar()
    }
}

/// Paul Kellet's refined pink filter at 44.1 kHz: (pole, input gain) of the leaky sections.
const KELLET: [(f64, f64); 5] = [
    (0.99886, 0.055_517_9),
    (0.99332, 0.075_075_9),
    (0.96900, 0.153_852_0),
    (0.86650, 0.310_485_6),
    (0.55000, 0.532_952_2),
];
/// Output scale giving an RMS of about 0.25 for uniform white input.
const PINK_GAIN: f64 = 0.1486;

/// Pink noise (-3 dB/octave, equal energy per octave) using Paul Kellet's refined filter,
/// accurate to a fraction of a dB from about 10 Hz to 20 kHz. Its corner frequencies are
/// re-derived for the actual sample rate.
#[derive(Clone, Debug)]
pub struct Pink {
    white: White,
    poles: [f64; 5],
    gains: [f64; 5],
    state: [f64; 5],
    hf: f64,
    last: f64,
}

impl Pink {
    pub fn new(sample_rate: f32, seed: u64) -> Pink {
        let sr = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate as f64
        } else {
            48_000.0
        };
        let mut poles = [0.0; 5];
        let mut gains = [0.0; 5];
        for (i, &(p, c)) in KELLET.iter().enumerate() {
            // Same corner frequency in Hz; same low-frequency gain per section.
            let p2 = p.powf(44_100.0 / sr);
            poles[i] = p2;
            gains[i] = c * (1.0 - p2) / (1.0 - p);
        }
        Pink {
            white: White::new(seed),
            poles,
            gains,
            state: [0.0; 5],
            hf: 0.0,
            last: 0.0,
        }
    }

    /// Restarts from `seed` with a cleared filter.
    pub fn reseed(&mut self, seed: u64) {
        self.white.reseed(seed);
        self.state = [0.0; 5];
        self.hf = 0.0;
        self.last = 0.0;
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        let w = self.white.next() as f64;
        let mut sum = 0.0;
        for i in 0..5 {
            let s = self.poles[i] * self.state[i] + self.gains[i] * w;
            self.state[i] = if s.abs() > 1e-30 { s } else { 0.0 };
            sum += self.state[i];
        }
        // Near-Nyquist correction section and one-sample-delayed term of the original design.
        self.hf = -0.7616 * self.hf - 0.016_898 * w;
        sum += self.hf + self.last + 0.5362 * w;
        self.last = 0.115_926 * w;
        sanitize((sum * PINK_GAIN) as f32)
    }
}

/// Brown (red) noise, -6 dB/octave: leaky-integrated white noise. The leak puts the corner at
/// 8 Hz so the signal cannot wander off as DC.
#[derive(Clone, Debug)]
pub struct Brown {
    white: White,
    leak: f64,
    gain: f64,
    y: f64,
}

impl Brown {
    pub fn new(sample_rate: f32, seed: u64) -> Brown {
        let sr = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate as f64
        } else {
            48_000.0
        };
        let leak = (-2.0 * std::f64::consts::PI * 8.0 / sr).exp();
        // Stationary variance of y = leak * y + g * w with var(w) = 1/3 is g^2 / (3 (1 - leak^2)).
        let gain = 0.25 * (3.0 * (1.0 - leak * leak)).sqrt();
        Brown {
            white: White::new(seed),
            leak,
            gain,
            y: 0.0,
        }
    }

    /// Restarts from `seed` with a cleared integrator.
    pub fn reseed(&mut self, seed: u64) {
        self.white.reseed(seed);
        self.y = 0.0;
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        let w = self.white.next() as f64;
        let y = self.leak * self.y + self.gain * w;
        self.y = if y.is_finite() && y.abs() > 1e-30 {
            y
        } else {
            0.0
        };
        self.y as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(mut f: impl FnMut() -> f32, n: usize) -> f64 {
        let mut s = 0.0f64;
        for _ in 0..n {
            let v = f() as f64;
            s += v * v;
        }
        (s / n as f64).sqrt()
    }

    #[test]
    fn levels_are_as_documented() {
        let mut w = White::new(1);
        let mut p = Pink::new(48_000.0, 1);
        let mut b = Brown::new(48_000.0, 1);
        let n = 480_000;
        let rw = rms(|| w.next(), n);
        let rp = rms(|| p.next(), n);
        let rb = rms(|| b.next(), n);
        assert!((rw - 0.577).abs() < 0.01, "{rw}");
        assert!((rp - 0.25).abs() < 0.03, "{rp}");
        assert!((rb - 0.25).abs() < 0.08, "{rb}");
    }
}
