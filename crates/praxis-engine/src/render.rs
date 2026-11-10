//! Offline renderer. OWNED BY WP-F (engine). The signature of [`render_offline`] is a FROZEN
//! CONTRACT. Phase 0 version: renders the score's length of silence and refuses layers.

use crate::api::*;
use crate::instrument::InstrumentFactory;
use praxis_dsp::StereoBuffer;
use praxis_score::{Playback, Score, timeline, validate};

/// Renders `score` offline. Deterministic for identical inputs.
pub fn render_offline(
    score: &Score,
    factory: &dyn InstrumentFactory,
    opts: &RenderOptions,
) -> Result<RenderOutput, RenderError> {
    let _ = factory;
    let report = validate::validate(score);
    if !report.is_valid() {
        return Err(RenderError::InvalidScore(format!("{:?}", report.errors)));
    }
    if !score.layers.is_empty() {
        return Err(RenderError::Other(
            "layer rendering is not implemented yet (WP-F)".into(),
        ));
    }
    let sr = opts.sample_rate;
    let len = timeline::score_length_samples(score, sr);
    let tail = timeline::tail_samples(score, sr);
    let master = StereoBuffer::silent((len + tail) as usize);
    Ok(RenderOutput {
        master,
        stems: Vec::new(),
        report: RenderReport {
            sample_rate: sr,
            length_samples: len,
            tail_samples: tail,
            is_loop: matches!(score.playback, Playback::Loop),
            loudness_before_lufs: f64::NEG_INFINITY,
            loudness_lufs: f64::NEG_INFINITY,
            true_peak_dbtp: f64::NEG_INFINITY,
            ..RenderReport::default()
        },
    })
}
