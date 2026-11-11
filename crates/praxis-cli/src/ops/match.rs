//! `match <clip>`: "make something like this" (FR-14 to FR-18, WP-M).
//!
//! The clip is decoded and measured into a Sound Profile, and the audio is dropped right there:
//! only the profile reaches the matcher, so the reference's audio never reaches the engine, the
//! project or an export (FR-18). The matcher renders candidates, measures them with the same
//! analyzer, refines them until the similarity stops improving, and checks the result's
//! originality. The payload lists every iteration's similarity.

use crate::error::{ErrorCode, OpError};
use crate::ops::check::decode;
use crate::ops::render::{RenderSettings, render_score};
use crate::ops::{score_summary, write_project};
use praxis_analyze::SoundProfile;
use praxis_engine::{InstrumentError, RenderError};
use praxis_instruments::Registry;
use praxis_match::{
    MatchError, MatchOptions, MatchOutcome, MatchRequest, match_reference, parse_key,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchClipRequest {
    /// The reference clip (audio or video).
    pub clip: PathBuf,
    /// Where to write the new `.praxis` project.
    pub out: PathBuf,
    /// Mood brief (FR-17), e.g. "melancholy".
    #[serde(default)]
    pub prompt: Option<String>,
    /// A second clip whose timbre to take while the first gives the groove (FR-17).
    #[serde(default)]
    pub timbre: Option<PathBuf>,
    #[serde(default)]
    pub bars: Option<u32>,
    /// Key, e.g. "A minor", "F#m", "Eb dorian".
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub bpm: Option<f64>,
    #[serde(default)]
    pub seed: Option<u64>,
    /// Most renders inside the loop (default 40).
    #[serde(default)]
    pub max_iterations: Option<usize>,
    /// Also render the result to this file (and check it).
    #[serde(default)]
    pub render: Option<PathBuf>,
}

/// Measures the clip(s) and runs the matcher; nothing is written. Used by [`match_clip`] and the
/// assistant's `match_reference` tool. Returns the request (holding the profiles) and the outcome.
pub fn matched(req: &MatchClipRequest) -> Result<(MatchRequest, MatchOutcome), OpError> {
    let key = match req.key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => Some(parse_key(k).ok_or_else(|| {
            OpError::new(
                ErrorCode::Error,
                format!("cannot read the key `{k}`; write e.g. \"A minor\", \"F#m\" or \"Eb dorian\""),
            )
        })?),
        None => None,
    };
    if let Some(b) = req.bars
        && !(1..=512).contains(&b)
    {
        return Err(OpError::new(ErrorCode::Error, "bars must be 1..=512"));
    }
    if let Some(bpm) = req.bpm
        && !(40.0..=220.0).contains(&bpm)
    {
        return Err(OpError::new(ErrorCode::Error, "bpm must be 40..=220"));
    }
    let reference = profile_clip(&req.clip)?;
    let timbre = req.timbre.as_deref().map(profile_clip).transpose()?;
    let mut options = MatchOptions::default();
    if let Some(n) = req.max_iterations {
        options.max_evals = n.clamp(10, 200);
    }
    let request = MatchRequest {
        reference,
        timbre,
        mood: req
            .prompt
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string),
        bars: req.bars,
        key,
        bpm: req.bpm,
        seed: req.seed.unwrap_or(0),
        options,
    };
    let registry = Registry::with_default_packs();
    let out = match_reference(&request, &registry).map_err(match_error)?;
    Ok((request, out))
}

pub fn match_clip(req: &MatchClipRequest) -> Result<Value, OpError> {
    let (request, out) = matched(req)?;
    write_project(&req.out, &out.score)?;
    let render = match &req.render {
        Some(path) => Some(render_score(&out.score, path, &RenderSettings::default())?),
        None => None,
    };
    let reference = &request.reference;
    let mut v = json!({
        "project": req.out.display().to_string(),
        "similarity": out.similarity,
        "iterations": out.iterations,
        "originality": out.originality,
        "originality_attempts": out.originality_attempts,
        "kind": out.kind,
        "made_with": out.made_with,
        "similarities": out.similarities,
        "params": out.params,
        "notes": out.notes,
        "reference": json!({
            "file_name": reference.source.file_name,
            "duration_s": reference.source.duration_s,
            "kind": reference.class.kind,
            "tempo_bpm": reference.tempo.as_ref().map(|t| t.bpm),
            "key": reference.key.as_ref().map(|k| k.key.display()),
        }),
        "score_summary": score_summary(&out.score),
    });
    if let Some(r) = render {
        v["render"] = r;
    }
    Ok(v)
}

/// Decodes a clip and measures it; the audio goes out of scope here.
fn profile_clip(path: &Path) -> Result<SoundProfile, OpError> {
    let clip = decode(path)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    praxis_match::analysis::profile_of(&clip.audio, clip.sample_rate, &name)
        .map_err(|e| OpError::new(ErrorCode::DecodeFailed, format!("{}: {e}", path.display())))
}

fn match_error(e: MatchError) -> OpError {
    match e {
        MatchError::Render(RenderError::Instrument(InstrumentError::PackMissing(p))) => {
            OpError::new(
                ErrorCode::PackMissing,
                format!("instrument pack `{p}` is not installed: run `praxis packs install {p}`"),
            )
        }
        MatchError::Render(r) => OpError::new(ErrorCode::RenderFailed, r.to_string()),
        MatchError::Analysis(m) => OpError::new(ErrorCode::DecodeFailed, m),
        other => OpError::new(ErrorCode::Error, other.to_string()),
    }
}
