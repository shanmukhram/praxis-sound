//! "Can't tell" checks (WP-D1c): tempo and key are inconclusive, not failed, when the material
//! cannot decide them, and still fail when it can. Renders by the test synthesiser (no engine).

mod features_synth;

use features_synth::*;
use praxis_analyze::verify::{Tolerances, VerifyReport, verify_render};
use praxis_score::{GeneratorClass, Grid, Mode, Pattern, PitchClass, Playback, Score};

/// The owner's hand-built loop (2026-11-11, take 1 of "storyloop"): 16 bars of A minor at
/// 128 BPM; a felt piano repeating A3 on the first eight eighths with D5 on the fourth and E4, B4
/// on steps 12-13 (only A, B, D and E: no third), and a soft kick on steps 0, 3, 4 and 5 of
/// every bar from bar 9.
fn owner_loop() -> Score {
    let piano = vec![
        note(0, 57, 0.8, 1.6),
        note(1, 57, 0.55, 1.6),
        note(2, 57, 0.65, 1.6),
        note(3, 57, 0.55, 1.6),
        note(3, 74, 0.6, 1.6),
        note(4, 57, 0.7, 1.6),
        note(5, 57, 0.55, 1.6),
        note(6, 57, 0.65, 1.6),
        note(7, 57, 0.55, 1.6),
        note(12, 64, 0.6, 1.6),
        note(13, 71, 0.6, 1.6),
    ];
    let mut s = score(
        128.0,
        16,
        key(PitchClass::A, Mode::Minor),
        Grid::Eighth,
        16,
        piano,
        GeneratorClass::Motif,
        Playback::Loop,
    );
    s.patterns.push(Pattern {
        id: "kick".to_string(),
        grid: Grid::Eighth,
        length_steps: 8,
        notes: [0, 3, 4, 5]
            .into_iter()
            .map(|st| note(st, 36, 0.62, 1.0))
            .collect(),
        spoken: None,
    });
    let mut kick = layer("kick", GeneratorClass::Drums, "kick", 16);
    kick.clips[0].bars.start = 9;
    s.layers.push(kick);
    s
}

fn piano_opts() -> RenderOpts {
    RenderOpts {
        decay_s: 0.8,
        drum_decay_s: 0.12,
        reverb: 0.3,
        ..RenderOpts::default()
    }
}

fn mastered(score: &Score, opts: &RenderOpts) -> praxis_dsp::StereoBuffer {
    let mut b = render(score, SR, opts);
    normalize_lufs(&mut b, SR, score.master.loudness.target_lufs);
    b
}

fn decided_passes(r: &VerifyReport) -> usize {
    r.checks
        .iter()
        .filter(|c| c.passed && !c.inconclusive)
        .count()
}

#[test]
fn owner_loop_without_a_third_cannot_tell_key() {
    let s = owner_loop();
    let b = mastered(&s, &piano_opts());
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    print_report("owner's hand-built loop", &r);
    println!("   summary: {}", r.summary());
    let k = check(&r, "key");
    assert!(k.inconclusive && !k.passed, "{k:?}");
    assert_eq!(
        k.cant_tell_reason(),
        Some("no third, so A minor vs A major can't be heard")
    );
    assert!(r.passed());
    for c in &r.checks {
        if c.name != "tempo" && c.name != "key" {
            assert!(c.passed && !c.inconclusive, "{c:?}");
        }
    }
}

#[test]
fn a_clear_third_in_the_wrong_mode_still_fails_key() {
    // The score says A minor but its piano plays C# (a major third over A): the render is in
    // A major, the notes can tell, and the detector is sure.
    let mut s = score(
        110.0,
        8,
        key(PitchClass::A, Mode::Minor),
        Grid::Eighth,
        8,
        accented_eighths(&[57, 64, 61, 64, 69, 64, 61, 64]),
        GeneratorClass::Arp,
        Playback::Loop,
    );
    let b = mastered(&s, &RenderOpts::default());
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    print_report("a major notes in an a minor score", &r);
    let k = check(&r, "key");
    assert!(!k.passed && !k.inconclusive, "{k:?}");
    assert!(!r.passed());
    // The same render against an A major score passes.
    s.key = key(PitchClass::A, Mode::Major);
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    assert!(check(&r, "key").passed);
}

#[test]
fn a_confident_wrong_tempo_still_fails() {
    let s = score(
        128.0,
        8,
        key(PitchClass::A, Mode::Minor),
        Grid::Eighth,
        8,
        accented_eighths(&[57, 64, 60, 64, 69, 64, 60, 64]),
        GeneratorClass::Arp,
        Playback::Loop,
    );
    let b = mastered(&s, &RenderOpts::default());
    // Unrelated tempo: 128 bpm audio checked against a 100 bpm score.
    let mut wrong = s.clone();
    wrong.tempo.bpm = 100.0;
    let r = verify_render(&wrong, &b, SR, &Tolerances::default());
    print_report("128 bpm render vs 100 bpm score", &r);
    let t = check(&r, "tempo");
    assert!(!t.passed && !t.inconclusive, "{t:?}");
    // A simple ratio (3/2) heard with confidence: still a failure.
    let mut wrong = s;
    wrong.tempo.bpm = 128.0 / 1.5;
    let r = verify_render(&wrong, &b, SR, &Tolerances::default());
    print_report("128 bpm render vs 85.33 bpm score", &r);
    let t = check(&r, "tempo");
    assert!(!t.passed && !t.inconclusive, "{t:?}");
    assert!(!r.passed());
}

#[test]
fn summary_counts_only_decided_checks() {
    let s = owner_loop();
    let b = mastered(&s, &piano_opts());
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    let unsure: Vec<&str> = r.inconclusive().map(|c| c.name.as_str()).collect();
    let total = r.decided().count();
    assert_eq!(total + unsure.len(), r.checks.len());
    assert_eq!(decided_passes(&r), total);
    assert!(
        r.summary()
            .starts_with(&format!("{total} of {total} checks passed \u{b7} "))
    );
    assert!(
        r.summary()
            .contains("key (no third, so A minor vs A major can't be heard)")
    );
    // JSON always carries the field.
    let v = serde_json::to_value(&r).unwrap();
    assert!(
        v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["inconclusive"].is_boolean())
    );
}
