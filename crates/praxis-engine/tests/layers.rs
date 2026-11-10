//! Layer selection (avoid-list, mute/solo, only_layers), one-shot length, gain/pan automation and
//! the frozen master gain.

mod common;

use common::*;
use praxis_engine::testing::TestFactory;
use praxis_engine::{
    Instrument, InstrumentContext, InstrumentError, InstrumentFactory, PresetInfo, RenderOptions,
    render_offline,
};
use praxis_score::timeline::TimeMap;
use praxis_score::*;

/// A factory whose `bell` declares itself a pad: the engine must see through the layer's class.
struct PadBellFactory;

impl InstrumentFactory for PadBellFactory {
    fn create(
        &self,
        spec: &InstrumentRef,
        ctx: &InstrumentContext,
    ) -> Result<Box<dyn Instrument>, InstrumentError> {
        TestFactory::new().create(spec, ctx)
    }
    fn describe(&self, kind: InstrumentKind, preset: &str) -> Option<PresetInfo> {
        let mut info = TestFactory::new().describe(kind, preset)?;
        if preset == "bell" {
            info.class = GeneratorClass::Pad;
        }
        Some(info)
    }
    fn presets(&self) -> Vec<PresetInfo> {
        TestFactory::new().presets()
    }
}

fn kick_only(score: &Score) -> Score {
    let mut s = score.clone();
    s.layers.retain(|l| l.id == "kick");
    s
}

#[test]
fn avoided_layer_class_is_blocked_by_the_engine() {
    let score = bell_score();
    let mut with_pad = score.clone();
    let mut pad = score.layer("piano").unwrap().clone();
    pad.id = "pad".into();
    pad.class = GeneratorClass::Pad;
    with_pad.layers.push(pad);
    // The validator flags it too, but the engine does not rely on that: it renders the rest.
    assert!(
        validate::validate(&with_pad)
            .errors
            .iter()
            .any(|e| e.code == validate::IssueCode::AvoidedClass)
    );
    let out = render(&with_pad, &opts(true));
    assert_eq!(out.report.blocked_layers, vec!["pad".to_string()]);
    assert!(out.stems.iter().all(|s| s.layer_id != "pad"));
    assert!(
        out.report
            .warnings
            .iter()
            .any(|w| w.contains("`pad` blocked"))
    );
    let plain = render(&score, &opts(true));
    assert!(buffers_equal(&out.master, &plain.master));
}

#[test]
fn preset_intrinsic_class_is_blocked_too() {
    let score = bell_score(); // avoid-list includes "pad"; the piano layer claims "motif"
    let out = render_offline(&score, &PadBellFactory, &opts(true)).unwrap();
    assert_eq!(out.report.blocked_layers, vec!["piano".to_string()]);
    assert_eq!(out.stems.len(), 1);
    assert_eq!(out.stems[0].layer_id, "kick");
    let kick = render(&kick_only(&score), &opts(false));
    assert!(buffers_equal(&out.master, &kick.master));
}

#[test]
fn mute_solo_and_only_layers() {
    let score = bell_score();
    let all = render(&score, &opts(true));

    let mut muted = score.clone();
    muted.layers[1].muted = true;
    let m = render(&muted, &opts(true));
    assert_eq!(m.stems.len(), 1);
    assert_eq!(m.stems[0].layer_id, "piano");
    assert!(buffers_equal(&m.master, stem(&all, "piano")));

    let mut solo = score.clone();
    solo.layers[1].solo = true;
    let s = render(&solo, &opts(true));
    assert_eq!(s.stems.len(), 1);
    assert!(buffers_equal(&s.master, stem(&all, "kick")));

    // only_layers is an explicit request: it renders the named layer even when muted.
    let o = RenderOptions {
        only_layers: vec!["kick".into()],
        ..opts(true)
    };
    let only = render(&muted, &o);
    assert_eq!(only.stems.len(), 1);
    assert!(buffers_equal(&only.master, stem(&all, "kick")));

    let o = RenderOptions {
        only_layers: vec!["nope".into()],
        ..opts(true)
    };
    let none = render(&score, &o);
    assert!(none.stems.is_empty());
    assert_eq!(none.master.peak_abs(), 0.0);
    assert!(none.report.warnings.iter().any(|w| w.contains("`nope`")));

    // Master equals the ordered sum of the stems.
    let (p, k) = (stem(&all, "piano"), stem(&all, "kick"));
    for i in (0..all.master.len()).step_by(997) {
        assert_eq!(
            all.master.left[i].to_bits(),
            (p.left[i] + k.left[i]).to_bits()
        );
    }
}

