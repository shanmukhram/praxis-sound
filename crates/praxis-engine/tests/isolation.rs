//! Edit isolation at stem level (PRD section 2, docs/contracts/patch.md "Audio-level isolation").
//!
//! The piano pattern is changed only in bars 13-16, by splitting the piano clip the way
//! `apply` does (copy-on-write, `pattern_offset_steps` keeps the phase):
//! `[1-11] piano_motif`, `[12-12] piano_motif +8 steps`, `[13-16] piano_b13-16` (edited copy).
//!
//! Expected (loop playback, L = 30 s):
//! - the kick stem is bit-identical over the whole render;
//! - the piano stem is bit-identical outside the edit window. In a loop the window is
//!   `[start(13), L)` plus its wrap into the next cycle, `[0, overhang + tail)`, where `overhang` is
//!   how far notes of bars 13-16 sound past the end of bar 16 (their note length) and `tail` is the
//!   piano instrument release plus its effect tails. So bars 1-12 are compared over
//!   `[overhang + tail, start(13))`.
//! - with a frozen master gain, the master is bit-identical outside that window widened by the
//!   master chain's tail and limiter settling after it and the limiter lookahead before bar 13
//!   (`praxis_engine::master::isolation_tail_samples`, `limiter_latency_samples`).

mod common;

use common::*;
use praxis_engine::RenderOptions;
use praxis_score::timeline::{self, TimeMap};
use praxis_score::*;
use praxis_testkit::{first_difference, identical_in};

fn split_piano(score: &Score, last_pattern: &str) -> Score {
    let mut s = score.clone();
    let piano = s.layers.iter_mut().find(|l| l.id == "piano").unwrap();
    let base = piano.clips[0].clone();
    assert_eq!(base.bars, BarRange { start: 1, end: 16 });
    let clip = |start, end, pattern: &str, offset| Clip {
        bars: BarRange { start, end },
        pattern: pattern.into(),
        pattern_offset_steps: offset,
        ..base.clone()
    };
    // 8 eighth steps per bar, 16-step pattern: bar 12 starts at step 88 % 16 = 8.
    piano.clips = vec![
        clip(1, 11, "piano_motif", 0),
        clip(12, 12, "piano_motif", 8),
        clip(13, 16, last_pattern, 0),
    ];
    s
}

fn edited_score(score: &Score) -> Score {
    let mut s = split_piano(score, "piano_b13-16");
    let mut p = s.pattern("piano_motif").unwrap().clone();
    p.id = "piano_b13-16".into();
    // A subtle melodic change in key: the bright notes move up a step, two notes get softer.
    for n in &mut p.notes {
        match n.step {
            4 if n.pitch == 72 => n.pitch = 74,
            12 if n.pitch == 69 => n.pitch = 71,
            6 | 14 => n.velocity *= 0.8,
            _ => {}
        }
    }
    s.patterns.push(p);
    assert!(
        validate::validate(&s).is_valid(),
        "{:?}",
        validate::validate(&s)
    );
    s
}

#[test]
fn splitting_a_clip_keeps_the_render_identical() {
    let score = bell_score();
    let split = split_piano(&score, "piano_motif");
    assert!(validate::validate(&split).is_valid());
    assert_eq!(
        timeline::expand_notes(&score, SR),
        timeline::expand_notes(&split, SR)
    );
    let a = render(&score, &opts(true));
    let b = render(&split, &opts(true));
    assert!(buffers_equal(&a.master, &b.master));
}

