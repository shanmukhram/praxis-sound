//! Plays a score or project through the speakers with the real instruments, for the Phase 3
//! hardware check.
//!
//! ```text
//! cargo run -p praxis-audio-io --release --example play -- <score.json | project.praxis>
//!     [--device NAME] [--buffer FRAMES] [--seconds S] [--list]
//! ```
//!
//! Instrument packs come from `PRAXIS_PACKS` (else a `packs/` folder near the current directory
//! or the executable). The master gain is the score's frozen gain, else the gain an offline
//! render applies (rendered here first, at the device's rate). One-shots play to their end;
//! loops play two cycles unless `--seconds` says otherwise. At the end it prints the device,
//! the buffer requested and the callback sizes the device actually used, xruns, late wakeups and
//! the callback render times.

use praxis_audio_io::device::{OutputDevice, OutputRequest, list_output_devices};
use praxis_engine::realtime::{RealtimeEngine, RealtimeOptions, prepare};
use praxis_engine::{RenderOptions, render_offline};
use praxis_instruments::Registry;
use praxis_score::Score;
use praxis_score::project::Project;
use std::path::PathBuf;
use std::time::{Duration, Instant};

struct Args {
    path: Option<PathBuf>,
    device: Option<String>,
    buffer: u32,
    seconds: Option<f64>,
    list: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        path: None,
        device: None,
        buffer: praxis_audio_io::device::DEFAULT_BUFFER_FRAMES,
        seconds: None,
        list: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--device" => a.device = Some(it.next().ok_or("--device needs a name")?),
            "--buffer" => {
                a.buffer = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--buffer needs a frame count")?
            }
            "--seconds" => {
                a.seconds = Some(
                    it.next()
                        .and_then(|v| v.parse().ok())
                        .ok_or("--seconds needs a number")?,
                )
            }
            "--list" => a.list = true,
            "-h" | "--help" => return Err(String::new()),
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => a.path = Some(PathBuf::from(other)),
        }
    }
    Ok(a)
}

