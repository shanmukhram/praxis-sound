//! The master bus (FR-25): master effects, loudness normalisation to `target_lufs` within 0.5 LU,
//! the true-peak limiter at `true_peak_dbtp` (+0.05 dB), `frozen_gain_db`, master-level edit
//! isolation (docs/contracts/patch.md) and seamless loops with reverb and the limiter on.

mod common;

use common::*;
use praxis_dsp::meter::{LoudnessMeter, true_peak_dbtp};
use praxis_engine::master;
use praxis_engine::testing::TestFactory;
use praxis_engine::{InstrumentFactory, RenderConfig, RenderOptions, render_offline_with};
use praxis_score::timeline::{self, TimeMap};
use praxis_score::*;
use praxis_testkit::identical_in;

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

fn lufs(x: &praxis_dsp::StereoBuffer) -> f64 {
    let mut m = LoudnessMeter::new(SR as f32);
    m.process(&x.left, &x.right);
    m.integrated_lufs()
}

/// Test signals: (name, score).
fn signals() -> Vec<(&'static str, Score)> {
    let bell = bell_score();
    let mut one = bell.clone();
    one.playback = Playback::OneShot { tail_seconds: 2.0 };
    // Hot: both layers +18 dB into a saturator on the piano, kick unfiltered.
    let mut hot = bell.clone();
    hot.layers[0].gain_db = 18.0;
    hot.layers[1].gain_db = 18.0;
    hot.layers[0].effects.push(effect(
        "drive",
        EffectKind::Saturation,
        &[("drive_db", 6.0)],
    ));
    hot.layers[1].effects.clear();
    // Very quiet: -40 dB piano alone.
    let mut quiet = bell.clone();
    quiet.layers.retain(|l| l.id == "piano");
    quiet.layers[0].gain_db = -40.0;
    // With master effects: a bus EQ and a glue compressor.
    let mut bus = bell.clone();
    bus.master.effects = vec![
        effect(
            "bus_eq",
            EffectKind::Eq,
            &[("low_db", -2.0), ("high_db", 1.5)],
        ),
        effect(
            "glue",
            EffectKind::Compressor,
            &[("threshold_db", -24.0), ("ratio", 2.0)],
        ),
    ];
    vec![
        ("bell loop", bell),
        ("bell one-shot", one),
        ("hot loop", hot),
        ("quiet piano loop", quiet),
        ("bus fx loop", bus),
    ]
}

#[test]
fn loudness_meets_the_target_and_true_peak_stays_under_the_ceiling() {
    for target in [-23.0, -16.0, -14.0] {
        for (name, mut score) in signals() {
            score.master.loudness.target_lufs = target;
            let out = render(&score, &opts(false));
            let r = &out.report;
            // Re-measure the delivered audio independently of the report.
            let measured = lufs(&out.master);
            let tp = true_peak_dbtp(&out.master.left, &out.master.right, SR as f32);
            println!(
                "target {target:>5.1} | {name:<17} before {:>7.2} LUFS, gain {:>6.2} dB, after {:>7.2} LUFS \
                 (re-measured {measured:>7.2}), true peak {tp:>6.2} dBTP",
                r.loudness_before_lufs, r.applied_gain_db, r.loudness_lufs
            );
            assert!(
                r.warnings.is_empty(),
                "{name} at {target}: {:?}",
                r.warnings
            );
            assert!(
                (r.loudness_lufs - target).abs() <= 0.5,
                "{name} at {target}"
            );
            assert!(
                (measured - target).abs() <= 0.5,
                "{name} at {target}: {measured}"
            );
            assert!((measured - r.loudness_lufs).abs() < 0.01);
            assert!(tp <= -1.0 + 0.05, "{name} at {target}: {tp} dBTP");
            assert!((tp - r.true_peak_dbtp).abs() < 1e-9);
        }
    }
}

