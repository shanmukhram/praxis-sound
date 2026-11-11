//! `verify_render` end to end on synthetic renders of real Scores (no engine): every check of
//! docs/contracts/sound-profile.md passes on a correct render and fails on the matching defect.

mod features_synth;

use features_synth::*;
use praxis_analyze::verify::{Tolerances, verify_render};
use praxis_dsp::StereoBuffer;
use praxis_score::{GeneratorClass, Grid, Mode, PitchClass, Playback, Score};

fn eighth_loop(bpm: f64, bars: u32) -> Score {
    score(
        bpm,
        bars,
        key(PitchClass::A, Mode::Minor),
        Grid::Eighth,
        8,
        accented_eighths(&[57, 64, 60, 64, 69, 64, 60, 64]),
        GeneratorClass::Arp,
        Playback::Loop,
    )
}

fn mastered(score: &Score, opts: &RenderOpts) -> StereoBuffer {
    let mut b = render(score, SR, opts);
    normalize_lufs(&mut b, SR, score.master.loudness.target_lufs);
    b
}

#[test]
fn correct_loops_pass_every_check() {
    for bpm in [90.0, 110.0, 128.0, 140.0, 170.0] {
        let s = eighth_loop(bpm, 8);
        let b = mastered(&s, &RenderOpts::default());
        let r = verify_render(&s, &b, SR, &Tolerances::default());
        print_report(&format!("eighth loop at {bpm} bpm"), &r);
        assert!(r.passed(), "{bpm}");
        assert!((r.tempo_bpm.unwrap() - bpm).abs() <= 2.0);
        assert_eq!(r.key.as_deref(), Some("a minor"));
        assert!(r.onsets_on_grid.unwrap() >= 0.9);
        assert!(r.grid_deviation_ms.unwrap() < 5.0);
        assert!((r.integrated_lufs.unwrap() + 16.0).abs() < 0.05);
        assert!(r.seam_excess_db.unwrap() <= 0.0);
    }
}

#[test]
fn check_names_follow_the_contract() {
    let s = eighth_loop(128.0, 4);
    let b = mastered(&s, &RenderOpts::default());
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    let names: Vec<&str> = r.checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "tempo",
            "key",
            "grid",
            "loudness",
            "true_peak",
            "clipping",
            "loop_seam",
            "silence"
        ]
    );
    // One-shots have no seam check.
    let mut s = s;
    s.playback = Playback::OneShot { tail_seconds: 1.0 };
    let opts = RenderOpts {
        wrap: false,
        tail_s: 1.0,
        ..RenderOpts::default()
    };
    let b = mastered(&s, &opts);
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    print_report("one-shot", &r);
    assert!(r.checks.iter().all(|c| c.name != "loop_seam"));
    assert!(r.seam_jump_ratio.is_none());
    assert!(r.passed());
}

#[test]
fn octave_error_fails_tempo() {
    // A render at 128 bpm verified against a score that says 64 bpm.
    let s = eighth_loop(128.0, 8);
    let b = mastered(&s, &RenderOpts::default());
    let mut wrong = s.clone();
    wrong.tempo.bpm = 64.0;
    wrong.bars = 4;
    let r = verify_render(&wrong, &b, SR, &Tolerances::default());
    print_report("128 bpm render vs 64 bpm score", &r);
    // Heard with confidence 0.85: a failure, not "can't tell".
    let t = check(&r, "tempo");
    assert!(!t.passed && !t.inconclusive);
}

#[test]
fn wrong_and_relative_keys_fail() {
    let s = eighth_loop(110.0, 8);
    let b = mastered(&s, &RenderOpts::default());
    // Relative major of A minor.
    let mut c_major = s.clone();
    c_major.key = key(PitchClass::C, Mode::Major);
    let r = verify_render(&c_major, &b, SR, &Tolerances::default());
    print_report("a minor render vs c major score", &r);
    assert!(!check(&r, "key").passed);
    let mut d_minor = s.clone();
    d_minor.key = key(PitchClass::D, Mode::Minor);
    let r = verify_render(&d_minor, &b, SR, &Tolerances::default());
    assert!(!check(&r, "key").passed);
    // Same tonic, same family: a dorian is compared as a minor.
    let mut dorian = s;
    dorian.key = key(PitchClass::A, Mode::Dorian);
    let r = verify_render(&dorian, &b, SR, &Tolerances::default());
    assert!(check(&r, "key").passed);
}

#[test]
fn unpitched_scores_skip_key() {
    let s = score(
        128.0,
        4,
        key(PitchClass::C, Mode::Major),
        Grid::Eighth,
        8,
        accented_eighths(&[36, 42, 38, 42]),
        GeneratorClass::Drums,
        Playback::Loop,
    );
    let opts = RenderOpts {
        clicks: true,
        gain: 0.9,
        ..RenderOpts::default()
    };
    // Not normalised: bare clicks have a crest factor no limiter-free master would allow.
    let b = render(&s, SR, &opts);
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    print_report("click loop (drums only)", &r);
    let k = check(&r, "key");
    assert!(k.passed);
    assert!(k.expected.starts_with("n/a"));
    assert!(check(&r, "tempo").passed);
    assert!(check(&r, "grid").passed);
}

