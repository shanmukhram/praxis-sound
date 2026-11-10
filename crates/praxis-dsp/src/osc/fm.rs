//! Phase-modulation ("FM", as in DX-style synths) operators and small 2- and 4-operator
//! helpers.

use super::{sanitize_sr, wrap_phase};
use crate::util::sanitize;
use std::f64::consts::TAU;

/// One sine operator: `index * sin(2 pi phase + pm + feedback * avg(last two outputs))`.
///
/// - `set_freq(hz)` sets the note frequency; the operator runs at `hz * ratio`.
/// - `index` scales the output. Feeding the output of one operator into another's
///   [`FmOperator::next_pm`] makes `index` the modulation index (peak phase deviation in
///   radians). For a carrier, `index` is its output level (1.0 = full scale).
/// - `feedback` is self-modulation in radians (0 = pure sine, about 1.0 = saw-like, above about
///   1.6 noisy). The fed-back signal is the average of the last two outputs, which keeps high
///   feedback from collapsing into chaotic oscillation.
///
/// Wide FM spectra can contain partials above Nyquist; keep `index * modulator frequency`
/// within reason for high notes.
#[derive(Clone, Debug)]
pub struct FmOperator {
    sr: f64,
    hz: f64,
    ratio: f64,
    inc: f64,
    phase: f64,
    index: f32,
    feedback: f32,
    y1: f32,
    y2: f32,
}

impl FmOperator {
    pub fn new(sample_rate: f32) -> FmOperator {
        FmOperator {
            sr: sanitize_sr(sample_rate),
            hz: 0.0,
            ratio: 1.0,
            inc: 0.0,
            phase: 0.0,
            index: 1.0,
            feedback: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn update_inc(&mut self) {
        self.inc = (self.hz * self.ratio / self.sr).clamp(-0.5, 0.5);
    }

    /// Note frequency in Hz (the operator runs at `hz * ratio`).
    pub fn set_freq(&mut self, hz: f32) {
        self.hz = if hz.is_finite() { hz as f64 } else { 0.0 };
        self.update_inc();
    }

    /// Frequency ratio to the note (1.0 = unison, 2.0 = an octave up, 3.5 = inharmonic...).
    /// Clamped to `0.0..=64.0`.
    pub fn set_ratio(&mut self, ratio: f32) {
        self.ratio = if ratio.is_finite() {
            (ratio as f64).clamp(0.0, 64.0)
        } else {
            1.0
        };
        self.update_inc();
    }

    /// Output scale; the modulation index in radians when this operator modulates another.
    /// Clamped to `0.0..=32.0`.
    pub fn set_index(&mut self, index: f32) {
        self.index = if index.is_finite() {
            index.clamp(0.0, 32.0)
        } else {
            0.0
        };
    }

    /// Self-feedback in radians, clamped to `0.0..=4.0`.
    pub fn set_feedback(&mut self, feedback: f32) {
        self.feedback = if feedback.is_finite() {
            feedback.clamp(0.0, 4.0)
        } else {
            0.0
        };
    }

    /// Restarts at `phase` cycles and clears the feedback memory.
    pub fn reset(&mut self, phase: f32) {
        self.phase = wrap_phase(phase);
        self.y1 = 0.0;
        self.y2 = 0.0;
    }

    /// The next sample without external modulation.
    #[inline]
    pub fn next(&mut self) -> f32 {
        self.next_pm(0.0)
    }

    /// The next sample with `pm` radians of phase modulation (usually another operator's
    /// output).
    #[inline]
    pub fn next_pm(&mut self, pm: f32) -> f32 {
        let pm = if pm.is_finite() { pm } else { 0.0 };
        let fb = self.feedback * 0.5 * (self.y1 + self.y2);
        let y = sanitize((self.phase * TAU + (pm + fb) as f64).sin() as f32);
        self.y2 = self.y1;
        self.y1 = y;
        self.phase += self.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        } else if self.phase < 0.0 {
            self.phase += 1.0;
        }
        y * self.index
    }
}

/// Classic two-operator FM: a modulator into a carrier.
#[derive(Clone, Debug)]
pub struct FmPair {
    pub carrier: FmOperator,
    pub modulator: FmOperator,
}

impl FmPair {
    /// Carrier at ratio 1 and full level; modulator at ratio 1 with index 0 (a pure sine until
    /// [`FmPair::set_index`] is raised).
    pub fn new(sample_rate: f32) -> FmPair {
        let mut modulator = FmOperator::new(sample_rate);
        modulator.set_index(0.0);
        FmPair {
            carrier: FmOperator::new(sample_rate),
            modulator,
        }
    }

    /// Note frequency in Hz.
    pub fn set_freq(&mut self, hz: f32) {
        self.carrier.set_freq(hz);
        self.modulator.set_freq(hz);
    }

    /// Modulator-to-note frequency ratio.
    pub fn set_ratio(&mut self, ratio: f32) {
        self.modulator.set_ratio(ratio);
    }

    /// Modulation index in radians.
    pub fn set_index(&mut self, index: f32) {
        self.modulator.set_index(index);
    }

