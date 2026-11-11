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
        let _ = std::fs::remove_file(profile_cache(&p));
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

/// Where the profile of a kept clip is cached (`<home>/clips/profiles/<clip file>.profile.json`):
/// saving the clip to the library adds exactly this profile, without measuring it again.
pub fn profile_cache(clip: &Path) -> PathBuf {
    let file = clip
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    clip.with_file_name(PROFILES_DIR)
        .join(format!("{file}{PROFILE_EXT}"))
}

const PROFILES_DIR: &str = "profiles";
const PROFILE_EXT: &str = ".profile.json";

/// Keeps and measures a dropped clip: its file and its Sound Profile (the `analyze` payload).
pub fn analyze_clip(home: &Path, name: &str, bytes: &[u8]) -> CmdResult<Clip> {
    let path = store_clip(home, name, bytes)?;
    let cache = profile_cache(&path);
    if let Some(dir) = cache.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    }
    let mut profile = ops::analyze(&ops::AnalyzeRequest {
        clip: path.clone(),
        out: Some(cache),
        beats_per_bar: 4,
    })
    .map_err(|e| clip_error(e, name, &path))?;
    if let Some(o) = profile.as_object_mut() {
        o.remove("output");
    }
    Ok(Clip {
        path: path.display().to_string(),
        name: name.rsplit(['/', '\\']).next().unwrap_or(name).to_string(),
        profile,
    })
}

// ------------------------------------------------------------------ the reference library

/// A library entry as the window reads it: the `list` entry's fields, and the profile itself
/// when it was asked for (`show`) or just saved.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LibraryEntry {
    pub id: String,
    pub name: String,
    pub tags: Vec<String>,
    pub kind: String,
    pub duration_s: f64,
    /// One line in plain words (`ops::analyze::summary`).
    pub summary: String,
    /// The analyze payload of the saved profile; `None` in a listing.
    pub profile: Option<Value>,
    /// The same clip (by its decoded PCM hash) was already kept: that entry got the name and tags.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub already: bool,
}

fn entry_of(e: &Value, profile: Option<Value>, already: bool) -> LibraryEntry {
    let s = |k: &str| e[k].as_str().unwrap_or("").to_string();
    let id = s("id");
    LibraryEntry {
        name: e["name"].as_str().map(str::to_string).unwrap_or_else(|| id.clone()),
        id,
        tags: e["tags"]
            .as_array()
            .map(|t| t.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default(),
        kind: s("kind"),
        duration_s: e["duration_s"].as_f64().unwrap_or(0.0),
        summary: s("summary"),
        profile,
        already,
    }
}

/// The library folder of the app's home (`<home>/library`: `PRAXIS_HOME/library` unless the
/// app keeps its sounds in the fallback home).
pub fn library_dir(home: &Path) -> PathBuf {
    home.join("library")
}

fn lib(home: &Path, action: ops::LibraryAction) -> CmdResult<Value> {
    Ok(ops::library(&ops::LibraryRequest {
        action,
        dir: Some(library_dir(home)),
    })?)
}

/// Every entry, in id order (as `list` gives them), without their profiles.
pub fn library_list(home: &Path) -> CmdResult<Vec<LibraryEntry>> {
    let v = lib(home, ops::LibraryAction::List)?;
    Ok(v["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| entry_of(e, None, false))
        .collect())
}

/// One entry with its profile.
pub fn library_show(home: &Path, id: &str) -> CmdResult<LibraryEntry> {
    let v = lib(home, ops::LibraryAction::Show { id: id.into() })?;
    Ok(entry_of(&v["entry"], Some(v["profile"].clone()), false))
}

/// The kept clip `clip` names, when it really is one of this home's dropped clips.
fn kept_clip(home: &Path, clip: &str) -> Option<PathBuf> {
    let root = std::fs::canonicalize(home.join(CLIPS_DIR)).ok()?;
    let p = std::fs::canonicalize(clip).ok()?;
    (p.starts_with(&root) && p.is_file()).then_some(p)
}

/// Saves a profile to the library under `name` with `tags`: the cached profile of the kept clip
/// `clip` (measured again if the cache is gone), else `profile` as the window has it.
pub fn library_add(
    home: &Path,
    name: &str,
    tags: &[String],
    clip: Option<&str>,
    profile: Option<&Value>,
) -> CmdResult<LibraryEntry> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CmdError::new(ErrorCode::BadJson, "give the reference a name"));
    }
    let kept = clip.and_then(|c| kept_clip(home, c));
    let mut temp = None;
    let input = match (&kept, profile) {
        (Some(c), _) if profile_cache(c).is_file() => profile_cache(c),
        (Some(c), _) => c.clone(),
        (None, Some(p)) => {
            let t = std::env::temp_dir().join(format!(
                "praxis-app-{}-{}{PROFILE_EXT}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            let text = serde_json::to_string(p)
                .map_err(|e| CmdError::new(ErrorCode::BadJson, e.to_string()))?;
            std::fs::write(&t, text).map_err(|e| io(&t, e))?;
            temp = Some(t.clone());
            t
        }
        (None, None) => {
            return Err(CmdError::new(
                ErrorCode::BadJson,
                "send the kept clip or its profile to save",
            ));
        }
    };
    let r = lib(
        home,
        ops::LibraryAction::Add {
            input,
            name: Some(name.into()),
            tags: tags.to_vec(),
            beats_per_bar: 4,
        },
    );
    if let Some(t) = temp {
        let _ = std::fs::remove_file(t);
    }
    let v = r?;
    let id = v["id"].as_str().unwrap_or_default().to_string();
    let mut entry = library_show(home, &id)?;
    entry.already = v["already"].as_bool().unwrap_or(false);
    Ok(entry)
}

/// Sets an entry's tags to exactly `tags` (the library's `tag` adds and removes).
pub fn library_tag(home: &Path, id: &str, tags: &[String]) -> CmdResult<LibraryEntry> {
    let now = library_show(home, id)?.tags;
    let want: Vec<String> = tags.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    let add = want.iter().filter(|t| !now.contains(t)).cloned().collect();
    let remove = now.iter().filter(|t| !want.contains(t)).cloned().collect();
    let v = lib(
        home,
        ops::LibraryAction::Tag {
            id: id.into(),
            add,
            remove,
        },
    )?;
    Ok(entry_of(&v["entry"], None, false))
}

pub fn library_remove(home: &Path, id: &str) -> CmdResult<bool> {
    let v = lib(home, ops::LibraryAction::Remove { id: id.into() })?;
    Ok(v["removed"].as_bool().unwrap_or(false))
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
