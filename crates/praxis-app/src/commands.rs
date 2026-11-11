//! What each Tauri command does, as plain functions over a sound folder, so they are tested
//! without a window. `shell.rs` wraps each one in a `#[tauri::command]`.
//!
//! Takes go through the command layer's take ops (`praxis_cli::ops::{ask, takes, restore, edit,
//! variations, keep_variation, open_view}`), the same functions the CLI and the MCP server call,
//! so the app never keeps a take store of its own. The project-level calls here (apply a patch,
//! undo, redo, render the live score, export) work on the sound's `project.praxis` through
//! `praxis_score::Project`, under the sound's `.lock` like every other writer.

use crate::sounds::{PROJECT_FILE, SoundInfo, TAKES_FILE, list_sounds};
use praxis_cli::ops::render::{RenderSettings, render_score};
use praxis_cli::ops::sound::{Sound, SoundLock, load_project, save_project};
use praxis_cli::ops::{self, ExportMidiRequest, export_midi, resolve_sound};
use praxis_cli::{ErrorCode, OpError};
use praxis_score::Score;
use praxis_score::patch::{PatchError, ScorePatch};
use praxis_score::project::Project;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// A failed command as the front end receives it (the CLI error shape, plus the patch error
/// when a patch was refused).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CmdError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_error: Option<Value>,
}

impl CmdError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> CmdError {
        CmdError {
            code: code.as_str().into(),
            message: message.into(),
            issues: Vec::new(),
            patch_error: None,
        }
    }
}

impl From<OpError> for CmdError {
    fn from(e: OpError) -> CmdError {
        CmdError {
            code: e.code.as_str().into(),
            message: e.message,
            issues: e.issues,
            patch_error: None,
        }
    }
}

impl std::fmt::Display for CmdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

pub type CmdResult<T> = Result<T, CmdError>;

fn io(path: &Path, e: impl std::fmt::Display) -> CmdError {
    CmdError::new(ErrorCode::Io, format!("{}: {e}", path.display()))
}

/// `PatchError` in the shape of the TypeScript `PatchError` union (app/src/engine/EngineAdapter.ts).
pub fn patch_error_json(e: &PatchError) -> Value {
    match e {
        PatchError::LockedLayer(layer) => json!({"kind": "locked_layer", "layer": layer}),
        PatchError::OutOfScope { index, op, reason } => {
            json!({"kind": "out_of_scope", "index": index, "op": op, "reason": reason})
        }
        PatchError::UnknownId {
            index,
            op,
            what,
            id,
        } => {
            json!({"kind": "unknown_id", "index": index, "op": op, "what": what, "id": id})
        }
        PatchError::DuplicateId {
            index,
            op,
            what,
            id,
        } => {
            json!({"kind": "duplicate_id", "index": index, "op": op, "what": what, "id": id})
        }
        PatchError::UnlockNotAllowed(layer) => {
            json!({"kind": "unlock_not_allowed", "layer": layer})
        }
        PatchError::Invalid(message) => json!({"kind": "invalid", "message": message}),
    }
}

fn rejected(e: &PatchError) -> CmdError {
    CmdError {
        code: ErrorCode::PatchRejected.as_str().into(),
        message: e.to_string(),
        issues: Vec::new(),
        patch_error: Some(patch_error_json(e)),
    }
}

/// The folder of a sound as the front end names it (a folder path from `list_sounds` or
/// `new_sound_path`; a bare name resolves under `PRAXIS_HOME/sounds`).
fn dir_of(sound: &str) -> CmdResult<PathBuf> {
    Ok(resolve_sound(sound)?)
}

/// The subfolders of a sound that hold audio the WebView plays: take renders, variations and
/// the live render.
pub const AUDIO_DIRS: [&str; 3] = ["takes", "variations", "live"];

/// `\\?\C:\x` -> `C:\x`: `canonicalize` gives verbatim paths on Windows, but the WebView asks
/// for plain ones (the take paths the ops return), and the asset scope compares them as given.
/// UNC verbatim paths (`\\?\UNC\...`) are kept as they are.
fn plain(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path,
    }
}

