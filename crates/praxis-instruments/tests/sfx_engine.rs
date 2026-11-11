//! Sound effects through the whole engine: the demo scores in `demos/sfx/` are valid, render
//! through `praxis_engine::render_offline` with the real registry, and the riser lands on the
//! bar-5 downbeat of a 120 BPM A-minor grid (its last sound is the sample before bar 5, and its
//! loudest 25 ms end within 5 ms of it).
//!
//! `cargo test -p praxis-instruments --release --test sfx_engine -- --ignored` also writes the
//! demo renders as 32-bit float WAVs to `target/tmp/sfx-demos/` (the committed FLACs in
//! `demos/sfx/` are made from them with ffmpeg, see `demos/sfx/README.md`).

use praxis_engine::{RenderOptions, RenderOutput, render_offline};
use praxis_instruments::{Packs, Registry};
use praxis_score::validate::validate;
use praxis_score::{Score, timeline::TimeMap};

const DEMOS: [(&str, &str); 7] = [
    (
        "whoosh",
        include_str!("../../../demos/sfx/whoosh.score.json"),
    ),
    (
        "impact",
        include_str!("../../../demos/sfx/impact.score.json"),
    ),
    ("riser", include_str!("../../../demos/sfx/riser.score.json")),
    ("drone", include_str!("../../../demos/sfx/drone.score.json")),
    (
        "glitch",
        include_str!("../../../demos/sfx/glitch.score.json"),
    ),
    ("tick", include_str!("../../../demos/sfx/tick.score.json")),
    (
        "riser_bar5",
        include_str!("../../../demos/sfx/riser_bar5.score.json"),
    ),
];

const SR: u32 = 48_000;

fn load(name: &str) -> Score {
    let json = DEMOS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, j)| *j)
        .unwrap_or_else(|| panic!("no demo {name}"));
    let score: Score = serde_json::from_str(json).unwrap_or_else(|e| panic!("{name}: {e}"));
    let report = validate(&score);
    assert!(report.is_valid(), "{name}: {:?}", report.errors);
    score
}

fn render(score: &Score, stems: bool) -> RenderOutput {
    let opts = RenderOptions {
        sample_rate: SR,
        stems,
        ..RenderOptions::default()
    };
    render_offline(score, &Registry::new(Packs::none()), &opts).expect("render")
}

/// End (exclusive) of the loudest 25 ms of `x` (see `loudness_peak` in `tests/sfx_presets.rs`).
fn loudness_peak(x: &[f32]) -> usize {
    let w = (0.025 * SR as f64) as usize;
    let mut acc: f64 = x[..w].iter().map(|v| (*v as f64) * (*v as f64)).sum();
    let (mut best, mut best_end) = (acc, w);
    for end in w + 1..=x.len() {
        let a = x[end - 1] as f64;
        let b = x[end - 1 - w] as f64;
        acc += a * a - b * b;
        if acc > best {
            best = acc;
            best_end = end;
        }
    }
    best_end
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

#[test]
fn every_sfx_demo_renders_through_the_engine() {
    for (name, _) in DEMOS {
        let score = load(name);
        let out = render(&score, false);
        let (l, r) = (&out.master.left, &out.master.right);
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()), "{name}");
        let pk = peak(l).max(peak(r));
        eprintln!(
            "{name:<11} {:.2} s, peak {pk:.3}, {:.1} LUFS, {:.1} dBTP, notes {}",
            l.len() as f64 / SR as f64,
            out.report.loudness_lufs,
            out.report.true_peak_dbtp,
            out.report.notes_rendered
        );
        assert!(
            out.report.blocked_layers.is_empty(),
            "{name}: blocked layers"
        );
        assert!(pk > 0.02 && pk < 1.0, "{name}: peak {pk}");
    }
}

#[test]
fn riser_lands_on_the_bar_5_downbeat_through_the_engine() {
    let score = load("riser_bar5");
    let out = render(&score, true);
    let bar5 = TimeMap::new(&score, SR).bar_start_sample(5) as usize;
    let stem = out
        .stems
        .iter()
        .find(|s| s.layer_id == "riser")
        .expect("riser stem");
    for (ch, x) in [("left", &stem.audio.left), ("right", &stem.audio.right)] {
        let last = x.iter().rposition(|v| *v != 0.0).expect("riser is silent") + 1;
        let first = x.iter().position(|v| *v != 0.0).unwrap_or(0);
        let bar3 = TimeMap::new(&score, SR).bar_start_sample(3) as usize;
        let at = loudness_peak(x);
        let before_ms = (bar5 as f64 - at as f64) * 1000.0 / SR as f64;
        eprintln!(
            "riser {ch}: starts at sample {first} (bar 3 = {bar3}), last sound at {} (bar 5 = {bar5}), loudest 25 ms ends {before_ms:.2} ms before bar 5",
            last - 1
        );
        assert_eq!(last, bar5, "riser {ch} must end on the bar-5 downbeat");
        assert!(
            (0.0..=5.0).contains(&before_ms),
            "riser {ch} peaks {before_ms} ms before bar 5"
        );
        // 8 beats at 120 BPM: the build starts exactly on bar 3.
        assert!(
            first.abs_diff(bar3) <= 1,
            "riser {ch} starts at {first}, bar 3 is {bar3}"
        );
    }
}

/// Writes the demo renders (float WAV) for the README and the listening check.
#[test]
#[ignore]
fn write_sfx_demo_renders() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("sfx-demos");
    std::fs::create_dir_all(&dir).expect("demo dir");
    for (name, _) in DEMOS {
        let out = render(&load(name), false);
        let path = dir.join(format!("{name}.wav"));
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: SR,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut w = hound::WavWriter::create(&path, spec).expect("wav");
        for (l, r) in out.master.left.iter().zip(out.master.right.iter()) {
            w.write_sample(*l).expect("write");
            w.write_sample(*r).expect("write");
        }
        w.finalize().expect("finalize");
        eprintln!("wrote {}", path.display());
    }
}
