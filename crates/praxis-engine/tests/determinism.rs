//! Determinism (FR-27) and sample-exact timing.

mod common;

use common::*;
use praxis_engine::testing::TestFactory;
use praxis_engine::{RenderOptions, render_offline};
use praxis_score::timeline;
use praxis_score::*;
use praxis_testkit::{Outcome, Tolerance, audio_hash, check};
use proptest::prelude::*;

#[test]
fn acceptance1_bell_renders_bit_identically_and_matches_golden() {
    let score = bell_score();
    let a = render(&score, &opts(true));
    let b = render(&score, &opts(true));
    assert_eq!(audio_hash(&a.master), audio_hash(&b.master));
    assert_eq!(a.stems.len(), 2);
    for (x, y) in a.stems.iter().zip(&b.stems) {
        assert_eq!(x.layer_id, y.layer_id);
        assert_eq!(audio_hash(&x.audio), audio_hash(&y.audio));
    }
    let r = &a.report;
    assert!(r.is_loop);
    assert_eq!(
        r.length_samples,
        30 * SR as u64,
        "16 bars at 128 BPM = 30 s"
    );
    assert_eq!(
        a.master.len(),
        30 * SR as usize,
        "a loop is exactly its bars long"
    );
    assert_eq!(r.warmup_cycles, 1);
    assert_eq!(r.tail_samples, 0);
    // Piano: 8 repeats of an 18-note 2-bar motif; kick: 8 bars x 2 hits.
    assert_eq!(r.notes_rendered, 8 * 18 + 16);
    assert!(r.blocked_layers.is_empty());
    // The mix bus (WP-F2) builds every effect and normalises the master.
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert!((r.loudness_lufs - score.master.loudness.target_lufs).abs() <= 0.5);
    assert!(r.true_peak_dbtp <= score.master.loudness.true_peak_dbtp + 0.05);
    // Not silent, not clipping.
    let peak = a.master.peak_abs();
    assert!(peak > 0.05 && peak < 1.0, "peak {peak}");
    let outcome = check("mix_acceptance1_bell", &a.master, SR, &Tolerance::default());
    assert!(matches!(
        outcome,
        Outcome::Identical | Outcome::FeaturesMatch | Outcome::Blessed
    ));
}

#[test]
fn acceptance1_bell_is_independent_of_block_size() {
    let score = bell_score();
    let base = render(&score, &opts(true));
    for bs in [64usize, 97, 1000] {
        let o = RenderOptions {
            block_size: bs,
            ..opts(true)
        };
        let x = render(&score, &o);
        assert!(buffers_equal(&base.master, &x.master), "block size {bs}");
        for (s, t) in base.stems.iter().zip(&x.stems) {
            assert!(
                buffers_equal(&s.audio, &t.audio),
                "stem {} at {bs}",
                s.layer_id
            );
        }
    }
}

/// A one-shot score with notes at deliberately odd sample positions.
fn odd_timing_score(offsets: &[f32]) -> Score {
    let mut s = Score::empty(2, 121.0);
    s.key = Key {
        tonic: PitchClass::A,
        mode: Mode::Minor,
    };
    s.playback = Playback::OneShot { tail_seconds: 1.0 };
    let notes = offsets
        .iter()
        .enumerate()
        .map(|(i, &off)| Note {
            step: 3 + 4 * i as u32,
            pitch: [57u8, 60, 64, 69][i % 4],
            velocity: 0.8,
            accent: if i % 2 == 0 {
                Accent::Strong
            } else {
                Accent::Bright
            },
            length: 1.7,
            offset: off,
        })
        .collect();
    s.patterns.push(Pattern {
        id: "p".into(),
        grid: Grid::Sixteenth,
        length_steps: 32,
        notes,
        spoken: None,
    });
    s.layers.push(Layer {
        id: "bell".into(),
        name: String::new(),
        class: GeneratorClass::Motif,
        instrument: InstrumentRef {
            kind: InstrumentKind::Synth,
            preset: "bell".into(),
            params: Default::default(),
        },
        clips: vec![Clip {
            bars: BarRange { start: 1, end: 2 },
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

fn first_nonzero(x: &[f32]) -> Option<usize> {
    x.iter().position(|v| *v != 0.0)
}

#[test]
fn note_starts_on_its_exact_sample_for_any_block_size() {
    let score = odd_timing_score(&[0.37]);
    let notes = timeline::expand_notes(&score, SR);
    assert_eq!(notes.len(), 1);
    let start = notes[0].start_sample as usize;
    assert!(
        !start.is_multiple_of(64),
        "start {start} should not sit on a block boundary"
    );
    let base = render(&score, &opts(false));
    for bs in [1usize, 7, 64, 128, 333, 4096] {
        let o = RenderOptions {
            block_size: bs,
            ..opts(false)
        };
        let out = render(&score, &o);
        assert_eq!(
            first_nonzero(&out.master.left),
            Some(start),
            "block size {bs}: note must start at sample {start}"
        );
        assert!(buffers_equal(&base.master, &out.master), "block size {bs}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 12,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn any_block_size_renders_identically(bs in 1usize..700, a in -0.5f32..0.5, b in -0.5f32..0.5) {
        let score = odd_timing_score(&[a, b, 0.21, -0.33]);
        let notes = timeline::expand_notes(&score, SR);
        let reference = render_offline(&score, &TestFactory::new(), &opts(false)).unwrap();
        let o = RenderOptions { block_size: bs, ..opts(false) };
        let out = render_offline(&score, &TestFactory::new(), &o).unwrap();
        prop_assert!(buffers_equal(&reference.master, &out.master));
        prop_assert_eq!(first_nonzero(&out.master.left), Some(notes[0].start_sample as usize));
    }
}
