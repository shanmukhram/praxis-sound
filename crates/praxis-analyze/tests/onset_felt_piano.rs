//! Onsets of a real felt-piano render (WP-D1b): the piano stem of fixtures/acceptance1.score.json
//! (soft notes over ringing bass notes, room reverb) must sit on the eighth grid.
//!
//! The analyzer crate does not depend on the engine, so the render comes from the `praxis` binary
//! (`$PRAXIS_BIN`, else `target/release/praxis` or `target/debug/praxis` of this workspace;
//! build it with `cargo build -p praxis-cli --release`). Skipped with a message when the binary or
//! the Upright Piano KW pack is missing (set `PRAXIS_PACKS` to a packs directory that has it).

use praxis_analyze::features::{mono, onset::detect_onsets};
use praxis_analyze::verify::{Tolerances, verify_render};
use praxis_dsp::StereoBuffer;
use std::path::{Path, PathBuf};
use std::process::Command;

const PACK: &str = "upright-piano-kw";

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn praxis_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PRAXIS_BIN") {
        return Some(PathBuf::from(p));
    }
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace().join("target"));
    let exe = format!("praxis{}", std::env::consts::EXE_SUFFIX);
    ["release", "debug"]
        .iter()
        .map(|p| target.join(p).join(&exe))
        .find(|p| p.is_file())
}

/// Reads a PCM 16/24-bit or 32-bit float WAV (plain or extensible format) as a stereo buffer.
fn read_wav(path: &Path) -> (u32, StereoBuffer) {
    let b = std::fs::read(path).expect("stem written");
    let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    assert!(b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WAVE");
    let (mut fmt, mut ch, mut sr, mut bits) = (0u16, 0usize, 0u32, 0u16);
    let mut data = 0..0;
    let mut p = 12;
    while p + 8 <= b.len() {
        let len = u32_at(p + 4) as usize;
        let body = p + 8;
        match &b[p..p + 4] {
            b"fmt " => {
                fmt = u16_at(body);
                ch = u16_at(body + 2) as usize;
                sr = u32_at(body + 4);
                bits = u16_at(body + 14);
                if fmt == 0xFFFE {
                    fmt = u16_at(body + 24);
                }
            }
            b"data" => data = body..(body + len).min(b.len()),
            _ => {}
        }
        p = body + len + (len & 1);
    }
    let bps = bits as usize / 8;
    let samples: Vec<f32> = b[data]
        .chunks_exact(bps)
        .map(|s| match (fmt, bits) {
            (3, 32) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
            (1, 24) => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
            (1, 16) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32_768.0,
            _ => panic!("unsupported WAV format {fmt} with {bits} bits"),
        })
        .collect();
    (sr, StereoBuffer::from_interleaved(&samples, ch))
}

fn run(bin: &Path, args: &[&str]) -> std::process::Output {
    Command::new(bin)
        .args(args)
        .current_dir(workspace())
        .output()
        .expect("praxis runs")
}

#[test]
fn felt_piano_stem_of_acceptance1_is_on_the_grid() {
    let Some(bin) = praxis_bin() else {
        println!(
            "SKIP: no praxis binary; build it with `cargo build -p praxis-cli --release` or set \
             PRAXIS_BIN"
        );
        return;
    };
    let packs = run(&bin, &["--json", "packs", "list"]);
    let listing = String::from_utf8_lossy(&packs.stdout);
    let installed = serde_json::from_str::<serde_json::Value>(&listing)
        .ok()
        .and_then(|v| {
            v["packs"].as_array().map(|ps| {
                ps.iter()
                    .any(|p| p["name"] == PACK && p["installed"] == serde_json::json!(true))
            })
        })
        .unwrap_or(false);
    if !installed {
        println!(
            "SKIP: the `{PACK}` pack is not installed; set PRAXIS_PACKS to a packs directory \
             that has it"
        );
        return;
    }
    let dir = std::env::temp_dir().join(format!("praxis-onset-felt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let score_path = workspace().join("fixtures/acceptance1.score.json");
    let master = dir.join("master.wav");
    let stems = dir.join("stems");
    let out = run(
        &bin,
        &[
            "--json",
            "render",
            score_path.to_str().expect("utf-8 path"),
            "-o",
            master.to_str().expect("utf-8 path"),
            "--format",
            "32f",
            "--stems",
            stems.to_str().expect("utf-8 path"),
        ],
    );
    assert!(
        out.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let score = praxis_score::from_json(&std::fs::read_to_string(&score_path).expect("fixture"))
        .expect("fixture is a valid score");
    let (sr, piano) = read_wav(&stems.join("piano.wav"));
    let (_, mixed) = read_wav(&master);
    let _ = std::fs::remove_dir_all(&dir);

    let report = |name: &str, audio: &StereoBuffer| {
        let r = verify_render(&score, audio, sr, &Tolerances::default());
        let grid = r.checks.iter().find(|c| c.name == "grid").expect("grid");
        let (_, onsets) = detect_onsets(&mono(&audio.left, &audio.right), sr);
        println!(
            "{name}: {} ({} onsets in the whole clip)",
            grid.measured,
            onsets.len()
        );
        (r.onsets_on_grid.unwrap_or(0.0), onsets.len())
    };
    let (stem_grid, stem_onsets) = report("piano stem", &piano);
    let (master_grid, _) = report("master", &mixed);
    // 16 bars of a 16-step motif: 128 distinct note starts.
    assert!(
        stem_grid >= 0.97,
        "piano stem: {:.1}% on grid",
        stem_grid * 100.0
    );
    assert!(
        master_grid >= 0.97,
        "master: {:.1}% on grid",
        master_grid * 100.0
    );
    assert!(
        stem_onsets >= 109,
        "most notes are detected, not dropped: {stem_onsets} of 128"
    );
}
