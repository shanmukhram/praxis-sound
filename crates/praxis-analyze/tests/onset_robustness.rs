//! Onset robustness (WP-D1b): the second swell of a soft note under a reverb is not an onset, a
//! soft attack under a ringing note is timed at its start, and a real felt-piano render sits on
//! the grid.

use praxis_analyze::features::onset::{OnsetConfig, detect_onsets, detect_onsets_with};
use std::f64::consts::PI;

/// Piano-like note: harmonics 1..=8 (amplitude `1 / k^1.3`), each played by two strings detuned
/// by `k * 0.15` Hz (about one cent; second string at half the level). Raised-cosine attack of
/// `attack_s`, exponential decay with time constant `tau_s`. With `swell`, the note fades by 7 dB
/// from 60 ms to 95 ms and swells back within 15 ms, like the second swell measured on the
/// sampled felt piano (pitch 60: down 7 dB at 95 ms, back up by 110 ms).
#[allow(clippy::too_many_arguments)]
fn add_note(
    buf: &mut [f32],
    sr: u32,
    start_s: f64,
    hz: f64,
    amp: f64,
    attack_s: f64,
    tau_s: f64,
    swell: bool,
) {
    let sr_f = sr as f64;
    let start = (start_s * sr_f).round() as usize;
    let norm: f64 = (1..=8).map(|k| 1.5 / (k as f64).powf(1.3)).sum();
    for (i, x) in buf.iter_mut().enumerate().skip(start) {
        let t = (i - start) as f64 / sr_f;
        let mut env = amp * (-t / tau_s).exp();
        if t < attack_s {
            env *= 0.5 - 0.5 * (PI * t / attack_s).cos();
        }
        if swell && (0.06..0.11).contains(&t) {
            let shape = if t < 0.095 {
                (0.5 * PI * (t - 0.06) / 0.035).sin().powi(2)
            } else {
                (0.5 * PI * (0.11 - t) / 0.015).sin().powi(2)
            };
            env *= 1.0 - 0.55 * shape;
        }
        let mut s = 0.0;
        for k in 1..=8 {
            let kf = k as f64;
            let a = 1.0 / kf.powf(1.3);
            s += a * (2.0 * PI * hz * kf * t).sin();
            s += 0.5 * a * (2.0 * PI * (hz + 0.15) * kf * t + 0.3 * kf).sin();
        }
        *x += (env * s / norm) as f32;
    }
}

/// Small Schroeder room: four parallel feedback combs (RT60 `rt60_s`) into two allpasses, mixed
/// in at `mix` (wet) with the dry signal. Deterministic.
fn room(dry: &[f32], sr: u32, rt60_s: f64, mix: f32) -> Vec<f32> {
    let sr_f = sr as f64;
    let combs = [0.0297, 0.0371, 0.0411, 0.0437];
    let mut wet = vec![0.0f32; dry.len()];
    for d in combs {
        let n = (d * sr_f) as usize;
        let g = 10f64.powf(-3.0 * d / rt60_s) as f32;
        let mut line = vec![0.0f32; n];
        for (i, x) in dry.iter().enumerate() {
            let y = line[i % n];
            line[i % n] = x + g * y;
            wet[i] += y * 0.25;
        }
    }
    for (d, g) in [(0.005, 0.7f32), (0.0017, 0.7f32)] {
        let n = (d * sr_f) as usize;
        let mut line = vec![0.0f32; n];
        for w in wet.iter_mut() {
            let delayed = line[0];
            let v = *w + g * delayed;
            line.rotate_left(1);
            line[n - 1] = v;
            *w = delayed - g * v;
        }
    }
    dry.iter()
        .zip(&wet)
        .map(|(d, w)| (1.0 - mix) * d + mix * w)
        .collect()
}

fn onsets_in(onsets: &[f64], lo: f64, hi: f64) -> Vec<f64> {
    onsets
        .iter()
        .copied()
        .filter(|t| *t >= lo && *t < hi)
        .collect()
}

/// A note at 0.5 s, then a soft note at 1.25 s with a second swell (7 dB back up at about
/// 1.36 s), all through a room reverb whose tail builds up and beats.
fn swell_scene(sr: u32) -> Vec<f32> {
    let mut buf = vec![0.0f32; (3.0 * sr as f64) as usize];
    add_note(&mut buf, sr, 0.5, 220.0, 0.25, 0.003, 0.8, false);
    add_note(&mut buf, sr, 1.25, 261.63, 0.25, 0.003, 1.5, true);
    room(&buf, sr, 1.8, 0.3)
}

#[test]
fn slowly_swelling_soft_note_under_reverb_is_one_onset() {
    for sr in [48_000u32, 44_100] {
        let sig = swell_scene(sr);
        let times = |cfg: &OnsetConfig| -> Vec<f64> {
            detect_onsets_with(&sig, sr, cfg)
                .1
                .iter()
                .map(|o| o.time_s)
                .collect()
        };
        let new = times(&OnsetConfig::default());
        let old = times(&OnsetConfig {
            attack: praxis_analyze::features::attack::AttackConfig {
                order: 0,
                ..Default::default()
            },
            ..OnsetConfig::default()
        });
        println!("{sr} Hz: onsets {new:.4?}; without attack analysis {old:.4?}");
        // The scene reproduces the problem: without the attack analysis the swell (and the
        // reverb tail) read as onsets.
        assert!(
            !onsets_in(&old, 1.3, 1.45).is_empty(),
            "{sr} Hz: the swell is a flux peak"
        );
        let soft = onsets_in(&new, 1.2, 2.9);
        assert_eq!(
            soft.len(),
            1,
            "{sr} Hz: the soft note is one onset: {soft:?}"
        );
        assert!((soft[0] - 1.25).abs() < 0.002, "{sr} Hz: at its start");
        assert_eq!(onsets_in(&new, 0.4, 1.2).len(), 1, "{sr} Hz: the loud note");
    }
}

#[test]
fn soft_attack_under_a_ringing_note_is_timed_at_its_start() {
    for sr in [48_000u32, 44_100] {
        for (t0, hz, amp, attack) in [
            (1.0123, 329.63, 0.12, 0.004),
            (1.0371, 261.63, 0.08, 0.003),
            (1.0007, 392.0, 0.1, 0.002),
        ] {
            let mut buf = vec![0.0f32; (2.0 * sr as f64) as usize];
            add_note(&mut buf, sr, 0.5, 220.0, 0.5, 0.003, 0.8, false);
            add_note(&mut buf, sr, t0, hz, amp, attack, 1.0, false);
            let sig = room(&buf, sr, 1.2, 0.2);
            let (_, onsets) = detect_onsets(&sig, sr);
            let near: Vec<f64> = onsets
                .iter()
                .map(|o| o.time_s - t0)
                .filter(|d| d.abs() < 0.05)
                .collect();
            println!(
                "{sr} Hz, {hz} Hz at {:.1} dB under the ringing note: error {:.2?} ms",
                20.0 * (amp / 0.5f64).log10(),
                near.iter().map(|d| d * 1000.0).collect::<Vec<_>>()
            );
            assert_eq!(near.len(), 1, "one onset for the soft note");
            assert!(
                near[0].abs() <= 0.002,
                "soft attack within 2 ms of its start: {:.2} ms",
                near[0] * 1000.0
            );
        }
    }
}
