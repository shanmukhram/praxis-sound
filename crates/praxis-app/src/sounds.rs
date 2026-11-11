//! Where sounds live and what the sounds list shows.
//!
//! A sound is a folder (`project.praxis`, `takes.json`, `takes/<n>/mix.wav`) written by the
//! command layer's take ops. The app only lists those folders and names new ones; it never
//! writes a take itself, so the app and agents (through the MCP server) share one history.

pub use praxis_cli::ops::sound::{PROJECT_FILE, TAKES_FILE};
use praxis_cli::ops::{load_score, praxis_home, score_summary};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// The Praxis Sound home folder (`PRAXIS_HOME`, default `Documents\Praxis Sound`), as the
/// command layer resolves it.
pub fn home() -> PathBuf {
    praxis_home()
}

/// The folder that holds one sub-folder per sound.
pub fn sounds_root(home: &Path) -> PathBuf {
    home.join("sounds")
}

/// One row of the sounds list.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SoundInfo {
    /// The folder name; stable while the sound exists.
    pub id: String,
    /// The score title, or the folder name before the first take.
    pub name: String,
    pub path: String,
    /// "A minor · 0:30 loop", or "New" before the first take.
    pub sub: String,
    pub takes: usize,
    /// Milliseconds since the Unix epoch of the newest change (for sorting, newest first).
    pub modified_ms: u64,
}

fn modified_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

/// The `takes` array length of a sound's `takes.json` (0 when there is none yet).
pub fn take_count(sound: &Path) -> usize {
    read_takes_json(sound)
        .and_then(|v| v.get("takes").and_then(Value::as_array).map(Vec::len))
        .unwrap_or(0)
}

/// The parsed `takes.json`, if it exists and is JSON.
pub fn read_takes_json(sound: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(sound.join(TAKES_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// "A minor · 0:30 loop" from the sound's project.
fn describe(project: &Path) -> Option<(String, String)> {
    let score = load_score(project).ok()?;
    let s = score_summary(&score);
    let secs = s["duration_s"].as_f64().unwrap_or(0.0).round() as u64;
    let looped = s["playback"]["mode"].as_str() == Some("loop");
    // "a minor" -> "A minor", as the app writes keys everywhere else.
    let key = s["key"].as_str().unwrap_or("?");
    let mut chars = key.chars();
    let key: String = chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    let sub = format!(
        "{} · {}:{:02}{}",
        key,
        secs / 60,
        secs % 60,
        if looped { " loop" } else { "" }
    );
    let title = s["title"].as_str().unwrap_or("").trim().to_string();
    Some((title, sub))
}

/// One sound folder as a list row. `None` when `dir` is not a sound (no project, no takes).
pub fn sound_info(dir: &Path) -> Option<SoundInfo> {
    let id = dir.file_name()?.to_string_lossy().into_owned();
    let project = dir.join(PROJECT_FILE);
    let takes_file = dir.join(TAKES_FILE);
    if !project.is_file() && !takes_file.is_file() {
        return None;
    }
    let (title, sub) = describe(&project).unwrap_or_default();
    let name = if title.is_empty() { id.clone() } else { title };
    Some(SoundInfo {
        name,
        path: dir.display().to_string(),
        sub: if sub.is_empty() { "New".into() } else { sub },
        takes: take_count(dir),
        modified_ms: modified_ms(&takes_file).max(modified_ms(&project)),
        id,
    })
}

/// Every sound under `root`, newest first. A missing root is an empty list.
pub fn list_sounds(root: &Path) -> Vec<SoundInfo> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<SoundInfo> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| sound_info(&e.path()))
        .collect();
    out.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms).then(a.id.cmp(&b.id)));
    out
}

/// A folder name from the first words of a request: "a calm 30-second felt piano loop in A
/// minor" -> "calm-30-second-felt-piano-loop". Never empty, never a reserved Windows name.
pub fn slug(text: &str) -> String {
    const SKIP: [&str; 10] = [
        "a", "an", "the", "make", "in", "of", "with", "and", "for", "to",
    ];
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .map(|w| w.trim_matches('-').to_lowercase())
        .filter(|w| !w.is_empty() && !SKIP.contains(&w.as_str()))
        .take(5)
        .collect();
    let mut s = words.join("-");
    s.truncate(48);
    let s = s.trim_end_matches('-').to_string();
    const RESERVED: [&str; 6] = ["con", "prn", "aux", "nul", "com1", "lpt1"];
    if s.is_empty() || RESERVED.contains(&s.as_str()) {
        "sound".into()
    } else {
        s
    }
}

/// A new, not yet existing folder under `root` for a sound started by `text`
/// (`calm-felt-piano-loop`, then `calm-felt-piano-loop-2`, ...). Does not create it.
pub fn new_sound_path(root: &Path, text: &str) -> PathBuf {
    let base = slug(text);
    let first = root.join(&base);
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|i| root.join(format!("{base}-{i}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_takes_the_first_words() {
        assert_eq!(
            slug("A calm 30-second felt piano loop in A minor"),
            "calm-30-second-felt-piano-loop"
        );
        assert_eq!(slug("   "), "sound");
        assert_eq!(slug("NUL"), "sound");
        assert_eq!(slug("Make it darker!"), "it-darker");
    }

    #[test]
    fn new_sound_paths_do_not_collide() {
        let dir = std::env::temp_dir().join(format!("praxis-app-slug-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bell-loop")).unwrap();
        assert_eq!(new_sound_path(&dir, "bell loop"), dir.join("bell-loop-2"));
        assert_eq!(new_sound_path(&dir, "kick"), dir.join("kick"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
