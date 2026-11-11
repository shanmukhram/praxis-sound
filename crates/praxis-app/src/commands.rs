//! What each Tauri command does, as plain functions over a sound folder, so they are tested
//! without a window. `shell.rs` wraps each one in a `#[tauri::command]`.
//!
//! Takes go through the command layer's take ops (`praxis_cli::ops`), the same functions the CLI
//! and the MCP server call. The project-level calls here (apply a patch, undo, redo, render the
//! live score, export) work on the sound's `project.praxis` through `praxis_score::Project`.

use crate::sounds::{PROJECT_FILE, SoundInfo, list_sounds};
use praxis_cli::ops::render::{RenderSettings, render_score};
use praxis_cli::ops::{ExportMidiRequest, export_midi};
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
        PatchError::UnknownId { index, op, what, id } => {
            json!({"kind": "unknown_id", "index": index, "op": op, "what": what, "id": id})
        }
        PatchError::DuplicateId { index, op, what, id } => {
            json!({"kind": "duplicate_id", "index": index, "op": op, "what": what, "id": id})
        }
        PatchError::UnlockNotAllowed(layer) => json!({"kind": "unlock_not_allowed", "layer": layer}),
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

// ------------------------------------------------------------------ the project file

pub fn project_path(sound: &Path) -> PathBuf {
    sound.join(PROJECT_FILE)
}

pub fn read_project(sound: &Path) -> CmdResult<Project> {
    let path = project_path(sound);
    let text = std::fs::read_to_string(&path).map_err(|e| io(&path, e))?;
    Project::from_json(&text)
        .map_err(|e| CmdError::new(ErrorCode::BadScore, format!("{}: {e}", path.display())))
}

/// Writes the project atomically (temp file, then rename), so a reader (an agent's MCP call, the
/// watcher) never sees half a file.
pub fn write_project(sound: &Path, project: &Project) -> CmdResult<()> {
    let path = project_path(sound);
    let text = project
        .to_json()
        .map_err(|e| CmdError::new(ErrorCode::Error, e.to_string()))?;
    let tmp = sound.join(format!(".{PROJECT_FILE}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text + "\n").map_err(|e| io(&tmp, e))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io(&path, e)
    })
}

/// The live score and the undo state, as the front end's `TakeState` needs them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Live {
    pub score: Value,
    pub can_undo: bool,
    pub can_redo: bool,
}

fn live_of(project: &Project) -> Live {
    Live {
        score: serde_json::to_value(&project.score).unwrap_or(Value::Null),
        can_undo: project.can_undo(),
        can_redo: project.can_redo(),
    }
}

/// The live score of a sound; `None` before its first take.
pub fn live(sound: &Path) -> CmdResult<Option<Live>> {
    if !project_path(sound).is_file() {
        return Ok(None);
    }
    Ok(Some(live_of(&read_project(sound)?)))
}

/// The result of `apply_patch`: the new live score and the exact inverse.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Applied {
    pub live: Live,
    pub inverse: Value,
}

/// A patch that makes no take (mute, solo, lock), committed to the project's undo history.
pub fn apply_patch(sound: &Path, patch: Value) -> CmdResult<Applied> {
    let patch: ScorePatch = serde_json::from_value(patch)
        .map_err(|e| CmdError::new(ErrorCode::BadJson, format!("patch: {e}")))?;
    let mut project = read_project(sound)?;
    let entry = project.commit(patch).map_err(|e| rejected(&e))?;
    let inverse = serde_json::to_value(&entry.inverse).unwrap_or(Value::Null);
    write_project(sound, &project)?;
    Ok(Applied {
        live: live_of(&project),
        inverse,
    })
}

/// The result of `undo` / `redo`: whether anything moved, and the live score after.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Stepped {
    pub moved: bool,
    pub live: Live,
}

