//! `library add|list|show|tag|remove`: the reference library (FR-19).
//!
//! Saved Sound Profiles, tagged and named, reusable as styles. **Profiles only, never audio**:
//! `add` analyses a clip (or reads a profile JSON written by `analyze -o`) and keeps only the
//! profile. Entries live in `PRAXIS_HOME/library/<id>.profile.json`, one SoundProfile per file,
//! with the style name in `style_name` and the tags in `tags`.
//!
//! The id is the name in lowercase letters, digits and dashes (`Rain loop` -> `rain-loop`),
//! made unique with `-2`, `-3` ... Adding a clip whose decoded PCM hash is already in the library
//! adds nothing new: the existing entry gains the new tags (and the new name, when one is given)
//! and the payload says `already: true`.

use crate::error::{ErrorCode, OpError, io_error};
use crate::ops::analyze::{payload, profile_clip, summary};
use crate::ops::ensure_parent;
use crate::ops::sound::praxis_home;
use praxis_analyze::SoundProfile;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// What a `library` call does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum LibraryAction {
    /// Analyse a clip (or read a profile JSON) and save its profile.
    Add {
        input: PathBuf,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default = "four")]
        beats_per_bar: u8,
    },
    /// Every entry, in id order.
    List,
    /// One entry's full profile.
    Show { id: String },
    /// Add and remove tags.
    Tag {
        id: String,
        #[serde(default)]
        add: Vec<String>,
        #[serde(default)]
        remove: Vec<String>,
    },
    /// Delete an entry (its profile file).
    Remove { id: String },
}

fn four() -> u8 {
    4
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryRequest {
    #[serde(flatten)]
    pub action: LibraryAction,
    /// Library folder; default `PRAXIS_HOME/library`.
    #[serde(default)]
    pub dir: Option<PathBuf>,
}

/// `PRAXIS_HOME/library`.
pub fn library_dir() -> PathBuf {
    praxis_home().join("library")
}

const EXT: &str = ".profile.json";

/// `Rain loop!` -> `rain-loop` (empty names become `profile`).
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() {
        "profile".to_string()
    } else {
        s.chars().take(60).collect()
    }
}

fn entry_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}{EXT}"))
}

fn check_id(id: &str) -> Result<(), OpError> {
    let ok = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(OpError::new(
            ErrorCode::Error,
            format!(
                "`{id}` is not a library id (lowercase letters, digits and dashes; see `library list`)"
            ),
        ))
    }
}

fn read_entry(path: &Path) -> Result<SoundProfile, OpError> {
    let text = std::fs::read_to_string(path).map_err(|e| io_error(path, e))?;
    serde_json::from_str(&text)
        .map_err(|e| OpError::new(ErrorCode::BadJson, format!("{}: {e}", path.display())))
}

fn write_entry(path: &Path, p: &SoundProfile) -> Result<(), OpError> {
    ensure_parent(path)?;
    let text = serde_json::to_string_pretty(p).unwrap_or_default() + "\n";
    std::fs::write(path, text).map_err(|e| io_error(path, e))
}

/// Every `(id, path)` in the library, sorted by id.
fn entries(dir: &Path) -> Result<Vec<(String, PathBuf)>, OpError> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).map_err(|e| io_error(dir, e))? {
        let e = e.map_err(|e| io_error(dir, e))?;
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(id) = name.strip_suffix(EXT) {
            out.push((id.to_string(), e.path()));
        }
    }
    out.sort();
    Ok(out)
}

fn find(dir: &Path, id: &str) -> Result<PathBuf, OpError> {
    check_id(id)?;
    let p = entry_path(dir, id);
    if p.is_file() {
        Ok(p)
    } else {
        Err(OpError::new(
            ErrorCode::Io,
            format!(
                "no library entry `{id}` in {} (see `library list`)",
                dir.display()
            ),
        ))
    }
}

fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        let t = t.trim();
        if !t.is_empty() && !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
    }
    out
}

/// The short listing of one entry.
pub fn entry_summary(id: &str, path: &Path, p: &SoundProfile) -> Value {
    json!({
        "id": id,
        "path": path.display().to_string(),
        "name": p.style_name,
        "tags": p.tags,
        "kind": p.class.kind,
        "duration_s": p.source.duration_s,
        "tempo_bpm": p.tempo.as_ref().map(|t| t.bpm),
        "key": p.key.as_ref().map(|k| k.key.display()),
        "file_name": p.source.file_name,
        "pcm_hash": p.source.pcm_hash,
        "summary": summary(p),
    })
}