#[test]
fn the_limiter_holds_any_ceiling_on_hot_material() {
    let (_, hot) = signals().remove(2);
    for ceiling in [-6.0, -3.0, -1.0, -0.1] {
        for target in [-16.0, -9.0] {
            let mut s = hot.clone();
            s.master.loudness.true_peak_dbtp = ceiling;
            s.master.loudness.target_lufs = target;
            let out = render(&s, &opts(false));
            let tp = true_peak_dbtp(&out.master.left, &out.master.right, SR as f32);
            println!(
                "ceiling {ceiling:>5.1} target {target:>5.1}: true peak {tp:.3} dBTP, loudness {:.2} LUFS \
                 (target {})",
                out.report.loudness_lufs,
                if out.report.warnings.is_empty() {
                    "met"
                } else {
                    "missed"
                }
            );
            assert!(tp <= ceiling + 0.05, "{tp} > {ceiling}");
        }
    }
    // normalize: false -> limiter only, no gain.
    let mut s = hot.clone();
    s.master.loudness.normalize = false;
    let out = render(&s, &opts(false));
    assert_eq!(out.report.applied_gain_db, 0.0);
    let tp = true_peak_dbtp(&out.master.left, &out.master.right, SR as f32);
    println!(
        "normalize off: before {:.2} LUFS, after {:.2} LUFS, true peak {tp:.3} dBTP",
        out.report.loudness_before_lufs, out.report.loudness_lufs
    );
    assert!(
        out.report.loudness_before_lufs > -12.0,
        "the hot mix is hot"
    );
    assert!(tp <= -1.0 + 0.05);
    assert!(out.report.loudness_lufs < out.report.loudness_before_lufs);
}

#[test]
fn frozen_gain_is_honoured() {
    let score = bell_score();
    let normal = render(&score, &opts(false));
    let g = normal.report.applied_gain_db;
    // Freezing the gain the normaliser chose reproduces the normalised render bit for bit.
    let mut frozen = score.clone();
    frozen.master.frozen_gain_db = Some(g);
    let f = render(&frozen, &opts(false));
    assert_eq!(f.report.applied_gain_db, g);
    assert!(buffers_equal(&f.master, &normal.master));
    // Another frozen gain is applied as is, whatever the target says.
    frozen.master.frozen_gain_db = Some(g - 6.0);
    frozen.master.loudness.target_lufs = -9.0;
    let q = render(&frozen, &opts(false));
    println!(
        "frozen: normalised gain {g:.3} dB -> {:.2} LUFS; frozen {:.3} dB -> {:.2} LUFS",
        normal.report.loudness_lufs, q.report.applied_gain_db, q.report.loudness_lufs
    );
    assert_eq!(q.report.applied_gain_db, g - 6.0);
    assert!((q.report.loudness_lufs - (normal.report.loudness_lufs - 6.0)).abs() < 0.1);
    assert_eq!(
        q.report.loudness_before_lufs,
        normal.report.loudness_before_lufs
    );
    // Without the master chain nothing is applied.
    let raw = render(
        &frozen,
        &RenderOptions {
            master: false,
            ..opts(false)
        },
    );
    assert_eq!(raw.report.applied_gain_db, 0.0);
}

fn edited_score(score: &Score) -> Score {
    let mut s = score.clone();
    let piano = s.layers.iter_mut().find(|l| l.id == "piano").unwrap();
    let base = piano.clips[0].clone();
    let clip = |start, end, pattern: &str, offset| Clip {
        bars: BarRange { start, end },
        pattern: pattern.into(),
        pattern_offset_steps: offset,
        ..base.clone()
    };
    piano.clips = vec![
        clip(1, 11, "piano_motif", 0),
        clip(12, 12, "piano_motif", 8),
        clip(13, 16, "piano_b13-16", 0),
    ];
    let mut p = s.pattern("piano_motif").unwrap().clone();
    p.id = "piano_b13-16".into();
    for n in &mut p.notes {
        match n.step {
            4 if n.pitch == 72 => n.pitch = 74,
            12 if n.pitch == 69 => n.pitch = 71,
            6 | 14 => n.velocity *= 0.8,
            _ => {}
        }
    }
    s.patterns.push(p);
    assert!(validate::validate(&s).is_valid());
    s
}

