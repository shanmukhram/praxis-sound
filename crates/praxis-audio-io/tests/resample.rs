//! Offline resampling with rubato.

mod common;

use common::*;
use praxis_audio_io::decode::resample;
use praxis_dsp::StereoBuffer;

#[test]
fn sine_44k1_to_48k_keeps_frequency_level_and_alignment() {
    let input = stereo_sines(1000.0, 3000.0, 0.5, 44_100, 44_100);
    let out = resample(&input, 44_100, 48_000);
    assert_eq!(out.len(), 48_000, "ceil(len * 48000 / 44100)");
    let fl = dominant_hz(&out.left, 48_000);
    let fr = dominant_hz(&out.right, 48_000);
    println!("44.1k -> 48k: {fl:.3} Hz / {fr:.3} Hz");
    assert!((fl - 1000.0).abs() < 0.1, "{fl}");
    assert!((fr - 3000.0).abs() < 0.1, "{fr}");
    // Level preserved away from the edges.
    let mid = &out.left[4800..43_200];
    let level = rms(mid);
    assert!((level - 0.5 / 2f64.sqrt()).abs() < 0.005, "rms {level}");
    // Delay trimmed: the output matches the ideal 48 kHz sine within a sub-sample shift.
    let (mut best_shift, mut best_err) = (0.0, f32::MAX);
    for k in -200..=200 {
        let shift = k as f64 / 100.0; // output samples
        let ideal: Vec<f32> = (4800..43_200)
            .map(|i| {
                let t = (i as f64 - shift) / 48_000.0;
                (0.5 * (2.0 * std::f64::consts::PI * 1000.0 * t).sin()) as f32
            })
            .collect();
        let err = max_abs_diff(&out.left[4800..43_200], &ideal);
        if err < best_err {
            (best_shift, best_err) = (shift, err);
        }
    }
    println!("best alignment: shift {best_shift} samples, max deviation {best_err:e}");
    assert!(best_shift.abs() < 1.0, "delay not trimmed: {best_shift}");
    assert!(best_err < 2e-3, "{best_err}");
}

#[test]
fn downsampling_and_determinism() {
    let input = stereo_sines(440.0, 440.0, 0.8, 96_000, 48_000);
    let a = resample(&input, 96_000, 44_100);
    let b = resample(&input, 96_000, 44_100);
    assert_eq!(a, b, "bit-identical reruns");
    assert_eq!(a.len(), 22_050);
    let f = dominant_hz(&a.left, 44_100);
    assert!((f - 440.0).abs() < 0.1, "{f}");
}

#[test]
fn equal_rates_and_edge_cases() {
    let input = stereo_sines(100.0, 200.0, 0.3, 48_000, 1000);
    assert_eq!(resample(&input, 48_000, 48_000), input);
    assert_eq!(resample(&input, 0, 48_000), input);
    assert!(resample(&StereoBuffer::default(), 44_100, 48_000).is_empty());
    // A clip shorter than one resampler chunk still comes out at the right length.
    let short = stereo_sines(100.0, 100.0, 0.3, 44_100, 100);
    assert_eq!(resample(&short, 44_100, 48_000).len(), 109);
}
