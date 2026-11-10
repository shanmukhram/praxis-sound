//! Effect-chain plumbing with injected processors: latency compensation keeps stems on the grid,
//! effect tails raise the loop warm-up, and effect parameters reach the processor.
//! The references are rendered with the same injected builder, so the fixture's own effects are
//! the same (pass-through) processors in both renders.

mod common;

use common::*;
use praxis_dsp::Processor;
use praxis_engine::effects::EffectBuilder;
use praxis_engine::testing::TestFactory;
use praxis_engine::{RenderConfig, render_offline_with};
use praxis_score::{Effect, EffectKind, Score};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// A pure delay of `n` samples that reports `n` samples of latency.
struct PureDelay {
    l: Vec<f32>,
    r: Vec<f32>,
    pos: usize,
}

impl Processor for PureDelay {
    fn prepare(&mut self, _sr: f32, _max_block: usize) {}
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = self.l.len();
        for i in 0..left.len() {
            let (x, y) = (left[i], right[i]);
            left[i] = self.l[self.pos];
            right[i] = self.r[self.pos];
            self.l[self.pos] = x;
            self.r[self.pos] = y;
            self.pos = (self.pos + 1) % n;
        }
    }
    fn set_param(&mut self, _index: usize, _value: f32) {}
    fn reset(&mut self) {
        self.l.fill(0.0);
        self.r.fill(0.0);
    }
    fn latency_samples(&self) -> usize {
        self.l.len()
    }
}

/// Passes audio through and claims a tail; records the last `mix` value it was given.
struct Tail {
    tail: usize,
    mix: Arc<AtomicU32>,
    mix_index: usize,
}

impl Processor for Tail {
    fn prepare(&mut self, _sr: f32, _max_block: usize) {}
    fn process(&mut self, _left: &mut [f32], _right: &mut [f32]) {}
    fn set_param(&mut self, index: usize, value: f32) {
        if index == self.mix_index {
            self.mix.store(value.to_bits(), Ordering::Relaxed);
        }
    }
    fn reset(&mut self) {}
    fn tail_samples(&self) -> usize {
        self.tail
    }
}

struct TestFx {
    delay: usize,
    tail: usize,
    mix: Arc<AtomicU32>,
}

impl EffectBuilder for TestFx {
    fn build(&self, effect: &Effect, _sr: f32, _max_block: usize) -> Option<Box<dyn Processor>> {
        match effect.kind {
            EffectKind::Delay => Some(Box::new(PureDelay {
                l: vec![0.0; self.delay],
                r: vec![0.0; self.delay],
                pos: 0,
            })),
            EffectKind::Reverb => Some(Box::new(Tail {
                tail: self.tail,
                mix: self.mix.clone(),
                mix_index: 5,
            })),
            _ => None,
        }
    }
}

fn delay_fx(id: &str) -> Effect {
    Effect {
        id: id.into(),
        kind: EffectKind::Delay,
        params: Default::default(),
        bypass: false,
        sidechain: None,
        impulse: None,
    }
}

fn fx(delay: usize, tail: usize) -> TestFx {
    TestFx {
        delay,
        tail,
        mix: Arc::new(AtomicU32::new(0)),
    }
}

#[test]
fn latency_is_compensated_on_layers_and_master() {
    let score = bell_score();
    let builder = fx(301, 0);
    let config = RenderConfig {
        effects: &builder,
        ..RenderConfig::default()
    };
    let plain = render_offline_with(&score, &TestFactory::new(), &opts(true), &config).unwrap();
    let mut delayed = score.clone();
    delayed.layers[1].effects.push(delay_fx("lookahead"));
    delayed.master.effects.push(delay_fx("master_lookahead"));
    let out = render_offline_with(&delayed, &TestFactory::new(), &opts(true), &config).unwrap();
    // A pure 301-sample delay, compensated, is the identity: stems and master stay on the grid.
    assert!(buffers_equal(stem(&out, "kick"), stem(&plain, "kick")));
    assert!(buffers_equal(stem(&out, "piano"), stem(&plain, "piano")));
    assert!(buffers_equal(&out.master, &plain.master));

    // The same holds for one-shots.
    let mut one = delayed.clone();
    one.playback = praxis_score::Playback::OneShot { tail_seconds: 1.0 };
    let mut one_plain = score.clone();
    one_plain.playback = one.playback.clone();
    let a = render_offline_with(&one, &TestFactory::new(), &opts(true), &config).unwrap();
    let b = render_offline_with(&one_plain, &TestFactory::new(), &opts(true), &config).unwrap();
    assert!(buffers_equal(&a.master, &b.master));
}

fn short_loop() -> Score {
    let mut s = bell_score();
    s.bars = 2;
    s.sections.clear();
    s.chords.clear();
    s.layers.retain(|l| l.id == "piano");
    s.layers[0].clips[0].bars.end = 2;
    s
}

#[test]
fn effect_tails_raise_loop_warmup() {
    let score = short_loop(); // L = 3.75 s; the test bell's release tail is about 5.2 s
    let pass = fx(1, 0);
    let base = render_offline_with(
        &score,
        &TestFactory::new(),
        &opts(false),
        &RenderConfig {
            effects: &pass,
            ..RenderConfig::default()
        },
    )
    .unwrap();
    assert_eq!(base.report.warmup_cycles, 2);
    let l = base.report.length_samples as usize;
    let instrument_tail = {
        use praxis_engine::InstrumentFactory;
        let ctx = praxis_engine::InstrumentContext {
            sample_rate: SR as f32,
            max_block: 128,
            key: score.key,
            bpm: score.tempo.bpm,
            seed: 0,
        };
        TestFactory::new()
            .create(&score.layers[0].instrument, &ctx)
            .unwrap()
            .tail_samples()
    };
    let overhang = (0.6 * 60.0 / 128.0 / 2.0 * SR as f64).round() as usize; // last note: 1.6 steps
    let reverb_tail = 10 * SR as usize;
    let builder = fx(1, reverb_tail);
    let config = RenderConfig {
        effects: &builder,
        ..RenderConfig::default()
    };
    let out = render_offline_with(&score, &TestFactory::new(), &opts(false), &config).unwrap();
    let want = (overhang + instrument_tail + reverb_tail).div_ceil(l) as u32;
    assert_eq!(out.report.warmup_cycles, want);
    // Pass-through processors with a tail do not change the audio.
    assert!(buffers_equal(&out.master, &base.master));
    // The reverb's `mix` from the score reached the processor (fixture: mix 0.2).
    assert_eq!(f32::from_bits(builder.mix.load(Ordering::Relaxed)), 0.2);

    // Tails are capped at 30 s.
    let huge = fx(1, 600 * SR as usize);
    let config = RenderConfig {
        effects: &huge,
        ..RenderConfig::default()
    };
    let capped = render_offline_with(&score, &TestFactory::new(), &opts(false), &config).unwrap();
    assert_eq!(
        capped.report.warmup_cycles,
        (30 * SR as usize).div_ceil(l) as u32
    );
}
