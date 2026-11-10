//! Helpers shared by the engine integration tests.
#![allow(dead_code)]

use praxis_dsp::StereoBuffer;
use praxis_engine::testing::TestFactory;
use praxis_engine::{RenderOptions, RenderOutput, render_offline};
use praxis_score::{Score, from_json};
use std::path::{Path, PathBuf};

pub const SR: u32 = 48_000;

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Loads and validates a fixture from `fixtures/`.
pub fn fixture(name: &str) -> Score {
    let path = workspace_root().join("fixtures").join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    from_json(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The hand-written acceptance-test-1 score with the test bell (Phase 1 exit fixture).
pub fn bell_score() -> Score {
    fixture("acceptance1_bell.score.json")
}

pub fn opts(stems: bool) -> RenderOptions {
    RenderOptions {
        sample_rate: SR,
        stems,
        ..RenderOptions::default()
    }
}

pub fn render(score: &Score, opts: &RenderOptions) -> RenderOutput {
    render_offline(score, &TestFactory::new(), opts).expect("render")
}

pub fn stem<'a>(out: &'a RenderOutput, id: &str) -> &'a StereoBuffer {
    &out.stems
        .iter()
        .find(|s| s.layer_id == id)
        .unwrap_or_else(|| panic!("no stem `{id}`"))
        .audio
}

pub fn rms(x: &[f32]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
}

pub fn bits_equal(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

pub fn buffers_equal(a: &StereoBuffer, b: &StereoBuffer) -> bool {
    bits_equal(&a.left, &b.left) && bits_equal(&a.right, &b.right)
}

/// Writes a 32-bit float stereo WAV (WAVE_FORMAT_IEEE_FLOAT with a `fact` chunk).
pub fn write_wav_f32(path: &Path, audio: &StereoBuffer, sample_rate: u32) -> std::io::Result<()> {
    let frames = audio.len() as u32;
    let data_bytes = frames * 2 * 4;
    let mut out = Vec::with_capacity(58 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(4 + 26 + 12 + 8 + data_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&18u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    out.extend_from_slice(&2u16.to_le_bytes()); // channels
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 8).to_le_bytes()); // byte rate
    out.extend_from_slice(&8u16.to_le_bytes()); // block align
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(&0u16.to_le_bytes()); // cbSize
    out.extend_from_slice(b"fact");
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&frames.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for i in 0..audio.len() {
        out.extend_from_slice(&audio.left[i].to_le_bytes());
        out.extend_from_slice(&audio.right[i].to_le_bytes());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, out)
}
