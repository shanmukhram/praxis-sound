//! A sound folder: the shared take history people and agents work on (ADR-0011).
//!
//! ```text
//! <sound>/
//!   project.praxis        the project (score, undo/redo, versions `take-1`, `take-2`, ...)
//!   takes.json            {format: "praxis-takes", version: 1, revision, current, takes[], views[]}
//!   takes/<n>/mix.wav     each take's master (24-bit WAV, 48 kHz)
//!   variations/<i>.wav    candidates from `variations` (none committed until kept)
//!   conversation.json     the sound assistant's history (`crate::agent::conversation`)
//!   agent/*.wav           the assistant's working renders (not takes)
//! ```
//!
//! Every write goes to a temporary file in the folder and is renamed over the old one, and every
//! write of `takes.json` bumps `revision`, so a watcher (the desktop app) sees a whole file and
//! can tell that a take was made by someone else (an agent through the MCP server). Writers take
//! the folder's `.lock` file for the whole read-modify-write.
//!
//! Where sounds live: a `sound` that names a folder (it contains a path separator, or is
//! absolute) is used as given; a bare name resolves to `PRAXIS_HOME/sounds/<name>`, where
//! `PRAXIS_HOME` defaults to `%USERPROFILE%\Praxis Sound` (`$HOME/Praxis Sound` elsewhere).
//! Not `Documents`: Windows' Controlled Folder Access (ransomware protection) blocks unknown
//! programs from writing there and reports it as "file not found" (owner's decision, 2026-11-11).

use crate::error::{ErrorCode, OpError, io_error};
use praxis_score::BarRange;
use praxis_score::patch::ScorePatch;
use praxis_score::project::Project;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const TAKES_FORMAT: &str = "praxis-takes";
pub const TAKES_VERSION: u32 = 1;
pub const PROJECT_FILE: &str = "project.praxis";
pub const TAKES_FILE: &str = "takes.json";
pub const LOCK_FILE: &str = ".lock";
/// Overrides where bare sound names live.
pub const HOME_ENV: &str = "PRAXIS_HOME";
/// View requests kept in `takes.json` (oldest dropped first).
pub const MAX_VIEWS: usize = 50;

/// How long a writer waits for another writer's lock before giving up.
const LOCK_WAIT: Duration = Duration::from_secs(600);
/// A lock older than this is left over from a crashed writer and is broken.
const LOCK_STALE: Duration = Duration::from_secs(1800);

/// A panel view of the app (`ViewName` in app/src/engine/EngineAdapter.ts).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewName {
    Overview,
    Rhythm,
    Notes,
    Mix,
    Export,
}

/// The scope of a pointed-at request: bars of one layer (`TakeScope`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TakeScope {
    /// Layer id.
    pub layer: String,
    /// 1-based, inclusive.
    pub bars: BarRange,
}

/// Where a take's audio is and what the engine measured (`RenderResult`: `audioUrl` <- `path`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TakeRender {
    /// In `takes.json`: relative to the sound folder (`takes/<n>/mix.wav`, forward slashes).
    /// In command payloads: absolute.
    pub path: String,
    /// The TypeScript `RenderReport`: `mock, sample_rate, length_samples, tail_samples, is_loop,
    /// loudness_before_lufs, loudness_lufs, true_peak_dbtp, applied_gain_db, warmup_cycles,
    /// notes_rendered, blocked_layers, render_seconds, warnings`.
    pub report: Value,
}

