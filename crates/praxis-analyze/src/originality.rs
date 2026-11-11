//! Originality guard (FR-18): a render may share a reference's character, never its melody.
//!
//! Both pitch tracks become note sequences, and the melodies are compared as **interval
//! sequences** (semitones between consecutive notes), so a copy in another key still matches:
//!
//! 1. **Notes**: runs of voiced 10 ms frames. A note ends at an unvoiced gap longer than 20 ms,
//!    where the pitch moves more than 0.7 semitone from the note's median for 3 frames in a row,
//!    or at an onset (re-struck note) once it is 50 ms long. Notes shorter than 50 ms are
//!    dropped; consecutive notes on the same semitone with no onset between them are merged.
//!    The note's pitch is its median.
//! 2. **Intervals**: rounded differences of consecutive note pitches, clamped to +-24.
//! 3. **n-gram overlap**: the share of the render's interval 3-grams and 4-grams (multisets) that
//!    also occur in the reference, averaged over n = 3 and 4.
//! 4. **DTW**: subsequence dynamic time warping of the shorter interval sequence against the
//!    longer (free start and end in the longer), match cost `min(|a - b|, 4) / 4`, plus
//!    [`WARP_PENALTY`] for every warping step (one interval matched to several), so an
//!    unrelated step-wise melody cannot be stretched onto another for free; the total cost is
//!    divided by the shorter sequence's length and similarity = 1 - cost.
//! 5. `melodic_similarity = 0.3 * n-gram overlap + 0.7 * DTW similarity` ([`NGRAM_WEIGHT`]):
//!    the n-grams catch exact shared phrases, DTW keeps a copy with a few changed notes close.
//!
//! The check fails when the melodic similarity reaches [`MELODIC_THRESHOLD`]. Fewer than
//! [`MIN_INTERVALS`] intervals on either side cannot carry a melody: the check passes with that
//! reason. `rhythmic_similarity` (the same n-gram overlap on inter-onset-interval ratios, in
//! quarter-octave steps) is reported, but shared rhythm is character, not copying, and never
//! fails the check.
//!
//! Calibration (2026-11-11, `crates/praxis-analyze/tests/originality.rs`): 12 seeded 16-note
//! melodies; 60 close copies (transposed, slower and transposed, faster and staccato, two notes
//! moved a tone, the first half in another key) against 36 unrelated melodies (another melody,
//! another at another tempo and timbre, the same rhythm with newly drawn pitches). The least
//! similar copy reads 0.702 (two notes changed), the most similar unrelated melody 0.492; the
//! threshold 0.6 sits between them.

use crate::profile::{PitchTrack, SoundProfile};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Melodic similarity at or above this fails the check (a close copy).
pub const MELODIC_THRESHOLD: f32 = 0.6;
/// Fewer intervals than this on either side: nothing to compare.
pub const MIN_INTERVALS: usize = 6;

/// Result of [`check`].
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct OriginalityReport {
    pub passed: bool,
    /// 0..1; at or above [`MELODIC_THRESHOLD`] is a close copy.
    pub melodic_similarity: f32,
    /// 0..1, reported only.
    pub rhythmic_similarity: f32,
    pub reason: String,
}

/// One note of a pitch track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteSeg {
    pub start_s: f64,
    pub dur_s: f64,
    /// Median fractional MIDI pitch.
    pub midi: f32,
}

fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