    /// Modulator self-feedback in radians.
    pub fn set_feedback(&mut self, feedback: f32) {
        self.modulator.set_feedback(feedback);
    }

    /// Restarts both operators at `phase` cycles.
    pub fn reset(&mut self, phase: f32) {
        self.carrier.reset(phase);
        self.modulator.reset(phase);
    }

    /// The next sample.
    #[inline]
    pub fn next(&mut self) -> f32 {
        let m = self.modulator.next();
        self.carrier.next_pm(m)
    }
}

/// Routing of a four-operator [`Fm4`]. Operators are numbered 1..4 as `ops[0]..ops[3]`;
/// `a -> b` means `a` modulates `b`. Carriers are summed and divided by their count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FmAlgorithm {
    /// 4 -> 3 -> 2 -> 1; carrier 1. The brightest, most complex.
    #[default]
    Stack,
    /// (4 -> 3) and (2 -> 1); carriers 1 and 3.
    TwoPairs,
    /// 2, 3 and 4 each -> 1; carrier 1.
    ThreeToOne,
    /// 4 -> 3 -> 1 and 2 -> 1; carrier 1.
    Branch,
    /// 4 -> (1, 2, 3); carriers 1, 2, 3.
    OneToThree,
    /// No modulation; carriers 1..4 (additive organ).
    Parallel,
}

/// Four FM operators with a selectable [`FmAlgorithm`]. Configure each operator through `ops`
/// (ratio, index, feedback); [`Fm4::set_freq`] sets the note frequency of all four.
#[derive(Clone, Debug)]
pub struct Fm4 {
    pub ops: [FmOperator; 4],
    pub algorithm: FmAlgorithm,
}

impl Fm4 {
    /// Four operators at ratio 1; operator 1 at full level, modulators at index 0.
    pub fn new(sample_rate: f32) -> Fm4 {
        let mut ops = [
            FmOperator::new(sample_rate),
            FmOperator::new(sample_rate),
            FmOperator::new(sample_rate),
            FmOperator::new(sample_rate),
        ];
        for op in ops.iter_mut().skip(1) {
            op.set_index(0.0);
        }
        Fm4 {
            ops,
            algorithm: FmAlgorithm::Stack,
        }
    }

    /// Note frequency in Hz for all operators.
    pub fn set_freq(&mut self, hz: f32) {
        for op in self.ops.iter_mut() {
            op.set_freq(hz);
        }
    }

    /// Restarts all operators at `phase` cycles.
    pub fn reset(&mut self, phase: f32) {
        for op in self.ops.iter_mut() {
            op.reset(phase);
        }
    }

    /// The next sample.
    #[inline]
    pub fn next(&mut self) -> f32 {
        let [o1, o2, o3, o4] = &mut self.ops;
        match self.algorithm {
            FmAlgorithm::Stack => {
                let m4 = o4.next();
                let m3 = o3.next_pm(m4);
                let m2 = o2.next_pm(m3);
                o1.next_pm(m2)
            }
            FmAlgorithm::TwoPairs => {
                let m2 = o2.next();
                let m4 = o4.next();
                0.5 * (o1.next_pm(m2) + o3.next_pm(m4))
            }
            FmAlgorithm::ThreeToOne => {
                let m = o2.next() + o3.next() + o4.next();
                o1.next_pm(m)
            }
            FmAlgorithm::Branch => {
                let m4 = o4.next();
                let m3 = o3.next_pm(m4);
                let m2 = o2.next();
                o1.next_pm(m2 + m3)
            }
            FmAlgorithm::OneToThree => {
                let m4 = o4.next();
                (o1.next_pm(m4) + o2.next_pm(m4) + o3.next_pm(m4)) * (1.0 / 3.0)
            }
            FmAlgorithm::Parallel => 0.25 * (o1.next() + o2.next() + o3.next() + o4.next()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_zero_pair_is_a_sine() {
        let mut p = FmPair::new(48_000.0);
        p.set_freq(440.0);
        for n in 0..1000 {
            let want = (TAU * 440.0 * n as f64 / 48_000.0).sin() as f32;
            assert!((p.next() - want).abs() < 1e-6);
        }
    }

    #[test]
    fn fm_adds_sidebands_and_stays_bounded() {
        let mut p = FmPair::new(48_000.0);
        p.set_freq(220.0);
        p.set_ratio(2.0);
        p.set_index(3.0);
        p.set_feedback(1.2);
        let mut f = Fm4::new(48_000.0);
        f.set_freq(220.0);
        for op in f.ops.iter_mut().skip(1) {
            op.set_index(2.0);
            op.set_feedback(2.0);
        }
        for alg in [
            FmAlgorithm::Stack,
            FmAlgorithm::TwoPairs,
            FmAlgorithm::ThreeToOne,
            FmAlgorithm::Branch,
            FmAlgorithm::OneToThree,
            FmAlgorithm::Parallel,
        ] {
            f.algorithm = alg;
            for _ in 0..10_000 {
                let a = p.next();
                let b = f.next();
                assert!(a.is_finite() && a.abs() <= 1.0001);
                assert!(b.is_finite() && b.abs() <= 2.0);
            }
        }
    }
}
