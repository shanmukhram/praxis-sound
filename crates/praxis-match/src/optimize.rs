//! A small, deterministic optimiser: coordinate descent with step halving over a box `[0, 1]^n`.
//!
//! The match loop (FR-15) has a handful of parameters, each mapped to `0..1`, and an objective
//! that is a render plus an analysis, so every evaluation is expensive and a little noisy in its
//! fine structure. Coordinate descent suits that: it needs no gradients, it only accepts strict
//! improvements (the best similarity never falls), and it stops when a full sweep at the smallest
//! step finds nothing better ("until the similarity stops improving") or the budget is spent.
//!
//! Determinism: the order of trials is fixed (dimension 0 first, `+step` before `-step`), so the
//! same objective gives the same trace.

/// When to stop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Most objective evaluations, the starting point included.
    pub max_evals: usize,
    /// First step, in the normalised `0..1` units of every dimension.
    pub start_step: f64,
    /// Stop once the step has been halved below this.
    pub min_step: f64,
    /// A trial must beat the best value by more than this to count as an improvement.
    pub min_gain: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            max_evals: 40,
            start_step: 0.25,
            min_step: 0.05,
            min_gain: 1e-4,
        }
    }
}

/// One evaluation of the objective.
#[derive(Clone, Debug, PartialEq)]
pub struct Trial {
    /// The point tried.
    pub x: Vec<f64>,
    /// The objective there; `None` when the evaluation failed.
    pub value: Option<f64>,
    /// The dimension that was moved (`None` for the starting point).
    pub dim: Option<usize>,
    /// True when this trial became the new best.
    pub accepted: bool,
    /// Best value after this trial.
    pub best: f64,
}

/// The result of a run.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub best_x: Vec<f64>,
    pub best_value: f64,
    pub trials: Vec<Trial>,
}

/// Maximises `f` over `[0, 1]^n` from `x0` (clamped into the box).
///
/// `f` returns `None` for a point it cannot evaluate; such points are never accepted.
pub fn coordinate_descent<F>(x0: &[f64], settings: &Settings, mut f: F) -> Outcome
where
    F: FnMut(&[f64]) -> Option<f64>,
{
    let mut x: Vec<f64> = x0.iter().map(|v| clamp01(*v)).collect();
    let mut trials = Vec::new();
    let first = f(&x).filter(|v| v.is_finite());
    let mut best = first.unwrap_or(f64::NEG_INFINITY);
    trials.push(Trial {
        x: x.clone(),
        value: first,
        dim: None,
        accepted: first.is_some(),
        best,
    });
    let max_evals = settings.max_evals.max(1);
    let mut step = settings.start_step.clamp(0.0, 1.0);
    if x.is_empty() || step <= 0.0 {
        return Outcome {
            best_x: x,
            best_value: best,
            trials,
        };
    }
    'outer: loop {
        let mut improved = false;
        for d in 0..x.len() {
            for dir in [1.0, -1.0] {
                if trials.len() >= max_evals {
                    break 'outer;
                }
                let mut cand = x.clone();
                cand[d] = clamp01(x[d] + dir * step);
                if (cand[d] - x[d]).abs() < 1e-12 {
                    continue;
                }
                let value = f(&cand).filter(|v| v.is_finite());
                let accepted = value.is_some_and(|v| v > best + settings.min_gain);
                if accepted {
                    best = value.unwrap_or(best);
                    x = cand.clone();
                    improved = true;
                }
                trials.push(Trial {
                    x: cand,
                    value,
                    dim: Some(d),
                    accepted,
                    best,
                });
                if accepted {
                    // Keep this dimension's new value and move on to the next dimension.
                    break;
                }
            }
        }
        if !improved {
            step *= 0.5;
            if step < settings.min_step {
                break;
            }
        }
    }
    Outcome {
        best_x: x,
        best_value: best,
        trials,
    }
}

fn clamp01(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bowl(x: &[f64]) -> Option<f64> {
        // Maximum 1.0 at (0.3, 0.8).
        Some(1.0 - (x[0] - 0.3).powi(2) - (x[1] - 0.8).powi(2))
    }

    #[test]
    fn finds_the_peak_of_a_bowl() {
        let s = Settings {
            max_evals: 200,
            start_step: 0.25,
            min_step: 0.01,
            min_gain: 1e-9,
        };
        let out = coordinate_descent(&[0.5, 0.5], &s, bowl);
        assert!((out.best_x[0] - 0.3).abs() < 0.03, "{:?}", out.best_x);
        assert!((out.best_x[1] - 0.8).abs() < 0.03, "{:?}", out.best_x);
        assert!(out.best_value > 0.999);
    }

    #[test]
    fn best_never_falls_and_budget_holds() {
        let s = Settings {
            max_evals: 12,
            ..Settings::default()
        };
        let out = coordinate_descent(&[0.0, 0.0], &s, bowl);
        assert!(out.trials.len() <= 12);
        for w in out.trials.windows(2) {
            assert!(w[1].best >= w[0].best);
        }
        assert!(out.best_value > out.trials[0].value.unwrap_or(0.0));
    }

    #[test]
    fn deterministic_and_failures_are_never_accepted() {
        let f = |x: &[f64]| if x[0] > 0.6 { None } else { Some(x[0]) };
        let a = coordinate_descent(&[0.5], &Settings::default(), f);
        let b = coordinate_descent(&[0.5], &Settings::default(), f);
        assert_eq!(a, b);
        assert!(a.best_x[0] <= 0.6);
        assert!(a.trials.iter().all(|t| !t.accepted || t.value.is_some()));
    }

    #[test]
    fn stops_when_nothing_improves() {
        let out = coordinate_descent(&[0.5, 0.5], &Settings::default(), |_| Some(1.0));
        // One sweep per step size: 4 trials at 0.25, 0.125 and 0.0625, then the step is below 0.05.
        assert_eq!(out.trials.len(), 1 + 3 * 4);
        assert!(out.trials.iter().skip(1).all(|t| !t.accepted));
    }
}
