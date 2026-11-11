//! Metrical evidence for choosing the tempo octave of a profile.
//!
//! The tempo estimator ([`super::tempo`]) finds the strongest pulse, but a pulse and its double
//! (or half) often explain the onsets about equally well: eighth notes at 160 BPM are sixteenths
//! at 80. These measures describe the onsets at one tempo level:
//!
//! - **pulse strength**: the mean onset-envelope peak (largest value within +-30 ms) at the
//!   tracked beats; **half-beat strength**: the same halfway between consecutive beats;
//!   **alternate ratio**: the mean at odd beats over the mean at even beats (taking the weaker
//!   as numerator, so 0..1);
//! - **tatums per beat**: the beat period over the most common inter-onset interval (onsets
//!   closer than 60 ms merged; the mode of IOIs rounded to 10 ms, weighted by count).

use super::onset::{Onset, OnsetEnvelope};

fn peak_near(env: &OnsetEnvelope, t: f64) -> f64 {
    let fps = env.frame_rate();
    let c = (t * fps).round() as isize;
    let r = (0.03 * fps).round().max(1.0) as isize;
    let mut m = 0.0f64;
    for i in (c - r)..=(c + r) {
        if i >= 0
            && let Some(v) = env.values.get(i as usize)
        {
            m = m.max(*v as f64);
        }
    }
    m
}

/// Measures at one tempo level.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Meter {
    pub beat_strength: f64,
    pub half_strength: f64,
    /// Weaker over stronger of odd and even beats, 0..1.
    pub alternate_ratio: f64,
}

/// Pulse, half-beat and alternate-beat strengths at the tracked `beats_s`.
pub fn meter(env: &OnsetEnvelope, beats_s: &[f64]) -> Meter {
    if beats_s.len() < 4 {
        return Meter::default();
    }
    let on: Vec<f64> = beats_s.iter().map(|b| peak_near(env, *b)).collect();
    let half: Vec<f64> = beats_s
        .windows(2)
        .map(|w| peak_near(env, 0.5 * (w[0] + w[1])))
        .collect();
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    let even: Vec<f64> = on.iter().step_by(2).copied().collect();
    let odd: Vec<f64> = on.iter().skip(1).step_by(2).copied().collect();
    let (e, o) = (mean(&even), mean(&odd));
    Meter {
        beat_strength: mean(&on),
        half_strength: mean(&half),
        alternate_ratio: if e.max(o) > 0.0 {
            e.min(o) / e.max(o)
        } else {
            0.0
        },
    }
}

/// Beat period over the most common inter-onset interval (0 with fewer than 4 onsets).
pub fn tatums_per_beat(onsets: &[Onset], bpm: f64) -> f64 {
    if onsets.len() < 4 || bpm <= 0.0 {
        return 0.0;
    }
    let mut times: Vec<f64> = Vec::new();
    for o in onsets {
        if times.last().is_none_or(|t| o.time_s - t >= 0.06) {
            times.push(o.time_s);
        }
    }
    let mut hist = std::collections::BTreeMap::<i64, usize>::new();
    for w in times.windows(2) {
        let d = ((w[1] - w[0]) * 100.0).round() as i64;
        if d > 0 {
            *hist.entry(d).or_insert(0) += 1;
        }
    }
    // Neighbouring 10 ms bins count together.
    let best = hist
        .keys()
        .map(|k| {
            let c = hist.get(&(k - 1)).copied().unwrap_or(0)
                + hist.get(k).copied().unwrap_or(0)
                + hist.get(&(k + 1)).copied().unwrap_or(0);
            (*k, c)
        })
        .fold((0i64, 0usize), |b, x| if x.1 > b.1 { x } else { b });
    if best.0 <= 0 {
        return 0.0;
    }
    (60.0 / bpm) / (best.0 as f64 / 100.0)
}
