//! Errors of project operations that are not plain patch errors.

use crate::patch::PatchError;
use crate::validate::Issue;

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error(transparent)]
    Patch(#[from] PatchError),
    #[error("not valid project JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not a Praxis project (format `{0}`, expected `praxis-project`)")]
    Format(String),
    #[error("unsupported project version {0} (this build reads version 1)")]
    Version(String),
    #[error("project score failed validation: {0:?}")]
    InvalidScore(Vec<Issue>),
    #[error("version name must not be empty")]
    EmptyVersionName,
    #[error("a version named `{0}` already exists")]
    DuplicateVersion(String),
    #[error("no version named `{0}`")]
    UnknownVersion(String),
    #[error("cannot restore version `{name}`: {reason}")]
    Restore { name: String, reason: String },
}
