//! Render-level edit isolation (FR-7, contracts/patch.md "Audio-level isolation"): on the
//! acceptance-1 Score, "change the last four bars to add a subtle melody" is planned with the
//! offline rules and both Scores are rendered by the real engine (with its effects, mix bus and
//! limiter) and the engine's test instruments standing in for the sampled presets, with the
//! master gain frozen as the take history freezes it after the first render.
//!
//! - The piano and kick stems are bit-identical over the whole render (the edit only adds a
//!   layer).
//! - The master is bit-identical over bars 1-12 outside the tail window: the new layer's notes,
//!   release and effect tail wrapped to the loop start, widened by the master chain's tail and
//!   the limiter's settling, and by the limiter lookahead before bar 13.

use praxis_engine::testing::TestFactory;
use praxis_engine::{InstrumentContext, InstrumentFactory, RenderOptions, render_offline};
use praxis_planner::edit::plan_edit;
use praxis_planner::{Catalogue, RuleProvider, plan_brief};
use praxis_score::timeline::{TimeMap, expand_notes, score_length_samples};
use praxis_score::{InstrumentKind, Score};

const SR: u32 = 48_000;
const BRIEF: &str = "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt \
piano playing a repeating rhythmic motif on an eighth-note grid with accents and overlapping \
sustain. Bars 9-16: the identical piano part plus a soft muted kick underneath. No new melody, \
no fills, no risers, sweeps, impacts or pads.";

/// The engine's test instruments: pitched layers play the test `bell`, drums the test kick.
fn on_test_instruments(score: &mut Score) {
    for l in &mut score.layers {
        if l.instrument.kind == InstrumentKind::Drum {
            l.instrument.preset = "soft_kick".into();
        } else {
            l.instrument.kind = InstrumentKind::Synth;
            l.instrument.preset = "bell".into();
        }
        l.instrument.params.clear();
    }
}

fn opts() -> RenderOptions {
    RenderOptions {
        sample_rate: SR,
        stems: true,
        ..RenderOptions::default()
    }
}

/// Left and right channels.
type Channels<'a> = (&'a [f32], &'a [f32]);

fn master(out: &praxis_engine::RenderOutput) -> Channels<'_> {
    (&out.master.left, &out.master.right)
}

fn stem<'a>(out: &'a praxis_engine::RenderOutput, id: &str) -> Channels<'a> {
    let s = out
        .stems
        .iter()
        .find(|s| s.layer_id == id)
        .unwrap_or_else(|| panic!("no stem {id}"));
    (&s.audio.left, &s.audio.right)
}

fn same(a: Channels, b: Channels, i: usize) -> bool {
    a.0[i].to_bits() == b.0[i].to_bits() && a.1[i].to_bits() == b.1[i].to_bits()
}

#[test]
fn last_four_bars_melody_leaves_the_master_of_bars_1_to_12_bit_identical() {
    let catalogue = Catalogue::standard();
    let mut before = plan_brief(BRIEF, &RuleProvider::new(), &catalogue, 1)
        .expect("plan")
        .score;
    let out = plan_edit(
        &before,
        "change the last four bars to add a subtle melody",
        &RuleProvider::new(),
        &catalogue,
        7,
    )
    .expect("edit");
    let mut after = out.score.clone();
    assert_eq!(out.new_layers.len(), 1);
    let melody = out.new_layers[0].clone();
    on_test_instruments(&mut before);
    on_test_instruments(&mut after);
    let factory = TestFactory::new();

    // Take 1 renders with normalisation; its gain is frozen for both renders.
    let first = render_offline(&before, &factory, &opts()).expect("render take 1");
    let gain = first.report.applied_gain_db;
    before.master.frozen_gain_db = Some(gain);
    after.master.frozen_gain_db = Some(gain);
    let a = render_offline(&before, &factory, &opts()).expect("render before");
    let b = render_offline(&after, &factory, &opts()).expect("render after");
    assert_eq!(a.report.applied_gain_db, gain);
    assert_eq!(b.report.applied_gain_db, gain);
    assert!(
        (0..a.master.len()).all(|i| same(master(&a), master(&first), i)),
        "a frozen gain equal to the measured one renders take 1 unchanged"
    );
    for l in &before.layers {
        let (x, y) = (stem(&a, &l.id), stem(&b, &l.id));
        assert!(
            (0..x.0.len()).all(|i| same(x, y, i)),
            "stem {} changed",
            l.id
        );
    }

    // The window (contracts/patch.md), wrapped to the loop start.
    let block = opts().block_size;
    let len = score_length_samples(&after, SR);
    let li = after.layers.iter().position(|l| l.id == melody).unwrap();
    let overhang = expand_notes(&after, SR)
        .into_iter()
        .filter(|n| n.layer == li)
        .map(|n| (n.start_sample + n.length_samples).saturating_sub(len))
        .max()
        .unwrap_or(0) as usize;
    let layer = after.layer(&melody).unwrap();
    let ctx = InstrumentContext {
        sample_rate: SR as f32,
        max_block: block,
        key: after.key,
        bpm: after.tempo.bpm,
        seed: 0,
    };
    let inst_tail = factory
        .create(&layer.instrument, &ctx)
        .unwrap()
        .tail_samples();
    let mut w = Vec::new();
    let chain = praxis_engine::effects::EffectChain::build(
        &layer.effects,
        &praxis_engine::effects::BuiltinEffects,
        SR as f32,
        block,
        after.tempo.bpm as f32,
        &melody,
        &mut w,
    );
    let master_tail = praxis_engine::master::isolation_tail_samples(
        &after.master,
        SR,
        block,
        after.tempo.bpm as f32,
    ) as usize;
    let lookahead =
        praxis_engine::master::limiter_latency_samples(&after.master.loudness, SR, block) as usize;
    let window_end =
        overhang + inst_tail + chain.tail_samples() + chain.latency_samples() + master_tail;
    let tm = TimeMap::new(&after, SR);
    let start13 = tm.bar_start_sample(13) as usize;
    assert!(window_end < start13 - lookahead);

    let first_diff = (window_end..a.master.len()).find(|&i| !same(master(&a), master(&b), i));
    let last_in_window = (0..window_end)
        .rev()
        .find(|&i| !same(master(&a), master(&b), i));
    println!(
        "render isolation: frozen gain {gain:.3} dB; window wraps to sample {window_end} \
         ({:.2} s); last master difference inside it at {last_in_window:?}; first difference \
         after it at {first_diff:?}; bar 13 starts at {start13}, lookahead {lookahead}",
        window_end as f64 / SR as f64
    );
    let first_diff = first_diff.expect("the melody is audible in bars 13-16");
    assert!(
        first_diff >= start13 - lookahead,
        "master differs at sample {first_diff}, inside bars 1-12 and outside the window"
    );
}
