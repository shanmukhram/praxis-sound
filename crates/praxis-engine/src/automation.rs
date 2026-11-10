//! Automation lanes (FR-24): parameter curves over bars, evaluated at sample positions.
//!
//! The engine evaluates every lane at fixed block boundaries of the grid (multiples of the block
//! size from time zero), never at event-split boundaries, so a lane's values do not depend on
//! where notes fall. Layer gain and pan are interpolated linearly between the values at the start
//! and end of each block (no zipper noise); instrument and effect parameters are set once per
//! block and the processors smooth them (docs/contracts/engine.md).
//!
//! For loops, lane time wraps at the loop length, so every cycle sees the same curve and the value
//! at the end of the last bar leads into the value at bar 1.

use praxis_score::timeline::TimeMap;
use praxis_score::{Automation, Curve};

/// One automation curve, with its points converted to sample positions.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    /// (sample position, value), sorted by position.
    points: Vec<(f64, f64)>,
    curve: Curve,
    /// Loop length in samples (lane time wraps), or `None` for one-shots.
    period: Option<u64>,
}

impl Lane {
    pub fn new(auto: &Automation, tm: &TimeMap, period: Option<u64>) -> Lane {
        let sr = tm.sample_rate as f64;
        let mut points: Vec<(f64, f64)> = auto
            .points
            .iter()
            .filter(|p| p.bar.is_finite() && p.value.is_finite())
            .map(|p| (tm.bar_pos_to_s(p.bar) * sr, p.value))
            .collect();
        // Stable sort keeps the author's order for equal positions (a step jump).
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        Lane {
            points,
            curve: auto.curve,
            period: period.filter(|p| *p > 0),
        }
    }

    /// A lane holding one constant value.
    pub fn constant(value: f64) -> Lane {
        Lane {
            points: vec![(0.0, value)],
            curve: Curve::Step,
            period: None,
        }
    }

    /// Value at absolute sample `t`. Before the first point the first value holds, after the last
    /// point the last value holds.
    pub fn value_at(&self, t: u64) -> f64 {
        let x = match self.period {
            Some(p) => (t % p) as f64,
            None => t as f64,
        };
        let pts = &self.points;
        let Some(first) = pts.first() else {
            return 0.0;
        };
        // First point strictly after x.
        let i = pts.partition_point(|p| p.0 <= x);
        if i == 0 {
            return first.1;
        }
        if i == pts.len() {
            return pts[pts.len() - 1].1;
        }
        let (a, b) = (pts[i - 1], pts[i]);
        let span = b.0 - a.0;
        if span <= 0.0 {
            return b.1;
        }
        let f = ((x - a.0) / span).clamp(0.0, 1.0);
        match self.curve {
            Curve::Step => a.1,
            Curve::Linear => a.1 + (b.1 - a.1) * f,
            Curve::Smooth => {
                let e = (1.0 - (std::f64::consts::PI * f).cos()) * 0.5;
                a.1 + (b.1 - a.1) * e
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use praxis_score::AutoPoint;

    fn tm() -> TimeMap {
        // 120 BPM 4/4 at 1000 Hz: one bar = 2 s = 2000 samples.
        TimeMap {
            sample_rate: 1000,
            bpm: 120.0,
            bar_whole_notes: 1.0,
        }
    }

    fn auto(curve: Curve, pts: &[(f64, f64)]) -> Automation {
        Automation {
            target: "gain_db".into(),
            curve,
            points: pts
                .iter()
                .map(|&(bar, value)| AutoPoint { bar, value })
                .collect(),
        }
    }

    #[test]
    fn curves() {
        let a = auto(Curve::Linear, &[(1.0, 0.0), (2.0, 10.0)]);
        let l = Lane::new(&a, &tm(), None);
        assert_eq!(l.value_at(0), 0.0);
        assert_eq!(l.value_at(1000), 5.0);
        assert_eq!(l.value_at(5000), 10.0);
        let s = Lane::new(&auto(Curve::Step, &[(1.0, 0.0), (2.0, 10.0)]), &tm(), None);
        assert_eq!(s.value_at(1999), 0.0);
        assert_eq!(s.value_at(2000), 10.0);
        let c = Lane::new(
            &auto(Curve::Smooth, &[(1.0, 0.0), (2.0, 10.0)]),
            &tm(),
            None,
        );
        assert!((c.value_at(1000) - 5.0).abs() < 1e-9);
        assert!(c.value_at(200) < 1.0);
    }

    #[test]
    fn loop_lanes_wrap() {
        let a = auto(Curve::Linear, &[(1.0, 0.0), (3.0, 10.0)]);
        let l = Lane::new(&a, &tm(), Some(4000));
        assert_eq!(l.value_at(4000 + 2000), l.value_at(2000));
        assert_eq!(Lane::constant(3.0).value_at(123), 3.0);
    }
}
