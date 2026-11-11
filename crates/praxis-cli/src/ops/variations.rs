//! `variations`: N alternates of a selection (bars and layers, FR-8), each a scoped patch with
//! its own seed, rendered to `variations/<i>.wav` next to the project and checked. Nothing is
//! committed: a variation of a sound becomes a take only through `keep_variation`.
//!
//! `variations/variations.json` records what was made (`format: "praxis-variations"`): the
//! source, the history position it was made at (`base_seq`, and `base_take` for a sound), the
//! scope, the seed and each variation's patch, so `keep_variation` can commit exactly what was
//! auditioned.

use crate::error::{ErrorCode, OpError, io_error};
use crate::ops::changes::changed_layers;
use crate::ops::edit::{drop_view_lock_note, edit_error, only_layers};
use crate::ops::load_score;
use crate::ops::render::{RenderSettings, render_score};
use crate::ops::sound::{Sound, SoundLock, load_project, resolve_sound, write_atomic};
use praxis_score::Score;
use praxis_score::patch::{Scope, ScorePatch};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const VARIATIONS_FORMAT: &str = "praxis-variations";
pub const VARIATIONS_FILE: &str = "variations.json";
/// At most this many variations per request.
pub const MAX_VARIATIONS: usize = 8;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VariationsRequest {
    /// A `.praxis` project. Exactly one of `project`, `score` and `sound`.
    #[serde(default)]
    pub project: Option<PathBuf>,
    /// A `.score.json` file.
    #[serde(default)]
    pub score: Option<PathBuf>,
    /// A sound folder (or a bare name under PRAXIS_HOME/sounds): varies the current take.
    #[serde(default)]
    pub sound: Option<String>,
    /// Bars (unset = the whole score) and layers (empty = every unlocked layer playing there).
    #[serde(default)]
    pub scope: Scope,
    /// How many (1..=8, default 3).
    #[serde(default = "default_n")]
    pub n: usize,
    #[serde(default = "default_seed")]
    pub seed: u64,
    /// Where the WAVs and `variations.json` go (default: `variations/` next to the source).
    #[serde(default)]
    pub out_dir: Option<PathBuf>,
    #[serde(default)]
    pub render_settings: RenderSettings,
}

fn default_n() -> usize {
    3
}

fn default_seed() -> u64 {
    crate::ops::generate::DEFAULT_SEED
}

/// One entry of `variations.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VariationRecord {
    /// 1-based.
    pub index: usize,
    pub seed: u64,
    pub summary: String,
    pub patch: ScorePatch,
    pub changed: BTreeMap<String, [u32; 2]>,
    /// The WAV, absolute.
    pub path: String,
    pub checks: Value,
}

/// `variations/variations.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VariationsFile {
    pub format: String,
    pub version: u32,
    /// The project, score or sound folder the variations were made from.
    pub source: String,
    /// The project's history position (`current_seq`) they were made at; 0 for a score file.
    pub base_seq: u64,
    /// The sound's current take they were made from (0 when not a sound).
    pub base_take: u32,
    pub scope: Scope,
    pub seed: u64,
    pub variations: Vec<VariationRecord>,
}

pub(crate) fn variations_dir(sound: &Sound) -> PathBuf {
    sound.dir.join("variations")
}

/// Reads a sound's `variations/variations.json`.
pub(crate) fn load_variations(dir: &Path) -> Result<VariationsFile, OpError> {
    let path = dir.join(VARIATIONS_FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| io_error(&path, e))?;
    let f: VariationsFile = serde_json::from_str(&text)
        .map_err(|e| OpError::new(ErrorCode::BadJson, format!("{}: {e}", path.display())))?;
    if f.format != VARIATIONS_FORMAT {
        return Err(OpError::new(
            ErrorCode::BadJson,
            format!("{}: not a `{VARIATIONS_FORMAT}` file", path.display()),
        ));
    }
    Ok(f)
}

