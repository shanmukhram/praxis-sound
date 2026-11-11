//! `check <audio> --score <score|project>`: measure a render against its Score.

use crate::error::{ErrorCode, OpError, io_error};
use crate::ops::load_score;
use praxis_analyze::verify::{Tolerances, VerifyReport, verify_render};
use praxis_audio_io::decode::{DecodeError, Decoded, decode_file};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckRequest {
    /// The rendered audio (WAV, FLAC, or anything `analyze` decodes).
    pub audio: PathBuf,
    /// The Score (or `.praxis` project) it was rendered from.
    pub score: PathBuf,
}

/// The `VerifyReport` fields plus the verdict ([`add_verdict`]). The CLI exits 1 when `passed` is
/// false.
pub fn check(req: &CheckRequest) -> Result<Value, OpError> {
    let score = load_score(&req.score)?;
    let clip = decode(&req.audio)?;
    let report = verify_render(
        &score,
        &clip.audio,
        clip.sample_rate,
        &Tolerances::default(),
    );
    let mut v = serde_json::to_value(&report).unwrap_or(Value::Null);
    add_verdict(&mut v, &report);
    v["audio"] = json!(req.audio.display().to_string());
    v["sample_rate"] = json!(clip.sample_rate);
    v["duration_s"] = json!(clip.duration_s);
    Ok(v)
}

/// Adds the verdict to a serialised `VerifyReport`: `passed` (every decided check passed),
/// `failed[]` (names of decided failures), `inconclusive[]` (names of checks the material cannot
/// decide: neither passed nor failed) and `summary`, the take's line in plain words ("6 of 6
/// checks passed · 2 can't tell from these notes: tempo (...), key (...)").
pub(crate) fn add_verdict(v: &mut Value, report: &VerifyReport) {
    let names = |pred: fn(&praxis_analyze::verify::Check) -> bool| {
        report
            .checks
            .iter()
            .filter(|c| pred(c))
            .map(|c| c.name.clone())
            .collect::<Vec<_>>()
    };
    v["passed"] = json!(report.passed());
    v["failed"] = json!(names(|c| c.failed()));
    v["inconclusive"] = json!(names(|c| c.inconclusive));
    v["summary"] = json!(report.summary());
}

/// Decodes an audio or video file; failures map to `io` (unreadable) or `decode_failed`.
pub(crate) fn decode(path: &Path) -> Result<Decoded, OpError> {
    decode_file(path).map_err(|e| match e {
        DecodeError::Io { .. } => io_error(path, e),
        other => OpError::new(
            ErrorCode::DecodeFailed,
            format!("{} ({}): {other}", path.display(), other.code()),
        ),
    })
}