fn not_a_sound(sound: &str) -> CmdError {
    CmdError::new(
        ErrorCode::Error,
        format!(
            "{sound} is not a Praxis sound folder (it has no takes.json of format praxis-takes)"
        ),
    )
}

/// The only folders the WebView may load files from for `sound`: its [`AUDIO_DIRS`], and only
/// when the folder is a real sound (a `takes.json` the take ops can read, format
/// `praxis-takes`). The path is canonicalized first, so `..` or a link cannot point the grant
/// somewhere else. Take titles and replies can come from agents, so the WebView never gets a
/// wider grant than this.
pub fn audio_dirs(sound: &str) -> CmdResult<Vec<PathBuf>> {
    let dir = dir_of(sound)?;
    let dir = std::fs::canonicalize(&dir).map_err(|_| not_a_sound(sound))?;
    let dir = plain(dir);
    if !dir.is_dir() || !dir.join(TAKES_FILE).is_file() {
        return Err(not_a_sound(sound));
    }
    // `Sound::open` checks `format` and `version` and that the project the takes need exists.
    Sound::open(&dir).map_err(|_| not_a_sound(sound))?;
    Ok(AUDIO_DIRS.iter().map(|d| dir.join(d)).collect())
}

/// The folder of a sound that has a project (its first take), checked before taking the lock,
/// which would otherwise create the folder.
pub(crate) fn made_sound(sound: &str) -> CmdResult<PathBuf> {
    let dir = dir_of(sound)?;
    if !dir.join(PROJECT_FILE).is_file() {
        return Err(no_take_yet(&dir));
    }
    Ok(dir)
}

// ------------------------------------------------------------------ sounds

pub fn sounds(root: &Path) -> Vec<SoundInfo> {
    list_sounds(root)
}

// ------------------------------------------------------------------ takes (the shared history)

/// A request in words, or a hand edit (`patch`), on a sound: `ops::ask` plans, renders, checks
/// and records the take. The take comes back with its score.
pub fn ask(mut req: ops::AskRequest) -> CmdResult<Value> {
    req.include_score = true;
    Ok(ops::ask(&req)?)
}

/// The sound's whole history (`ops::takes`), each take with its score.
pub fn takes(sound: &str) -> CmdResult<Value> {
    Ok(ops::takes(&ops::TakesRequest {
        sound: sound.into(),
        include_scores: true,
    })?)
}

/// Goes back to take `n` through an undoable patch.
pub fn restore(req: &ops::RestoreRequest) -> CmdResult<Value> {
    Ok(ops::restore(req)?)
}

/// A prompt edit of a project that makes no take (`ops::edit`, the CLI's `praxis edit`).
pub fn edit(req: &ops::EditRequest) -> CmdResult<Value> {
    Ok(ops::edit(req)?)
}

/// Rendered, checked candidates; none is committed until one is kept.
pub fn variations(req: &ops::VariationsRequest) -> CmdResult<Value> {
    Ok(ops::variations(req)?)
}

/// Commits a variation as a new take.
pub fn keep_variation(mut req: ops::KeepVariationRequest) -> CmdResult<Value> {
    req.include_score = true;
    Ok(ops::keep_variation(&req)?)
}

/// Records a view request; the app shows it ("Notes opened by ...").
pub fn open_view(req: &ops::OpenViewRequest) -> CmdResult<Value> {
    Ok(ops::open_view(req)?)
}

// ------------------------------------------------------------------ the project (no take)

/// The live score, the undo state and the current take after a project-level call.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Live {
    pub score: Value,
    pub can_undo: bool,
    pub can_redo: bool,
    pub current: u32,
}

pub(crate) fn live_of(project: &Project, current: u32) -> Live {
    Live {
        score: serde_json::to_value(&project.score).unwrap_or(Value::Null),
        can_undo: project.can_undo(),
        can_redo: project.can_redo(),
        current,
    }
}

pub(crate) fn no_take_yet(dir: &Path) -> CmdError {
    CmdError::new(
        ErrorCode::Error,
        format!("{} has no take yet; ask for a sound first", dir.display()),
    )
}

/// The result of `apply_patch`: the new live score and the exact inverse.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Applied {
    pub live: Live,
    pub inverse: Value,
}