/// `{source, out_dir, file, scope, seed, base_seq, base_take, variations: [{index, seed, summary,
/// patch, changed_bars, changed_layers, changed_by_layer, render}]}`.
pub fn variations(req: &VariationsRequest) -> Result<Value, OpError> {
    if req.n == 0 || req.n > MAX_VARIATIONS {
        return Err(OpError::new(
            ErrorCode::Error,
            format!("ask for 1 to {MAX_VARIATIONS} variations, not {}", req.n),
        ));
    }
    let given = [
        req.project.is_some(),
        req.score.is_some(),
        req.sound.is_some(),
    ];
    if given.iter().filter(|g| **g).count() != 1 {
        return Err(OpError::new(
            ErrorCode::Error,
            "give exactly one of project, score and sound",
        ));
    }
    // The base Score, where it came from, and the default output folder.
    let mut _lock = None;
    let (score, source, base_seq, base_take, default_dir): (Score, PathBuf, u64, u32, PathBuf) =
        if let Some(name) = &req.sound {
            let dir = resolve_sound(name)?;
            _lock = Some(SoundLock::acquire(&dir)?);
            let sound = Sound::open(&dir)?;
            let Some(project) = &sound.project else {
                return Err(OpError::new(
                    ErrorCode::Error,
                    format!("{} has no takes yet; ask for a sound first", dir.display()),
                ));
            };
            (
                project.score.clone(),
                dir.clone(),
                project.current_seq(),
                sound.takes.current,
                variations_dir(&sound),
            )
        } else if let Some(p) = &req.project {
            let project = load_project(p)?;
            let dir = p.parent().unwrap_or(Path::new(".")).join("variations");
            (
                project.score.clone(),
                p.clone(),
                project.current_seq(),
                0,
                dir,
            )
        } else {
            let p = req.score.as_ref().expect("checked above");
            let dir = p.parent().unwrap_or(Path::new(".")).join("variations");
            (load_score(p)?, p.clone(), 0, 0, dir)
        };
    let out_dir = req.out_dir.clone().unwrap_or(default_dir);
    // Named layers are exact ids: the planner sees only them as variable (see `only_layers`).
    let (view, view_locks) = if req.scope.layers.is_empty() {
        (score.clone(), Vec::new())
    } else {
        for id in &req.scope.layers {
            if score.layer(id).is_none() {
                return Err(OpError::new(
                    ErrorCode::PatchRejected,
                    format!("there is no layer `{id}` to vary"),
                ));
            }
        }
        only_layers(&score, &req.scope.layers)
    };
    let named_locked = req
        .scope
        .layers
        .iter()
        .any(|id| score.layer(id).is_some_and(|l| l.locked));
    let mut made =
        praxis_planner::variations(&view, &req.scope, req.n, req.seed).map_err(edit_error)?;
    for v in &mut made {
        // The same patch on the real Score.
        let applied = praxis_score::apply::apply(&score, &v.patch).map_err(|e| {
            OpError::new(
                ErrorCode::PatchRejected,
                format!("variation {}: {e}", v.index + 1),
            )
        })?;
        v.score = applied.score;
        v.inverse = applied.inverse;
        // The "left alone (locked)" remark names only layers locked for the view, unless a
        // requested layer is itself locked (then the remark stays).
        if !named_locked {
            v.summary = drop_view_lock_note(&v.summary, &view_locks, &view_locks);
        }
    }

    let mut records = Vec::new();
    let mut items = Vec::new();
    for v in &made {
        let index = v.index + 1;
        let path = out_dir.join(format!("{index}.wav"));
        let render = render_score(&v.score, &path, &req.render_settings)?;
        let changed = changed_layers(&score, &v.score);
        let bars = v.patch.scope.bars.unwrap_or(praxis_score::BarRange {
            start: 1,
            end: score.bars,
        });
        items.push(json!({
            "index": index,
            "seed": v.seed,
            "summary": v.summary,
            "patch": v.patch,
            "changed_bars": [bars.start, bars.end],
            "changed_layers": changed.keys().collect::<Vec<_>>(),
            "changed_by_layer": changed,
            "render": render,
        }));
        records.push(VariationRecord {
            index,
            seed: v.seed,
            summary: v.summary.clone(),
            patch: v.patch.clone(),
            changed,
            path: path.display().to_string(),
            checks: render["check"].clone(),
        });
    }
    let file = VariationsFile {
        format: VARIATIONS_FORMAT.into(),
        version: 1,
        source: source.display().to_string(),
        base_seq,
        base_take,
        scope: req.scope.clone(),
        seed: req.seed,
        variations: records,
    };
    let file_path = out_dir.join(VARIATIONS_FILE);
    let text = serde_json::to_string_pretty(&file)
        .map_err(|e| OpError::new(ErrorCode::Error, e.to_string()))?;
    write_atomic(&file_path, (text + "\n").as_bytes())?;
    let passed = items
        .iter()
        .filter(|i| i["render"]["check"]["passed"] == json!(true))
        .count();
    Ok(json!({
        "source": source.display().to_string(),
        "out_dir": out_dir.display().to_string(),
        "file": file_path.display().to_string(),
        "scope": req.scope,
        "seed": req.seed,
        "base_seq": base_seq,
        "base_take": base_take,
        "passed": passed,
        "variations": items,
    }))
}
