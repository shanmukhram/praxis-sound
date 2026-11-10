//! Seamless loop rendering (FR-26): tails wrap around, the captured cycle is in steady state, and
//! the seam is no different from any other point of a continuous render.

mod common;

use common::*;
use praxis_engine::testing::TestFactory;
use praxis_engine::{RenderConfig, render_offline_with};
use praxis_score::*;

/// (wrap jump |x[0] - x[L-1]|, mean and max adjacent-sample jump within `w` samples either side).
fn seam_stats(x: &[f32], w: usize) -> (f64, f64, f64) {
    let l = x.len();
    let jump = (x[0] as f64 - x[l - 1] as f64).abs();
    let mut diffs = Vec::new();
    for i in (l - 1 - w)..(l - 1) {
        diffs.push((x[i + 1] as f64 - x[i] as f64).abs());
    }
    for i in 0..w {
        diffs.push((x[i + 1] as f64 - x[i] as f64).abs());
    }
    let mean = diffs.iter().sum::<f64>() / diffs.len() as f64;
    let max = diffs.iter().cloned().fold(0.0, f64::max);
    (jump, mean, max)
}

fn second_diff(c: &[f32], j: usize) -> f64 {
    (c[j + 1] as f64 - 2.0 * c[j] as f64 + c[j - 1] as f64).abs()
}

#[test]
fn captured_cycle_is_steady_state() {
    let score = bell_score();
    let normal = render(&score, &opts(true));
    assert_eq!(normal.report.warmup_cycles, 1);
    let config = RenderConfig {
        min_warmup_cycles: 3,
        ..RenderConfig::default()
    };
    let longer = render_offline_with(&score, &TestFactory::new(), &opts(true), &config).unwrap();
    assert_eq!(longer.report.warmup_cycles, 3);
    // Cycle 1 and cycle 3 of the continuous render are bit-identical: the captured cycle already
    // contains every tail of the cycle before it, so x[L-1] -> x[0] is exactly the continuous
    // transition from one cycle into the next.
    assert!(buffers_equal(&normal.master, &longer.master));
    for (a, b) in normal.stems.iter().zip(&longer.stems) {
        assert!(buffers_equal(&a.audio, &b.audio), "stem {}", a.layer_id);
    }
}

#[test]
fn loop_seam_is_seamless() {
    let score = bell_score();
    let out = render(&score, &opts(false));
    let x = &out.master.left;
    let l = x.len();
    let w = SR as usize / 100; // 10 ms either side
    let (jump, mean, max) = seam_stats(x, w);
    let ratio = jump / mean;
    println!(
        "seam: |x[0]-x[L-1]| = {jump:.3e}; adjacent jumps within 10 ms: mean {mean:.3e}, max {max:.3e}; jump/mean = {ratio:.3}"
    );
    assert!(x[l - 1] != 0.0, "tails ring at the end of bar 16");
    assert!(
        jump <= max,
        "the wrap jump is no larger than nearby sample-to-sample steps"
    );
    assert!(ratio < 3.0, "wrap jump ratio {ratio}");

    // Three concatenated copies: no transient at the joins beyond what the music does nearby.
    let mut c = Vec::with_capacity(3 * l);
    for _ in 0..3 {
        c.extend_from_slice(x);
    }
    for join in [l, 2 * l] {
        let at_join = second_diff(&c, join - 1).max(second_diff(&c, join));
        let around = ((join - w)..(join + w))
            .filter(|&j| j != join - 1 && j != join)
            .map(|j| second_diff(&c, j))
            .fold(0.0, f64::max);
        println!("join at {join}: second difference {at_join:.3e}, max nearby {around:.3e}");
        assert!(at_join <= around, "click at join {join}");
    }
    // The left and right channels wrap the same way (centred mono layers).
    assert!(bits_equal(&out.master.left, &out.master.right));
}

/// 16 bars at 128 BPM (30 s loop) with one bell note in `bar`, ending exactly at the bar end.
fn single_note_loop(bar: u32, release_s: f64) -> Score {
    let mut s = Score::empty(16, 128.0);
    s.key = Key {
        tonic: PitchClass::A,
        mode: Mode::Minor,
    };
    s.playback = Playback::Loop;
    s.patterns.push(Pattern {
        id: "p".into(),
        grid: Grid::Eighth,
        length_steps: 8,
        notes: vec![Note {
            step: 4,
            pitch: 57,
            velocity: 0.8,
            accent: Accent::Normal,
            length: 4.0,
            offset: 0.0,
        }],
        spoken: None,
    });
    let mut params = std::collections::BTreeMap::new();
    params.insert("release_s".to_string(), release_s);
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
            bars: BarRange {
                start: bar,
                end: bar,
            },
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
        effects: vec![],
        automation: vec![],
        seed_offset: 0,
    });
    s
}

#[test]
fn long_release_at_bar_16_continues_into_sample_0() {
    let out = render(&single_note_loop(16, 2.0), &opts(false));
    let x = &out.master.left;
    let l = x.len();
    let first_100ms = rms(&x[..SR as usize / 10]);
    println!(
        "release wrapped into bar 1: x[0] = {:.4}, rms of first 100 ms = {first_100ms:.4}",
        x[0]
    );
    assert!(x[0] != 0.0);
    assert!(first_100ms > 0.01, "release audible at the loop start");
    let (jump, mean, _) = seam_stats(x, 480);
    assert!(jump < 3.0 * mean, "no step at the seam: {jump} vs {mean}");
    // It is the continuation of the note, decaying: later in bar 1 it is quieter.
    assert!(rms(&x[SR as usize..SR as usize + 4800]) < first_100ms);
    assert!(x[l - 1] != 0.0);

    // Control: the same note in bar 4 with a short release has died out before the loop ends,
    // so the start of the cycle is silent.
    let quiet = render(&single_note_loop(4, 0.45), &opts(false));
    assert!(quiet.master.left[..SR as usize].iter().all(|v| *v == 0.0));

    // One-shot: nothing wraps; the release sounds in the tail instead.
    let mut one = single_note_loop(16, 2.0);
    one.playback = Playback::OneShot { tail_seconds: 3.0 };
    let o = render(&one, &opts(false));
    assert!(o.master.left[..SR as usize].iter().all(|v| *v == 0.0));
    assert!(rms(&o.master.left[l..l + SR as usize / 10]) > 0.01);
}
