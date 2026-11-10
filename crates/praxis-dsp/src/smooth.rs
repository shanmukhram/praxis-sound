//! Parameter smoothing, so knob moves and automation never click.

// `next()` returning a bare value is the crate-wide contract.
#![allow(clippy::should_implement_trait)]

/// How [`Smoothed`] approaches a new target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SmoothMode {
    /// A straight ramp that lands exactly on the target after the smoothing time.
    #[default]
    Linear,
    /// A one-pole glide, fast at first then easing in: within -80 dB of the change after the
    /// smoothing time, at which point it snaps to the exact target. Natural for gains and
    /// cutoffs.
    Exponential,
}

/// Remaining fraction of the change after the smoothing time in exponential mode (-80 dB).
const EXP_RESIDUAL: f64 = 1e-4;

/// A value that glides to its target over a fixed time.
///
/// Each [`Smoothed::set_target`] starts a new glide from the current value lasting
/// `round(time * sample_rate)` samples; [`Smoothed::next`] advances one sample. After exactly
/// that many calls the value equals the target and [`Smoothed::is_settled`] is true.
#[derive(Clone, Debug)]
pub struct Smoothed {
    mode: SmoothMode,
    sr: f64,
    steps: u64,
    current: f64,
    target: f64,
    inc: f64,
    coeff: f64,
    remaining: u64,
}

impl Smoothed {
    /// `time_s` is the smoothing time in seconds; starts settled at `initial`.
    pub fn new(sample_rate: f32, time_s: f32, mode: SmoothMode, initial: f32) -> Smoothed {
        let sr = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate as f64
        } else {
            48_000.0
        };
        let initial = if initial.is_finite() {
            initial as f64
        } else {
            0.0
        };
        let mut s = Smoothed {
            mode,
            sr,
            steps: 0,
            current: initial,
            target: initial,
            inc: 0.0,
            coeff: 0.0,
            remaining: 0,
        };
        s.set_time(time_s);
        s
    }

    /// A linear smoother.
    pub fn linear(sample_rate: f32, time_s: f32, initial: f32) -> Smoothed {
        Smoothed::new(sample_rate, time_s, SmoothMode::Linear, initial)
    }

    /// An exponential smoother.
    pub fn exponential(sample_rate: f32, time_s: f32, initial: f32) -> Smoothed {
        Smoothed::new(sample_rate, time_s, SmoothMode::Exponential, initial)
    }

    /// Smoothing time in seconds (applies from the next `set_target`).
    pub fn set_time(&mut self, time_s: f32) {
        let t = if time_s.is_finite() {
            time_s.clamp(0.0, 600.0) as f64
        } else {
            0.0
        };
        self.steps = (t * self.sr).round() as u64;
        self.coeff = if self.steps > 0 {
            EXP_RESIDUAL.powf(1.0 / self.steps as f64)
        } else {
            0.0
        };
    }

    pub fn set_mode(&mut self, mode: SmoothMode) {
        self.mode = mode;
    }

    /// Starts gliding from the current value to `target`. Setting the current target again
    /// changes nothing.
    pub fn set_target(&mut self, target: f32) {
        if !target.is_finite() {
            return;
        }
        let t = target as f64;
        if t == self.target {
            return;
        }
        self.target = t;
        if self.steps == 0 {
            self.current = t;
            self.remaining = 0;
            return;
        }
        self.remaining = self.steps;
        self.inc = (t - self.current) / self.steps as f64;
    }

    /// Jumps to `value` immediately (for example on note start).
    pub fn snap(&mut self, value: f32) {
        if value.is_finite() {
            self.current = value as f64;
            self.target = value as f64;
            self.remaining = 0;
        }
    }

    /// Advances one sample and returns the new value.
    #[inline]
    pub fn next(&mut self) -> f32 {
        if self.remaining > 0 {
            self.remaining -= 1;
            if self.remaining == 0 {
                self.current = self.target;
            } else {
                match self.mode {
                    SmoothMode::Linear => self.current += self.inc,
                    SmoothMode::Exponential => {
                        self.current = self.target + (self.current - self.target) * self.coeff;
                    }
                }
            }
        }
        self.current as f32
    }

    /// True once the value has reached its target.
    pub fn is_settled(&self) -> bool {
        self.remaining == 0
    }

    /// Current value (without advancing).
    pub fn value(&self) -> f32 {
        self.current as f32
    }

    pub fn target(&self) -> f32 {
        self.target as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_time_is_immediate() {
        let mut s = Smoothed::linear(48_000.0, 0.0, 0.0);
        s.set_target(1.0);
        assert!(s.is_settled());
        assert_eq!(s.next(), 1.0);
    }
}
