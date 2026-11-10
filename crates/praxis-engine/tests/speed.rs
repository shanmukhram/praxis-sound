//! Render speed. Prints the realtime factor (audio seconds per wall-clock second); run in release
//! for meaningful numbers:
//! `cargo test -p praxis-engine --release --test speed -- --nocapture`

mod common;

use common::*;
use praxis_engine::render_offline;
use praxis_engine::testing::TestFactory;
use praxis_score::*;
use std::time::Instant;

fn realtime_factor(score: &Score, stems: bool, runs: usize) -> (f64, f64) {
    let mut best = f64::MAX;
    let mut audio_s = 0.0;
    for _ in 0..runs {
        let t0 = Instant::now();
        let out = render_offline(score, &TestFactory::new(), &opts(stems)).unwrap();
        best = best.min(t0.elapsed().as_secs_f64());
        audio_s = out.master.len() as f64 / SR as f64;
    }
    (audio_s / best, best)
}

#[test]
fn acceptance_fixture_renders_faster_than_realtime() {
    let score = bell_score();
    let (rtf, secs) = realtime_factor(&score, true, 3);
    println!(
        "acceptance1_bell (30 s loop, 2 layers, 1 warm-up cycle, stems on): {secs:.3} s wall, realtime factor {rtf:.1}x (debug_assertions = {})",
        cfg!(debug_assertions)
    );
    assert!(rtf > 1.0, "realtime factor {rtf}");
}

#[test]
fn two_minute_eight_layer_piece_speed() {
    // 64 bars at 128 BPM = 120 s, one-shot, eight bell layers playing the motif in turn.
    let mut score = bell_score();
    score.bars = 64;
    score.sections.clear();
    score.chords.clear();
    score.playback = Playback::OneShot { tail_seconds: 2.0 };
    let template = score.layer("piano").unwrap().clone();
    score.layers.clear();
    for i in 0..8 {
        let mut l = template.clone();
        l.id = format!("bell{i}");
        l.clips[0].bars = BarRange { start: 1, end: 64 };
        l.clips[0].transpose = [0i8, 12, -12, 0, 12, 0, -12, 12][i];
        l.pan = (i as f32 - 3.5) / 4.0;
        l.gain_db = -9.0;
        l.seed_offset = i as u64;
        score.layers.push(l);
    }
    assert!(validate::validate(&score).is_valid());
    let (rtf, secs) = realtime_factor(&score, false, 1);
    println!(
        "2-minute 8-layer piece: {secs:.3} s wall, realtime factor {rtf:.1}x (target >= 20x in release)"
    );
    assert!(rtf > 1.0);
}
