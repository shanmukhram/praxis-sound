//! Groove: onsets folded onto the detected beat grid, per bar, and swing.
//!
//! Each onset gets a fractional *beat position* by interpolating between the tracked beats
//! (extrapolating with the beat period before the first and after the last), counted from the
//! downbeat, and folded into one bar (`beats_per_bar` beats).
//!
//! - **Swing**: off-beat eighths are onsets whose position within the beat lies in 0.40..0.72;
//!   swing = `(median - 0.5) / (2/3 - 0.5)` clamped to 0..1 (0 straight, 1 full triplet swing),
//!   0 with fewer than 3 off-beats.
//! - **Grid**: the coarsest grid that places at least 90 % of the onsets within 0.08 beat of a
//!   step: quarter, eighth, sixteenth, then eighth and sixteenth triplets (with swing of 0.5 or
//!   more the eighth triplet is tried before the sixteenth). Every onset counts once (soft
//!   off-beats matter as much as accents); onsets weaker than a quarter of the median strength
//!   are ignored. When no grid reaches 90 %, the grid that places the most.
//! - **Step weights**: onset strength summed per grid step of the bar, divided by the number of
//!   bars, normalised to a maximum of 1.
//! - **Regularity** (for classification): the fraction of onset strength within 0.05 beat of a
//!   sixteenth (random onsets read about 0.4, sequenced music close to 1).

use super::onset::Onset;
use crate::profile::{Groove, TempoEstimate};
use praxis_score::Grid;

const GRID_TOL_BEATS: f64 = 0.08;
const GRID_COVERAGE: f64 = 0.9;
const REGULARITY_TOL_BEATS: f64 = 0.05;

fn steps_per_beat(g: Grid) -> u32 {
    match g {
        Grid::Quarter => 1,
        Grid::Eighth => 2,
        Grid::Sixteenth => 4,
        Grid::EighthTriplet => 3,
        Grid::SixteenthTriplet => 6,
    }
}

/// Fractional beat position of `t` given beat times (`beats` sorted) and the beat period.
pub fn beat_position(beats: &[f64], period: f64, t: f64) -> f64 {
    if beats.is_empty() || period <= 0.0 {
        return 0.0;
    }
    if t <= beats[0] {
        return (t - beats[0]) / period;
    }
    let last = beats.len() - 1;
    if t >= beats[last] {
        return last as f64 + (t - beats[last]) / period;
    }
    let i = beats.partition_point(|b| *b <= t).saturating_sub(1);
    let span = (beats[i + 1] - beats[i]).max(1e-9);
    i as f64 + (t - beats[i]) / span
}

/// Distance (in beats) from `pos` to the nearest step of a grid with `spb` steps per beat.
fn grid_distance(pos: f64, spb: u32) -> f64 {
    let x = pos * spb as f64;
    (x - x.round()).abs() / spb as f64
}

struct Folded {
    /// Position within the bar, beats (0..beats_per_bar).
    bar_pos: f64,
    strength: f64,
}

fn fold(tempo: &TempoEstimate, onsets: &[Onset]) -> Vec<Folded> {
    let period = 60.0 / tempo.bpm.max(1e-6);
    let bpb = tempo.beats_per_bar.max(1) as f64;
    let beats = &tempo.beats_s;
    let d = beats
        .iter()
        .position(|b| (*b - tempo.downbeat_s).abs() < 1e-9)
        .unwrap_or(0) as f64;
    onsets
        .iter()
        .map(|o| {
            let p = beat_position(beats, period, o.time_s) - d;
            Folded {
                bar_pos: p.rem_euclid(bpb),
                strength: (o.strength as f64).max(1e-6),
            }
        })
        .collect()
}

fn coverage(folded: &[Folded], spb: u32, tol: f64) -> f64 {
    let total: f64 = folded.iter().map(|f| f.strength).sum();
    if total <= 0.0 {
        return 0.0;
    }
    folded
        .iter()
        .filter(|f| grid_distance(f.bar_pos, spb) <= tol)
        .map(|f| f.strength)
        .sum::<f64>()
        / total
}

/// Swing 0..1 from off-beat eighths (see the module docs).
fn swing_of(folded: &[Folded]) -> f64 {
    let mut off: Vec<f64> = folded
        .iter()
        .map(|f| f.bar_pos - f.bar_pos.floor())
        .filter(|x| (0.40..=0.72).contains(x))
        .collect();
    if off.len() < 3 {
        return 0.0;
    }
    off.sort_by(|a, b| a.total_cmp(b));
    let m = off[off.len() / 2];
    ((m - 0.5) / (2.0 / 3.0 - 0.5)).clamp(0.0, 1.0)
}

/// Strength-weighted fraction of onsets within 0.05 beat of a sixteenth.
pub fn regularity(tempo: &TempoEstimate, onsets: &[Onset]) -> f64 {
    coverage(&fold(tempo, onsets), 4, REGULARITY_TOL_BEATS)
}

