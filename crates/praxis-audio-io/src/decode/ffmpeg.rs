//! Decoding through an external `ffmpeg` executable (ADR-0004). ffmpeg is run as a separate
//! process, never linked, and is located only through `PRAXIS_FFMPEG` or absolute `PATH`
//! entries (never the current directory or the directory of the user's file).
//!
//! Command: `ffmpeg -nostdin -v error -protocol_whitelist file -i file:<path> -vn -ac 2
//! -f f32le -ar 48000 pipe:1`. The `file:` prefix and the protocol whitelist keep a crafted
//! file name or playlist from making ffmpeg open network or other protocol URLs.

use super::container::Probe;
use super::{DecodeError, DecodeOptions, Decoded, FfmpegSetting, Format, from_interleaved};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Output rate requested from ffmpeg.
pub(crate) const FFMPEG_RATE: u32 = 48_000;

/// The ffmpeg executable that [`super::decode_file`] would use: `PRAXIS_FFMPEG` if set and
/// pointing at a file, otherwise the first `ffmpeg` in an absolute `PATH` directory.
pub fn find_ffmpeg() -> Option<PathBuf> {
    resolve(&FfmpegSetting::Auto)
}

fn resolve(setting: &FfmpegSetting) -> Option<PathBuf> {
    if cfg!(target_family = "wasm") {
        return None;
    }
    match setting {
        FfmpegSetting::Disabled => None,
        FfmpegSetting::Path(p) => p.is_file().then(|| p.clone()),
        FfmpegSetting::Auto => {
            if let Some(p) = std::env::var_os("PRAXIS_FFMPEG").filter(|v| !v.is_empty()) {
                let p = PathBuf::from(p);
                return p.is_file().then_some(p);
            }
            let names: &[&str] = if cfg!(windows) {
                &["ffmpeg.exe"]
            } else {
                &["ffmpeg"]
            };
            let path = std::env::var_os("PATH")?;
            std::env::split_paths(&path)
                .filter(|dir| dir.is_absolute())
                .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
                .find(|p| p.is_file())
        }
    }
}

pub(crate) fn decode_path(
    path: &Path,
    format: Format,
    probe: Probe,
    opts: &DecodeOptions,
) -> Result<Decoded, DecodeError> {
    if probe.has_audio == Some(false) {
        return Err(DecodeError::NoAudioStream);
    }
    let Some(exe) = resolve(&opts.ffmpeg) else {
        return Err(missing(format));
    };
    run(&exe, path, format, probe)
}

pub(crate) fn decode_bytes(
    bytes: &[u8],
    hint: Option<&str>,
    format: Format,
    probe: Probe,
    opts: &DecodeOptions,
) -> Result<Decoded, DecodeError> {
    if probe.has_audio == Some(false) {
        return Err(DecodeError::NoAudioStream);
    }
    let Some(exe) = resolve(&opts.ffmpeg) else {
        return Err(missing(format));
    };
    let tmp = TempFile::create(bytes, hint)?;
    run(&exe, &tmp.path, format, probe)
}

fn missing(format: Format) -> DecodeError {
    if format == Format::Unknown {
        DecodeError::UnknownFormat
    } else {
        DecodeError::FfmpegMissing { format }
    }
}

fn run(exe: &Path, input: &Path, format: Format, probe: Probe) -> Result<Decoded, DecodeError> {
    let input = std::path::absolute(input).unwrap_or_else(|_| input.to_path_buf());
    let mut arg = std::ffi::OsString::from("file:");
    arg.push(input.as_os_str());
    let out = Command::new(exe)
        .args([
            "-nostdin",
            "-v",
            "error",
            "-protocol_whitelist",
            "file",
            "-i",
        ])
        .arg(arg)
        .args([
            "-vn",
            "-ac",
            "2",
            "-f",
            "f32le",
            "-ar",
            &FFMPEG_RATE.to_string(),
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let out = match out {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(missing(format)),
        Err(e) => {
            return Err(DecodeError::FfmpegFailed {
                status: "not started".into(),
                message: e.to_string(),
            });
        }
    };
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(classify_failure(&stderr, &out.status.to_string()));
    }
    if out.stdout.is_empty() && stderr.contains("does not contain any stream") {
        return Err(DecodeError::NoAudioStream);
    }
    let data: Vec<f32> = out
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| {
            let v = f32::from_le_bytes(*c);
            if v.is_finite() { v } else { 0.0 }
        })
        .collect();
    Ok(from_interleaved(&data, 2, FFMPEG_RATE, probe.kind))
}

fn classify_failure(stderr: &str, status: &str) -> DecodeError {
    let s = stderr.to_ascii_lowercase();
    if s.contains("does not contain any stream")
        || s.contains("matches no streams")
        || s.contains("output file is empty")
    {
        return DecodeError::NoAudioStream;
    }
    if s.contains("invalid data found when processing input") {
        return DecodeError::UnknownFormat;
    }
    let message: String = stderr.trim().chars().take(2000).collect();
    DecodeError::FfmpegFailed {
        status: status.to_string(),
        message,
    }
}

/// A uniquely named file in the system temp directory, removed on drop.
struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn create(bytes: &[u8], hint: Option<&str>) -> Result<TempFile, DecodeError> {
        use std::io::Write;
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        // Only a short alphanumeric extension from the hint survives; it helps ffmpeg's probe
        // for raw streams and cannot change the directory or form an option.
        let ext: String = hint
            .and_then(|h| Path::new(h).extension())
            .and_then(|e| e.to_str())
            .map(|e| {
                e.chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .take(8)
                    .collect()
            })
            .unwrap_or_default();
        let dir = std::env::temp_dir();
        let mut last_err = None;
        for _ in 0..16 {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let mut name = format!("praxis-decode-{}-{n}", std::process::id());
            if !ext.is_empty() {
                name.push('.');
                name.push_str(&ext);
            }
            let path = dir.join(name);
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    let tmp = TempFile { path };
                    f.write_all(bytes).map_err(|source| DecodeError::Io {
                        path: tmp.path.clone(),
                        source,
                    })?;
                    return Ok(tmp);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last_err = Some(e),
                Err(source) => return Err(DecodeError::Io { path, source }),
            }
        }
        Err(DecodeError::Io {
            path: dir,
            source: last_err.unwrap_or_else(|| std::io::Error::other("no free temp name")),
        })
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
