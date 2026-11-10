//! Shared helpers for the audio-io integration tests.
#![allow(dead_code)]

use praxis_dsp::StereoBuffer;
use std::path::{Path, PathBuf};

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

pub fn repo_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join(name)
}

/// A fresh, empty directory under the cargo target tmp dir.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("praxis-audio-io")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

pub fn sine(freq: f64, amp: f64, sample_rate: u32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| {
            (amp * (2.0 * std::f64::consts::PI * freq * i as f64 / sample_rate as f64).sin()) as f32
        })
        .collect()
}

pub fn stereo_sines(fl: f64, fr: f64, amp: f64, sample_rate: u32, frames: usize) -> StereoBuffer {
    StereoBuffer {
        left: sine(fl, amp, sample_rate, frames),
        right: sine(fr, amp, sample_rate, frames),
    }
}

/// Hann-windowed Goertzel power of `x` at `freq`.
fn power_at(x: &[f32], freq: f64, sample_rate: u32) -> f64 {
    let n = x.len();
    let w = 2.0 * std::f64::consts::PI * freq / sample_rate as f64;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for (i, &v) in x.iter().enumerate() {
        let win = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos();
        let s0 = v as f64 * win + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    s1 * s1 + s2 * s2 - coeff * s1 * s2
}

/// Frequency with the most energy in 40..4000 Hz, searched on a 0.5 s window from the middle
/// of `x`: a 2 Hz coarse scan, then a 0.02 Hz refinement.
pub fn dominant_hz(x: &[f32], sample_rate: u32) -> f64 {
    let n = (sample_rate as usize / 2).min(x.len());
    let start = (x.len() - n) / 2;
    let seg = &x[start..start + n];
    let mut best = (0.0, f64::MIN);
    let mut f = 40.0;
    while f <= 4000.0 {
        let p = power_at(seg, f, sample_rate);
        if p > best.1 {
            best = (f, p);
        }
        f += 2.0;
    }
    let centre = best.0;
    let mut f = centre - 2.0;
    while f <= centre + 2.0 {
        let p = power_at(seg, f, sample_rate);
        if p > best.1 {
            best = (f, p);
        }
        f += 0.02;
    }
    best.0
}

pub fn rms(x: &[f32]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|&v| v as f64 * v as f64).sum::<f64>() / x.len() as f64).sqrt()
}

pub fn max_abs_diff(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "length mismatch");
    a.iter()
        .zip(b)
        .fold(0.0f32, |m, (x, y)| m.max((x - y).abs()))
}

pub fn ffmpeg_or_skip(test: &str) -> Option<PathBuf> {
    let f = praxis_audio_io::decode::find_ffmpeg();
    if f.is_none() {
        println!("skipping {test}: ffmpeg not found on PATH or PRAXIS_FFMPEG");
    }
    f
}
