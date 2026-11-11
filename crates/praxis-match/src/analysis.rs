//! The one place that calls WP-D2's measurements: profile of a render, character similarity and
//! the originality check. Matching is analysis by synthesis: the reference and every render are
//! measured by the same code (PRD design rule), so this module only adapts types.

use crate::error::MatchError;
use praxis_analyze::SoundProfile;
use praxis_analyze::originality::{self, OriginalityReport};
use praxis_analyze::similarity;
use praxis_dsp::StereoBuffer;
use serde::{Deserialize, Serialize};

/// Measures a render. The name goes into `source.file_name` only.
pub fn profile_of(
    audio: &StereoBuffer,
    sample_rate: u32,
    name: &str,
) -> Result<SoundProfile, MatchError> {
    praxis_analyze::profile::measure(&audio.left, &audio.right, sample_rate, name)
        .map_err(|e| MatchError::Analysis(e.to_string()))
}

/// Character similarity, 0..1 (contracts/sound-profile.md).
pub fn similarity(reference: &SoundProfile, render: &SoundProfile) -> f32 {
    let s = similarity::similarity(reference, render);
    if s.is_finite() { s.clamp(0.0, 1.0) } else { 0.0 }
}

/// The originality guard's check of one render against one reference (FR-18).
pub fn originality(reference: &SoundProfile, render: &SoundProfile) -> Originality {
    let r: OriginalityReport = originality::check(reference, render);
    Originality {
        passed: r.passed,
        melodic_similarity: r.melodic_similarity,
        rhythmic_similarity: r.rhythmic_similarity,
        reason: r.reason.clone(),
        reference: reference
            .style_name
            .clone()
            .unwrap_or_else(|| reference.source.file_name.clone()),
    }
}

/// The originality verdict the matcher reports (the analyzer's report plus which reference it
/// was checked against).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Originality {
    pub passed: bool,
    pub melodic_similarity: f32,
    pub rhythmic_similarity: f32,
    pub reason: String,
    pub reference: String,
}
