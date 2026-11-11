//! Real-device soak (opt-in, it plays through the speakers): set `PRAXIS_DEVICE_TEST=1`.
//! Opens the default output at a 256-frame buffer, plays acceptance test 1 through the real-time
//! engine (felt piano when the pack is installed via `PRAXIS_PACKS`, the bell otherwise) for
//! `PRAXIS_DEVICE_SECONDS` (default 30) and requires 0 xruns. Run in release:
//! `PRAXIS_DEVICE_TEST=1 cargo test -p praxis-audio-io --release --test device_soak -- --nocapture`

use praxis_audio_io::device::{OutputDevice, OutputRequest, list_output_devices};
use praxis_engine::realtime::{PreparedGraph, RealtimeEngine, RealtimeOptions, prepare};
use praxis_engine::testing::TestFactory;
use praxis_engine::{RenderOptions, render_offline};
use praxis_instruments::Registry;
use praxis_score::{Score, from_json};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn enabled() -> bool {
    std::env::var("PRAXIS_DEVICE_TEST").is_ok_and(|v| v == "1")
}

fn fixture(name: &str) -> Score {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    from_json(&std::fs::read_to_string(&p).unwrap()).unwrap()
}

fn acceptance_graph(sr: u32) -> (PreparedGraph, &'static str) {
    let registry = Registry::with_default_packs();
    let opts = RenderOptions {
        sample_rate: sr,
        ..RenderOptions::default()
    };
    let piano = fixture("acceptance1.score.json");
    if let Ok(out) = render_offline(&piano, &registry, &opts) {
        let rt = RealtimeOptions {
            sample_rate: sr,
            master_gain_db: Some(out.report.applied_gain_db),
        };
        return (prepare(&piano, &registry, &rt).unwrap(), "felt piano");
    }
    let bell = fixture("acceptance1_bell.score.json");
    let out = render_offline(&bell, &TestFactory::new(), &opts).unwrap();
    let rt = RealtimeOptions {
        sample_rate: sr,
        master_gain_db: Some(out.report.applied_gain_db),
    };
    (prepare(&bell, &TestFactory::new(), &rt).unwrap(), "bell")
}

#[test]
fn the_default_device_plays_acceptance_1_without_xruns() {
    if !enabled() {
        println!("SKIP: set PRAXIS_DEVICE_TEST=1 to play through the default output device");
        return;
    }
    for d in list_output_devices().expect("devices") {
        println!(
            "{} {} {:?} Hz {:?} ch {:?}",
            if d.is_default { "*" } else { " " },
            d.name,
            d.sample_rate,
            d.channels,
            d.sample_format
        );
    }
    let seconds: f64 = std::env::var("PRAXIS_DEVICE_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30.0);
    let device = OutputDevice::open(&OutputRequest::default()).expect("open the default output");
    let sr = device.sample_rate();
    let (graph, what) = acceptance_graph(sr);
    let (mut engine, mut control) = RealtimeEngine::with_graph(graph);
    let stream = device
        .start(move |l, r, ctx| {
            engine.set_device_counters(ctx.xruns, ctx.late_callbacks);
            engine.process(l, r);
        })
        .expect("start the stream");
    control.play();
    let t0 = Instant::now();
    let mut peak = 0.0f32;
    let mut st = Vec::new();
    while t0.elapsed().as_secs_f64() < seconds {
        std::thread::sleep(Duration::from_millis(100));
        st.clear();
        control.drain_status(&mut st);
        peak = st.iter().fold(peak, |m, s| m.max(s.peak[0]));
    }
    control.stop();
    std::thread::sleep(Duration::from_millis(100));
    let s = stream.stats();
    println!(
        "device soak ({what}, {seconds} s): `{}` {} Hz {} ch, requested buffer {:?}, \
         callbacks {} of {}..{} frames (first {}); xruns {} (reported {}, drained {}, \
         deadline {}), late wakeups {}, errors {}; render mean {:.3} ms p99 {:.3} ms \
         max {:.3} ms; max gap {:.2} ms",
        stream.device_name(),
        stream.sample_rate(),
        stream.channels(),
        stream.requested_buffer_frames(),
        s.callbacks,
        s.min_callback_frames,
        s.max_callback_frames,
        s.first_callback_frames,
        s.xruns(),
        s.reported_xruns,
        s.starved,
        s.deadline_misses,
        s.late_wakeups,
        s.errors,
        s.mean_render_ms,
        s.p99_render_ms,
        s.max_render_ms,
        s.max_interval_ms
    );
    assert!(peak > 0.01, "it played");
    assert!(s.callbacks > 0);
    assert_eq!(s.xruns(), 0, "xruns");
}
