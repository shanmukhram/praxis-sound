//! Decoding of dropped clips (FR-11) and offline resampling. WP-E audio-io; see
//! docs/contracts/audio-io.md and docs/adr/0004-decoding-without-mpl.md.
//!
//! The format is detected from the first bytes of the data, never from the file extension.
//! WAV (hound, with a small fallback reader for 64-bit float and other layouts hound rejects),
//! FLAC (claxon), Ogg Vorbis (lewton) and MP3 (nanomp3) are decoded in-process. Everything else
//! (M4A/AAC, MP4, MOV, WebM/MKV, Opus, AIFF, ...) is handed to an external `ffmpeg` executable,
//! found through `PRAXIS_FFMPEG` or an absolute `PATH` entry. ffmpeg is never linked or shipped,
//! and nothing found inside a user file is ever executed.

mod container;
mod detect;
mod ffmpeg;
mod native;

use praxis_dsp::StereoBuffer;
use std::path::{Path, PathBuf};

pub use detect::{Format, detect_format};
pub use ffmpeg::find_ffmpeg;

/// Whether the decoded clip came from an audio file or the audio track of a video.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Audio,
    Video,
}

/// A decoded clip at its native sample rate (48 kHz when ffmpeg decoded it).
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    /// Stereo audio. Mono sources are duplicated to both sides; sources with more than two
    /// channels keep their first two.
    pub audio: StereoBuffer,
    pub sample_rate: u32,
    /// Channel count of the source stream as stored in the file. For clips decoded through
    /// ffmpeg (which is asked to downmix with `-ac 2`) this is 2.
    pub channels: u16,
    pub duration_s: f64,
    pub source_kind: SourceKind,
}

/// Where to find ffmpeg for the formats that are not decoded natively.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum FfmpegSetting {
    /// `PRAXIS_FFMPEG` if set, otherwise the first `ffmpeg` executable in an absolute `PATH`
    /// directory.
    #[default]
    Auto,
    /// This executable, exactly.
    Path(PathBuf),
    /// Never run ffmpeg; those formats report `ffmpeg_missing`.
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DecodeOptions {
    pub ffmpeg: FfmpegSetting,
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("not a recognised audio or video file")]
    UnknownFormat,
    #[error("invalid WAV file: {0}")]
    Wav(String),
    #[error("invalid FLAC file: {0}")]
    Flac(String),
    #[error("invalid Ogg Vorbis file: {0}")]
    Vorbis(String),
    #[error("invalid MP3 file: {0}")]
    Mp3(String),
    #[error(
        "{format:?} files need ffmpeg, which was not found; install ffmpeg or set PRAXIS_FFMPEG \
         to its path"
    )]
    FfmpegMissing { format: Format },
    #[error("ffmpeg could not decode the file ({status}): {message}")]
    FfmpegFailed { status: String, message: String },
    #[error("the file has no audio stream")]
    NoAudioStream,
}

impl DecodeError {
    /// Stable machine-readable code for JSON output and the UI.
    pub fn code(&self) -> &'static str {
        match self {
            DecodeError::Io { .. } => "io",
            DecodeError::UnknownFormat => "unknown_format",
            DecodeError::Wav(_) => "invalid_wav",
            DecodeError::Flac(_) => "invalid_flac",
            DecodeError::Vorbis(_) => "invalid_vorbis",
            DecodeError::Mp3(_) => "invalid_mp3",
            DecodeError::FfmpegMissing { .. } => "ffmpeg_missing",
            DecodeError::FfmpegFailed { .. } => "ffmpeg_failed",
            DecodeError::NoAudioStream => "no_audio_stream",
        }
    }
}

/// Decodes an audio or video file. Equivalent to [`decode_file_with`] with default options.
pub fn decode_file(path: &Path) -> Result<Decoded, DecodeError> {
    decode_file_with(path, &DecodeOptions::default())
}