/// The documented isolation window for a bars 13-16 piano edit of `score` (a loop):
/// (start(13), stem window end, master window end, limiter lookahead) in samples. The window is
/// `[start(13) - lookahead, L)` plus its wrap `[0, end)`.
fn window(score: &Score, edited: &Score) -> (usize, usize, usize, usize) {
    let tm = TimeMap::new(score, SR);
    let len = timeline::score_length_samples(score, SR) as usize;
    let start13 = tm.bar_start_sample(13) as usize;
    let overhang = [score, edited]
        .iter()
        .flat_map(|s| timeline::expand_notes(s, SR))
        .filter(|n| n.layer == 0 && n.bar >= 13)
        .map(|n| (n.start_sample + n.length_samples).saturating_sub(len as u64))
        .max()
        .unwrap() as usize;
    let ctx = praxis_engine::InstrumentContext {
        sample_rate: SR as f32,
        max_block: 128,
        key: score.key,
        bpm: score.tempo.bpm,
        seed: 0,
    };
    let instrument_tail = TestFactory::new()
        .create(&score.layers[0].instrument, &ctx)
        .unwrap()
        .tail_samples();
    let mut w = Vec::new();
    let chain_tail = praxis_engine::effects::EffectChain::build(
        &score.layers[0].effects,
        &praxis_engine::effects::BuiltinEffects,
        SR as f32,
        128,
        score.tempo.bpm as f32,
        "piano",
        &mut w,
    )
    .tail_samples();
    let master_tail =
        master::isolation_tail_samples(&score.master, SR, 128, score.tempo.bpm as f32) as usize;
    let lookahead = master::limiter_latency_samples(&score.master.loudness, SR, 128) as usize;
    let stem_end = overhang + instrument_tail + chain_tail;
    (start13, stem_end, stem_end + master_tail, lookahead)
}

fn last_diff_before(
    x: &praxis_dsp::StereoBuffer,
    y: &praxis_dsp::StereoBuffer,
    end: usize,
) -> usize {
    (0..end)
        .rev()
        .find(|&i| {
            x.left[i].to_bits() != y.left[i].to_bits()
                || x.right[i].to_bits() != y.right[i].to_bits()
        })
        .unwrap_or(0)
}

fn max_diff(
    x: &praxis_dsp::StereoBuffer,
    y: &praxis_dsp::StereoBuffer,
    r: std::ops::Range<usize>,
) -> f32 {
    r.map(|i| {
        (x.left[i] - y.left[i])
            .abs()
            .max((x.right[i] - y.right[i]).abs())
    })
    .fold(0.0, f32::max)
}

#[test]
fn frozen_gain_keeps_untouched_bars_of_the_master_bit_identical() {
    // Bars 13-16 of the piano are edited. With `frozen_gain_db` set, the master must be
    // bit-identical outside the documented window (docs/contracts/patch.md): the stem window
    // widened by the master chain's tail and the limiter's settling after it, and by the limiter
    // lookahead before bar 13. The piano's reverb is bypassed here: see the next test.
    let mut score = bell_score();
    score.master.frozen_gain_db = Some(4.5);
    score.layers[0].effects[0].bypass = true;
    // A master chain with a tail of its own.
    score.master.effects = vec![effect(
        "glue",
        EffectKind::Compressor,
        &[("threshold_db", -30.0)],
    )];
    let edited = edited_score(&score);
    let (start13, stem_end, master_end, lookahead) = window(&score, &edited);
    assert!(master_end < start13 - lookahead);

    let a = render(&score, &opts(true));
    let b = render(&edited, &opts(true));
    assert_eq!(a.report.applied_gain_db, 4.5);
    assert!(buffers_equal(stem(&a, "kick"), stem(&b, "kick")));
    let last_stem = last_diff_before(stem(&a, "piano"), stem(&b, "piano"), start13);
    let last_master = last_diff_before(&a.master, &b.master, start13 - lookahead);
    let first_late = (master_end..a.master.len())
        .find(|&i| a.master.left[i].to_bits() != b.master.left[i].to_bits())
        .unwrap();
    println!(
        "master isolation (dry piano): start(13) = {start13}, lookahead {lookahead}; stem window wraps          to {stem_end} ({:.2} s), master window to {master_end} ({:.2} s); last differing sample          before bar 13: stem {last_stem} ({:.2} s), master {last_master} ({:.2} s); first master          difference after the wrap: {first_late}",
        stem_end as f64 / SR as f64,
        master_end as f64 / SR as f64,
        last_stem as f64 / SR as f64,
        last_master as f64 / SR as f64,
    );
    assert!(identical_in(
        stem(&a, "piano"),
        stem(&b, "piano"),
        stem_end..start13
    ));
    assert!(identical_in(
        &a.master,
        &b.master,
        master_end..start13 - lookahead
    ));
    assert!(first_late >= start13 - lookahead);
    assert!(!buffers_equal(&a.master, &b.master));
}