pub fn undo(sound: &Path) -> CmdResult<Stepped> {
    let mut project = read_project(sound)?;
    let moved = project.undo().map_err(|e| rejected(&e))?.is_some();
    if moved {
        write_project(sound, &project)?;
    }
    Ok(Stepped {
        moved,
        live: live_of(&project),
    })
}

pub fn redo(sound: &Path) -> CmdResult<Stepped> {
    let mut project = read_project(sound)?;
    let moved = project.redo().map_err(|e| rejected(&e))?.is_some();
    if moved {
        write_project(sound, &project)?;
    }
    Ok(Stepped {
        moved,
        live: live_of(&project),
    })
}

/// The score of take `n` (its project version `take-<n>`), or the live score for `None`.
pub fn take_score(sound: &Path, n: Option<u32>) -> CmdResult<Score> {
    let project = read_project(sound)?;
    match n {
        None => Ok(project.score),
        Some(n) => project
            .version(&format!("take-{n}"))
            .map(|v| v.score.clone())
            .ok_or_else(|| CmdError::new(ErrorCode::Error, format!("there is no take {n}"))),
    }
}

/// Renders the live score (with mute / solo since the take) to `<sound>/live/mix.wav` and
/// checks it. The payload is the CLI `render` payload.
pub fn render_live(sound: &Path) -> CmdResult<Value> {
    let score = take_score(sound, None)?;
    let out = sound.join("live").join("mix.wav");
    Ok(render_score(&score, &out, &RenderSettings::default())?)
}

// ------------------------------------------------------------------ sounds

pub fn sounds(root: &Path) -> Vec<SoundInfo> {
    list_sounds(root)
}

// ------------------------------------------------------------------ export

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ExportArgs {
    pub sound: PathBuf,
    /// The take to export; the live score when absent.
    #[serde(default)]
    pub take: Option<u32>,
    /// `wav16`, `wav24`, `wav32f`, `flac`, `stems`, `midi`, `project`.
    pub kinds: Vec<String>,
    /// The output folder; relative paths resolve inside the sound folder.
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
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let s = s.trim_matches('-');
    if s.is_empty() { "sound".into() } else { s.into() }
}

/// Writes the chosen kinds of a take (or the live score) into the destination folder.
pub fn export(args: &ExportArgs) -> CmdResult<Exported> {
    let score = take_score(&args.sound, args.take)?;
    let dir = {
        let d = PathBuf::from(&args.destination);
        if d.is_absolute() { d } else { args.sound.join(d) }
    };
    std::fs::create_dir_all(&dir).map_err(|e| io(&dir, e))?;
    let base = dir.join(safe_base(&args.base_name));
    let with_ext = |ext: &str| PathBuf::from(format!("{}{ext}", base.display()));
    let mut files = Vec::new();
    let mut stems_done = false;
    for kind in &args.kinds {
        match kind.as_str() {
            "wav16" | "wav24" | "wav32f" | "flac" => {
                let (format, ext) = match kind.as_str() {
                    "wav16" => ("16", ".wav"),
                    "wav24" => ("24", ".wav"),
                    "wav32f" => ("32f", ".wav"),
                    _ => ("flac", ".flac"),
                };
                let out = if kind == "flac" {
                    with_ext(ext)
                } else {
                    with_ext(&format!("-{kind}{ext}"))
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
            "stems" if !stems_done => {
                stems_done = true;
                let stems_dir = with_ext("-stems");
                let settings = RenderSettings {
                    stems: Some(stems_dir.clone()),
                    ..RenderSettings::default()
                };
                let mix = stems_dir.join("mix.wav");
                let payload = render_score(&score, &mix, &settings)?;
                for s in payload["stems"].as_array().into_iter().flatten() {
                    let p = s.get("path").and_then(Value::as_str).or(s.as_str());
                    if let Some(p) = p {
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
                praxis_cli::ops::write_project(&out, &score)?;
                files.push(ExportFile {
                    kind: "project".into(),
                    path: out.display().to_string(),
                });
            }
            "stems" => {}
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