/// A patch that makes no take (mute, solo, lock), committed to the project's undo history.
/// `takes.json` is left alone: no take was made.
pub fn apply_patch(sound: &str, patch: Value) -> CmdResult<Applied> {
    let patch: ScorePatch = serde_json::from_value(patch)
        .map_err(|e| CmdError::new(ErrorCode::BadJson, format!("patch: {e}")))?;
    let dir = made_sound(sound)?;
    let _lock = SoundLock::acquire(&dir)?;
    let s = Sound::open(&dir)?;
    let mut project = s.project.ok_or_else(|| no_take_yet(&dir))?;
    let entry = project.commit(patch).map_err(|e| rejected(&e))?;
    let inverse = serde_json::to_value(&entry.inverse).unwrap_or(Value::Null);
    save_project(&dir.join(PROJECT_FILE), &project)?;
    Ok(Applied {
        live: live_of(&project, s.takes.current),
        inverse,
    })
}

/// The result of `undo` / `redo`: whether anything moved, and the live score after.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Stepped {
    pub moved: bool,
    pub live: Live,
}

/// The take whose score the live score equals: the current one when it still matches, else the
/// newest match. `None` when the live score is no take's (a mute or solo since, say).
fn matching_take(s: &Sound, project: &Project) -> Option<u32> {
    let is = |n: u32| {
        s.takes
            .take(n)
            .and_then(|t| project.version(&t.version))
            .is_some_and(|v| v.score == project.score)
    };
    if is(s.takes.current) {
        return Some(s.takes.current);
    }
    s.takes.takes.iter().rev().map(|t| t.n).find(|&n| is(n))
}

/// Undoes or redoes one step of the project's history (a take, a go-back or a plain edit). When
/// the step lands on another take's score, that take becomes current and `takes.json` gets a
/// new revision, as going back does.
fn step(sound: &str, redo: bool) -> CmdResult<Stepped> {
    let dir = made_sound(sound)?;
    let _lock = SoundLock::acquire(&dir)?;
    let mut s = Sound::open(&dir)?;
    let Some(mut project) = s.project.take() else {
        return Err(no_take_yet(&dir));
    };
    let moved = if redo {
        project.redo().map_err(|e| rejected(&e))?.is_some()
    } else {
        project.undo().map_err(|e| rejected(&e))?.is_some()
    };
    if !moved {
        return Ok(Stepped {
            moved,
            live: live_of(&project, s.takes.current),
        });
    }
    let current = matching_take(&s, &project).unwrap_or(s.takes.current);
    let live = live_of(&project, current);
    if current == s.takes.current {
        save_project(&dir.join(PROJECT_FILE), &project)?;
    } else {
        s.takes.current = current;
        s.project = Some(project);
        s.save()?;
    }
    Ok(Stepped { moved, live })
}

pub fn undo(sound: &str) -> CmdResult<Stepped> {
    step(sound, false)
}

pub fn redo(sound: &str) -> CmdResult<Stepped> {
    step(sound, true)
}

/// The score of take `n` (its project version `take-<n>`), or the live score for `None`.
pub fn take_score(dir: &Path, n: Option<u32>) -> CmdResult<Score> {
    let project = load_project(&dir.join(PROJECT_FILE))?;
    match n {
        None => Ok(project.score),
        Some(n) => project
            .version(&format!("take-{n}"))
            .map(|v| v.score.clone())
            .ok_or_else(|| CmdError::new(ErrorCode::Error, format!("there is no take {n}"))),
    }
}

/// What the real-time engine plays for `sound`: the live score, and the master gain the current
/// take's offline render applied (used when the score has no frozen gain; real time never
/// measures loudness).
pub fn playback_score(sound: &str) -> CmdResult<(Score, Option<f64>)> {
    let dir = made_sound(sound)?;
    let s = Sound::open(&dir)?;
    let project = s.project.ok_or_else(|| no_take_yet(&dir))?;
    let gain = s
        .takes
        .take(s.takes.current)
        .and_then(|t| t.render.report.get("applied_gain_db"))
        .and_then(Value::as_f64)
        .filter(|g| g.is_finite());
    Ok((project.score, gain))
}