#[test]
fn one_shot_is_bars_plus_tail() {
    let mut score = bell_score();
    score.playback = Playback::OneShot { tail_seconds: 2.5 };
    let out = render(&score, &opts(true));
    let len = 30 * SR as usize;
    let tail = (2.5 * SR as f64) as usize;
    assert_eq!(out.master.len(), len + tail);
    assert_eq!(out.report.length_samples, len as u64);
    assert_eq!(out.report.tail_samples, tail as u64);
    assert_eq!(out.report.warmup_cycles, 0);
    assert!(!out.report.is_loop);
    assert!(out.stems.iter().all(|s| s.audio.len() == len + tail));
    // The release of the last notes rings into the tail and decays.
    let early = rms(&out.master.left[len..len + SR as usize / 4]);
    let late = rms(&out.master.left[len + tail - SR as usize / 4..]);
    assert!(early > 1e-3 && late < early * 0.1, "{early} {late}");
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

#[test]
fn gain_and_pan_automation() {
    let score = bell_score();
    let tm = TimeMap::new(&score, SR);
    let bar = |b: u32| tm.bar_start_sample(b) as usize;
    let base = render(&score, &opts(true));

    let mut fade = score.clone();
    automate(
        &mut fade,
        "piano",
        "gain_db",
        Curve::Linear,
        &[(1.0, -60.0), (9.0, 0.0)],
    );
    let f = render(&fade, &opts(true));
    let p = &stem(&f, "piano").left;
    let b = &stem(&base, "piano").left;
    let ratio = |r: std::ops::Range<usize>| rms(&p[r.clone()]) / rms(&b[r]);
    assert!(ratio(bar(2)..bar(3)) < 0.01, "bar 2 is ~-50 dB");
    assert!(ratio(bar(8)..bar(9)) > 0.5 && ratio(bar(8)..bar(9)) < 1.0);
    assert!(
        (ratio(bar(10)..bar(12)) - 1.0).abs() < 1e-6,
        "full level after bar 9"
    );
    assert!(buffers_equal(stem(&f, "kick"), stem(&base, "kick")));

    let mut pan = score.clone();
    automate(
        &mut pan,
        "piano",
        "pan",
        Curve::Step,
        &[(1.0, -1.0), (9.0, 1.0)],
    );
    let q = render(&pan, &opts(true));
    let s = stem(&q, "piano");
    // Hard left in bars 1-8 (one block of ramp at the step), hard right from bar 9.
    assert!(s.right[..bar(9) - 128].iter().all(|v| *v == 0.0));
    // pan_gains(1.0) leaves cos(pi/2) ~ -4.4e-8 on the left in f32.
    // The lane wraps with the loop: the last block ramps back towards bar 1's hard left.
    let end = s.left.len() - 128;
    assert!(rms(&s.left[bar(9) + 128..end]) < 1e-6);
    assert!(rms(&s.left[end..]) > 1e-4);
    assert!(rms(&s.left[bar(2)..bar(3)]) > 0.01 && rms(&s.right[bar(10)..bar(11)]) > 0.01);

    // A static gain change scales the stem.
    let mut quiet = score.clone();
    quiet.layers[0].gain_db = -6.0;
    let g = render(&quiet, &opts(true));
    let r = rms(&stem(&g, "piano").left) / rms(&stem(&base, "piano").left);
    assert!((r - 10f64.powf(-6.0 / 20.0)).abs() < 1e-4, "{r}");
}

#[test]
fn instrument_parameter_automation_reaches_the_instrument() {
    let mut score = bell_score();
    score.layers.retain(|l| l.id == "piano");
    let base = render(&score, &opts(false));
    let mut long = score.clone();
    automate(
        &mut long,
        "piano",
        "instrument.release_s",
        Curve::Step,
        &[(1.0, 3.0)],
    );
    let l = render(&long, &opts(false));
    assert!(rms(&l.master.left) > rms(&base.master.left) * 1.2);
    let mut bad = score.clone();
    automate(
        &mut bad,
        "piano",
        "instrument.nope",
        Curve::Step,
        &[(1.0, 1.0)],
    );
    let b = render(&bad, &opts(false));
    assert!(
        b.report
            .warnings
            .iter()
            .any(|w| w.contains("instrument.nope"))
    );
}

#[test]
fn frozen_master_gain_is_applied() {
    let score = bell_score();
    let base = render(&score, &opts(false));
    let mut frozen = score.clone();
    frozen.master.frozen_gain_db = Some(-6.0);
    let f = render(&frozen, &opts(false));
    assert_eq!(f.report.applied_gain_db, -6.0);
    let g = praxis_dsp::util::db_to_gain_f64(-6.0) as f32;
    assert!(
        base.master
            .left
            .iter()
            .zip(&f.master.left)
            .all(|(a, b)| (a * g).to_bits() == b.to_bits())
    );
    // Without the master chain the sum is returned untouched.
    let raw = render(
        &frozen,
        &RenderOptions {
            master: false,
            ..opts(false)
        },
    );
    assert_eq!(raw.report.applied_gain_db, 0.0);
    assert!(buffers_equal(&raw.master, &base.master));
}

#[test]
fn bad_options_and_unknown_presets_are_errors() {
    let score = bell_score();
    let zero = RenderOptions {
        block_size: 0,
        ..opts(false)
    };
    assert!(render_offline(&score, &TestFactory::new(), &zero).is_err());
    let mut unknown = score.clone();
    unknown.layers[0].instrument.preset = "felt_piano".into();
    assert!(matches!(
        render_offline(&unknown, &TestFactory::new(), &opts(false)),
        Err(praxis_engine::RenderError::Instrument(_))
    ));
    let mut invalid = score.clone();
    invalid.tempo.bpm = 1000.0;
    assert!(matches!(
        render_offline(&invalid, &TestFactory::new(), &opts(false)),
        Err(praxis_engine::RenderError::InvalidScore(_))
    ));
}