fn load(path: &PathBuf) -> Result<Score, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    if text.contains("\"praxis-project\"") {
        Project::from_json(&text)
            .map(|p| p.score)
            .map_err(|e| format!("{}: {e}", path.display()))
    } else {
        praxis_score::from_json(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

fn main() {
    if let Err(e) = run() {
        if e.is_empty() {
            eprintln!(
                "usage: play <score.json | project.praxis> [--device NAME] [--buffer FRAMES] [--seconds S] [--list]"
            );
        } else {
            eprintln!("error: {e}");
        }
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    if args.list {
        for d in list_output_devices().map_err(|e| e.to_string())? {
            println!(
                "{} {} [{}] {:?} Hz, {:?} ch, {:?}, buffer range {:?}",
                if d.is_default { "*" } else { " " },
                d.name,
                d.id,
                d.sample_rate,
                d.channels,
                d.sample_format,
                d.buffer_range
            );
        }
        if args.path.is_none() {
            return Ok(());
        }
    }
    let path = args.path.ok_or_else(String::new)?;
    let score = load(&path)?;

    let device = OutputDevice::open(&OutputRequest {
        device: args.device.clone(),
        buffer_frames: args.buffer,
        sample_rate: None,
    })
    .map_err(|e| e.to_string())?;
    let sr = device.sample_rate();
    println!(
        "device: {} ({} Hz, {} ch, {}), requested buffer {:?} frames",
        device.name(),
        sr,
        device.channels(),
        device.sample_format(),
        device.requested_buffer_frames()
    );

    let registry = Registry::with_default_packs();
    match &registry.packs().root {
        Some(root) => println!("packs: {}", root.display()),
        None => println!("packs: none found (set PRAXIS_PACKS)"),
    }

    // Master gain: frozen, else what an offline render applies.
    let gain_db = match score.master.frozen_gain_db {
        Some(g) => {
            println!("master: frozen gain {g:.2} dB");
            None
        }
        None => {
            let t0 = Instant::now();
            let out = render_offline(
                &score,
                &registry,
                &RenderOptions {
                    sample_rate: sr,
                    ..RenderOptions::default()
                },
            )
            .map_err(|e| format!("offline render: {e}"))?;
            println!(
                "master: offline render in {:.2} s: {:.2} LUFS, {:.2} dBTP, gain {:.2} dB",
                t0.elapsed().as_secs_f64(),
                out.report.loudness_lufs,
                out.report.true_peak_dbtp,
                out.report.applied_gain_db
            );
            Some(out.report.applied_gain_db)
        }
    };

    let t0 = Instant::now();
    let graph = prepare(
        &score,
        &registry,
        &RealtimeOptions {
            sample_rate: sr,
            master_gain_db: gain_db,
        },
    )
    .map_err(|e| format!("prepare: {e}"))?;
    for w in graph.warnings() {
        println!("warning: {w}");
    }
    let seconds = args.seconds.unwrap_or_else(|| {
        let n = if graph.is_loop() {
            2 * graph.length_samples()
        } else {
            graph.end_samples()
        };
        n as f64 / sr as f64
    });
    println!(
        "prepared in {:.2} s: {} {:.1} s, latency {} samples; playing {seconds:.1} s",
        t0.elapsed().as_secs_f64(),
        if graph.is_loop() { "loop" } else { "one-shot" },
        graph.length_samples() as f64 / sr as f64,
        graph.latency_samples()
    );

    let (mut engine, mut control) = RealtimeEngine::with_graph(graph);
    let stream = device
        .start(move |l, r, ctx| {
            engine.set_device_counters(ctx.xruns, ctx.late_callbacks);
            engine.process(l, r);
        })
        .map_err(|e| e.to_string())?;
    control.play();

    let start = Instant::now();
    let mut last_print = Instant::now();
    let mut peak = [0.0f32; 2];
    let mut statuses = Vec::new();
    loop {
        std::thread::sleep(Duration::from_millis(50));
        statuses.clear();
        control.drain_status(&mut statuses);
        control.collect_garbage();
        for s in &statuses {
            peak[0] = peak[0].max(s.peak[0]);
            peak[1] = peak[1].max(s.peak[1]);
        }
        let s = control.last_status();
        if last_print.elapsed() >= Duration::from_secs(2) {
            println!(
                "  {:6.1} s  peak {:5.1} / {:5.1} dBFS  xruns {}",
                s.position as f64 / sr as f64,
                20.0 * peak[0].max(1e-9).log10(),
                20.0 * peak[1].max(1e-9).log10(),
                s.xruns
            );
            peak = [0.0; 2];
            last_print = Instant::now();
        }
        if s.ended || start.elapsed().as_secs_f64() >= seconds {
            break;
        }
    }
    control.stop();
    std::thread::sleep(Duration::from_millis(100));
    let st = stream.stats();
    let name = stream.device_name().to_string();
    let requested = stream.requested_buffer_frames();
    drop(stream);
    println!(
        "done: device `{name}`, {sr} Hz, requested buffer {requested:?} frames, callbacks of {}..{} frames (first {}), {} callbacks",
        st.min_callback_frames, st.max_callback_frames, st.first_callback_frames, st.callbacks
    );
    println!(
        "xruns {} (reported {}, drained buffer {}, deadline misses {}), late wakeups {}, stream errors {}",
        st.xruns(),
        st.reported_xruns,
        st.starved,
        st.deadline_misses,
        st.late_wakeups,
        st.errors
    );
    println!(
        "callback render time: mean {:.3} ms, p99 {:.3} ms, max {:.3} ms; longest gap between callbacks {:.2} ms",
        st.mean_render_ms, st.p99_render_ms, st.max_render_ms, st.max_interval_ms
    );
    Ok(())
}