/// Renders the live score (with any mute / solo since the take) to `<sound>/live/mix.wav` and
/// checks it. The payload is the CLI `render` payload.
pub fn render_live(sound: &str) -> CmdResult<Value> {
    let dir = dir_of(sound)?;
    let score = take_score(&dir, None)?;
    let out = dir.join("live").join("mix.wav");
    Ok(render_score(&score, &out, &RenderSettings::default())?)
}

// ------------------------------------------------------------------ export

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ExportArgs {
    pub sound: String,
    /// The take to export; the live score when absent.
    #[serde(default)]
    pub take: Option<u32>,
    /// `wav16`, `wav24`, `wav32f`, `flac`, `stems`, `midi`, `project`.
    pub kinds: Vec<String>,
    /// The output folder; a relative path resolves inside the sound folder.
    pub destination: String,
    pub base_name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExportFile {
    pub kind: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Exported {
    pub ok: bool,
    pub files: Vec<ExportFile>,
    pub message: String,
}

fn safe_base(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-');
    if s.is_empty() {
        "sound".into()
    } else {
        s.into()
    }
}

/// Writes the chosen kinds of a take (or the live score) into the destination folder.
pub fn export(args: &ExportArgs) -> CmdResult<Exported> {
    let sound = dir_of(&args.sound)?;
    let score = take_score(&sound, args.take)?;
    let dir = {
        let d = PathBuf::from(&args.destination);
        if d.is_absolute() { d } else { sound.join(d) }
    };
    std::fs::create_dir_all(&dir).map_err(|e| io(&dir, e))?;
    let base = dir.join(safe_base(&args.base_name));
    let with_ext = |ext: &str| PathBuf::from(format!("{}{ext}", base.display()));
    let mut files = Vec::new();
    let mut stems_done = false;
    for kind in &args.kinds {
        match kind.as_str() {
            "wav16" | "wav24" | "wav32f" | "flac" => {
                let (format, out) = match kind.as_str() {
                    "wav16" => ("16", with_ext("-wav16.wav")),
                    "wav24" => ("24", with_ext("-wav24.wav")),
                    "wav32f" => ("32f", with_ext("-wav32f.wav")),
                    _ => ("flac", with_ext(".flac")),
                };
                let settings = RenderSettings {
                    format: format.into(),
                    ..RenderSettings::default()
                };
                render_score(&score, &out, &settings)?;
                files.push(ExportFile {
                    kind: kind.clone(),
                    path: out.display().to_string(),
                });
            }
            "stems" => {
                if stems_done {
                    continue;
                }
                stems_done = true;
                let stems_dir = with_ext("-stems");
                let settings = RenderSettings {
                    stems: Some(stems_dir.clone()),
                    ..RenderSettings::default()
                };
                let payload = render_score(&score, &stems_dir.join("mix.wav"), &settings)?;
                for p in payload["stems"].as_array().into_iter().flatten() {
                    if let Some(p) = p.as_str() {
                        files.push(ExportFile {
                            kind: "stems".into(),
                            path: p.into(),
                        });
                    }
                }
            }
            "midi" => {
                let input = with_ext(".score.json.tmp");
                let text = serde_json::to_string(&score)
                    .map_err(|e| CmdError::new(ErrorCode::Error, e.to_string()))?;
                std::fs::write(&input, text).map_err(|e| io(&input, e))?;
                let out = with_ext(".mid");
                let r = export_midi(&ExportMidiRequest {
                    input: input.clone(),
                    out: out.clone(),
                });
                let _ = std::fs::remove_file(&input);
                r?;
                files.push(ExportFile {
                    kind: "midi".into(),
                    path: out.display().to_string(),
                });
            }
            "project" => {
                let out = with_ext(".praxis");
                ops::write_project(&out, &score)?;
                files.push(ExportFile {
                    kind: "project".into(),
                    path: out.display().to_string(),
                });
            }
            other => {
                return Err(CmdError::new(
                    ErrorCode::BadFormat,
                    format!("unknown export kind `{other}`"),
                ));
            }
        }
    }
    let message = format!("Wrote {} files to {}.", files.len(), dir.display());
    Ok(Exported {
        ok: true,
        files,
        message,
    })
}