/// Notes of a pitch track (see the module docs). `onsets_s` mark re-struck notes.
pub fn notes(track: &PitchTrack, onsets_s: &[f64]) -> Vec<NoteSeg> {
    let hop = track.hop_s.max(1e-4) as f64;
    let min_frames = ((0.05 / hop).round() as usize).max(1);
    let max_gap = ((0.02 / hop).round() as usize).max(1);
    let mut onset_frames: Vec<usize> = onsets_s
        .iter()
        .map(|t| (t / hop).round().max(0.0) as usize)
        .collect();
    onset_frames.sort_unstable();
    let is_onset = |i: usize| onset_frames.binary_search(&i).is_ok();
    let mut out: Vec<(usize, usize, f32, bool)> = Vec::new(); // start, len, midi, struck
    let mut cur: Vec<f32> = Vec::new();
    let mut start = 0usize;
    let mut gap = 0usize;
    let mut away = 0usize;
    let mut struck = false;
    let flush = |cur: &mut Vec<f32>, start: usize, struck: bool, out: &mut Vec<_>| {
        if cur.len() >= min_frames {
            let len = cur.len();
            out.push((start, len, median(cur), struck));
        }
        cur.clear();
    };
    for (i, p) in track.midi.iter().enumerate() {
        match p {
            Some(m) => {
                if cur.is_empty() {
                    start = i;
                    struck = is_onset(i) || (i > 0 && is_onset(i - 1));
                    cur.push(*m);
                    gap = 0;
                    away = 0;
                    continue;
                }
                let mut tmp = cur.clone();
                let med = median(&mut tmp);
                if (m - med).abs() > 0.7 {
                    away += 1;
                } else {
                    away = 0;
                }
                let restruck = cur.len() >= min_frames && is_onset(i);
                if away >= 3 || restruck {
                    // The new note starts where the pitch left (or at the onset).
                    let back = if restruck { 0 } else { away - 1 };
                    let keep = cur.len() - back;
                    let tail: Vec<f32> = cur.split_off(keep);
                    flush(&mut cur, start, struck, &mut out);
                    start = i - back;
                    struck = restruck;
                    cur = tail;
                    away = 0;
                }
                cur.push(*m);
                gap = 0;
            }
            None => {
                if !cur.is_empty() {
                    gap += 1;
                    if gap > max_gap {
                        flush(&mut cur, start, struck, &mut out);
                        gap = 0;
                    }
                }
            }
        }
    }
    flush(&mut cur, start, struck, &mut out);
    // Merge same-semitone neighbours that were not re-struck.
    let mut merged: Vec<(usize, usize, f32, bool)> = Vec::new();
    for n in out {
        if let Some(last) = merged.last_mut()
            && !n.3
            && (last.2.round() - n.2.round()).abs() < 0.5
        {
            last.1 = n.0 + n.1 - last.0;
            continue;
        }
        merged.push(n);
    }
    merged
        .iter()
        .map(|(s, l, m, _)| NoteSeg {
            start_s: *s as f64 * hop,
            dur_s: *l as f64 * hop,
            midi: *m,
        })
        .collect()
}

/// Rounded semitone steps between consecutive notes, clamped to +-24.
pub fn intervals(notes: &[NoteSeg]) -> Vec<i32> {
    notes
        .windows(2)
        .map(|w| ((w[1].midi - w[0].midi).round() as i32).clamp(-24, 24))
        .collect()
}

/// Share of `b`'s n-grams (multiset) also in `a`. `None` when `b` has no n-gram.
pub fn ngram_overlap<T: Ord + Clone>(a: &[T], b: &[T], n: usize) -> Option<f64> {
    if n == 0 || b.len() < n {
        return None;
    }
    let mut ca: BTreeMap<&[T], usize> = BTreeMap::new();
    for g in a.windows(n) {
        *ca.entry(g).or_insert(0) += 1;
    }
    let mut cb: BTreeMap<&[T], usize> = BTreeMap::new();
    for g in b.windows(n) {
        *cb.entry(g).or_insert(0) += 1;
    }
    let total: usize = cb.values().sum();
    let shared: usize = cb
        .iter()
        .map(|(g, c)| (*c).min(ca.get(g).copied().unwrap_or(0)))
        .sum();
    Some(shared as f64 / total as f64)
}

/// Extra cost of a warping step (one interval matched to several), so DTW cannot stretch an
/// unrelated melody onto another for free.
pub const WARP_PENALTY: f64 = 0.5;

