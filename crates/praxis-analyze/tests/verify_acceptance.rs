//! `verify_render` on the acceptance-1 Score (fixtures/acceptance1.score.json: 16 bars, 4/4,
//! A minor, 128 BPM loop, piano motif with a kick from bar 9), rendered by the test synthesiser
//! (additive tones, wrapped noise reverb), not the engine.

mod features_synth;

use features_synth::*;
use praxis_analyze::verify::{Tolerances, verify_render};
use praxis_score::Score;

fn fixture(json: &str) -> Score {
    praxis_score::from_json(json).expect("fixture is a valid score")
}

fn acceptance() -> Score {
    fixture(include_str!("../../../fixtures/acceptance1.score.json"))
}

fn piano_opts() -> RenderOpts {
    RenderOpts {
        decay_s: 0.8,
        drum_decay_s: 0.12,
        reverb: 0.3,
        ..RenderOpts::default()
    }
}

#[test]
fn acceptance_1_render_passes() {
    for (name, s) in [
        ("acceptance1", acceptance()),
        (
            "acceptance1_bell",
            fixture(include_str!(
                "../../../fixtures/acceptance1_bell.score.json"
            )),
        ),
    ] {
        let mut b = render(&s, SR, &piano_opts());
        normalize_lufs(&mut b, SR, -16.0);
        let r = verify_render(&s, &b, SR, &Tolerances::default());
        print_report(name, &r);
        assert!(r.passed(), "{name}");
        // Every check decided: nothing in acceptance 1 is "can't tell".
        assert_eq!(r.inconclusive().count(), 0, "{name}");
        assert_eq!(r.summary(), "8 of 8 checks passed", "{name}");
        assert!((r.tempo_bpm.unwrap() - 128.0).abs() < 0.5);
        assert_eq!(r.key.as_deref(), Some("a minor"));
    }
}

#[test]
fn acceptance_1_at_44100() {
    let s = acceptance();
    let mut b = render(&s, 44_100, &piano_opts());
    normalize_lufs(&mut b, 44_100, -16.0);
    let r = verify_render(&s, &b, 44_100, &Tolerances::default());
    print_report("acceptance1 at 44.1 kHz", &r);
    assert!(r.passed());
}

#[test]
fn acceptance_1_defects_are_caught() {
    let s = acceptance();
    // Ringing piano and bass tails not wrapped (rendered as a one-shot and cut to the loop
    // length). With the test's dense noise reverb on, the downbeat's own reverb masks the cut;
    // see features_seam::reverb_cut_masked_by_a_louder_downbeat_is_not_flagged.
    let cut = RenderOpts {
        wrap: false,
        reverb: 0.0,
        ..piano_opts()
    };
    let mut b = render(&s, SR, &cut);
    normalize_lufs(&mut b, SR, -16.0);
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    print_report("acceptance1, tails cut at the wrap", &r);
    assert!(!check(&r, "loop_seam").passed);

    // Loud master: over the loudness target and the true-peak ceiling.
    let mut b = render(&s, SR, &piano_opts());
    normalize_lufs(&mut b, SR, -9.0);
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    print_report("acceptance1 at -9 LUFS", &r);
    assert!(!check(&r, "loudness").passed);

    // Rendered in the wrong key (transposed up a tone: B minor).
    let mut wrong = s.clone();
    for l in wrong.layers.iter_mut() {
        for c in l.clips.iter_mut() {
            c.transpose = 2;
        }
    }
    let mut b = render(&wrong, SR, &piano_opts());
    normalize_lufs(&mut b, SR, -16.0);
    let r = verify_render(&s, &b, SR, &Tolerances::default());
    print_report("acceptance1 rendered a tone too high", &r);
    let k = check(&r, "key");
    assert!(
        !k.passed && !k.inconclusive,
        "a clear wrong key is a failure: {k:?}"
    );
    assert_eq!(r.key.as_deref(), Some("b minor"));
}
