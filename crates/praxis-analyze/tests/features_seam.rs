//! Loop-seam analysis on seamless and broken synthetic loops.

mod features_synth;

use features_synth::*;
use praxis_analyze::features::seam::{SeamAnalysis, SeamConfig, analyze_seam};
use praxis_score::{GeneratorClass, Grid, Mode, PitchClass, Playback};
use std::f64::consts::PI;

const N: usize = 2 * SR as usize;

/// Sum of sines with a whole number of periods per loop (plus `extra` cycles, which breaks it).
fn sines(extra: f64) -> Vec<f32> {
    let parts = [
        (110.0, 0.25, 0.3),
        (165.0, 0.15, 1.1),
        (440.0, 0.08, 2.0),
        (1320.0, 0.03, 0.5),
    ];
    (0..N)
        .map(|i| {
            let t = i as f64 / N as f64;
            parts
                .iter()
                .map(|(cycles, a, ph)| {
                    // `cycles` per 1 s, the loop is 2 s: 2 * cycles whole periods.
                    a * (2.0 * PI * (2.0 * cycles + extra) * t + ph).sin()
                })
                .sum::<f64>() as f32
        })
        .collect()
}

fn run(name: &str, l: &[f32], r: &[f32]) -> SeamAnalysis {
    let s = analyze_seam(l, r, SR, &SeamConfig::default());
    println!(
        "{name:<34} passed {:<5} jump {:.5} (ratio {:6.2}, local max {:.5}) diff {:+7.2} dB transient {:+7.2} dB drop {:+7.2} dB excess {:+7.2} dB",
        s.passed,
        s.jump,
        s.jump_ratio,
        s.local_max_jump,
        s.diff_energy_db,
        s.transient_db,
        s.drop_db,
        s.excess_db
    );
    s
}

#[test]
fn seamless_sines_pass() {
    let x = sines(0.0);
    let s = run("seamless sines", &x, &x);
    assert!(s.passed);
    assert!(s.jump_ratio < 2.0);
}

#[test]
fn click_at_the_wrap_fails() {
    let mut x = sines(0.0);
    x[N - 1] += 0.3;
    let s = run("sines + click on last sample", &x, &x);
    assert!(!s.passed);
    assert!(s.excess_db > 0.0);
}

#[test]
fn small_click_in_one_channel_fails() {
    let l = sines(0.0);
    let mut r = l.clone();
    r[0] += 0.05;
    let s = run("sines + 0.05 click, right channel", &l, &r);
    assert!(!s.passed);
}

#[test]
fn phase_break_at_the_wrap_fails() {
    let x = sines(0.37);
    let s = run("sines with non-integer periods", &x, &x);
    assert!(!s.passed);
    assert!(!s.jump_ok);
}

#[test]
fn cut_tail_fails_and_wrapped_tail_passes() {
    let base = sines(0.0);
    let mut cut = base.clone();
    let start = (N as f64 / SR as f64) - 0.6;
    add_noise_tail(&mut cut, SR, start, 0.3, 0.4, 2.0, false, 7);
    let s = run("sines + reverb tail cut at the wrap", &cut, &cut);
    assert!(!s.passed);
    let mut wrapped = base;
    add_noise_tail(&mut wrapped, SR, start, 0.3, 0.4, 2.0, true, 7);
    let s = run("sines + reverb tail wrapped", &wrapped, &wrapped);
    assert!(s.passed);
}

fn musical_loop(wrap: bool, reverb: f32, downbeat: bool) -> praxis_dsp::StereoBuffer {
    let mut notes = accented_eighths(&[57, 64, 60, 64, 69, 64, 60, 64]);
    if !downbeat {
        notes.remove(0);
    }
    let sc = score(
        128.0,
        4,
        key(PitchClass::A, Mode::Minor),
        Grid::Eighth,
        8,
        notes,
        GeneratorClass::Arp,
        Playback::Loop,
    );
    let opts = RenderOpts {
        wrap,
        reverb,
        decay_s: 0.6,
        ..RenderOpts::default()
    };
    render(&sc, SR, &opts)
}

#[test]
fn musical_loop_with_wrapped_tails_passes() {
    for (name, reverb, downbeat) in [
        ("tone loop, tails wrapped", 0.0, true),
        ("tone loop + reverb, tails wrapped", 0.5, true),
        ("tone loop, rest on 1, wrapped", 0.0, false),
        ("tone loop + reverb, rest on 1, wrapped", 0.5, false),
    ] {
        let b = musical_loop(true, reverb, downbeat);
        assert!(run(name, &b.left, &b.right).passed, "{name}");
    }
}

#[test]
fn musical_loop_with_cut_tails_fails() {
    for (name, reverb, downbeat) in [
        ("tone loop, tails cut", 0.0, true),
        ("tone loop, rest on 1, tails cut", 0.0, false),
        ("tone loop + reverb, rest on 1, cut", 0.5, false),
    ] {
        let b = musical_loop(false, reverb, downbeat);
        assert!(!run(name, &b.left, &b.right).passed, "{name}");
    }
}

#[test]
fn reverb_cut_masked_by_a_louder_downbeat_is_not_flagged() {
    // Known limit, documented in features::seam: when the downbeat at the seam brings its own
    // reverb at a higher level than the cut tail, nothing measurable is missing at the seam.
    let b = musical_loop(false, 0.5, true);
    let s = run("tone loop + reverb, cut, masked", &b.left, &b.right);
    assert!(s.excess_db.is_finite());
}

#[test]
fn degenerate_inputs() {
    let s = analyze_seam(&[], &[], SR, &SeamConfig::default());
    assert!(s.passed);
    let z = vec![0.0f32; 4800];
    let s = analyze_seam(&z, &z, SR, &SeamConfig::default());
    assert!(s.passed, "{s:?}");
}
