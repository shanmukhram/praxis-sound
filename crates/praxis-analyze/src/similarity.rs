//! Character similarity of two Sound Profiles, 0..1 (FR-14, FR-15).
//!
//! A weighted distance over normalised features (the profile's `features` vector, read by name
//! from [`crate::profile::FEATURE_NAMES`]), never the waveform. Each group's distance is 0..1;
//! `similarity = 1 - sum(weight * distance) / sum(weight)`.
//!
//! | Group (weight) | Distance |
//! | --- | --- |
//! | tempo (1.5) | `|log2(a / b)| / 0.3`, capped at 1; 1 when only one has a confident tempo (confidence >= 0.2), 0 when neither |
//! | groove (1.5) | mean of swing difference, `|log2|` of the onsets-per-bar ratio / 2, off-beat weight difference, regularity difference |
//! | spectral balance (2) | mean of: mean octave-band difference / 12 dB, tilt difference / 6 dB/oct, `|log2|` centroid ratio / 2, flatness difference / 0.3, brightness spread difference / 0.5 oct |
//! | envelope (1.5) | mean of `|ln|` ratios of attack, decay, release (+10 ms each) / ln 10, sustain difference, crest difference / 12 dB, loudness range difference / 12 LU |
//! | tonality (1) | mean of tonality difference / 0.5, voiced fraction difference |
//! | width (0.5) | mean of width difference / 0.5, correlation difference / 1 |
//! | reverb (0.75) | `|ln|` RT60 ratio / ln 4 when both have one, 0.5 when only one has, 0 when neither |
//! | kind (1) | `1 - sum(min(pa, pb))` over the music / sfx / ambience probabilities |
//!
//! Every distance is capped at 1. Features missing from a profile (an older, shorter vector)
//! count as distance 0.5 for their term.

use crate::analysis::feature;
use crate::profile::SoundProfile;

fn f(p: &SoundProfile, name: &str) -> Option<f64> {
    feature(p, name).map(|v| v as f64)
}

fn cap(x: f64) -> f64 {
    if x.is_finite() {
        x.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// `|a - b| / scale`, capped; 0.5 when either is missing.
fn diff(a: &SoundProfile, b: &SoundProfile, name: &str, scale: f64) -> f64 {
    match (f(a, name), f(b, name)) {
        (Some(x), Some(y)) => cap((x - y).abs() / scale),
        _ => 0.5,
    }
}

/// `|log2(a / b)| / scale` over values offset by `eps`; 0.5 when either is missing.
fn log_ratio(a: &SoundProfile, b: &SoundProfile, name: &str, eps: f64, scale: f64) -> f64 {
    match (f(a, name), f(b, name)) {
        (Some(x), Some(y)) => cap(((x.max(0.0) + eps) / (y.max(0.0) + eps)).log2().abs() / scale),
        _ => 0.5,
    }
}

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

/// Per-group distances (tempo, groove, spectral, envelope, tonality, width, reverb, kind).
pub fn group_distances(a: &SoundProfile, b: &SoundProfile) -> [(&'static str, f64, f64); 8] {
    let conf = |p: &SoundProfile| f(p, "tempo_confidence").unwrap_or(0.0) >= 0.2;
    let tempo = match (conf(a), conf(b)) {
        (true, true) => log_ratio(a, b, "tempo_bpm", 0.0, 0.3),
        (false, false) => 0.0,
        _ => 1.0,
    };
    let groove = mean(&[
        diff(a, b, "swing", 1.0),
        log_ratio(a, b, "onsets_per_bar", 0.5, 2.0),
        diff(a, b, "offbeat_weight", 1.0),
        diff(a, b, "regularity", 1.0),
    ]);
    let bands = [
        "band_31_db",
        "band_63_db",
        "band_125_db",
        "band_250_db",
        "band_500_db",
        "band_1k_db",
        "band_2k_db",
        "band_4k_db",
        "band_8k_db",
        "band_16k_db",
    ];
    // Band levels below -60 dB are all "nothing there".
    let band = mean(
        &bands
            .iter()
            .map(|n| match (f(a, n), f(b, n)) {
                (Some(x), Some(y)) => cap((x.max(-60.0) - y.max(-60.0)).abs() / 12.0),
                _ => 0.5,
            })
            .collect::<Vec<_>>(),
    );
    let spectral = mean(&[
        band,
        diff(a, b, "tilt_db_per_oct", 6.0),
        log_ratio(a, b, "centroid_hz", 1.0, 2.0),
        diff(a, b, "flatness", 0.3),
        diff(a, b, "brightness_sd_oct", 0.5),
    ]);
    let ln10 = std::f64::consts::LN_10;
    let lnr = |name: &str| log_ratio(a, b, name, 0.01, ln10 / std::f64::consts::LN_2);
    let envelope = mean(&[
        lnr("attack_s"),
        lnr("decay_s"),
        lnr("release_s"),
        diff(a, b, "sustain", 1.0),
        diff(a, b, "crest_db", 12.0),
        diff(a, b, "loudness_range_lu", 12.0),
    ]);
    let tonality = mean(&[
        diff(a, b, "tonality", 0.5),
        diff(a, b, "voiced_fraction", 1.0),
    ]);
    let width = mean(&[
        diff(a, b, "stereo_width", 0.5),
        diff(a, b, "stereo_correlation", 1.0),
    ]);
    let has_rt = |p: &SoundProfile| f(p, "reverb_rt60_s").unwrap_or(0.0) > 0.0;
    let reverb = match (has_rt(a), has_rt(b)) {
        (true, true) => log_ratio(a, b, "reverb_rt60_s", 0.0, 2.0),
        (false, false) => 0.0,
        _ => 0.5,
    };
    let kind = {
        let names = ["class_music", "class_sfx", "class_ambience"];
        let mut shared = 0.0;
        let mut ok = true;
        for n in names {
            match (f(a, n), f(b, n)) {
                (Some(x), Some(y)) => shared += x.min(y),
                _ => ok = false,
            }
        }
        if ok { cap(1.0 - shared) } else { 0.5 }
    };
    [
        ("tempo", 1.5, tempo),
        ("groove", 1.5, groove),
        ("spectral", 2.0, spectral),
        ("envelope", 1.5, envelope),
        ("tonality", 1.0, tonality),
        ("width", 0.5, width),
        ("reverb", 0.75, reverb),
        ("kind", 1.0, kind),
    ]
}

/// Character similarity of two profiles, 0..1 (1 = the same character).
pub fn similarity(a: &SoundProfile, b: &SoundProfile) -> f32 {
    let g = group_distances(a, b);
    let wsum: f64 = g.iter().map(|x| x.1).sum();
    let d: f64 = g.iter().map(|x| x.1 * x.2).sum::<f64>() / wsum;
    cap(1.0 - d) as f32
}
