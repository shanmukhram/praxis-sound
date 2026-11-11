//! The real-time engine reproduces the offline render sample for sample (WP-R "Done when"):
//! acceptance test 1 (bell fixture) as a one-shot from bar 1, with the master gain fixed, pulled
//! in 256-frame and odd 97-frame callbacks. Also a stress graph with latency everywhere
//! (master-chain limiter, a lookahead limiter on a layer, a sidechain keyed across them, layer
//! and effect automation) and the "last applied gain" path without a frozen gain.

mod common;

use common::*;
use praxis_dsp::StereoBuffer;
use praxis_engine::realtime::{Command, RealtimeEngine, RealtimeOptions, prepare};
use praxis_engine::testing::TestFactory;
use praxis_score::*;

fn one_shot(mut s: Score) -> Score {
    s.playback = Playback::OneShot { tail_seconds: 2.0 };
    s
}

/// The score with `master.frozen_gain_db` set to what a normalising render applies.
fn frozen(score: &Score) -> Score {
    let out = render(score, &opts(false));
    let mut s = score.clone();
    s.master.frozen_gain_db = Some(out.report.applied_gain_db);
    s
}

/// Plays `score` from 0 in `callback`-frame callbacks for `frames` frames.
fn play(score: &Score, gain_db: Option<f64>, callback: usize, frames: usize) -> StereoBuffer {
    let graph = prepare(
        score,
        &TestFactory::new(),
        &RealtimeOptions {
            sample_rate: SR,
            master_gain_db: gain_db,
        },
    )
    .expect("prepare");
    let (mut engine, mut control) = RealtimeEngine::with_graph(graph);
    control.send(Command::Play).unwrap();
    let mut out = StereoBuffer::silent(frames);
    let (mut l, mut r) = (vec![0.0f32; callback], vec![0.0f32; callback]);
    let mut i = 0;
    while i < frames {
        engine.process(&mut l, &mut r);
        let n = callback.min(frames - i);
        out.left[i..i + n].copy_from_slice(&l[..n]);
        out.right[i..i + n].copy_from_slice(&r[..n]);
        i += n;
        control.poll();
    }
    out
}

/// Panics with the first difference when `a` and `b` are not bit-identical.
fn assert_bit_identical(what: &str, a: &StereoBuffer, b: &StereoBuffer) {
    assert_eq!(a.len(), b.len(), "{what}: lengths");
    for (ch, (x, y)) in [(&a.left, &b.left), (&a.right, &b.right)]
        .into_iter()
        .enumerate()
    {
        if let Some(i) = (0..x.len()).find(|&i| x[i].to_bits() != y[i].to_bits()) {
            let max = x
                .iter()
                .zip(y.iter())
                .map(|(p, q)| (p - q).abs())
                .fold(0.0f32, f32::max);
            panic!(
                "{what}: channel {ch} first differs at sample {i} of {} ({} vs {}); max |diff| {max:e}",
                x.len(),
                x[i],
                y[i]
            );
        }
    }
}

#[test]
fn acceptance1_one_shot_plays_bit_identically_in_256_and_97_frame_callbacks() {
    let score = frozen(&one_shot(bell_score()));
    let offline = render(&score, &opts(false));
    let n = offline.master.len();
    assert!(rms(&offline.master.left) > 1e-3, "the render is not silent");
    println!(
        "acceptance1 one-shot: {n} samples ({:.1} s), frozen gain {:.2} dB",
        n as f64 / SR as f64,
        score.master.frozen_gain_db.unwrap()
    );
    for cb in [256usize, 97] {
        // One callback past the end: the engine stops by itself and plays silence.
        let rt = play(&score, None, cb, n + cb);
        let head = StereoBuffer {
            left: rt.left[..n].to_vec(),
            right: rt.right[..n].to_vec(),
        };
        assert_bit_identical(&format!("{cb}-frame callbacks"), &head, &offline.master);
        assert!(
            rt.left[n..].iter().all(|v| *v == 0.0),
            "silence after the end"
        );
    }
}

#[test]
fn the_last_applied_gain_reproduces_a_normalising_render() {
    // No frozen gain: the caller passes the gain the offline render applied (report).
    let score = one_shot(bell_score());
    let offline = render(&score, &opts(false));
    let rt = play(
        &score,
        Some(offline.report.applied_gain_db),
        256,
        offline.master.len(),
    );
    assert_bit_identical("applied gain", &rt, &offline.master);
}

fn effect(id: &str, kind: EffectKind, params: &[(&str, f64)], sidechain: Option<&str>) -> Effect {
    Effect {
        id: id.into(),
        kind,
        params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        bypass: false,
        sidechain: sidechain.map(Into::into),
        impulse: None,
    }
}

/// The bell fixture with latency and cross-layer wiring everywhere.
fn stress_score() -> Score {
    let mut s = one_shot(bell_score());
    s.master.effects = vec![
        effect("bus_eq", EffectKind::Eq, &[("low_db", 2.0)], None),
        effect(
            "bus_lim",
            EffectKind::Limiter,
            &[("ceiling_dbtp", -6.0), ("lookahead_ms", 4.0)],
            None,
        ),
    ];
    let kick = s.layers.iter_mut().find(|l| l.id == "kick").unwrap();
    kick.effects.push(effect(
        "kick_lim",
        EffectKind::Limiter,
        &[("ceiling_dbtp", -3.0), ("lookahead_ms", 2.0)],
        None,
    ));
    let piano = s.layers.iter_mut().find(|l| l.id == "piano").unwrap();
    piano.effects.insert(
        0,
        effect(
            "duck",
            EffectKind::Compressor,
            &[("threshold_db", -30.0), ("ratio", 4.0)],
            Some("kick"),
        ),
    );
    piano.automation.push(Automation {
        target: "gain_db".into(),
        curve: Curve::Linear,
        points: vec![
            AutoPoint {
                bar: 1.0,
                value: -6.0,
            },
            AutoPoint {
                bar: 9.0,
                value: 0.0,
            },
        ],
    });
    piano.automation.push(Automation {
        target: "effects.room.mix".into(),
        curve: Curve::Smooth,
        points: vec![
            AutoPoint {
                bar: 1.0,
                value: 0.1,
            },
            AutoPoint {
                bar: 16.0,
                value: 0.5,
            },
        ],
    });
    s
}

#[test]
fn a_graph_with_latency_sidechain_and_automation_plays_bit_identically() {
    let score = frozen(&stress_score());
    let offline = render(&score, &opts(false));
    assert!(
        offline.report.warnings.is_empty(),
        "{:?}",
        offline.report.warnings
    );
    let graph = prepare(&score, &TestFactory::new(), &RealtimeOptions::default()).unwrap();
    println!(
        "stress graph: output latency {} samples, warnings {:?}",
        graph.latency_samples(),
        graph.warnings()
    );
    assert!(
        graph.latency_samples() > 300,
        "master chain and limiter latency"
    );
    for cb in [256usize, 97] {
        let rt = play(&score, None, cb, offline.master.len());
        assert_bit_identical(
            &format!("stress, {cb}-frame callbacks"),
            &rt,
            &offline.master,
        );
    }
}