/// A profile from `input`: a JSON profile (as `analyze -o` writes) or an audio/video clip.
fn load_or_analyze(input: &Path, beats_per_bar: u8) -> Result<SoundProfile, OpError> {
    let is_json = input
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    if is_json {
        return read_entry(input);
    }
    Ok(profile_clip(input, beats_per_bar)?.0)
}

fn add(
    dir: &Path,
    input: &Path,
    name: Option<&str>,
    tags: &[String],
    beats_per_bar: u8,
) -> Result<Value, OpError> {
    let mut p = load_or_analyze(input, beats_per_bar)?;
    let tags = clean_tags(tags);
    let name = name.map(str::trim).filter(|n| !n.is_empty());
    // Same clip already kept?
    for (id, path) in entries(dir)? {
        let Ok(mut existing) = read_entry(&path) else {
            continue;
        };
        if !p.source.pcm_hash.is_empty() && existing.source.pcm_hash == p.source.pcm_hash {
            let mut all = existing.tags.clone();
            all.extend(tags.iter().cloned());
            existing.tags = clean_tags(&all);
            if let Some(n) = name {
                existing.style_name = Some(n.to_string());
            }
            write_entry(&path, &existing)?;
            return Ok(json!({
                "library": dir.display().to_string(),
                "id": id,
                "already": true,
                "entry": entry_summary(&id, &path, &existing),
            }));
        }
    }
    let stem = input
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = stem
        .strip_suffix(EXT)
        .or_else(|| stem.rsplit_once('.').map(|(a, _)| a))
        .unwrap_or(&stem)
        .to_string();
    let style = name
        .map(str::to_string)
        .or_else(|| p.style_name.clone())
        .unwrap_or(stem);
    let base = slug(&style);
    let mut id = base.clone();
    let mut n = 2;
    while entry_path(dir, &id).exists() {
        id = format!("{base}-{n}");
        n += 1;
    }
    let mut all = p.tags.clone();
    all.extend(tags);
    p.tags = clean_tags(&all);
    p.style_name = Some(style);
    let path = entry_path(dir, &id);
    write_entry(&path, &p)?;
    Ok(json!({
        "library": dir.display().to_string(),
        "id": id,
        "already": false,
        "entry": entry_summary(&id, &path, &p),
    }))
}

pub fn library(req: &LibraryRequest) -> Result<Value, OpError> {
    let dir = req.dir.clone().unwrap_or_else(library_dir);
    match &req.action {
        LibraryAction::Add {
            input,
            name,
            tags,
            beats_per_bar,
        } => {
            let mut v = add(&dir, input, name.as_deref(), tags, *beats_per_bar)?;
            v["action"] = json!("add");
            Ok(v)
        }
        LibraryAction::List => {
            let mut list = Vec::new();
            let mut unreadable = Vec::new();
            for (id, path) in entries(&dir)? {
                match read_entry(&path) {
                    Ok(p) => list.push(entry_summary(&id, &path, &p)),
                    Err(e) => unreadable.push(json!({"id": id, "error": e.message})),
                }
            }
            Ok(json!({
                "action": "list",
                "library": dir.display().to_string(),
                "count": list.len(),
                "entries": list,
                "unreadable": unreadable,
            }))
        }
        LibraryAction::Show { id } => {
            let path = find(&dir, id)?;
            let p = read_entry(&path)?;
            Ok(json!({
                "action": "show",
                "library": dir.display().to_string(),
                "id": id,
                "entry": entry_summary(id, &path, &p),
                "profile": payload(&p, None, None),
            }))
        }
        LibraryAction::Tag { id, add, remove } => {
            let path = find(&dir, id)?;
            let mut p = read_entry(&path)?;
            let remove = clean_tags(remove);
            let mut all: Vec<String> = p
                .tags
                .iter()
                .filter(|t| !remove.contains(t))
                .cloned()
                .collect();
            all.extend(add.iter().cloned());
            p.tags = clean_tags(&all);
            write_entry(&path, &p)?;
            Ok(json!({
                "action": "tag",
                "library": dir.display().to_string(),
                "id": id,
                "entry": entry_summary(id, &path, &p),
            }))
        }
        LibraryAction::Remove { id } => {
            let path = find(&dir, id)?;
            std::fs::remove_file(&path).map_err(|e| io_error(&path, e))?;
            Ok(json!({
                "action": "remove",
                "library": dir.display().to_string(),
                "id": id,
                "removed": true,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Rain loop!"), "rain-loop");
        assert_eq!(slug("  --Café 2 "), "caf-2");
        assert_eq!(slug("***"), "profile");
        assert!(check_id("rain-loop-2").is_ok());
        assert!(check_id("../x").is_err());
        assert!(check_id("").is_err());
    }
}
