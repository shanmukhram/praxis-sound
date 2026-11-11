//! Typed errors of the matcher.

use praxis_engine::RenderError;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MatchError {
    /// The request cannot be matched as asked (no reference, a bad bar count...).
    #[error("{0}")]
    Invalid(String),
    /// Measuring a render failed.
    #[error("analysis failed: {0}")]
    Analysis(String),
    /// The planner could not turn the character into a Score.
    #[error("could not build a score: {0}")]
    Plan(String),
    /// Every candidate failed to render.
    #[error(transparent)]
    Render(#[from] RenderError),
    /// No candidate passed the originality check.
    #[error("every attempt was too close to the reference's melody: {0}")]
    NotOriginal(String),
}
