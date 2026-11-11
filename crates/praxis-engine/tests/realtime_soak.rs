//! Simulated soak (WP-R "Done when"): 60 s of acceptance test 1 (bell fixture, the 30 s loop,
//! so two full cycles) pulled in 256-frame callbacks at 48 kHz, timing every callback against the
//! 5.33 ms budget. Run in release for meaningful numbers:
//! `cargo test -p praxis-engine --release --test realtime_soak -- --nocapture`
//! The felt-piano variant (real instruments) is `praxis-audio-io`'s `device_sim_soak`.

mod common;

use common::*;
use praxis_engine::realtime::{Command, RealtimeEngine, RealtimeOptions, prepare};
use praxis_engine::testing::TestFactory;
use std::time::Instant;

const FRAMES: usize = 256;

#[test]
fn sixty_seconds_at_256_frames_stay_far_under_the_budget() {
    let mut score = bell_score();
    score.master.frozen_gain_db = Some(0.0);
    let graph = prepare(&score, &TestFactory::new(), &RealtimeOptions::default()).unwrap();
    let (mut e, mut c) = RealtimeEngine::with_graph(graph);
    c.send(Command::Play).unwrap();
    let callbacks = 60 * SR as usize / FRAMES;
    let budget_ms = FRAMES as f64 / SR as f64 * 1000.0;
    let (mut l, mut r) = ([0.0f32; FRAMES], [0.0f32; FRAMES]);
    let mut times = Vec::with_capacity(callbacks);
    let mut peak = 0.0f32;
    for _ in 0..callbacks {
        let t0 = Instant::now();
        e.process(&mut l, &mut r);
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
        peak = l.iter().fold(peak, |m, v| m.max(v.abs()));
        c.poll();
    }
    let s = c.poll();
    assert!(s.playing && peak > 0.01, "it played");
    times.sort_by(f64::total_cmp);
    let max = times[times.len() - 1];
    let p99 = times[times.len() * 99 / 100];
    let p50 = times[times.len() / 2];
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    println!(
        "soak (bell, {callbacks} callbacks of {FRAMES} frames, budget {budget_ms:.2} ms): \
         mean {mean:.3} ms, median {p50:.3} ms, p99 {p99:.3} ms, max {max:.3} ms \
         (debug_assertions = {})",
        cfg!(debug_assertions)
    );
    assert!(p99 < budget_ms / 4.0, "p99 {p99} ms");
    // Wall-clock timing: the p99 bound above is the performance check. A single callback can
    // still be descheduled by the OS on a loaded machine or CI runner, so the worst case only
    // has to stay rare (at most 0.1% of callbacks over budget), not absent.
    let over = times.iter().filter(|t| **t >= budget_ms).count();
    assert!(
        over * 1000 <= times.len(),
        "{over} of {} callbacks over the {budget_ms:.2} ms budget (worst {max} ms)",
        times.len()
    );
}