/// Subsequence DTW similarity of interval sequences (see the module docs). 0 when either is
/// empty.
pub fn dtw_similarity(a: &[i32], b: &[i32]) -> f64 {
    let (q, r) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let (n, m) = (q.len(), r.len());
    if n == 0 || m == 0 {
        return 0.0;
    }
    let cost = |i: usize, j: usize| ((q[i] - r[j]).abs().min(4)) as f64 / 4.0;
    // Row 0: the query may start anywhere in the reference.
    let mut prev: Vec<f64> = (0..m).map(|j| cost(0, j)).collect();
    for j in 1..m {
        prev[j] = prev[j].min(prev[j - 1] + cost(0, j) + WARP_PENALTY);
    }
    let mut cur = vec![0.0f64; m];
    for i in 1..n {
        for j in 0..m {
            let c = cost(i, j);
            let mut best = prev[j] + WARP_PENALTY;
            if j > 0 {
                best = best.min(prev[j - 1]).min(cur[j - 1] + WARP_PENALTY);
            }
            cur[j] = c + best;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    // ... and end anywhere. Normalised by the query length.
    let best = prev.iter().fold(f64::INFINITY, |a, b| a.min(*b));
    (1.0 - best / n as f64).clamp(0.0, 1.0)
}

/// Mean of the 3-gram and 4-gram overlaps (0 when the render is too short for them).
pub fn ngram_score(reference: &[i32], render: &[i32]) -> f64 {
    let g3 = ngram_overlap(reference, render, 3).unwrap_or(0.0);
    let g4 = ngram_overlap(reference, render, 4).unwrap_or(0.0);
    0.5 * (g3 + g4)
}

/// Melodic similarity of two interval sequences, 0..1 (see the module docs).
pub fn melodic_similarity(reference: &[i32], render: &[i32]) -> f64 {
    NGRAM_WEIGHT * ngram_score(reference, render)
        + (1.0 - NGRAM_WEIGHT) * dtw_similarity(reference, render)
}

/// Weight of the n-gram overlap in [`melodic_similarity`] (the rest is DTW).
pub const NGRAM_WEIGHT: f64 = 0.3;

/// Inter-onset-interval ratios in quarter-octave steps (tempo-invariant rhythm).
pub fn ioi_ratios(onsets_s: &[f64]) -> Vec<i32> {
    let iois: Vec<f64> = onsets_s
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| *d > 0.02)
        .collect();
    iois.windows(2)
        .map(|w| ((w[1] / w[0]).log2() * 4.0).round().clamp(-16.0, 16.0) as i32)
        .collect()
}

/// Compares a render with its reference (see the module docs).
pub fn check(reference: &SoundProfile, render: &SoundProfile) -> OriginalityReport {
    let ra = ioi_ratios(&reference.onsets_s);
    let rb = ioi_ratios(&render.onsets_s);
    let rhythmic = match (ngram_overlap(&ra, &rb, 3), ngram_overlap(&ra, &rb, 4)) {
        (Some(a), Some(b)) => 0.5 * (a + b),
        (Some(a), None) => a,
        _ => 0.0,
    };
    let ia = intervals(&notes(&reference.pitch, &reference.onsets_s));
    let ib = intervals(&notes(&render.pitch, &render.onsets_s));
    if ia.len() < MIN_INTERVALS || ib.len() < MIN_INTERVALS {
        return OriginalityReport {
            passed: true,
            melodic_similarity: 0.0,
            rhythmic_similarity: rhythmic as f32,
            reason: format!(
                "not enough melody to compare ({} and {} note steps; {MIN_INTERVALS} needed)",
                ia.len(),
                ib.len()
            ),
        };
    }
    let m = melodic_similarity(&ia, &ib) as f32;
    let passed = m < MELODIC_THRESHOLD;
    let reason = if passed {
        format!(
            "melody differs from the reference (similarity {m:.2}, a copy is {MELODIC_THRESHOLD:.2} or more)"
        )
    } else {
        format!(
            "melody is a close copy of the reference (similarity {m:.2}, limit {MELODIC_THRESHOLD:.2})"
        )
    };
    OriginalityReport {
        passed,
        melodic_similarity: m,
        rhythmic_similarity: rhythmic as f32,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_and_transposed_intervals_match() {
        let a = vec![2, 2, -4, 5, -1, -2, 3, -3, 2];
        assert!((melodic_similarity(&a, &a) - 1.0).abs() < 1e-9);
        let b = vec![-1, 3, 1, -5, 2, 2, -2, 4, -1];
        assert!(melodic_similarity(&a, &b) < 0.7);
    }

    #[test]
    fn dtw_finds_an_excerpt() {
        let a = vec![1, 2, 3, 4, 5, -3, -2, 2, 1, -5, 7];
        let b = vec![5, -3, -2, 2];
        assert!((dtw_similarity(&a, &b) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn notes_split_on_pitch_change_and_onsets() {
        let mut midi = Vec::new();
        midi.extend(std::iter::repeat_n(Some(60.0f32), 30));
        midi.extend(std::iter::repeat_n(Some(64.0f32), 30));
        midi.extend(std::iter::repeat_n(None, 5));
        midi.extend(std::iter::repeat_n(Some(64.0f32), 30));
        midi.extend(std::iter::repeat_n(Some(64.0f32), 30));
        let t = PitchTrack { hop_s: 0.01, midi };
        let n = notes(&t, &[0.0, 0.3, 0.65, 0.95]);
        let p: Vec<i32> = n.iter().map(|x| x.midi.round() as i32).collect();
        assert_eq!(p, vec![60, 64, 64, 64], "{n:?}");
    }
}
