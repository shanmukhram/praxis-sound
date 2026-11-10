//! `analyze <clip>`: measurements of an audio or video clip.
//!
//! The full Sound Profile (FR-12, docs/contracts/sound-profile.md) is WP-D2 and not built yet.
//! Until it is, this prints the features that exist, under the profile's own field names
//! (`source`, `tempo`, `key`, `loudness`, `onsets_s`), with `complete: false` and the list of
//! profile fields still `missing`. When the profile lands the payload only gains fields.

use crate::error::{ErrorCode, OpError, io_error};
use crate::ops::check::decode;
use crate::ops::{ensure_parent, finite};
use praxis_analyze::features::{self, loudness, onset, seam, tempo};
use praxis_audio_io::decode::SourceKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Profile fields not measured yet.
pub const MISSING_PROFILE_FIELDS: &[&str] = &[
    "source.pcm_hash",
    "class",
    "envelope",
    "spectral",
    "groove",
    "pitch",
    "tonality",
    "stereo",
    "reverb",
    "sections",
    "features",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnalyzeRequest {
    pub clip: PathBuf,
    /// Also write the payload (without the envelope) to this JSON file.
    #[serde(default)]
    pub out: Option<PathBuf>,
    /// Beats per bar for downbeat detection.
    #[serde(default = "four")]
    pub beats_per_bar: u8,
}

fn four() -> u8 {
    4
}

pub fn analyze(req: &AnalyzeRequest) -> Result<Value, OpError> {
    let clip = decode(&req.clip)?;
    let a = &clip.audio;
    let sr = clip.sample_rate;
    let mono = features::mono(&a.left, &a.right);
    let loud = loudness::measure(&a.left, &a.right, sr)
        .map_err(|e| OpError::new(ErrorCode::DecodeFailed, e.to_string()))?;
    let tempo_est = tempo::analyze(&mono, sr, req.beats_per_bar.max(1));
    let key_est = features::key::analyze(&mono, sr);
    let (_, onsets) = onset::detect_onsets(&mono, sr);
    let s = seam::analyze_seam(&a.left, &a.right, sr, &seam::SeamConfig::default());
    let file_name = req
        .clip
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut v = json!({
        "complete": false,
        "missing": MISSING_PROFILE_FIELDS,
        "source": {
            "file_name": file_name,
            "path": req.clip.display().to_string(),
            "duration_s": clip.duration_s,
            "sample_rate": sr,
            "channels": clip.channels,
            "source_kind": match clip.source_kind {
                SourceKind::Audio => "audio",
                SourceKind::Video => "video",
            },
        },
        "tempo": tempo_est,
        "key": key_est.as_ref().map(|k| json!({
            "key": k.key,
            "name": k.key.display(),
            "confidence": k.confidence,
            "chroma": k.chroma,
        })),
        "loudness": loud,
        "onsets_s": onsets.iter().map(|o| o.time_s).collect::<Vec<_>>(),
        "loop_seam": {
            "passed": s.passed,
            "jump_ratio": finite(s.jump_ratio),
            "excess_db": finite(s.excess_db),
        },
    });
    if let Some(out) = &req.out {
        ensure_parent(out)?;
        let text = serde_json::to_string_pretty(&v).unwrap_or_default() + "\n";
        std::fs::write(out, text).map_err(|e| io_error(out, e))?;
        v["output"] = json!(out.display().to_string());
    }
    Ok(v)
}