/// One take (`Take` in app/src/engine/EngineAdapter.ts, field for field; its `score` is the
/// project version named [`TakeRecord::version`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TakeRecord {
    /// 1-based take number within the sound.
    pub n: u32,
    /// `"You"` or `"Agent · <name>"`.
    pub who: String,
    /// The request as asked (with the scope appended when there was one).
    pub ask: String,
    pub title: String,
    /// The short plain-language reply.
    pub say: String,
    /// The patch that made this take from the take before it; null for a new sound.
    pub patch: Option<ScorePatch>,
    /// Bars each changed layer was changed in: `{layer: [from, to]}`. Empty for a new sound.
    pub changed: BTreeMap<String, [u32; 2]>,
    /// What stayed identical, in words (measured on the renders).
    pub untouched: String,
    pub render: TakeRender,
    /// The verify report (`VerifyReport`) plus `passed`, `failed[]`, `inconclusive[]`, `summary` and
    /// `layers_checked[]`.
    pub checks: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opens: Option<ViewName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<TakeScope>,
    /// Extra: the project version holding this take's score (`take-<n>`).
    pub version: String,
    /// Extra: the take this one was made from (0 for a new sound).
    #[serde(default)]
    pub from: u32,
    /// Extra: runs of bars whose master samples are bit-identical to take `from`.
    #[serde(default)]
    pub identical_bars: Vec<[u32; 2]>,
    /// Extra: the planner that answered (`offline-rules`, `claude`, `direct-plan`, `hand`).
    #[serde(default)]
    pub provider: String,
    /// Extra: the planner / arranger seed.
    #[serde(default)]
    pub seed: u64,
    /// The reference clip or library style the request came with (`ask`'s `reference`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<crate::ops::takes::ReferenceArg>,
}

/// A request to show a panel view ("Notes opened by cut-assistant").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewRequest {
    /// The `takes.json` revision this request was written in.
    pub revision: u64,
    pub view: ViewName,
    /// `"Praxis"` (for a person's request) or the agent's name.
    pub by: String,
    /// The take that was current.
    pub take: u32,
}

/// `takes.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TakesFile {
    pub format: String,
    pub version: u32,
    /// Bumped on every write.
    pub revision: u64,
    /// The take that is playing now (0 before the first take).
    pub current: u32,
    pub takes: Vec<TakeRecord>,
    #[serde(default)]
    pub views: Vec<ViewRequest>,
}

impl Default for TakesFile {
    fn default() -> Self {
        TakesFile {
            format: TAKES_FORMAT.into(),
            version: TAKES_VERSION,
            revision: 0,
            current: 0,
            takes: Vec::new(),
            views: Vec::new(),
        }
    }
}

impl TakesFile {
    pub fn take(&self, n: u32) -> Option<&TakeRecord> {
        self.takes.iter().find(|t| t.n == n)
    }

    pub fn next_n(&self) -> u32 {
        self.takes.iter().map(|t| t.n).max().unwrap_or(0) + 1
    }
}

/// `PRAXIS_HOME`, else `<user profile>/Praxis Sound` (outside the folders Windows protects).
pub fn praxis_home() -> PathBuf {
    if let Ok(h) = std::env::var(HOME_ENV)
        && !h.trim().is_empty()
    {
        return PathBuf::from(h.trim());
    }
    let user = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    user.join("Praxis Sound")
}

/// The folder of a sound: a path as given, or a bare name under `PRAXIS_HOME/sounds/`.
pub fn resolve_sound(sound: &str) -> Result<PathBuf, OpError> {
    let s = sound.trim();
    if s.is_empty() {
        return Err(OpError::new(
            ErrorCode::Error,
            "name the sound: a folder path, or a bare name kept under PRAXIS_HOME/sounds",
        ));
    }
    let p = Path::new(s);
    let is_path = p.is_absolute() || s.contains(['/', '\\']) || s == "." || s == "..";
    let dir = if is_path {
        p.to_path_buf()
    } else {
        if s.contains([':', '*', '?', '"', '<', '>', '|']) {
            return Err(OpError::new(
                ErrorCode::Error,
                format!("`{s}` cannot be a folder name; use letters, digits, spaces, - or _"),
            ));
        }
        praxis_home().join("sounds").join(s)
    };
    Ok(std::path::absolute(&dir).unwrap_or(dir))
}

/// Holds a sound folder's `.lock` until dropped.
#[derive(Debug)]
pub struct SoundLock {
    path: PathBuf,
}

impl SoundLock {
    /// Creates `dir` if needed and takes its lock, waiting for another writer to finish.
    pub fn acquire(dir: &Path) -> Result<SoundLock, OpError> {
        SoundLock::acquire_file(dir, LOCK_FILE)
    }

