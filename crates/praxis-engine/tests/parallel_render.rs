//! Offline renders on several worker threads (WP-Z2): the layers run on threads chunk by chunk
//! and are summed in score order, so the master and every stem are bit-identical to the
//! single-threaded render for any thread count, loops and one-shots alike, with effects,
//! automation-free and with chunk boundaries that do not fall on block boundaries of the piece.

mod common;

use common::*;
use praxis_engine::render::{RenderConfig, render_offline_threads, render_threads};
use praxis_engine::testing::TestFactory;
use praxis_score::*;

fn bits(b: &praxis_dsp::StereoBuffer) -> Vec<u32> {
    b.left.iter().chain(&b.right).map(|x| x.to_bits()).collect()
}

/// Eight layers of the bell motif (different octaves, pans and gains), some with effects.
fn eight_layers(playback: Playback, bars: u32) -> Score {
    let mut score = bell_score();
    score.bars = bars;
    score.sections.clear();
    score.chords.clear();
    score.playback = playback;
    let template = score.layer("piano").unwrap().clone();
    score.layers.clear();
    for i in 0..8 {
        let mut l = template.clone();
        l.id = format!("bell{i}");
        l.clips[0].bars = BarRange {
            start: 1 + (i as u32 % 3),
            end: bars,
        };
        l.clips[0].transpose = [0i8, 12, -12, 0, 12, 0, -12, 12][i];
        l.pan = (i as f32 - 3.5) / 4.0;
        l.gain_db = -9.0 - i as f32 * 0.5;
        l.seed_offset = i as u64;
        if i % 2 == 1 {
            l.effects = vec![Effect {
                id: "room".into(),
                kind: EffectKind::Reverb,
                params: [("decay_s".to_string(), 1.8), ("mix".to_string(), 0.25)]
                    .into_iter()
                    .collect(),
                bypass: false,
                sidechain: None,
                impulse: None,
            }];
        }
        score.layers.push(l);
    }
    assert!(validate::validate(&score).is_valid());
    score
}

fn assert_same_for_every_thread_count(score: &Score) {
    let config = RenderConfig::default();
    let one = render_offline_threads(score, &TestFactory::new(), &opts(true), &config, 1).unwrap();
    for threads in [2, 3, 8] {
        let many =
            render_offline_threads(score, &TestFactory::new(), &opts(true), &config, threads)
                .unwrap();
        assert_eq!(
            bits(&many.master),
            bits(&one.master),
            "{threads} threads: master"
        );
        assert_eq!(many.stems.len(), one.stems.len());
        for (a, b) in many.stems.iter().zip(&one.stems) {
            assert_eq!(a.layer_id, b.layer_id);
            assert_eq!(
                bits(&a.audio),
                bits(&b.audio),
                "{threads} threads: stem {}",
                a.layer_id
            );
        }
        assert_eq!(
            many.report.loudness_lufs.to_bits(),
            one.report.loudness_lufs.to_bits()
        );
    }
}

#[test]
fn a_loop_renders_bit_identically_on_any_number_of_threads() {
    assert_same_for_every_thread_count(&eight_layers(Playback::Loop, 16));
}

#[test]
fn a_one_shot_renders_bit_identically_on_any_number_of_threads() {
    // 23 bars at 128 BPM: the piece ends inside a chunk and inside a block.
    assert_same_for_every_thread_count(&eight_layers(Playback::OneShot { tail_seconds: 1.3 }, 23));
}

#[test]
fn the_thread_count_is_capped() {
    let n = render_threads();
    assert!((1..=8).contains(&n), "{n}");
}
