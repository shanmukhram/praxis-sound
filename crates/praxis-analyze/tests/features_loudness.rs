//! Loudness meter (BS.1770 via ebur128) against known values.

mod features_synth;

use features_synth::*;
use praxis_analyze::features::loudness::measure;
use praxis_analyze::features::{FLOOR_DB, FeatureError};
use std::f64::consts::PI;

fn sine(sr: u32, hz: f64, amp: f64, seconds: f64) -> Vec<f32> {
    (0..(seconds * sr as f64) as usize)
        .map(|i| (amp * (2.0 * PI * hz * i as f64 / sr as f64).sin()) as f32)
        .collect()
}

#[test]
fn reference_sine_reads_minus_20_lufs() {
    // A 997 Hz sine at -20 dBFS peak in both channels reads -20 LUFS (BS.1770 calibration).
    let s = sine(SR, 997.0, 0.1, 10.0);
    let l = measure(&s, &s, SR).unwrap();
    println!(
        "997 Hz -20 dBFS stereo: {:.3} LUFS, LRA {:.2} LU, TP {:.2} dBTP, peak {:.2} dBFS, RMS {:.2} dBFS, crest {:.2} dB",
        l.integrated_lufs, l.range_lu, l.true_peak_dbtp, l.sample_peak_dbfs, l.rms_dbfs, l.crest_db
    );
    assert!((l.integrated_lufs + 20.0).abs() < 0.1);
    assert!((l.sample_peak_dbfs + 20.0).abs() < 0.01);
    assert!((l.rms_dbfs + 23.01).abs() < 0.02);
    assert!((l.crest_db - 3.01).abs() < 0.02);
    assert!(l.true_peak_dbtp >= l.sample_peak_dbfs && l.true_peak_dbtp < -19.9);
    assert!(l.range_lu < 0.5);
}

#[test]
fn inter_sample_peak_is_seen_by_true_peak() {
    // fs/4 sine with 45 degree phase: samples sit at 0.707 of the real peak (-3 dB).
    let sr = 48_000;
    let s: Vec<f32> = (0..sr as usize * 2)
        .map(|i| (0.9 * (PI / 2.0 * i as f64 + PI / 4.0).sin()) as f32)
        .collect();
    let l = measure(&s, &s, sr).unwrap();
    println!(
        "fs/4 at 45 deg: sample peak {:.2} dBFS, true peak {:.2} dBTP",
        l.sample_peak_dbfs, l.true_peak_dbtp
    );
    assert!(l.true_peak_dbtp - l.sample_peak_dbfs > 2.5);
}

#[test]
fn silence_and_errors() {
    let z = vec![0.0f32; 48_000];
    let l = measure(&z, &z, SR).unwrap();
    assert_eq!(l.integrated_lufs, FLOOR_DB);
    assert_eq!(l.true_peak_dbtp, FLOOR_DB);
    assert_eq!(measure(&z, &z, 0), Err(FeatureError::SampleRate));
    assert!(matches!(
        measure(&z, &z[..10], SR),
        Err(FeatureError::ChannelLength { .. })
    ));
}

#[test]
fn normalisation_helper_hits_target() {
    let mut b = stereo(sine(SR, 220.0, 0.3, 6.0));
    normalize_lufs(&mut b, SR, -16.0);
    let l = measure(&b.left, &b.right, SR).unwrap();
    assert!(
        (l.integrated_lufs + 16.0).abs() < 0.01,
        "{}",
        l.integrated_lufs
    );
}