    /// The same wait-and-break-stale lock on another file of the folder (the assistant holds
    /// `.agent-lock` for a whole turn, so two turns on one sound run one after the other while
    /// the folder's own `.lock` stays free for each write).
    pub fn acquire_file(dir: &Path, file: &str) -> Result<SoundLock, OpError> {
        std::fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
        let path = dir.join(file);
        let t0 = Instant::now();
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    use std::io::Write;
                    let _ = writeln!(f, "{}", std::process::id());
                    return Ok(SoundLock { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|m| m.elapsed().ok())
                        .is_some_and(|age| age > LOCK_STALE);
                    if stale {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if t0.elapsed() > LOCK_WAIT {
                        return Err(OpError::new(
                            ErrorCode::Io,
                            format!(
                                "{} is held by another writer; try again when it finishes",
                                path.display()
                            ),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(io_error(&path, e)),
            }
        }
    }
}

impl Drop for SoundLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Writes `bytes` to a temporary file next to `path` and renames it over `path`.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), OpError> {
    crate::ops::ensure_parent(path)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let tmp = path.with_file_name(format!(".{name}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| io_error(&tmp, e))?;
    // A reader holding the file open can make the rename fail briefly on Windows.
    let mut last = None;
    for _ in 0..40 {
        match std::fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Err(io_error(
        path,
        last.map_or_else(|| "rename failed".to_string(), |e| e.to_string()),
    ))
}

/// Writes a project atomically.
pub fn save_project(path: &Path, project: &Project) -> Result<(), OpError> {
    let text = project
        .to_json()
        .map_err(|e| OpError::new(ErrorCode::Error, e.to_string()))?;
    write_atomic(path, (text + "\n").as_bytes())
}

/// Reads a project file.
pub fn load_project(path: &Path) -> Result<Project, OpError> {
    let text = std::fs::read_to_string(path).map_err(|e| io_error(path, e))?;
    Project::from_json(&text).map_err(|e| {
        let code = match e {
            praxis_score::project::ProjectError::Json(_) => ErrorCode::BadJson,
            _ => ErrorCode::BadScore,
        };
        OpError::new(code, format!("{}: {e}", path.display()))
    })
}

/// A sound folder, read into memory.
#[derive(Debug)]
pub struct Sound {
    pub dir: PathBuf,
    /// None until the first take.
    pub project: Option<Project>,
    pub takes: TakesFile,
}

impl Sound {
    /// Reads the folder (which may not exist yet: then it is an empty sound).
    pub fn open(dir: &Path) -> Result<Sound, OpError> {
        let project_path = dir.join(PROJECT_FILE);
        let takes_path = dir.join(TAKES_FILE);
        let project = if project_path.is_file() {
            Some(load_project(&project_path)?)
        } else {
            None
        };
        let takes = if takes_path.is_file() {
            let text =
                std::fs::read_to_string(&takes_path).map_err(|e| io_error(&takes_path, e))?;
            let t: TakesFile = serde_json::from_str(&text).map_err(|e| {
                OpError::new(ErrorCode::BadJson, format!("{}: {e}", takes_path.display()))
            })?;
            if t.format != TAKES_FORMAT || t.version != TAKES_VERSION {
                return Err(OpError::new(
                    ErrorCode::BadJson,
                    format!(
                        "{}: expected format `{TAKES_FORMAT}` version {TAKES_VERSION}, found `{}` \
                         version {}",
                        takes_path.display(),
                        t.format,
                        t.version
                    ),
                ));
            }
            t
        } else {
            TakesFile::default()
        };
        if project.is_none() && !takes.takes.is_empty() {
            return Err(OpError::new(
                ErrorCode::Io,
                format!(
                    "{} lists takes but {} is missing",
                    takes_path.display(),
                    project_path.display()
                ),
            ));
        }
        Ok(Sound {
            dir: dir.to_path_buf(),
            project,
            takes,
        })
    }

    pub fn project_path(&self) -> PathBuf {
        self.dir.join(PROJECT_FILE)
    }

    pub fn takes_path(&self) -> PathBuf {
        self.dir.join(TAKES_FILE)
    }

    /// `takes/<n>/mix.wav`, relative (as stored) and absolute.
    pub fn mix_rel(n: u32) -> String {
        format!("takes/{n}/mix.wav")
    }

    pub fn mix_path(&self, n: u32) -> PathBuf {
        self.dir.join("takes").join(n.to_string()).join("mix.wav")
    }

    /// The absolute path of a stored (relative) render path.
    pub fn absolute(&self, rel: &str) -> PathBuf {
        let p = Path::new(rel);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            rel.split('/')
                .fold(self.dir.clone(), |acc, part| acc.join(part))
        }
    }

    /// Writes the project (when there is one) and then `takes.json` with the next revision, in
    /// that order, so a watcher that sees the new revision also finds the project it refers to.
    pub fn save(&mut self) -> Result<(), OpError> {
        if let Some(p) = &self.project {
            save_project(&self.project_path(), p)?;
        }
        self.takes.revision += 1;
        if self.takes.views.len() > MAX_VIEWS {
            let drop = self.takes.views.len() - MAX_VIEWS;
            self.takes.views.drain(..drop);
        }
        let text = serde_json::to_string_pretty(&self.takes)
            .map_err(|e| OpError::new(ErrorCode::Error, e.to_string()))?;
        write_atomic(&self.takes_path(), (text + "\n").as_bytes())
    }

    /// A take as a command returns it: the render path made absolute, and the take's score
    /// (from its project version) under `score` when asked for.
    pub fn take_json(&self, t: &TakeRecord, with_score: bool) -> Value {
        let mut v = serde_json::to_value(t).unwrap_or(Value::Null);
        v["render"]["path"] = json!(self.absolute(&t.render.path).display().to_string());
        if with_score {
            v["score"] = self
                .project
                .as_ref()
                .and_then(|p| p.version(&t.version))
                .map(|ver| serde_json::to_value(&ver.score).unwrap_or(Value::Null))
                .unwrap_or(Value::Null);
        }
        v
    }
}

/// `"You"`, or `"Agent · <name>"` for anything else (an agent's own name, with or without the
/// prefix). Empty means `default`.
pub fn normalize_who(who: Option<&str>, default: &str) -> String {
    let w = who
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .unwrap_or(default);
    if w.eq_ignore_ascii_case("you") {
        return "You".into();
    }
    let name = agent_name(w);
    if name.is_empty() {
        "You".into()
    } else {
        format!("Agent \u{b7} {name}")
    }
}

/// The agent's name in a `who` (`"Agent · cut-assistant"` -> `"cut-assistant"`).
pub fn agent_name(who: &str) -> &str {
    let w = who.trim();
    for prefix in ["Agent \u{b7} ", "Agent \u{b7}", "Agent: ", "Agent "] {
        if let Some(rest) = w.strip_prefix(prefix) {
            return rest.trim();
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn who_is_you_or_an_agent() {
        assert_eq!(normalize_who(None, "You"), "You");
        assert_eq!(normalize_who(Some(" you "), "Agent · x"), "You");
        assert_eq!(
            normalize_who(Some("cut-assistant"), "You"),
            "Agent · cut-assistant"
        );
        assert_eq!(
            normalize_who(Some("Agent · cut-assistant"), "You"),
            "Agent · cut-assistant"
        );
        assert_eq!(normalize_who(Some(""), "Agent · mcp"), "Agent · mcp");
        assert_eq!(agent_name("Agent · cut-assistant"), "cut-assistant");
    }

    #[test]
    fn bare_names_live_under_praxis_home() {
        let p = resolve_sound("my loop").unwrap();
        assert!(p.ends_with(Path::new("sounds").join("my loop")), "{p:?}");
        let q = resolve_sound("./here/loop").unwrap();
        assert!(q.is_absolute() && q.ends_with(Path::new("here").join("loop")));
        assert!(resolve_sound("  ").is_err());
        assert!(resolve_sound("a?b").is_err());
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("praxis-sound-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let f = dir.join("takes.json");
        write_atomic(&f, b"one").unwrap();
        write_atomic(&f, b"two").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"two");
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(names.len(), 1);
        let lock = SoundLock::acquire(&dir).unwrap();
        assert!(dir.join(LOCK_FILE).is_file());
        drop(lock);
        assert!(!dir.join(LOCK_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
