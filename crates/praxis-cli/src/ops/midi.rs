//! `export-midi`: a Standard MIDI File (type 1, 480 PPQ) from a score or project.

use crate::error::{OpError, io_error};
use crate::ops::{ensure_parent, load_score};
use praxis_audio_io::midi::{PPQ, score_to_midi};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportMidiRequest {
    pub input: PathBuf,
    pub out: PathBuf,
}

/// `{output, tracks, ppq, bytes, layers[]}`. `tracks` counts the conductor track plus one track
/// per layer (muted layers included).
pub fn export_midi(req: &ExportMidiRequest) -> Result<Value, OpError> {
    let score = load_score(&req.input)?;
    let bytes = score_to_midi(&score);
    ensure_parent(&req.out)?;
    std::fs::write(&req.out, &bytes).map_err(|e| io_error(&req.out, e))?;
    Ok(json!({
        "output": req.out.display().to_string(),
        "tracks": score.layers.len() + 1,
        "ppq": PPQ,
        "bytes": bytes.len(),
        "layers": score.layers.iter().map(|l| l.id.clone()).collect::<Vec<_>>(),
    }))
}
