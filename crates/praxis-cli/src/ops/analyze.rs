//! `analyze <clip>`: the Sound Profile of an audio or video clip (FR-12, FR-13,
//! docs/contracts/sound-profile.md).
//!
//! The payload is the complete `SoundProfile` (every field, `complete: true`, `missing: []`),
//! plus fields for people and agents: `source.path`, `source.source_kind`, `key.name`,
//! `feature_names` (the names of `features`, in order), `loop_seam` and a one-line `summary`.
//! A file written with `-o` holds the same payload and reads back as a `SoundProfile` (the
//! extra fields are ignored), so `library add profile.json` accepts it.

use crate::error::{ErrorCode, OpError, io_error};
use crate::ops::check::decode;
use crate::ops::{ensure_parent, finite};
use praxis_analyze::features::seam;
use praxis_analyze::profile::{ClipKind, FEATURE_NAMES, SoundProfile};
use praxis_analyze::{ProfileOptions, sound_profile};
use praxis_audio_io::decode::{Decoded, SourceKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Profile fields not measured yet (none: the profile is complete).
pub const MISSING_PROFILE_FIELDS: &[&str] = &[];

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

/// The file name of `path` (no directories), as the profile stores it.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Profile of decoded audio.
pub fn profile_decoded(
    clip: &Decoded,
    file_name: &str,
    beats_per_bar: u8,
) -> Result<SoundProfile, OpError> {
    let a = &clip.audio;
    sound_profile(
        &a.left,
        &a.right,
        clip.sample_rate,
        &ProfileOptions {
            file_name: file_name.to_string(),
            channels: clip.channels,
            beats_per_bar: beats_per_bar.max(1),
        },
    )
    .map_err(|e| OpError::new(ErrorCode::DecodeFailed, e.to_string()))
}

/// Decodes `path` and measures its profile.
pub fn profile_clip(path: &Path, beats_per_bar: u8) -> Result<(SoundProfile, Decoded), OpError> {
    let clip = decode(path)?;
    let p = profile_decoded(&clip, &file_name(path), beats_per_bar)?;
    Ok((p, clip))
}

fn kind_word(k: ClipKind) -> &'static str {
    match k {
        ClipKind::Music => "music",
        ClipKind::Sfx => "sound effect",
        ClipKind::Ambience => "ambience",
    }
}

/// One line in plain words: what the clip is and its main measurements.
pub fn summary(p: &SoundProfile) -> String {
    let mut parts = vec![format!(
        "{} ({:.0}% sure)",
        kind_word(p.class.kind),
        100.0 * p.class.confidence
    )];
    parts.push(format!("{:.1} s", p.source.duration_s));
    // A tempo is only news for music; a key for anything tonal (tuned effects too).
    if let Some(t) = &p.tempo
        && t.confidence >= 0.25
        && p.class.kind == ClipKind::Music
    {
        parts.push(format!("{:.0} BPM", t.bpm));
    }
    if let Some(k) = &p.key
        && k.confidence >= 0.2
        && p.tonality >= 0.3
    {
        parts.push(k.key.display());
    }
    parts.push(format!("{:.1} LUFS", p.loudness.integrated_lufs));
    parts.push(if p.tonality >= 0.6 {
        "tonal".to_string()
    } else if p.tonality <= 0.3 {
        "noisy".to_string()
    } else {
        "part tonal, part noise".to_string()
    });
    if let Some(r) = &p.reverb
        && r.confidence >= 0.3
    {
        parts.push(format!("decays in {:.1} s", r.rt60_s));
    }
    parts.join(" \u{b7} ")
}

/// The analyze payload of a profile (see the module docs).
pub fn payload(p: &SoundProfile, path: Option<&Path>, source_kind: Option<SourceKind>) -> Value {
    let mut v = serde_json::to_value(p).unwrap_or(Value::Null);
    v["complete"] = json!(true);
    v["missing"] = json!(MISSING_PROFILE_FIELDS);
    if let Some(path) = path {
        v["source"]["path"] = json!(path.display().to_string());
    }
    if let Some(k) = source_kind {
        v["source"]["source_kind"] = json!(match k {
            SourceKind::Audio => "audio",
            SourceKind::Video => "video",
        });
    }
    if let Some(k) = &p.key {
        v["key"]["name"] = json!(k.key.display());
    }
    v["feature_names"] = json!(FEATURE_NAMES);
    v["summary"] = json!(summary(p));
    v
}

pub fn analyze(req: &AnalyzeRequest) -> Result<Value, OpError> {
    let (p, clip) = profile_clip(&req.clip, req.beats_per_bar)?;
    let a = &clip.audio;
    let s = seam::analyze_seam(
        &a.left,
        &a.right,
        clip.sample_rate,
        &seam::SeamConfig::default(),
    );
    let mut v = payload(&p, Some(&req.clip), Some(clip.source_kind));
    v["loop_seam"] = json!({
        "passed": s.passed,
        "jump_ratio": finite(s.jump_ratio),
        "excess_db": finite(s.excess_db),
    });
    if let Some(out) = &req.out {
        ensure_parent(out)?;
        let text = serde_json::to_string_pretty(&v).unwrap_or_default() + "\n";
        std::fs::write(out, text).map_err(|e| io_error(out, e))?;
        v["output"] = json!(out.display().to_string());
    }
    Ok(v)
}
