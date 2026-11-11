//! R        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()) && e.path() != current)
ference clips in the desktop app (WP-U6; FR-11, FR-12, FR-19).
//!
//! The window sends a dropped clip's bytes (`clip_analyze`). They are kept under
//! `<home>/clips/` so the sound assistant can measure the file in full with its `analyze_clip`
//! tool when a request carries the clip, and measured with the shared command layer's
//! `ops::analyze` (the Sound Profile, `docs/contracts/sound-profile.md`). Only the newest
//! [`KEEP_CLIPS`] copies are kept. The WebView plays the dropped file itself for the A/B, so it
//! is never granted access to this folder.
//!
//! The reference library is the command layer's (`PRAXIS_HOME/library/`, profiles only, never
//! audio); the commands here wrap it in the shapes the front end reads.

use crate::commands::{CmdError, CmdResult};
use praxis_cli::ErrorCode;
use praxis_cli::ops;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The folder under the app's home where dropped clips are kept.
pub const CLIPS_DIR: &str = "clips";
/// How many dropped clips are kept (oldest removed first).
pub const KEEP_CLIPS: usize = 24;
/// Larger clips are refused (the front end refuses them before reading them).
pub const MAX_CLIP_BYTES: usize = 512 * 1024 * 1024;
/// Audio and video a clip may be (FR-11). Video and M4A are decoded by ffmpeg when present.
pub const CLIP_EXTS: [&str; 8] = ["wav", "mp3", "flac", "ogg", "m4a", "mp4", "mov", "webm"];
/// The request header carrying the clip's file name, percent-encoded.
pub const NAME_HEADER: &str = "x-clip-name";

/// A measured clip: where the app keeps it, and its Sound Profile.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Clip {
    pub path: String,
    pub name: String,
    pub profile: Value,
}

/// Decodes `%XX` escapes (the front end's `encodeURIComponent`); invalid escapes stay as they are.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The clip's file name without any folder, made safe for a file name; its extension lower-case.
/// `None` when it has no extension a clip may have.
pub fn safe_clip_name(name: &str) -> Option<String> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let (stem, ext) = base.rsplit_once('.')?;
    let ext = ext.to_ascii_lowercase();
    if !CLIP_EXTS.contains(&ext.as_str()) {
        return None;
    }
    let stem: String = stem
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ') {
                c
            } else {
                '-'
            }
        })
        .take(80)
        .collect();
    let stem = stem.trim_matches(|c: char| c == '-' || c == ' ');
    let stem = if stem.is_empty() { "clip" } else { stem };
    Some(format!("{stem}.{ext}"))
}