#[test]
fn with_the_piano_reverb_untouched_bars_differ_only_at_rounding_level() {
    // The FDN reverb recirculates its state: an edit leaves differences of one or two float
    // rounding steps that never die out completely, so "bit-identical" holds only up to about
    // -140 dBFS outside the window (reported as a contract issue). Inside the window the edit is
    // audible.
    let mut score = bell_score();
    score.master.frozen_gain_db = Some(3.0);
    let edited = edited_score(&score);
    let (start13, stem_end, master_end, lookahead) = window(&score, &edited);
    let a = render(&score, &opts(true));
    let b = render(&edited, &opts(true));
    assert!(buffers_equal(stem(&a, "kick"), stem(&b, "kick")));
    let ds = max_diff(stem(&a, "piano"), stem(&b, "piano"), stem_end..start13);
    let dm = max_diff(&a.master, &b.master, master_end..start13 - lookahead);
    let inside = max_diff(&a.master, &b.master, start13..a.master.len());
    let db = |x: f32| 20.0 * (x.max(1e-30) as f64).log10();
    println!(
        "isolation with reverb: max |difference| outside the window: stem {:.1} dBFS, master {:.1} dBFS; inside: {:.1} dBFS",
        db(ds),
        db(dm),
        db(inside)
    );
    assert!(db(ds) < -120.0 && db(dm) < -120.0);
    assert!(db(inside) > -40.0);
}

/// (wrap jump |x[0] - x[L-1]|, max adjacent-sample jump within `w` samples either side).
fn seam(x: &[f32], w: usize) -> (f64, f64) {
    let l = x.len();
    let jump = (x[0] as f64 - x[l - 1] as f64).abs();
    let near = (l - 1 - w..l - 1)
        .chain(0..w)
        .map(|i| (x[i + 1] as f64 - x[i] as f64).abs())
        .fold(0.0, f64::max);
    (jump, near)
}

fn second_diff(c: &[f32], j: usize) -> f64 {
    (c[j + 1] as f64 - 2.0 * c[j] as f64 + c[j - 1] as f64).abs()
}

