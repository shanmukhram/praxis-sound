//! Hit points (FR-10): the tempo is chosen so every hit falls on the grid, then each hit becomes
//! an exact grid position (bar, step) plus a small micro-timing nudge.
//!
//! Score v1 has one constant tempo and one time signature, so the only free variable is the
//! tempo. The solver tries every tempo in the allowed range in 0.01 BPM steps and keeps the one
//! where, in order: the fewest hits are more than [`TOLERANCE_S`] from a sixteenth (or
//! sixteenth-triplet) step, the hits sit on the strongest positions (bar line, beat, half beat,
//! grid step), the largest distance is smallest, and the tempo is nearest the preferred one.

use praxis_score::{Grid, TimeSignature};

/// A hit within this of a grid step counts as on the grid.
pub const TOLERANCE_S: f64 = 0.012;

/// How the bar count follows the tempo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    /// The bar count is fixed.
    Bars(u32),
    /// Whole bars nearest this many seconds.
    Seconds(f64),
}

/// The solved tempo and bar count.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Solved {
    pub bpm: f64,
    pub bars: u32,
    /// Largest distance of a hit from its grid step, seconds.
    pub max_residual_s: f64,
}

fn beats_per_bar(ts: TimeSignature) -> f64 {
    // Compound metres (6/8, 9/8, 12/8) count dotted-quarter beats.
    if ts.denominator == 8 && ts.numerator % 3 == 0 && ts.numerator > 3 {
        (ts.numerator / 3) as f64
    } else {
        ts.numerator.max(1) as f64
    }
}

/// Metric strength of time `t` (0 bar line, 1 beat, 2 half beat, 3 grid step), and its distance
/// in seconds from the nearest grid step.
fn position(t: f64, bar_s: f64, beat_s: f64, step_s: f64) -> (u32, f64) {
    let near = |unit: f64| (t - (t / unit).round() * unit).abs();
    let residual = near(step_s);
    let level = if near(bar_s) <= TOLERANCE_S {
        0
    } else if near(beat_s) <= TOLERANCE_S {
        1
    } else if near(beat_s / 2.0) <= TOLERANCE_S {
        2
    } else {
        3
    };
    (level, residual)
}

/// Bar length in seconds at `bpm`.
pub fn bar_seconds(ts: TimeSignature, bpm: f64) -> f64 {
    ts.bar_quarters() * 60.0 / bpm
}

/// Chooses the tempo in `range` (BPM, inclusive) that lands `hits` (seconds) best on `grid`.
/// Candidates where a hit would fall within half a beat (at least 0.5 s) of the end are skipped.
pub fn solve_tempo(
    hits: &[f64],
    ts: TimeSignature,
    grid: Grid,
    preferred: f64,
    range: (f64, f64),
    length: Length,
) -> Option<Solved> {
    let lo = (range.0.min(range.1) * 100.0).ceil() as i64;
    let hi = (range.0.max(range.1) * 100.0).floor() as i64;
    let mut best: Option<((usize, u32, i64, i64), Solved)> = None;
    for c in lo..=hi {
        let bpm = c as f64 / 100.0;
        let bar_s = bar_seconds(ts, bpm);
        let beat_s = bar_s / beats_per_bar(ts);
        let step_s = grid.step_whole_notes() * 4.0 * 60.0 / bpm;
        let bars = match length {
            Length::Bars(b) => b.max(1),
            Length::Seconds(s) => ((s / bar_s).round() as u32).max(1),
        };
        let end = bars as f64 * bar_s;
        // Room after the last hit: half a beat, and never less than the half second plan
        // validation asks for.
        if hits.iter().any(|h| *h > end - (beat_s / 2.0).max(0.5)) {
            continue;
        }
        let mut far = 0usize;
        let mut levels = 0u32;
        let mut max_r = 0.0f64;
        for &h in hits {
            let (level, r) = position(h, bar_s, beat_s, step_s);
            if r > TOLERANCE_S {
                far += 1;
            }
            levels += level;
            max_r = max_r.max(r);
        }
        // Residuals in whole milliseconds, so the metric strength decides between near equals.
        let key = (
            far,
            levels,
            (max_r * 1000.0).round() as i64,
            ((bpm - preferred).abs() * 100.0).round() as i64,
        );
        let solved = Solved {
            bpm,
            bars,
            max_residual_s: max_r,
        };
        if best.as_ref().is_none_or(|(k, _)| key < *k) {
            best = Some((key, solved));
        }
    }
    best.map(|(_, s)| s)
}

/// Where a hit lands: 1-based bar, step in the bar on `grid`, and the nudge as a fraction of a
/// step (-0.5..=0.5) that puts the onset exactly on the hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landing {
    pub bar: u32,
    pub step: u32,
    pub offset: f32,
    /// Position within the bar, 0..1.
    pub frac: f64,
}

pub fn landing(t: f64, ts: TimeSignature, bpm: f64, grid: Grid) -> Option<Landing> {
    let spb = ts.steps_per_bar(grid)?;
    let step_s = grid.step_whole_notes() * 4.0 * 60.0 / bpm;
    let idx = (t / step_s).round().max(0.0) as u64;
    let offset = ((t - idx as f64 * step_s) / step_s).clamp(-0.5, 0.5) as f32;
    let bar = (idx / spb as u64) as u32 + 1;
    let step = (idx % spb as u64) as u32;
    Some(Landing {
        bar,
        step,
        offset: (offset * 10_000.0).round() / 10_000.0,
        frac: step as f64 / spb as f64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOUR: TimeSignature = TimeSignature {
        numerator: 4,
        denominator: 4,
    };

    #[test]
    fn three_hits_land_within_tolerance_in_a_range() {
        let hits = [42.0, 65.0, 98.0];
        let s = solve_tempo(
            &hits,
            FOUR,
            Grid::Sixteenth,
            100.0,
            (88.0, 120.0),
            Length::Seconds(120.0),
        )
        .expect("solved");
        assert!(s.max_residual_s <= TOLERANCE_S, "{s:?}");
        assert!((88.0..=120.0).contains(&s.bpm));
        let bar_s = bar_seconds(FOUR, s.bpm);
        assert!((s.bars as f64 * bar_s - 120.0).abs() <= bar_s / 2.0 + 1e-9);
        for h in hits {
            let l = landing(h, FOUR, s.bpm, Grid::Sixteenth).unwrap();
            let step_s = 15.0 / s.bpm;
            let at = ((l.bar - 1) as f64 * 16.0 + l.step as f64 + l.offset as f64) * step_s;
            assert!((at - h).abs() < 1e-3, "{h} -> {at}");
            assert!(l.offset.abs() as f64 * step_s <= TOLERANCE_S + 1e-6);
        }
    }

    #[test]
    fn a_single_hit_prefers_a_bar_line() {
        // 30 s at 120 BPM is exactly bar 16's line.
        let s = solve_tempo(
            &[30.0],
            FOUR,
            Grid::Sixteenth,
            118.0,
            (114.0, 122.0),
            Length::Bars(40),
        )
        .unwrap();
        let bar_s = bar_seconds(FOUR, s.bpm);
        let r = (30.0 / bar_s - (30.0 / bar_s).round()).abs() * bar_s;
        assert!(r <= TOLERANCE_S, "{s:?}");
    }
}
