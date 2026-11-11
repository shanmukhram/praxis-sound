//! Renders a brief natively, the desktop's way (offline planner, packs read from disk,
//! `render_offline`), to a raw file the browser test compares the WebAssembly renders with.
//!
//! `cargo run -p praxis-wasm --example native_render --release -- <out.f32> [brief]`
//!
//! The file is the frame count (u32 LE) then the left and the right channel (f32 LE). Packs come
//! from `PRAXIS_PACKS` or the nearest `packs/` folder (as `praxis render` finds them). Prints the
//! render's measurements as one JSON line.

use praxis_engine::{RenderOptions, render_offline};
use praxis_instruments::packs::Packs;
use praxis_instruments::registry::Registry;
use std::io::Write;

const BRIEF: &str = "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt \
piano playing a repeating rhythmic motif on an eighth-note grid, with accents and overlapping \
sustain. Bars 9-16: the identical piano part, plus a soft muted kick layered underneath. No new \
melody, no fills. No risers, sweeps, impacts or pads anywhere.";

fn packs() -> Packs {
    if std::env::var_os("PRAXIS_PACKS").is_some() {
        return Packs::discover();
    }
    // Prefer a packs/ folder that holds the piano (a worktree's own packs/ may be empty).
    let cwd = std::env::current_dir().unwrap_or_default();
    cwd.ancestors()
        .map(|d| d.join("packs"))
        .find(|p| p.join("upright-piano-kw").join("pack.json").is_file())
        .map(Packs::at)
        .unwrap_or_else(Packs::discover)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(out) = args.next() else {
        eprintln!("usage: native_render <out.f32> [brief]");
        std::process::exit(2);
    };
    let brief = args.next().unwrap_or_else(|| BRIEF.to_string());
    let plan = praxis_wasm::ops::plan_brief(&brief, 1);
    if plan["ok"] != true {
        eprintln!("{}", plan["message"]);
        std::process::exit(1);
    }
    let score: praxis_score::Score =
        serde_json::from_value(plan["score"].clone()).expect("the planner's score parses");
    let registry = Registry::new(packs());
    let t0 = std::time::Instant::now();
    let r = match render_offline(&score, &registry, &RenderOptions::default()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("render failed: {e}");
            std::process::exit(1);
        }
    };
    let seconds = t0.elapsed().as_secs_f64();
    let frames = r.master.len();
    let mut bytes = Vec::with_capacity(4 + frames * 8);
    bytes.extend_from_slice(&(frames as u32).to_le_bytes());
    for x in r.master.left.iter().chain(r.master.right.iter()) {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    if let Some(dir) = std::path::Path::new(&out).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::File::create(&out).and_then(|mut f| f.write_all(&bytes)) {
        eprintln!("{out}: {e}");
        std::process::exit(1);
    }
    let report = praxis_wasm::ops::report_json(&r.report, frames, seconds, &[]);
    println!(
        "{}",
        serde_json::json!({ "out": out, "frames": frames, "report": report })
    );
}
