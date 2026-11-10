//! Objective checks of a render against its Score. FROZEN CONTRACT for the types and the
//! `verify_render` signature; WP-D implements the body. The CLI `praxis check` prints this report.

use praxis_dsp::StereoBuffer;
use praxis_score::Score;
use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Check {
    /// "tempo", "key", "grid", "loudness", "true_peak", "loop_seam", "clipping", "silence".
    pub name: String,
    pub passed: bool,
    pub measured: String,
    pub expected: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct VerifyReport {
    pub checks: Vec<Check>,
    /// Detected tempo, BPM.
    pub tempo_bpm: Option<f64>,
    /// Detected key, e.g. "a minor".
    pub key: Option<String>,
    /// Median absolute onset deviation from the nearest grid step, ms.
    pub grid_deviation_ms: Option<f64>,
    /// Fraction of detected onsets within 10 ms of a grid step.
    pub onsets_on_grid: Option<f64>,
    pub integrated_lufs: Option<f64>,
    pub true_peak_dbtp: Option<f64>,
    /// Sample-to-sample jump at the loop point relative to the typical jump (1.0 = ordinary).
    pub seam_jump_ratio: Option<f64>,
    /// Seam discontinuity minus the local noise floor, dB (<= 0 passes).
    pub seam_excess_db: Option<f64>,
}

impl VerifyReport {
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed)
    }
}

/// Tolerances for [`verify_render`]. Defaults follow PRD section 2.
#[derive(Clone, Debug, PartialEq)]
pub struct Tolerances {
    pub tempo_bpm: f64,
    pub loudness_lu: f64,
    pub onset_ms: f64,
    pub min_on_grid: f64,
}

impl Default for Tolerances {
    fn default() -> Self {
        Tolerances {
            tempo_bpm: 2.0,
            loudness_lu: 0.5,
            onset_ms: 10.0,
            min_on_grid: 0.9,
        }
    }
}

/// Measures `audio` (the master render of `score`) and compares it with what the score says.
pub fn verify_render(
    score: &Score,
    audio: &StereoBuffer,
    sample_rate: u32,
    tol: &Tolerances,
) -> VerifyReport {
    let _ = (score, audio, sample_rate, tol);
    VerifyReport::default()
}
