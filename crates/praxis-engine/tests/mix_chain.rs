//! Layer effect chains from `Effect` specs (FR-23) with the real `praxis_dsp::fx` processors,
//! effect parameter automation (FR-24), and the determinism golden of the mixed acceptance score.

mod common;

use common::*;
use praxis_engine::RenderOptions;
use praxis_score::timeline::TimeMap;
use praxis_score::*;
use praxis_testkit::{Outcome, Tolerance, audio_hash, check};

fn raw(stems: bool) -> RenderOptions {
    RenderOptions {
        master: false,
        ..opts(stems)
    }
}

fn effect(id: &str, kind: EffectKind, params: &[(&str, f64)]) -> Effect {
    Effect {
        id: id.into(),
        kind,
        params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        bypass: false,
        sidechain: None,
        impulse: None,
    }
}

fn automate(score: &mut Score, layer: &str, target: &str, curve: Curve, pts: &[(f64, f64)]) {
    let l = score.layers.iter_mut().find(|l| l.id == layer).unwrap();
    l.automation.push(Automation {
        target: target.into(),
        curve,
        points: pts
            .iter()
            .map(|&(bar, value)| AutoPoint { bar, value })
            .collect(),
    });
}

fn piano_only() -> Score {
    let mut s = bell_score();
    s.layers.retain(|l| l.id == "piano");
    s
}

#[test]
fn the_piano_room_reverb_renders() {
    // The fixture's piano carries `room` (reverb, decay 2.4 s, mix 0.2): it must be built and
    // audible, with no warning about effects.
    let score = piano_only();
    let wet = render(&score, &raw(true));
    assert!(
        wet.report.warnings.iter().all(|w| !w.contains("skipped")),
        "{:?}",
        wet.report.warnings
    );
    let mut dry_score = score.clone();
    dry_score.layers[0].effects[0].bypass = true;
    let dry = render(&dry_score, &raw(true));
    let (w, d) = (stem(&wet, "piano"), stem(&dry, "piano"));
    assert!(!buffers_equal(w, d));
    // The reverb decorrelates the channels; the dry centred piano is mono.
    assert!(bits_equal(&d.left, &d.right));
    assert!(!bits_equal(&w.left, &w.right));
    // Wet energy between the notes: the residual (wet - dry) carries clear energy.
    let resid: Vec<f32> = w.left.iter().zip(&d.left).map(|(a, b)| a - b).collect();
    let ratio_db = 20.0 * (rms(&resid) / rms(&d.left)).log10();
    println!("piano room reverb: residual {ratio_db:.1} dB relative to the dry stem");
    assert!(ratio_db > -30.0 && ratio_db < 0.0, "{ratio_db}");
}

#[test]
fn every_effect_kind_renders_in_a_layer_chain() {
    let kinds = [
        effect("eq", EffectKind::Eq, &[("low_db", 3.0)]),
        effect("filter", EffectKind::Filter, &[("cutoff_hz", 3000.0)]),
        effect("comp", EffectKind::Compressor, &[("threshold_db", -30.0)]),
        effect("sat", EffectKind::Saturation, &[("drive_db", 12.0)]),
        effect("delay", EffectKind::Delay, &[("feedback", 0.4)]),
        effect("verb", EffectKind::Reverb, &[]),
        Effect {
            impulse: Some("room_small".into()),
            ..effect("conv", EffectKind::ConvolutionReverb, &[])
        },
        effect("chorus", EffectKind::Chorus, &[]),
        effect("width", EffectKind::Width, &[("width", 1.5)]),
        effect("lim", EffectKind::Limiter, &[("ceiling_dbtp", -6.0)]),
        effect("gain", EffectKind::Gain, &[("gain_db", -3.0)]),
    ];
    let base = render(&piano_only(), &raw(true));
    for fx in kinds {
        let mut s = piano_only();
        s.layers[0].effects = vec![fx.clone()];
        let out = render(&s, &raw(true));
        assert!(out.report.warnings.is_empty(), "{:?}", out.report.warnings);
        let x = stem(&out, "piano");
        assert!(x.left.iter().all(|v| v.is_finite()));
        assert!(rms(&x.left) > 1e-3, "{} silent", fx.id);
        assert!(
            !buffers_equal(x, stem(&base, "piano")),
            "{} had no effect",
            fx.id
        );
    }
    // An unknown impulse response is a warning and the effect is skipped.
    let mut s = piano_only();
    s.layers[0].effects = vec![Effect {
        impulse: Some("cathedral".into()),
        ..effect("conv", EffectKind::ConvolutionReverb, &[])
    }];
    let out = render(&s, &raw(true));
    assert!(
        out.report
            .warnings
            .iter()
            .any(|w| w.contains("`conv` (convolution_reverb) skipped")),
        "{:?}",
        out.report.warnings
    );
    let mut dry = piano_only();
    dry.layers[0].effects.clear();
    assert!(buffers_equal(
        stem(&out, "piano"),
        stem(&render(&dry, &raw(true)), "piano")
    ));
}

