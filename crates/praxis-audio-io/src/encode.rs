//! FLAC export and per-layer stems (FR-32). WP-E audio-io; see docs/contracts/audio-io.md.
//!
//! Quantisation matches [`crate::wav::write_wav`]: plain rounding of the clamped sample times
//! `2^(bits-1) - 1`, no dither, so exports are deterministic.

use crate::wav::{WavError, WavFormat, write_wav};
use praxis_dsp::StereoBuffer;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("FLAC export supports 16 or 24 bits, not {0}")]
    UnsupportedBits(u8),
    #[error("sample rate {0} Hz cannot be stored in FLAC")]
    UnsupportedRate(u32),
    #[error("FLAC encoder failed: {0}")]
    Flac(String),
    #[error(transparent)]
    Wav(#[from] WavError),
    #[error("cannot write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("two stems map to the same file name {0}")]
    DuplicateStem(String),
}

impl EncodeError {
    /// Stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            EncodeError::UnsupportedBits(_) => "unsupported_bits",
            EncodeError::UnsupportedRate(_) => "unsupported_rate",
            EncodeError::Flac(_) => "flac_encoder",
            EncodeError::Wav(_) => "wav_write",
            EncodeError::Io { .. } => "io",
            EncodeError::DuplicateStem(_) => "duplicate_stem",
        }
    }
}

/// Container and sample format for stems.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StemFormat {
    Wav(WavFormat),
    /// FLAC at 16 or 24 bits.
    Flac {
        bits: u8,
    },
}

impl Default for StemFormat {
    fn default() -> Self {
        StemFormat::Wav(WavFormat::default())
    }
}

impl StemFormat {
    pub fn extension(self) -> &'static str {
        match self {
            StemFormat::Wav(_) => "wav",
            StemFormat::Flac { .. } => "flac",
        }
    }
}

fn quantize(v: f32, full: f64) -> i32 {
    let x = if v.is_finite() { v as f64 } else { 0.0 };
    (x.clamp(-1.0, 1.0) * full).round() as i32
}

/// Encodes stereo audio as a FLAC stream in memory (16 or 24 bits).
pub fn flac_bytes(
    audio: &StereoBuffer,
    sample_rate: u32,
    bits: u8,
) -> Result<Vec<u8>, EncodeError> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;

    if bits != 16 && bits != 24 {
        return Err(EncodeError::UnsupportedBits(bits));
    }
    // STREAMINFO stores the rate in 20 bits.
    if sample_rate == 0 || sample_rate >= (1 << 20) {
        return Err(EncodeError::UnsupportedRate(sample_rate));
    }
    let full = ((1i64 << (bits - 1)) - 1) as f64;
    let mut samples = Vec::with_capacity(audio.len() * 2);
    for i in 0..audio.len() {
        samples.push(quantize(audio.left[i], full));
        samples.push(quantize(audio.right[i], full));
    }
    let mut config = flacenc::config::Encoder::default();
    // One thread: output does not depend on scheduling, and no threads are spawned per export.
    config.multithread = false;
    let config = config
        .into_verified()
        .map_err(|(_, e)| EncodeError::Flac(e.to_string()))?;
    let block_size = config.block_size;
    let source =
        flacenc::source::MemSource::from_samples(&samples, 2, bits as usize, sample_rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, block_size)
        .map_err(|e| EncodeError::Flac(e.to_string()))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| EncodeError::Flac(e.to_string()))?;
    Ok(sink.as_slice().to_vec())
}

/// Writes stereo audio as FLAC (16 or 24 bits).
pub fn write_flac(
    path: &Path,
    audio: &StereoBuffer,
    sample_rate: u32,
    bits: u8,
) -> Result<(), EncodeError> {
    let bytes = flac_bytes(audio, sample_rate, bits)?;
    std::fs::write(path, bytes).map_err(|source| EncodeError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// File stem for a layer: `[A-Za-z0-9_-]` kept, everything else becomes `_`.
pub fn stem_file_name(layer: &str, format: StemFormat) -> String {
    let mut name: String = layer
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if name.is_empty() {
        name.push_str("stem");
    }
    format!("{name}.{}", format.extension())
}

/// Writes one file per stem into `dir` (created if missing), named `<layer>.wav` or
/// `<layer>.flac`. Returns the written paths in input order. Fails before writing anything if
/// two layers would share a file name.
pub fn write_stems(
    dir: &Path,
    stems: &[(String, StereoBuffer)],
    sample_rate: u32,
    format: StemFormat,
) -> Result<Vec<PathBuf>, EncodeError> {
    let mut names = std::collections::BTreeSet::new();
    let mut paths = Vec::with_capacity(stems.len());
    for (layer, _) in stems {
        let name = stem_file_name(layer, format);
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(EncodeError::DuplicateStem(name));
        }
        paths.push(dir.join(name));
    }
    if let StemFormat::Flac { bits } = format
        && bits != 16
        && bits != 24
    {
        return Err(EncodeError::UnsupportedBits(bits));
    }
    std::fs::create_dir_all(dir).map_err(|source| EncodeError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for ((_, audio), path) in stems.iter().zip(&paths) {
        match format {
            StemFormat::Wav(f) => write_wav(path, audio, sample_rate, f)?,
            StemFormat::Flac { bits } => write_flac(path, audio, sample_rate, bits)?,
        }
    }
    Ok(paths)
}
