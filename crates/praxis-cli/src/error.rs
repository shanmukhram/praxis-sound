//! Coded errors. The codes are the ones listed in docs/contracts/cli.md; never rename one.

use serde::Serialize;
use serde_json::{Value, json};

/// Stable machine-readable error code (docs/contracts/cli.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Io,
    BadJson,
    BadScore,
    InvalidScore,
    BadFormat,
    RenderFailed,
    NoApiKey,
    ProviderFailed,
    PackMissing,
    DecodeFailed,
    PatchRejected,
    PackVerifyFailed,
    Error,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Io => "io",
            ErrorCode::BadJson => "bad_json",
            ErrorCode::BadScore => "bad_score",
            ErrorCode::InvalidScore => "invalid_score",
            ErrorCode::BadFormat => "bad_format",
            ErrorCode::RenderFailed => "render_failed",
            ErrorCode::NoApiKey => "no_api_key",
            ErrorCode::ProviderFailed => "provider_failed",
            ErrorCode::PackMissing => "pack_missing",
            ErrorCode::DecodeFailed => "decode_failed",
            ErrorCode::PatchRejected => "patch_rejected",
            ErrorCode::PackVerifyFailed => "pack_verify_failed",
            ErrorCode::Error => "error",
        }
    }
}

/// A command failure: a contract code, a message for people, and (for validation failures)
/// the individual issues, each `{code, path, message}` with `path` a JSON pointer.
#[derive(Clone, Debug, PartialEq)]
pub struct OpError {
    pub code: ErrorCode,
    pub message: String,
    pub issues: Vec<Value>,
}

impl OpError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> OpError {
        OpError {
            code,
            message: message.into(),
            issues: Vec::new(),
        }
    }

    pub fn with_issues(mut self, issues: Vec<Value>) -> OpError {
        self.issues = issues;
        self
    }

    /// The `error` object of the JSON envelope. `issues` appears only when there are any.
    pub fn to_json(&self) -> Value {
        let mut v = json!({ "code": self.code.as_str(), "message": self.message });
        if !self.issues.is_empty() {
            v["issues"] = Value::Array(self.issues.clone());
        }
        v
    }
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for OpError {}

pub(crate) fn io_error(path: &std::path::Path, e: impl std::fmt::Display) -> OpError {
    OpError::new(ErrorCode::Io, format!("{}: {e}", path.display()))
}