#[test]
fn loops_stay_seamless_with_reverb_and_the_limiter_on() {
    // Reverb on the piano (fixture), a long reverb and a delay on the bus, and a loud target so
    // the limiter works hard.
    let mut score = bell_score();
    score.master.loudness.target_lufs = -9.0;
    score.master.effects = vec![
        effect(
            "bus_verb",
            EffectKind::Reverb,
            &[("decay_s", 3.5), ("mix", 0.15)],
        ),
        effect("echo", EffectKind::Delay, &[("feedback", 0.9)]),
    ];
    let out = render(&score, &opts(false));
    let r = &out.report;
    println!(
        "seam score: warm-up {} cycles, {:.2} LUFS, true peak {:.2} dBTP, gain {:.2} dB, warnings {:?}",
        r.warmup_cycles, r.loudness_lufs, r.true_peak_dbtp, r.applied_gain_db, r.warnings
    );
    assert!(r.warmup_cycles >= 1);
    assert!(r.true_peak_dbtp <= -1.0 + 0.05);
    for (ch, x) in [("left", &out.master.left), ("right", &out.master.right)] {
        let l = x.len();
        let w = SR as usize / 100;
        let (jump, near) = seam(x, w);
        let mut c = Vec::with_capacity(3 * l);
        for _ in 0..3 {
            c.extend_from_slice(x);
        }
        let join = l;
        let at = second_diff(&c, join - 1).max(second_diff(&c, join));
        let around = ((join - w)..(join + w))
            .filter(|&j| j != join - 1 && j != join)
            .map(|j| second_diff(&c, j))
            .fold(0.0, f64::max);
        println!(
            "seam {ch}: |x[0]-x[L-1]| = {jump:.3e} (max nearby step {near:.3e}); second difference \
             at the join {at:.3e} (max nearby {around:.3e})"
        );
        assert!(jump <= near, "{ch}: step at the wrap");
        assert!(at <= around, "{ch}: click at the wrap");
    }

    // The limiter is in steady state: on a time-invariant mix driven hard into the limiter,
    // more warm-up cycles give a bit-identical master.
    let mut dry = bell_score();
    for l in &mut dry.layers {
        l.effects.clear();
    }
    dry.master.loudness.target_lufs = -9.0;
    let more = |s: &Score, k: u32| {
        let config = RenderConfig {
            min_warmup_cycles: k,
            ..RenderConfig::default()
        };
        render_offline_with(s, &TestFactory::new(), &opts(false), &config).unwrap()
    };
    let d1 = render(&dry, &opts(false));
    let d3 = more(&dry, 3);
    assert!(d1.report.applied_gain_db > 6.0);
    assert!(
        buffers_equal(&d1.master, &d3.master),
        "limiter steady state"
    );

    // With reverbs on the layer and the bus, more warm-up gives the same bus loudness (time-varying
    // effects such as a chorus would change the samples, not the level). (The 0.9-feedback delay is
    // left out here: its tail is capped at 30 s for warm-up, so its echoes are not settled.)
    let mut verb = score.clone();
    verb.master.effects.truncate(1);
    let v1 = render(&verb, &opts(false));
    let v3 = more(&verb, v1.report.warmup_cycles + 2);
    let dl = (v1.report.loudness_before_lufs - v3.report.loudness_before_lufs).abs();
    println!(
        "steady state: dry + limiter bit-identical with 3 warm-up cycles; with reverbs the bus loudness          differs by {dl:.4} LU"
    );
    assert!(dl < 0.05);
}

#[test]
fn the_reverb_tail_wraps_into_sample_0() {
    // One bell note in bar 15 with a short release: dry, the start of the loop is silent; with a
    // long reverb on the layer the tail rings across the seam into bar 1.
    let mut s = Score::empty(16, 128.0);
    s.playback = Playback::Loop;
    s.patterns.push(Pattern {
        id: "p".into(),
        grid: Grid::Eighth,
        length_steps: 8,
        notes: vec![Note {
            step: 0,
            pitch: 69,
            velocity: 0.8,
            accent: Accent::Normal,
            length: 2.0,
            offset: 0.0,
        }],
        spoken: None,
    });
    let mut params = std::collections::BTreeMap::new();
    params.insert("release_s".to_string(), 0.3);
    s.layers.push(Layer {
        id: "bell".into(),
        name: String::new(),
        class: GeneratorClass::Motif,
        instrument: InstrumentRef {
            kind: InstrumentKind::Synth,
            preset: "bell".into(),
            params,
        },
        clips: vec![Clip {
            bars: BarRange { start: 15, end: 15 },
            pattern: "p".into(),
            pattern_offset_steps: 0,
            transpose: 0,
            velocity_scale: 1.0,
        }],
        locked: false,
        muted: false,
        solo: false,
        gain_db: 0.0,
        pan: 0.0,
        effects: vec![effect(
            "hall",
            EffectKind::Reverb,
            &[("decay_s", 8.0), ("mix", 0.5)],
        )],
        automation: vec![],
        seed_offset: 0,
    });
    let wet = render(&s, &opts(false));
    let mut dry_s = s.clone();
    dry_s.layers[0].effects[0].bypass = true;
    let dry = render(&dry_s, &opts(false));
    let head = SR as usize / 10;
    println!(
        "reverb wrap: wet x[0] = {:.3e}, rms of the first 100 ms {:.3e}; dry rms {:.3e}",
        wet.master.left[0],
        rms(&wet.master.left[..head]),
        rms(&dry.master.left[..head])
    );
    assert!(dry.master.left[..head].iter().all(|v| *v == 0.0));
    assert!(wet.master.left[0] != 0.0);
    assert!(rms(&wet.master.left[..head]) > 1e-3);
    let (jump, near) = seam(&wet.master.left, 480);
    assert!(jump <= near * 1.5, "{jump} vs {near}");
}