/// Groove of `onsets` on the grid of `tempo`. `None` with fewer than 4 onsets or no beats.
pub fn analyze(tempo: &TempoEstimate, onsets: &[Onset]) -> Option<Groove> {
    if onsets.len() < 4 || tempo.beats_s.len() < 2 || tempo.bpm <= 0.0 {
        return None;
    }
    let folded = fold(tempo, onsets);
    let swing = swing_of(&folded);
    let mut order = vec![Grid::Quarter, Grid::Eighth];
    if swing >= 0.5 {
        order.extend([Grid::EighthTriplet, Grid::Sixteenth]);
    } else {
        order.extend([Grid::Sixteenth, Grid::EighthTriplet]);
    }
    order.push(Grid::SixteenthTriplet);
    // Count-based coverage: each onset (above a quarter of the median strength) weighs 1.
    let mut strengths: Vec<f64> = folded.iter().map(|f| f.strength).collect();
    strengths.sort_by(|a, b| a.total_cmp(b));
    let floor = 0.25 * strengths[strengths.len() / 2];
    let counted: Vec<Folded> = folded
        .iter()
        .filter(|f| f.strength >= floor)
        .map(|f| Folded {
            bar_pos: f.bar_pos,
            strength: 1.0,
        })
        .collect();
    let covs: Vec<(Grid, f64)> = order
        .iter()
        .map(|g| (*g, coverage(&counted, steps_per_beat(*g), GRID_TOL_BEATS)))
        .collect();
    let grid = covs
        .iter()
        .find(|(_, c)| *c >= GRID_COVERAGE)
        .map(|(g, _)| *g)
        .unwrap_or_else(|| {
            covs.iter()
                .fold((Grid::Sixteenth, -1.0), |b, (g, c)| {
                    if *c > b.1 + 1e-9 { (*g, *c) } else { b }
                })
                .0
        });
    let spb = steps_per_beat(grid);
    let bpb = tempo.beats_per_bar.max(1) as u32;
    let steps = spb * bpb;
    let mut w = vec![0.0f64; steps as usize];
    for f in &folded {
        let s = (f.bar_pos * spb as f64).round() as u32 % steps;
        w[s as usize] += f.strength;
    }
    let period = 60.0 / tempo.bpm;
    let first = tempo.beats_s.first().copied().unwrap_or(0.0);
    let last = tempo.beats_s.last().copied().unwrap_or(first);
    let bars = ((last - first + period) / (period * bpb as f64)).max(1.0);
    let max = w.iter().fold(0.0f64, |m, v| m.max(*v));
    let step_weights = w
        .iter()
        .map(|v| if max > 0.0 { (*v / max) as f32 } else { 0.0 })
        .collect();
    let inside = onsets
        .iter()
        .filter(|o| o.time_s >= first - period / 2.0 && o.time_s <= last + period)
        .count();
    Some(Groove {
        grid,
        steps_per_bar: steps,
        step_weights,
        swing: swing as f32,
        onsets_per_bar: (inside as f64 / bars) as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempo(bpm: f64, beats: usize) -> TempoEstimate {
        let p = 60.0 / bpm;
        TempoEstimate {
            bpm,
            confidence: 0.8,
            beats_s: (0..beats).map(|i| i as f64 * p).collect(),
            downbeat_s: 0.0,
            beats_per_bar: 4,
        }
    }

    fn onset(t: f64, s: f32) -> Onset {
        Onset {
            time_s: t,
            sample: 0,
            frame: 0,
            strength: s,
        }
    }

    #[test]
    fn straight_eighths_and_swung_eighths() {
        let t = tempo(120.0, 32);
        let straight: Vec<Onset> = (0..60).map(|i| onset(i as f64 * 0.25, 1.0)).collect();
        let g = analyze(&t, &straight).unwrap();
        assert_eq!(g.grid, Grid::Eighth);
        assert_eq!(g.steps_per_bar, 8);
        assert!(g.swing < 0.1);
        assert!((g.onsets_per_bar - 8.0).abs() < 0.7, "{}", g.onsets_per_bar);
        let swung: Vec<Onset> = (0..30)
            .flat_map(|b| {
                let t0 = b as f64 * 0.5;
                [onset(t0, 1.0), onset(t0 + 0.5 * 2.0 / 3.0, 0.6)]
            })
            .collect();
        let g = analyze(&t, &swung).unwrap();
        assert!(g.swing > 0.9, "{}", g.swing);
        assert_eq!(g.grid, Grid::EighthTriplet);
    }

    #[test]
    fn accents_show_in_step_weights() {
        let t = tempo(100.0, 40);
        let p = 0.6;
        let ons: Vec<Onset> = (0..36)
            .map(|i| onset(i as f64 * p, if i % 4 == 0 { 1.0 } else { 0.4 }))
            .collect();
        let g = analyze(&t, &ons).unwrap();
        assert_eq!(g.grid, Grid::Quarter);
        assert_eq!(g.step_weights[0], 1.0);
        assert!(g.step_weights[1] < 0.5);
        assert!(regularity(&t, &ons) > 0.95);
    }
}
