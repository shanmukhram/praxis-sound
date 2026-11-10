//! Key estimation over the 24 major and minor keys.
//!
//! The mean chroma is correlated (Pearson) with every rotation of two key-profile pairs:
//! Krumhansl-Schmuckler (probe-tone ratings, Krumhansl & Kessler 1982) and Temperley (Kostka-Payne
//! corpus, Temperley 2007). The key score is the mean of the two correlations; the best key wins.
//! Confidence is the best score (clamped to 0..1) scaled by how clearly it beats the runner-up:
//! `best * clamp((best - second) / 0.1, 0, 1)`.

use super::chroma::chromagram;
use crate::profile::KeyEstimate;
use praxis_score::{Key, Mode, PitchClass};

/// Krumhansl-Kessler major profile, tonic first.
pub const KRUMHANSL_MAJOR: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
/// Krumhansl-Kessler minor profile, tonic first.
pub const KRUMHANSL_MINOR: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];
/// Temperley (Kostka-Payne) major profile, tonic first.
pub const TEMPERLEY_MAJOR: [f32; 12] = [
    0.748, 0.060, 0.488, 0.082, 0.670, 0.460, 0.096, 0.715, 0.104, 0.366, 0.057, 0.400,
];
/// Temperley (Kostka-Payne) minor profile, tonic first.
pub const TEMPERLEY_MINOR: [f32; 12] = [
    0.712, 0.084, 0.474, 0.618, 0.049, 0.460, 0.105, 0.747, 0.404, 0.067, 0.133, 0.330,
];

/// Result of [`estimate_key`].
#[derive(Clone, Debug, PartialEq)]
pub struct KeyResult {
    pub key: Key,
    pub confidence: f32,
    /// Combined correlation of the winning key.
    pub score: f32,
    /// Scores of all 24 keys: index `tonic` for major, `12 + tonic` for minor (C = 0).
    pub scores: [f32; 24],
}

fn pearson(a: &[f32; 12], b: &[f32; 12]) -> f64 {
    let ma = a.iter().map(|v| *v as f64).sum::<f64>() / 12.0;
    let mb = b.iter().map(|v| *v as f64).sum::<f64>() / 12.0;
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for i in 0..12 {
        let x = a[i] as f64 - ma;
        let y = b[i] as f64 - mb;
        sab += x * y;
        saa += x * x;
        sbb += y * y;
    }
    if saa <= 0.0 || sbb <= 0.0 {
        0.0
    } else {
        sab / (saa * sbb).sqrt()
    }
}

fn rotate(profile: &[f32; 12], tonic: usize) -> [f32; 12] {
    let mut out = [0.0f32; 12];
    for (i, v) in profile.iter().enumerate() {
        out[(i + tonic) % 12] = *v;
    }
    out
}

/// Combined KS + Temperley correlation for each of the 24 keys (see [`KeyResult::scores`]).
pub fn key_scores(chroma: &[f32; 12]) -> [f32; 24] {
    let mut out = [0.0f32; 24];
    for tonic in 0..12 {
        let ks = pearson(chroma, &rotate(&KRUMHANSL_MAJOR, tonic));
        let te = pearson(chroma, &rotate(&TEMPERLEY_MAJOR, tonic));
        out[tonic] = (0.5 * (ks + te)) as f32;
        let ks = pearson(chroma, &rotate(&KRUMHANSL_MINOR, tonic));
        let te = pearson(chroma, &rotate(&TEMPERLEY_MINOR, tonic));
        out[12 + tonic] = (0.5 * (ks + te)) as f32;
    }
    out
}

/// Best key for a chroma vector. `None` for a flat (e.g. silent) chroma.
pub fn estimate_key(chroma: &[f32; 12]) -> Option<KeyResult> {
    let max = chroma.iter().fold(0.0f32, |m, v| m.max(*v));
    let min = chroma.iter().fold(f32::INFINITY, |m, v| m.min(*v));
    if max.is_nan() || max <= 0.0 || max - min <= max * 1e-6 {
        return None;
    }
    let scores = key_scores(chroma);
    let mut order: Vec<usize> = (0..24).collect();
    // Stable order: ties resolve to the lower index (major before minor, C first).
    order.sort_by(|a, b| scores[*b].total_cmp(&scores[*a]));
    let best = order[0];
    let second = order[1];
    let b = scores[best];
    let margin = ((b - scores[second]) / 0.1).clamp(0.0, 1.0);
    let key = Key {
        tonic: PitchClass::ALL[best % 12],
        mode: if best < 12 { Mode::Major } else { Mode::Minor },
    };
    Some(KeyResult {
        key,
        confidence: b.clamp(0.0, 1.0) * margin,
        score: b,
        scores,
    })
}

/// The major/minor family a mode is compared as: `Major` for major, Lydian, Mixolydian and
/// major pentatonic; `Minor` for the minor modes, Dorian, Phrygian, Locrian and minor pentatonic;
/// `None` for chromatic (no key).
pub fn mode_family(mode: Mode) -> Option<Mode> {
    match mode {
        Mode::Major | Mode::Lydian | Mode::Mixolydian | Mode::MajorPentatonic => Some(Mode::Major),
        Mode::Minor
        | Mode::HarmonicMinor
        | Mode::MelodicMinor
        | Mode::Dorian
        | Mode::Phrygian
        | Mode::Locrian
        | Mode::MinorPentatonic => Some(Mode::Minor),
        Mode::Chromatic => None,
    }
}

/// Key analysis of a mono signal (chromagram + profiles). `None` for silence or flat chroma.
pub fn analyze(signal: &[f32], sample_rate: u32) -> Option<KeyEstimate> {
    let chroma = chromagram(signal, sample_rate).mean();
    let r = estimate_key(&chroma)?;
    Some(KeyEstimate {
        key: r.key,
        confidence: r.confidence,
        chroma,
    })
}
