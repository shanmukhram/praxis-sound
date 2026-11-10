//! Golden-audio harness. Compares a render with a stored golden record two ways:
//!
//! 1. **Hash**: blake3 of the exact f32 sample bits. Bit-identical output is promised per
//!    platform (PRD: "on the same platform"), so hashes are stored per `os-arch` key.
//! 2. **Features**: length, peak, RMS and per-second RMS envelope, within tolerances. These hold
//!    across platforms, where libm differences may change the last bits.
//!
//! Records live in `tests/golden/<name>.json` at the workspace root. Set `PRAXIS_BLESS=1` to
//! (re)write the record for the current platform after a deliberate change.

#![forbid(unsafe_code)]

use praxis_dsp::StereoBuffer;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Features {
    pub sample_rate: u32,
    pub frames: usize,
    pub peak: f32,
    pub rms: f32,
    /// RMS per 1 s window, both channels.
    pub rms_per_second: Vec<f32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GoldenRecord {
    /// Hash per platform key, e.g. "windows-x86_64".
    pub hashes: BTreeMap<String, String>,
    pub features: Features,
}

#[derive(Clone, Debug)]
pub struct Tolerance {
    /// Absolute tolerance for peak/RMS values (linear).
    pub level: f32,
}

impl Default for Tolerance {
    fn default() -> Self {
        Tolerance { level: 1e-3 }
    }
}

pub fn platform_key() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// blake3 over the little-endian bits of every left then right sample.
pub fn audio_hash(audio: &StereoBuffer) -> String {
    let mut h = blake3::Hasher::new();
    for ch in [&audio.left, &audio.right] {
        for v in ch.iter() {
            h.update(&v.to_bits().to_le_bytes());
        }
    }
    h.finalize().to_hex().to_string()
}

pub fn features(audio: &StereoBuffer, sample_rate: u32) -> Features {
    let n = audio.len();
    let mut sum = 0.0f64;
    for i in 0..n {
        sum += (audio.left[i] as f64).powi(2) + (audio.right[i] as f64).powi(2);
    }
    let rms = if n == 0 {
        0.0
    } else {
        (sum / (2 * n) as f64).sqrt() as f32
    };
    let win = sample_rate.max(1) as usize;
    let rms_per_second = (0..n.div_ceil(win))
        .map(|w| {
            let (a, b) = (w * win, ((w + 1) * win).min(n));
            let s: f64 = (a..b)
                .map(|i| (audio.left[i] as f64).powi(2) + (audio.right[i] as f64).powi(2))
                .sum();
            ((s / (2 * (b - a)).max(1) as f64).sqrt()) as f32
        })
        .collect();
    Features {
        sample_rate,
        frames: n,
        peak: audio.peak_abs(),
        rms,
        rms_per_second,
    }
}

pub fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden")
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Hash matched for this platform.
    Identical,
    /// No hash stored for this platform; features matched.
    FeaturesMatch,
    /// Record written because PRAXIS_BLESS=1 (or none existed and blessing was requested).
    Blessed,
}

/// Checks `audio` against golden record `name`. Panics with a readable diff on mismatch, so it
/// can be called directly from `#[test]` functions.
pub fn check(name: &str, audio: &StereoBuffer, sample_rate: u32, tol: &Tolerance) -> Outcome {
    let path = golden_dir().join(format!("{name}.json"));
    let hash = audio_hash(audio);
    let feats = features(audio, sample_rate);
    let bless = std::env::var("PRAXIS_BLESS").is_ok_and(|v| v == "1");
    let existing: Option<GoldenRecord> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    if bless {
        let mut rec = existing.unwrap_or(GoldenRecord {
            hashes: BTreeMap::new(),
            features: feats.clone(),
        });
        rec.hashes.insert(platform_key(), hash);
        rec.features = feats;
        std::fs::create_dir_all(golden_dir()).expect("create tests/golden");
        std::fs::write(&path, serde_json::to_string_pretty(&rec).unwrap()).expect("write golden");
        return Outcome::Blessed;
    }
    let rec = existing.unwrap_or_else(|| {
        panic!(
            "no golden record {}; render it, listen/inspect, then rerun with PRAXIS_BLESS=1",
            path.display()
        )
    });
    compare_features(name, &rec.features, &feats, tol);
    match rec.hashes.get(&platform_key()) {
        Some(h) if *h == hash => Outcome::Identical,
        Some(h) => panic!(
            "golden `{name}`: hash changed on {} ({h} -> {hash}) although features match; output is no longer bit-identical",
            platform_key()
        ),
        None => Outcome::FeaturesMatch,
    }
}

fn compare_features(name: &str, want: &Features, got: &Features, tol: &Tolerance) {
    assert_eq!(
        want.sample_rate, got.sample_rate,
        "golden `{name}`: sample rate"
    );
    assert_eq!(want.frames, got.frames, "golden `{name}`: length in frames");
    let close = |a: f32, b: f32| (a - b).abs() <= tol.level;
    assert!(
        close(want.peak, got.peak),
        "golden `{name}`: peak {} vs {}",
        want.peak,
        got.peak
    );
    assert!(
        close(want.rms, got.rms),
        "golden `{name}`: rms {} vs {}",
        want.rms,
        got.rms
    );
    for (i, (a, b)) in want
        .rms_per_second
        .iter()
        .zip(&got.rms_per_second)
        .enumerate()
    {
        assert!(
            close(*a, *b),
            "golden `{name}`: rms in second {i}: {a} vs {b}"
        );
    }
}

/// First sample index where two buffers differ (either channel), or None if identical.
pub fn first_difference(a: &StereoBuffer, b: &StereoBuffer) -> Option<usize> {
    let n = a.len().min(b.len());
    (0..n)
        .find(|&i| {
            a.left[i].to_bits() != b.left[i].to_bits()
                || a.right[i].to_bits() != b.right[i].to_bits()
        })
        .or(if a.len() != b.len() { Some(n) } else { None })
}

/// True when `a` and `b` are bit-identical in the sample range `range`.
pub fn identical_in(a: &StereoBuffer, b: &StereoBuffer, range: std::ops::Range<usize>) -> bool {
    range.clone().all(|i| {
        i < a.len()
            && i < b.len()
            && a.left[i].to_bits() == b.left[i].to_bits()
            && a.right[i].to_bits() == b.right[i].to_bits()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_difference() {
        let mut a = StereoBuffer::silent(100);
        let b = a.clone();
        assert_eq!(audio_hash(&a), audio_hash(&b));
        assert_eq!(first_difference(&a, &b), None);
        a.right[42] = 0.5;
        assert_ne!(audio_hash(&a), audio_hash(&b));
        assert_eq!(first_difference(&a, &b), Some(42));
        assert!(identical_in(&a, &b, 0..42));
        assert!(!identical_in(&a, &b, 0..43));
        let f = features(&a, 50);
        assert_eq!(f.rms_per_second.len(), 2);
    }
}