#[test]
fn effect_parameter_automation_reaches_the_processor() {
    let score = piano_only();
    let tm = TimeMap::new(&score, SR);
    let bar = |b: u32| tm.bar_start_sample(b) as usize;
    let mut dry = score.clone();
    dry.layers[0].effects.clear();
    let dry = render(&dry, &raw(true));
    let base = render(&score, &raw(true));

    // Reverb mix: 0 (dry) in bars 1-8, fully wet from bar 9 (step).
    let mut auto = score.clone();
    automate(
        &mut auto,
        "piano",
        "effects.room.mix",
        Curve::Step,
        &[(1.0, 0.0), (9.0, 1.0)],
    );
    let a = render(&auto, &raw(true));
    assert!(a.report.warnings.is_empty(), "{:?}", a.report.warnings);
    let (x, d, b) = (stem(&a, "piano"), stem(&dry, "piano"), stem(&base, "piano"));
    let diff = |p: &[f32], q: &[f32]| {
        let r: Vec<f32> = p.iter().zip(q).map(|(u, v)| u - v).collect();
        rms(&r)
    };
    // Bars 4-7 (after the wrapped tail of the wet bars has died down): close to dry.
    let quiet = diff(&x.left[bar(4)..bar(8)], &d.left[bar(4)..bar(8)]);
    let wet = diff(&x.left[bar(11)..bar(15)], &d.left[bar(11)..bar(15)]);
    let fixture = diff(&b.left[bar(11)..bar(15)], &d.left[bar(11)..bar(15)]);
    println!(
        "reverb mix automation: |x-dry| bars 4-7 {quiet:.2e}, bars 11-14 {wet:.2e} (mix 0.2: {fixture:.2e})"
    );
    assert!(quiet < wet * 0.05);
    assert!(wet > fixture * 2.0);

    // (Out-of-range automation values are rejected by the validator; the engine clamps anyway.)

    // A filter cutoff step from fully open to 150 Hz at bar 9 darkens the piano.
    let mut sweep = score.clone();
    sweep.layers[0]
        .effects
        .push(effect("lp", EffectKind::Filter, &[("cutoff_hz", 20000.0)]));
    automate(
        &mut sweep,
        "piano",
        "effects.lp.cutoff_hz",
        Curve::Step,
        &[(1.0, 20000.0), (9.0, 150.0)],
    );
    let s = render(&sweep, &raw(true));
    let p = stem(&s, "piano");
    let r = |b0: u32, b1: u32| rms(&p.left[bar(b0)..bar(b1)]) / rms(&b.left[bar(b0)..bar(b1)]);
    println!(
        "cutoff step: level ratio bars 2-3 {:.3}, bars 11-15 {:.3}",
        r(2, 3),
        r(11, 15)
    );
    assert!(r(2, 3) > 0.9);
    assert!(r(11, 15) < 0.7);
    // From bar 11 on it matches a static 150 Hz filter (the automation value reached it).
    let mut fixed = score.clone();
    fixed.layers[0]
        .effects
        .push(effect("lp", EffectKind::Filter, &[("cutoff_hz", 150.0)]));
    let f = render(&fixed, &raw(true));
    let fs = stem(&f, "piano");
    let m = rms(&p.left[bar(11)..bar(15)]) / rms(&fs.left[bar(11)..bar(15)]);
    assert!((m - 1.0).abs() < 0.01, "{m}");

    // Automating a bypassed (not built) effect: a warning, nothing else changes. (Unknown
    // effects and parameters are rejected by the validator before rendering.)
    let mut off = score.clone();
    off.layers[0].effects[0].bypass = true;
    let off_plain = render(&off, &raw(true));
    automate(
        &mut off,
        "piano",
        "effects.room.mix",
        Curve::Step,
        &[(1.0, 1.0)],
    );
    let o = render(&off, &raw(true));
    assert!(
        o.report
            .warnings
            .iter()
            .any(|w| w.contains("automation target `effects.room.mix` skipped")),
        "{:?}",
        o.report.warnings
    );
    assert!(buffers_equal(stem(&o, "piano"), stem(&off_plain, "piano")));
}

#[test]
fn mixed_acceptance1_bell_is_deterministic_and_matches_golden() {
    let score = bell_score();
    let a = render(&score, &opts(true));
    let b = render(&score, &opts(true));
    assert_eq!(audio_hash(&a.master), audio_hash(&b.master));
    for (x, y) in a.stems.iter().zip(&b.stems) {
        assert_eq!(audio_hash(&x.audio), audio_hash(&y.audio));
    }
    let r = &a.report;
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    println!(
        "mix_acceptance1_bell: before {:.2} LUFS, after {:.2} LUFS, true peak {:.2} dBTP, gain {:.2} dB, warm-up {}",
        r.loudness_before_lufs,
        r.loudness_lufs,
        r.true_peak_dbtp,
        r.applied_gain_db,
        r.warmup_cycles
    );
    assert!((r.loudness_lufs - score.master.loudness.target_lufs).abs() <= 0.5);
    assert!(r.true_peak_dbtp <= score.master.loudness.true_peak_dbtp + 0.05);
    let outcome = check("mix_acceptance1_bell", &a.master, SR, &Tolerance::default());
    assert!(matches!(
        outcome,
        Outcome::Identical | Outcome::FeaturesMatch | Outcome::Blessed
    ));
}
