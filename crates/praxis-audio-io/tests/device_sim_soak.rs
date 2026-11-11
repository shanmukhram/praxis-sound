//! Simulated soak with the real instruments (WP-R "Done when"): 60 s of acceptance test 1 pulled
//! from the real-time engine in 256-frame callbacks at 48 kHz, every callback timed against the
//! 5.33 ms budget. Felt piano (`fixtures/acceptance1.score.json`, Upright Piano KW pack) when the
//! pack is installed (`PRAXIS_PACKS`), the bell fixture otherwise. Run in release:
//! `cargo test -p praxis-audio-io --release --test device_sim_soak -- --nocapture`

use praxis_engine::realtime::{PreparedGraph, RealtimeEngine, RealtimeOptions, prepare};
use praxis_engine::testing::TestFactory;
use praxis_engine::{RenderOptions, render_offline};
use praxis_instruments::Registry;
use praxis_score::{Score, from_json};
use std::path::PathBuf;
use std::time::Instant;

const SR: u32 = 48_000;
const FRAMES: usize = 256;

fn fixture(name: &str) -> Score {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    from_json(&std::fs::read_to_string(&p).unwrap()).unwrap()
}

/// The acceptance-1 graph at the gain an offline render applies, and what it plays.
fn acceptance_graph() -> (PreparedGraph, &'static str) {
    let registry = Registry::with_default_packs();
    let opts = RenderOptions {
        sample_rate: SR,
        ..RenderOptions::default()
    };
    let piano = fixture("acceptance1.score.json");
    if let Ok(out) = render_offline(&piano, &registry, &opts) {
        let rt = RealtimeOptions {
            sample_rate: SR,
            master_gain_db: Some(out.report.applied_gain_db),
        };
        return (prepare(&piano, &registry, &rt).unwrap(), "felt piano");
    }
    println!("felt piano pack not installed (set PRAXIS_PACKS); using the bell fixture");
    let bell = fixture("acceptance1_bell.score.json");
    let out = render_offline(&bell, &TestFactory::new(), &opts).unwrap();
    let rt = RealtimeOptions {
        sample_rate: SR,
        master_gain_db: Some(out.report.applied_gain_db),
    };
    (prepare(&bell, &TestFactory::new(), &rt).unwrap(), "bell")
}

#[test]
fn sixty_seconds_of_acceptance_1_at_256_frames_stay_far_under_the_budget() {
    let (graph, what) = acceptance_graph();
    let (mut e, mut c) = RealtimeEngine::with_graph(graph);
    c.play();
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
    assert!(c.poll().playing && peak > 0.01, "it played");
    times.sort_by(f64::total_cmp);
    let max = times[times.len() - 1];
    let p99 = times[times.len() * 99 / 100];
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    println!(
        "soak ({what}, {callbacks} callbacks of {FRAMES} frames, budget {budget_ms:.2} ms): \
         mean {mean:.3} ms, p99 {p99:.3} ms, max {max:.3} ms, peak {:.1} dBFS",
        20.0 * peak.log10()
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
