//! `validate`: report every issue in a score file or in the score of a `.praxis` project.

use crate::error::{ErrorCode, OpError, io_error};
use crate::ops::to_values;
use praxis_score::Score;
use praxis_score::project::PROJECT_FORMAT;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValidateRequest {
    /// A `.score.json` file or a `.praxis` project (detected by its `"format"` field).
    pub input: PathBuf,
}

/// Valid scores return `{valid, errors, warnings}`; invalid ones fail with `invalid_score`, the
/// same payload as the message and every error under `issues`.
pub fn validate(req: &ValidateRequest) -> Result<Value, OpError> {
    let text = std::fs::read_to_string(&req.input).map_err(|e| io_error(&req.input, e))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|e| OpError::new(ErrorCode::BadJson, e.to_string()))?;
    let is_project = value.get("format").and_then(Value::as_str) == Some(PROJECT_FORMAT);
    let score_value = if is_project {
        value
            .get("score")
            .cloned()
            .ok_or_else(|| OpError::new(ErrorCode::BadScore, "the project has no `score` field"))?
    } else {
        value
    };
    let score: Score = serde_json::from_value(score_value)
        .map_err(|e| OpError::new(ErrorCode::BadScore, e.to_string()))?;
    let report = praxis_score::validate::validate(&score);
    let valid = report.is_valid();
    let v = json!({
        "valid": valid,
        "errors": report.errors,
        "warnings": report.warnings,
        "input_kind": if is_project { "project" } else { "score" },
    });
    if valid {
        Ok(v)
    } else {
        Err(OpError::new(
            ErrorCode::InvalidScore,
            serde_json::to_string(&v).unwrap_or_default(),
        )
        .with_issues(to_values(&report.errors)))
    }
}