#[test]
fn off_grid_onsets_fail_grid() {
    let s = eighth_loop(128.0, 8);
    for (name, shift, jitter) in [
        ("every note 30 ms late", 0.03, 0.0),
        ("notes alternately 20 ms early/late", 0.0, 0.02),
    ] {
        let opts = RenderOpts {
            shift_s: shift,
            jitter_s: jitter,
            ..RenderOpts::default()
        };
        let b = mastered(&s, &opts);
        let r = verify_render(&s, &b, SR, &Tolerances::default());
        print_report(name, &r);
        assert!(!check(&r, "grid").passed, "{name}");
        assert!(r.onsets_on_grid.unwrap() < 0.5);
        assert!(r.grid_deviation_ms.unwrap() > 15.0);
    }
    // Light humanisation (+-4 ms) stays on the grid.
    let opts = RenderOpts {
        jitter_s: 0.004,
        ..RenderOpts::default()
    };
    let b = mastered(&s, &opts);
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    print_report("notes +-4 ms", &r);
    assert!(check(&r, "grid").passed);
}

#[test]
fn loudness_target_and_tolerance() {
    let s = eighth_loop(128.0, 8);
    let mut b = render(&s, SR, &RenderOpts::default());
    normalize_lufs(&mut b, SR, -16.0);
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    print_report("normalised to -16 LUFS", &r);
    assert!(check(&r, "loudness").passed);
    normalize_lufs(&mut b, SR, -20.0);
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    print_report("normalised to -20 LUFS", &r);
    assert!(!check(&r, "loudness").passed);
    assert!((r.integrated_lufs.unwrap() + 20.0).abs() < 0.05);
    normalize_lufs(&mut b, SR, -16.4);
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    assert!(check(&r, "loudness").passed, "0.4 LU off is inside 0.5 LU");
    normalize_lufs(&mut b, SR, -16.6);
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    assert!(
        !check(&r, "loudness").passed,
        "0.6 LU off is outside 0.5 LU"
    );
}

#[test]
fn clipping_and_true_peak() {
    let s = eighth_loop(128.0, 4);
    let mut b = render(&s, SR, &RenderOpts::default());
    normalize_lufs(&mut b, SR, -6.0);
    for x in b.left.iter_mut().chain(b.right.iter_mut()) {
        *x = x.clamp(-1.0, 1.0);
    }
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    print_report("clipped render", &r);
    assert!(!check(&r, "clipping").passed);
    assert!(!check(&r, "true_peak").passed);
    assert!(r.true_peak_dbtp.unwrap() >= 0.0);
    // A NaN fails clipping as well.
    let mut b = mastered(&s, &RenderOpts::default());
    b.left[100] = f32::NAN;
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    assert!(!check(&r, "clipping").passed);
}

#[test]
fn seam_defects_fail_loop_seam() {
    let s = score(
        128.0,
        4,
        key(PitchClass::A, Mode::Minor),
        Grid::Eighth,
        8,
        accented_eighths(&[57, 64, 60, 64, 69, 64, 60, 64]),
        GeneratorClass::Arp,
        Playback::Loop,
    );
    let wrapped = RenderOpts {
        decay_s: 0.6,
        ..RenderOpts::default()
    };
    let good = mastered(&s, &wrapped);
    let r = verify_render(&s, &good, SR, &Tolerances::default());
    print_report("seamless loop", &r);
    assert!(check(&r, "loop_seam").passed);

    let mut click = good.clone();
    let n = click.len();
    click.left[n - 1] += 0.2;
    click.right[n - 1] += 0.2;
    let r = verify_render(&s, &click, SR, &Tolerances::default());
    print_report("loop with a click at the wrap", &r);
    assert!(!check(&r, "loop_seam").passed);
    assert!(r.seam_excess_db.unwrap() > 0.0);

    let cut = mastered(
        &s,
        &RenderOpts {
            wrap: false,
            ..wrapped
        },
    );
    let r = verify_render(&s, &cut, SR, &Tolerances::default());
    print_report("loop with tails cut at the wrap", &r);
    assert!(!check(&r, "loop_seam").passed);
}

#[test]
fn silent_render_fails_silence() {
    let s = eighth_loop(128.0, 4);
    let n = praxis_score::timeline::score_length_samples(&s, SR) as usize;
    let r = verify_render(&s, &StereoBuffer::silent(n), SR, &Tolerances::default());
    print_report("silent render", &r);
    assert!(!check(&r, "silence").passed);
    assert!(!check(&r, "loudness").passed);
    assert!(!r.passed());
    // Half the bars missing.
    let mut b = mastered(&s, &RenderOpts::default());
    for i in n / 2..n {
        b.left[i] = 0.0;
        b.right[i] = 0.0;
    }
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    assert!(!check(&r, "silence").passed);
}

#[test]
fn empty_inputs_do_not_panic() {
    let s = eighth_loop(128.0, 4);
    let r = verify_render(&s, &StereoBuffer::default(), SR, &Tolerances::default());
    assert!(!r.passed());
    let r = verify_render(&s, &StereoBuffer::silent(10), 0, &Tolerances::default());
    assert!(!r.passed());
    // An empty score over a silent render (fixtures/empty.score.json): nothing to measure, so
    // every musical check is n/a and the report passes.
    let e = Score::empty(2, 120.0);
    let n = praxis_score::timeline::score_length_samples(&e, SR) as usize;
    let r = verify_render(
        &e,
        &StereoBuffer::silent(n + SR as usize * 4),
        SR,
        &Tolerances::default(),
    );
    print_report("empty score, silent render", &r);
    assert!(r.passed());
    assert!(check(&r, "loudness").expected.starts_with("n/a"));
}

#[test]
fn verification_is_deterministic() {
    let s = eighth_loop(140.0, 4);
    let b = mastered(&s, &RenderOpts::default());
    let a = verify_render(&s, &b, SR, &Tolerances::default());
    let c = verify_render(&s, &b, SR, &Tolerances::default());
    assert_eq!(a, c);
}
