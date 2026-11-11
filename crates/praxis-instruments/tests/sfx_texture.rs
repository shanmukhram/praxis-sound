//! Sound-effect texture (WP-H2): model-free measurements of what separates a recorded effect
//! from a plain oscillator patch, on real renders through the `Instrument` path. Each preset
//! has dynamics (a crest factor above a steady oscillator's), a stereo image (left and right are
//! not copies: the room and decorrelated layers), and takes that differ by seed but keep their
//! shape; the drone's partials move against each other and it breathes between its harmonics
//! instead of holding a static hum.

mod synth_common;

use praxis_score::InstrumentKind;
use synth_common::*;

const SFX: InstrumentKind = InstrumentKind::Sfx;
const PRESETS: [&str; 6] = ["whoosh", "impact", "riser", "drone", "glitch", "tick"];
const A3: u8 = 57;
const SR_F: f64 = SR as f64;

fn render_one(preset: &str, seed: u64, len: usize) -> (Vec<f32>, Vec<f32>) {
    let mut inst = create(SFX, preset, &[]);
    let tail = inst.tail_samples();
    let mut n = note(1, 0, A3, 0.8, len);
    n.on.seed = seed;
    render(inst.as_mut(), &[n], len + tail + 2400)
}

fn note_len(preset: &str) -> usize {
    match preset {
        "riser" | "drone" => 96_000,
        _ => 2400,
    }
}

/// Peak over RMS of the sounding part, dB.
fn crest_db(x: &[f32]) -> f64 {
    let end = sound_end(x);
    let start = x.iter().position(|v| *v != 0.0).unwrap_or(0);
    let s = &x[start..end];
    20.0 * (peak(s) as f64 / rms(s).max(1e-12) as f64).log10()
}

/// Correlation of left and right over the sounding part.
fn lr_correlation(l: &[f32], r: &[f32]) -> f64 {
    let end = sound_end(l).max(sound_end(r));
    let (mut lr, mut ll, mut rr) = (0.0, 0.0, 0.0);
    for i in 0..end {
        let (a, b) = (l[i] as f64, r[i] as f64);
        lr += a * b;
        ll += a * a;
        rr += b * b;
    }
    lr / (ll * rr).sqrt().max(1e-30)
}

/// Magnitude of `x` at `hz` (Hann-windowed DTFT).
fn magnitude(x: &[f32], hz: f64) -> f64 {
    let n = x.len();
    let w = std::f64::consts::TAU * hz / SR_F;
    let (mut re, mut im) = (0.0, 0.0);
    for (i, v) in x.iter().enumerate() {
        let win = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
        let v = *v as f64 * win;
        re += v * (w * i as f64).cos();
        im -= v * (w * i as f64).sin();
    }
    (re * re + im * im).sqrt()
}

#[test]
fn every_sfx_preset_has_dynamics_and_a_stereo_image() {
    for p in PRESETS {
        let (l, r) = render_one(p, 0x5EED, note_len(p));
        let crest = crest_db(&mono(&(l.clone(), r.clone())));
        let corr = lr_correlation(&l, &r);
        eprintln!("{p:<7} crest {crest:.1} dB, left/right correlation {corr:.3}");
        // A steady oscillator is ~3 dB; the old WP-H glitch measured 5 dB.
        assert!(
            crest > 8.0,
            "{p}: crest factor {crest} dB (a steady oscillator)"
        );
        assert!(corr < 0.98, "{p}: left and right are copies ({corr})");
    }
}

#[test]
fn takes_differ_by_seed_but_keep_their_level() {
    for p in PRESETS {
        let takes: Vec<Vec<f32>> = (0..4)
            .map(|s| mono(&render_one(p, 100 + s, note_len(p))))
            .collect();
        let levels: Vec<f64> = takes.iter().map(|x| rms(x) as f64).collect();
        let lo = levels.iter().cloned().fold(f64::INFINITY, f64::min);
        let hi = levels.iter().cloned().fold(0.0, f64::max);
        eprintln!("{p:<7} take levels {levels:.4?}");
        // A glitch's takes differ in pattern (how many gaps and stutters), so their levels
        // spread more; the others are one gesture each.
        let limit = if p == "glitch" { 3.5 } else { 1.6 };
        assert!(hi / lo < limit, "{p}: take levels {lo}..{hi}");
        for w in takes.windows(2) {
            assert_ne!(w[0], w[1], "{p}: two seeds gave the same take");
        }
    }
}

#[test]
fn drone_partials_drift_and_it_breathes_between_harmonics() {
    // An 8 s drone on A3: second and third harmonics measured in 250 ms frames after the attack.
    let len = 8 * SR as usize;
    let x = mono(&render_one("drone", 7, len));
    let f0 = midi_hz(A3);
    let frame = (0.25 * SR_F) as usize;
    let mut range = Vec::new();
    for h in [2.0, 3.0] {
        let mags: Vec<f64> = (8..32)
            .map(|k| magnitude(&x[k * frame..(k + 1) * frame], h * f0))
            .collect();
        let lo = mags.iter().cloned().fold(f64::INFINITY, f64::min);
        let hi = mags.iter().cloned().fold(0.0, f64::max);
        range.push(20.0 * (hi / lo.max(1e-12)).log10());
    }
    // Breath: energy halfway between harmonics relative to the harmonics themselves.
    let seg = &x[2 * SR as usize..len];
    let on: f64 = (1..=4).map(|h| magnitude(seg, h as f64 * f0)).sum();
    let between: f64 = (1..=4).map(|h| magnitude(seg, (h as f64 + 0.5) * f0)).sum();
    let breath_db = 20.0 * (between / on.max(1e-12)).log10();
    eprintln!(
        "drone: harmonic 2 moves {:.1} dB, harmonic 3 {:.1} dB; between-harmonic level {breath_db:.1} dB",
        range[0], range[1]
    );
    assert!(
        range.iter().all(|r| *r > 3.0),
        "drone partials static: {range:?}"
    );
    // The WP-H drone measured -50 dB here (a clean harmonic stack: the hum).
    assert!(
        breath_db > -40.0,
        "drone has no breath between harmonics: {breath_db} dB"
    );
}