/// FNV-1a over the bytes: the same clip dropped twice is kept once.
fn content_id(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn io(path: &Path, e: impl std::fmt::Display) -> CmdError {
    CmdError::new(ErrorCode::Io, format!("{}: {e}", path.display()))
}

fn refuse(name: &str) -> CmdError {
    CmdError::new(
        ErrorCode::BadFormat,
        format!(
            "{name} is not a clip Praxis can read; drop audio (WAV, MP3, FLAC, OGG, M4A) or video (MP4, MOV, WebM)"
        ),
    )
}

/// Keeps a dropped clip's bytes under `<home>/clips/` and returns the file. The same bytes under
/// the same name are written once.
pub fn store_clip(home: &Path, name: &str, bytes: &[u8]) -> CmdResult<PathBuf> {
    let safe = safe_clip_name(name).ok_or_else(|| refuse(name))?;
    if bytes.is_empty() {
        return Err(CmdError::new(ErrorCode::BadFormat, format!("{name} is empty")));
    }
    if bytes.len() > MAX_CLIP_BYTES {
        return Err(CmdError::new(
            ErrorCode::BadFormat,
            format!("{name} is larger than 512 MB; trim it to the part you want to match"),
        ));
    }
    let dir = home.join(CLIPS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| io(&dir, e))?;
    let path = dir.join(format!("{}-{safe}", content_id(bytes)));
    let same = std::fs::metadata(&path).is_ok_and(|m| m.len() == bytes.len() as u64);
    if !same {
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, bytes).map_err(|e| io(&tmp, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| io(&path, e))?;
    }
    prune(&dir, KEEP_CLIPS, &path);
    Ok(path)
}

/// Keeps `current` and the `keep - 1` newest other clips; removes the rest.
fn prune(dir: &Path, keep: usize, current: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()) && e.path() != current)
        .map(|e| {
            let t = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (t, e.path())
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    for (_, p) in files.into_iter().skip(keep.saturating_sub(1)) {
        let _ = std::fs::remove_file(p);
    }
}

/// The decoder's own code when the command layer reported a decode failure ("... (ffmpeg_missing):
/// ..."), so the window can say what to do; the kept copy's path is replaced by the clip's name.
fn clip_error(e: praxis_cli::OpError, name: &str, path: &Path) -> CmdError {
    let mut err = CmdError::from(e);
    for code in ["ffmpeg_missing", "ffmpeg_failed", "unsupported_format"] {
        if err.message.contains(&format!("({code})")) {
            err.code = code.into();
        }
    }
    err.message = err.message.replace(&path.display().to_string(), name);
    err
}

/// Keeps and measures a dropped clip: its file and its Sound Profile.
pub fn analyze_clip(home: &Path, name: &str, bytes: &[u8]) -> CmdResult<Clip> {
    let path = store_clip(home, name, bytes)?;
    let profile = ops::analyze(&ops::AnalyzeRequest {
        clip: path.clone(),
        out: None,
        beats_per_bar: 4,
    })
    .map_err(|e| clip_error(e, name, &path))?;
    Ok(Clip {
        path: path.display().to_string(),
        name: name.rsplit(['/', '\\']).next().unwrap_or(name).to_string(),
        profile,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_made_safe_and_checked() {
        assert_eq!(safe_clip_name("Rain loop.WAV").as_deref(), Some("Rain loop.wav"));
        assert_eq!(safe_clip_name(r"C:\x\..\hit.mp4").as_deref(), Some("hit.mp4"));
        assert_eq!(safe_clip_name("../../evil.wav").as_deref(), Some("evil.wav"));
        assert_eq!(safe_clip_name("a/b:c?.flac").as_deref(), Some("b-c.flac"));
        assert_eq!(safe_clip_name("notes.txt"), None);
        assert_eq!(safe_clip_name("noext"), None);
        assert_eq!(safe_clip_name(".wav").as_deref(), Some("clip.wav"));
    }

    #[test]
    fn percent_escapes_decode() {
        assert_eq!(percent_decode("rain%20loop%20%C3%A9.wav"), "rain loop é.wav");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz.wav"), "%zz.wav");
    }

    #[test]
    fn only_the_newest_clips_are_kept() {
        let home = std::env::temp_dir().join(format!("praxis-clips-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let mut last = PathBuf::new();
        for i in 0..(KEEP_CLIPS + 3) {
            last = store_clip(&home, &format!("c{i}.wav"), &[i as u8 + 1; 8]).unwrap();
        }
        let n = std::fs::read_dir(home.join(CLIPS_DIR)).unwrap().count();
        assert_eq!(n, KEEP_CLIPS);
        assert!(last.is_file());
        // The same bytes under the same name are kept once.
        let again = store_clip(&home, &format!("c{}.wav", KEEP_CLIPS + 2), &[(KEEP_CLIPS + 3) as u8; 8]).unwrap();
        assert_eq!(again, last);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn refuses_other_files_and_empty_clips() {
        let home = std::env::temp_dir().join(format!("praxis-clips-refuse-{}", std::process::id()));
        assert_eq!(store_clip(&home, "a.txt", b"x").unwrap_err().code, "bad_format");
        assert_eq!(store_clip(&home, "a.wav", b"").unwrap_err().code, "bad_format");
        let _ = std::fs::remove_dir_all(&home);
    }
}
