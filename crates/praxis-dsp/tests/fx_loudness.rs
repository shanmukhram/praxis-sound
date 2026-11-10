//! LoudnessMeter and true-peak meter against the `ebur128` reference.

mod fx_support;
use fx_support::*;

use ebur128::{EbuR128, Mode};
use praxis_dsp::meter::{
    LoudnessMeter, integrated_lufs, rms_dbfs, sample_peak_dbfs, true_peak_dbtp,
};

struct Reference {
    integrated: f64,
    lra: f64,
    true_peak: f64,
    momentary: f64,
    short_term: f64,
}

fn reference(l: &[f32], r: &[f32], sr: u32) -> Reference {
    let mut m = EbuR128::new(2, sr, Mode::I | Mode::LRA | Mode::TRUE_PEAK | Mode::S).unwrap();
    m.add_frames_planar_f32(&[l, r]).unwrap();
    let tp = m.true_peak(0).unwrap().max(m.true_peak(1).unwrap());
    Reference {
        integrated: m.loudness_global().unwrap(),
        lra: m.loudness_range().unwrap(),
        true_peak: 20.0 * tp.log10(),
        momentary: m.loudness_momentary().unwrap(),
        short_term: m.loudness_shortterm().unwrap(),
    }
}

fn ours(l: &[f32], r: &[f32], sr: f32, block: usize) -> LoudnessMeter {
    let mut m = LoudnessMeter::new(sr);
    let mut i = 0;
    while i < l.len() {
        let e = (i + block).min(l.len());
        m.process(&l[i..e], &r[i..e]);
        i = e;
    }
    m
}

fn compare(name: &str, l: &[f32], r: &[f32], sr: f32) {
    let want = reference(l, r, sr as u32);
    let m = ours(l, r, sr, 333);
    let got = m.integrated_lufs();
    println!(
        "{name} @ {sr}: integrated ours {got:.4} ebur128 {:.4} (diff {:.4} LU); LRA ours {:.3} ebur128 {:.3}; TP ours {:.4} ebur128 {:.4}",
        want.integrated,
        got - want.integrated,
        m.loudness_range_lu(),
        want.lra,
        true_peak_dbtp(l, r, sr),
        want.true_peak
    );
    assert!(
        (got - want.integrated).abs() < 0.1,
        "{name}: integrated {got} vs ebur128 {}",
        want.integrated
    );
    assert!(
        (m.loudness_range_lu() - want.lra).abs() < 0.1,
        "{name}: LRA {} vs ebur128 {}",
        m.loudness_range_lu(),
        want.lra
    );
    // signals end on a sub-block boundary in these tests, so the windows line up exactly
    assert!(
        (m.momentary_lufs() - want.momentary).abs() < 0.1,
        "{name} momentary"
    );
    assert!(
        (m.short_term_lufs() - want.short_term).abs() < 0.1,
        "{name} short-term"
    );
    assert!(
        (true_peak_dbtp(l, r, sr) - want.true_peak).abs() < 0.05,
        "{name}: true peak"
    );
    assert!((integrated_lufs(l, r, sr) - got).abs() < 1e-9);
}

#[test]
fn integrated_matches_ebur128_on_pink_noise() {
    for sr in [48000.0f32, 44100.0] {
        let n = (sr * 12.0) as usize;
        compare("pink", &pink(5, 0.5, n), &pink(6, 0.5, n), sr);
    }
}

#[test]
fn integrated_matches_ebur128_on_sine() {
    let n = (SR * 10.0) as usize;
    let s = sine(1000.0, 0.5, n, SR);
    compare("sine 1k", &s, &s, SR);
    // a 997 Hz sine at -20 dBFS peak in one channel
    let s = sine(997.0, 0.1, n, SR);
    let z = vec![0.0f32; n];
    compare("sine left only", &s, &z, SR);
    // EBU Tech 3341 sanity: 1 kHz sine at -23 dBFS peak in both channels reads -23 LUFS
    let a = 10f32.powf(-23.0 / 20.0);
    let s = sine(1000.0, a, (SR * 20.0) as usize, SR);
    let m = ours(&s, &s, SR, 512);
    assert!(
        (m.integrated_lufs() + 23.0).abs() < 0.1,
        "{}",
        m.integrated_lufs()
    );
}

#[test]
fn integrated_matches_ebur128_on_music() {
    let (l, r) = music(20.0, SR);
    compare("music", &l, &r, SR);
    let (l, r) = music(20.0, 44100.0);
    compare("music 44.1k", &l, &r, 44100.0);
}

#[test]
fn silence_and_reset() {
    let z = vec![0.0f32; 48000];
    let mut m = ours(&z, &z, SR, 1000);
    assert_eq!(m.integrated_lufs(), f64::NEG_INFINITY);
    assert_eq!(m.momentary_lufs(), f64::NEG_INFINITY);
    assert_eq!(true_peak_dbtp(&z, &z, SR), f64::NEG_INFINITY);
    assert_eq!(sample_peak_dbfs(&z, &z), f64::NEG_INFINITY);
    assert_eq!(rms_dbfs(&z, &z), f64::NEG_INFINITY);
    let s = sine(1000.0, 0.5, 48000, SR);
    m.process(&s, &s);
    assert!(m.integrated_lufs().is_finite());
    m.reset();
    assert_eq!(m.integrated_lufs(), f64::NEG_INFINITY);
    m.process(&s, &s);
    let fresh = ours(&s, &s, SR, 1000);
    assert_eq!(m.integrated_lufs(), fresh.integrated_lufs());
}
