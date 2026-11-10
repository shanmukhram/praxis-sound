//! Spoken rhythm (FR-5). OWNED BY WP-A (score-ops). Signatures are a FROZEN CONTRACT.
//! "DON din DON don TIN" -> one step per syllable: DON strong, din soft, don normal, TIN bright,
//! "-" or "." a rest, "~" a tie that extends the previous note by one step.

use crate::types::Accent;

#[derive(Clone, Debug, PartialEq)]
pub enum SpokenStep {
    Hit { accent: Accent, velocity: f32 },
    Rest,
    Tie,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("cannot read spoken rhythm token `{token}` at position {position}")]
pub struct SpokenError {
    pub token: String,
    pub position: usize,
}

pub fn parse_spoken(text: &str) -> Result<Vec<SpokenStep>, SpokenError> {
    let _ = text;
    Ok(Vec::new())
}