/// Decodes an audio or video file, detecting the format from its content.
pub fn decode_file_with(path: &Path, opts: &DecodeOptions) -> Result<Decoded, DecodeError> {
    let io_err = |source| DecodeError::Io {
        path: path.to_path_buf(),
        source,
    };
    let head = read_head(path, detect::HEAD_LEN).map_err(io_err)?;
    let format = detect_format(&head);
    if format.is_native() {
        let bytes = std::fs::read(path).map_err(io_err)?;
        match native::decode(format, &bytes) {
            Err(native::NativeError::NeedsFfmpeg) => {}
            other => return other.map_err(native::NativeError::into_decode_error),
        }
    } else if format == Format::Unknown && head.is_empty() {
        return Err(DecodeError::UnknownFormat);
    }
    let kind = container::source_kind_of_file(format, path);
    ffmpeg::decode_path(path, format, kind, opts)
}

/// Decodes a clip held in memory (drag and drop in the browser build). `hint` is the original
/// file name, if known; it only names the temporary file handed to ffmpeg and is never used
/// to choose the decoder.
pub fn decode_bytes(bytes: &[u8], hint: Option<&str>) -> Result<Decoded, DecodeError> {
    decode_bytes_with(bytes, hint, &DecodeOptions::default())
}

/// [`decode_bytes`] with options.
pub fn decode_bytes_with(
    bytes: &[u8],
    hint: Option<&str>,
    opts: &DecodeOptions,
) -> Result<Decoded, DecodeError> {
    let format = detect_format(bytes);
    if format.is_native() {
        match native::decode(format, bytes) {
            Err(native::NativeError::NeedsFfmpeg) => {}
            other => return other.map_err(native::NativeError::into_decode_error),
        }
    } else if format == Format::Unknown && bytes.is_empty() {
        return Err(DecodeError::UnknownFormat);
    }
    let kind = container::source_kind_of_bytes(format, bytes);
    ffmpeg::decode_bytes(bytes, hint, format, kind, opts)
}

fn read_head(path: &Path, len: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path)?;
    let mut head = Vec::with_capacity(len);
    f.take(len as u64).read_to_end(&mut head)?;
    Ok(head)
}

/// Builds a [`Decoded`] from interleaved samples.
pub(crate) fn from_interleaved(
    data: &[f32],
    channels: u16,
    sample_rate: u32,
    source_kind: SourceKind,
) -> Decoded {
    let audio = StereoBuffer::from_interleaved(data, channels.max(1) as usize);
    let duration_s = if sample_rate > 0 {
        audio.len() as f64 / sample_rate as f64
    } else {
        0.0
    };
    Decoded {
        audio,
        sample_rate,
        channels,
        duration_s,
        source_kind,
    }
}

/// Offline, deterministic sample-rate conversion with rubato's synchronous FFT resampler.
/// Returns a copy when the rates are equal (or either is zero). The output has
/// `ceil(len * to / from)` frames and is aligned with the input (the resampler delay is
/// trimmed).
pub fn resample(audio: &StereoBuffer, from_hz: u32, to_hz: u32) -> StereoBuffer {
    use rubato::Resampler;
    use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;

    if from_hz == to_hz || from_hz == 0 || to_hz == 0 {
        return audio.clone();
    }
    if audio.is_empty() {
        return StereoBuffer::default();
    }
    let frames = audio.len();
    let input: Vec<Vec<f64>> = [&audio.left, &audio.right]
        .iter()
        .map(|ch| {
            ch.iter()
                .map(|&v| if v.is_finite() { v as f64 } else { 0.0 })
                .collect()
        })
        .collect();
    let expected = ((frames as u128 * to_hz as u128).div_ceil(from_hz as u128)) as usize;
    let resampled = (|| {
        let adapter = SequentialSliceOfVecs::new(&input, 2, frames).ok()?;
        let mut rs = rubato::Fft::<f64>::new(
            from_hz as usize,
            to_hz as usize,
            1024,
            2,
            rubato::FixedSync::Both,
        )
        .ok()?;
        rs.process_all(&adapter, frames, None).ok()
    })();
    let Some(out) = resampled else {
        // Construction only fails for invalid rates, which are excluded above.
        return StereoBuffer::silent(expected);
    };
    let data = out.take_data();
    // rubato sizes the output with a floating-point ceil, which can add one frame; the length
    // here is the exact integer ceil.
    let n = (data.len() / 2).min(expected);
    let mut b = StereoBuffer::silent(expected);
    for i in 0..n {
        b.left[i] = data[2 * i] as f32;
        b.right[i] = data[2 * i + 1] as f32;
    }
    b
}
