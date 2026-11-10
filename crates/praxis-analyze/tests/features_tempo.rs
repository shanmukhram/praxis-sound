//! Tempo, beats and onsets on synthetic eighth-grid material at known tempos.

mod features_synth;

use features_synth::*;
use praxis_analyze::features::mono;
use praxis_analyze::features::onset::detect_onsets;
use praxis_analyze::features::tempo::{TempoConfig, analyze, estimate_tempo};
use praxis_score::{GeneratorClass, Grid, Mode, PitchClass, Playback};

const BPMS: [f64; 5] = [90.0, 110.0, 128.0, 140.0, 170.0];

fn eighth_score(bpm: f64, class: GeneratorClass) -> praxis_score::Score {
    score(
        bpm,
        8,
        key(PitchClass::A, Mode::Minor),
        Grid::Eighth,
        8,
        accented_eighths(&[57, 64, 60, 64, 69, 64, 60, 64]),
        class,
        Playback::Loop,
    )
}

fn detected_bpm(buf: &praxis_dsp::StereoBuffer) -> (f64, f32) {
    let m = mono(&buf.left, &buf.right);
    let (env, _) = detect_onsets(&m, SR);
    let t = estimate_tempo(&env, &TempoConfig::default()).expect("tempo");
    (t.bpm, t.confidence)
}

#[test]
fn tone_pulses_at_known_tempos() {
    for bpm in BPMS {
        let s = eighth_score(bpm, GeneratorClass::Arp);
        let buf = render(&s, SR, &RenderOpts::default());
        let (got, conf) = detected_bpm(&buf);
        println!("tone pulses {bpm:>5.1} bpm -> {got:.2} bpm (confidence {conf:.2})");
        assert!((got - bpm).abs() <= 2.0, "{bpm}: got {got}");
    }
}

#[test]
fn click_tracks_at_known_tempos() {
    for bpm in BPMS {
        let s = eighth_score(bpm, GeneratorClass::Percussion);
        let opts = RenderOpts {
            clicks: true,
            gain: 0.5,
            ..RenderOpts::default()
        };
        let buf = render(&s, SR, &opts);
        let (got, conf) = detected_bpm(&buf);
        println!("click track {bpm:>5.1} bpm -> {got:.2} bpm (confidence {conf:.2})");
        assert!((got - bpm).abs() <= 2.0, "{bpm}: got {got}");
    }
}

#[test]
fn onsets_are_placed_within_a_few_ms() {
    let bpm = 128.0;
    let s = eighth_score(bpm, GeneratorClass::Arp);
    let buf = render(&s, SR, &RenderOpts::default());
    let m = mono(&buf.left, &buf.right);
    let (_, onsets) = detect_onsets(&m, SR);
    let step = 60.0 / bpm / 2.0;
    let mut worst = 0.0f64;
    let mut sum = 0.0;
    for o in &onsets {
        let d = (o.time_s - (o.time_s / step).round() * step).abs() * 1000.0;
        worst = worst.max(d);
        sum += d;
    }
    let expected = 8 * 8;
    println!(
        "onsets: {} detected of {expected}, mean |error| {:.2} ms, worst {:.2} ms",
        onsets.len(),
        sum / onsets.len() as f64,
        worst
    );
    assert!(onsets.len() >= expected - 1 && onsets.len() <= expected + 1);
    assert!(sum / (onsets.len() as f64) < 3.0, "mean onset error");
    assert!(worst < 5.0, "worst onset error {worst} ms");
}

#[test]
fn beats_and_downbeat() {
    let bpm = 110.0;
    let s = eighth_score(bpm, GeneratorClass::Arp);
    let buf = render(&s, SR, &RenderOpts::default());
    let m = mono(&buf.left, &buf.right);
    let t = analyze(&m, SR, 4).expect("tempo");
    let period = 60.0 / bpm;
    let gaps: Vec<f64> = t.beats_s.windows(2).map(|w| w[1] - w[0]).collect();
    let worst = gaps
        .iter()
        .map(|g| (g - period).abs())
        .fold(0.0f64, f64::max);
    let bar = 4.0 * period;
    let db_phase = (t.downbeat_s / bar - (t.downbeat_s / bar).round()).abs() * bar;
    println!(
        "beats: {} beats, worst gap error {:.1} ms, downbeat at {:.3} s ({:.1} ms from a bar line)",
        t.beats_s.len(),
        worst * 1000.0,
        t.downbeat_s,
        db_phase * 1000.0
    );
    assert!(t.beats_s.len() >= 28);
    assert!(worst < 0.02);
    assert!(db_phase < 0.02);
}

#[test]
fn tempo_is_deterministic() {
    let s = eighth_score(140.0, GeneratorClass::Arp);
    let buf = render(&s, SR, &RenderOpts::default());
    assert_eq!(detected_bpm(&buf), detected_bpm(&buf));
}