#[test]
fn editing_bars_13_16_of_the_piano_leaves_everything_else_bit_identical() {
    // The piano's FDN reverb is bypassed: its recirculating state keeps float rounding
    // differences (about -140 dBFS) alive after an edit, so bit identity holds only for
    // finite-memory chains (mix_master.rs measures the reverb case).
    let mut score = bell_score();
    score.layers[0].effects[0].bypass = true;
    let edited = edited_score(&score);
    let tm = TimeMap::new(&score, SR);
    let len = timeline::score_length_samples(&score, SR) as usize;
    let start13 = tm.bar_start_sample(13) as usize;

    // Note level: bars 1-12 of the piano expand to the same events.
    let piano_notes = |s: &Score, max_bar: u32| {
        timeline::expand_notes(s, SR)
            .into_iter()
            .filter(|n| n.layer == 0 && n.bar <= max_bar)
            .collect::<Vec<_>>()
    };
    assert_eq!(piano_notes(&score, 12), piano_notes(&edited, 12));
    assert_ne!(piano_notes(&score, 16), piano_notes(&edited, 16));

    // Window: notes of bars 13-16 that sound past the loop end, plus the piano's tail.
    let overhang = [&score, &edited]
        .iter()
        .flat_map(|s| timeline::expand_notes(s, SR))
        .filter(|n| n.layer == 0 && n.bar >= 13)
        .map(|n| (n.start_sample + n.length_samples).saturating_sub(len as u64))
        .max()
        .unwrap() as usize;
    let tail = {
        use praxis_engine::InstrumentFactory;
        let ctx = praxis_engine::InstrumentContext {
            sample_rate: SR as f32,
            max_block: 128,
            key: score.key,
            bpm: score.tempo.bpm,
            seed: 0,
        };
        let piano = score.layer("piano").unwrap();
        let mut w = Vec::new();
        praxis_engine::testing::TestFactory::new()
            .create(&piano.instrument, &ctx)
            .unwrap()
            .tail_samples()
            + praxis_engine::effects::EffectChain::build(
                &piano.effects,
                &praxis_engine::effects::BuiltinEffects,
                SR as f32,
                128,
                score.tempo.bpm as f32,
                "piano",
                &mut w,
            )
            .tail_samples()
    };
    let wrap_end = overhang + tail;
    assert!(wrap_end < start13);

    let a = render(&score, &opts(true));
    let b = render(&edited, &opts(true));

    // Kick: bit-identical over the whole render.
    assert!(buffers_equal(stem(&a, "kick"), stem(&b, "kick")));

    // Piano: identical outside the window, different inside it.
    let (pa, pb) = (stem(&a, "piano"), stem(&b, "piano"));
    assert!(
        identical_in(pa, pb, wrap_end..start13),
        "piano bars 1-12 (after the wrapped tail) must be bit-identical"
    );
    let first = first_difference(pa, pb).expect("the edit is audible");
    let last_wrapped = (0..start13)
        .rev()
        .find(|&i| {
            pa.left[i].to_bits() != pb.left[i].to_bits()
                || pa.right[i].to_bits() != pb.right[i].to_bits()
        })
        .unwrap_or(0);
    let first_in_edit = (start13..len)
        .find(|&i| pa.left[i].to_bits() != pb.left[i].to_bits())
        .expect("bars 13-16 changed");
    println!(
        "isolation: L = {len}, start(13) = {start13}, overhang = {overhang}, tail = {tail}; \
         excluded window = [{start13}, {len}) + wrapped [0, {wrap_end}); \
         first differing sample = {first}, last differing sample before bar 13 = {last_wrapped} \
         ({:.3} s into bar 1), first difference inside bars 13-16 = {first_in_edit}",
        last_wrapped as f64 / SR as f64
    );
    assert!(last_wrapped < wrap_end);
    assert!(first_in_edit >= start13);

    // Master with a frozen gain: the same window holds for the mix.
    let mut fa = score.clone();
    let mut fb = edited.clone();
    fa.master.frozen_gain_db = Some(-3.0);
    fb.master.frozen_gain_db = Some(-3.0);
    let ma = render(&fa, &opts(false));
    let mb = render(&fb, &opts(false));
    let master_end = wrap_end
        + praxis_engine::master::isolation_tail_samples(&fa.master, SR, 128, fa.tempo.bpm as f32)
            as usize;
    let lookahead =
        praxis_engine::master::limiter_latency_samples(&fa.master.loudness, SR, 128) as usize;
    assert!(identical_in(
        &ma.master,
        &mb.master,
        master_end..start13 - lookahead
    ));
}

#[test]
fn one_shot_edit_leaves_bars_1_12_identical() {
    let mut score = bell_score();
    score.playback = Playback::OneShot { tail_seconds: 4.0 };
    let edited = edited_score(&score);
    let start13 = TimeMap::new(&score, SR).bar_start_sample(13) as usize;
    let o = RenderOptions {
        block_size: 256,
        ..opts(true)
    };
    let a = render(&score, &o);
    let b = render(&edited, &o);
    assert!(buffers_equal(stem(&a, "kick"), stem(&b, "kick")));
    // Nothing wraps in a one-shot: the piano is identical up to the first edited bar.
    assert!(identical_in(
        stem(&a, "piano"),
        stem(&b, "piano"),
        0..start13
    ));
    assert_eq!(
        first_difference(stem(&a, "piano"), stem(&b, "piano")).map(|i| i >= start13),
        Some(true)
    );
}
