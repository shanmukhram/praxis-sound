//! WAV writing (FR-32). FROZEN CONTRACT for the signature.

use praxis_dsp::StereoBuffer;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WavFormat {
    Pcm16,
    #[default]
    Pcm24,
    Float32,
}

impl WavFormat {
    pub fn parse(s: &str) -> Option<WavFormat> {
        match s {
            "16" | "pcm16" | "wav16" => Some(WavFormat::Pcm16),
            "24" | "pcm24" | "wav24" => Some(WavFormat::Pcm24),
            "32f" | "f32" | "float32" | "wav32f" => Some(WavFormat::Float32),
            _ => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WavError {
    #[error(transparent)]
    Hound(#[from] hound::Error),
}

/// Writes a stereo WAV. PCM formats are TPDF-free (plain rounding, clamped) so output stays
/// deterministic; dithering is a later, seeded option.
pub fn write_wav(
    path: &Path,
    audio: &StereoBuffer,
    sample_rate: u32,
    format: WavFormat,
) -> Result<(), WavError> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: match format {
            WavFormat::Pcm16 => 16,
            WavFormat::Pcm24 => 24,
            WavFormat::Float32 => 32,
        },
        sample_format: match format {
            WavFormat::Float32 => hound::SampleFormat::Float,
            _ => hound::SampleFormat::Int,
        },
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for i in 0..audio.len() {
        for v in [audio.left[i], audio.right[i]] {
            match format {
                WavFormat::Pcm16 => w.write_sample(quantize(v, 32767.0) as i16)?,
                WavFormat::Pcm24 => w.write_sample(quantize(v, 8_388_607.0))?,
                WavFormat::Float32 => w.write_sample(v)?,
            }
        }
    }
    w.finalize()?;
    Ok(())
}

fn quantize(v: f32, full: f64) -> i32 {
    let x = if v.is_finite() { v as f64 } else { 0.0 };
    (x.clamp(-1.0, 1.0) * full).round() as i32
}

/// Reads any PCM/float WAV into a stereo buffer (mono duplicated). Returns (audio, sample rate).
pub fn read_wav(path: &Path) -> Result<(StereoBuffer, u32), WavError> {
    let mut r = hound::WavReader::open(path)?;
    let spec = r.spec();
    let data: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let full = (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>()
                .map(|s| s.map(|v| v as f32 / full))
                .collect::<Result<_, _>>()?
        }
    };
    Ok((
        StereoBuffer::from_interleaved(&data, spec.channels as usize),
        spec.sample_rate,
    ))
}
