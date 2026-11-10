//! Offline render API. FROZEN CONTRACT. See docs/contracts/engine.md.

use praxis_dsp::StereoBuffer;

/// Offline render settings. Determinism: identical options + score + factory => identical output.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderOptions {
    pub sample_rate: u32,
    /// Fixed processing block size for offline renders. Default 128.
    pub block_size: usize,
    /// Also return one stem per layer (post layer effects and gain/pan, pre master).
    pub stems: bool,
    /// Apply the master chain (effects, loudness normalisation, true-peak limiter).
    pub master: bool,
    /// Render only these layer ids (empty = all, honouring mute/solo).
    pub only_layers: Vec<String>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            sample_rate: 48_000,
            block_size: 128,
            stems: false,
            master: true,
            only_layers: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stem {
    pub layer_id: String,
    pub audio: StereoBuffer,
}

/// Measurements the engine takes while rendering.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RenderReport {
    pub sample_rate: u32,
    /// Musical length (bars) in samples. For loops this is the exact loop length.
    pub length_samples: u64,
    /// Samples of tail after the bars (one-shots only; loops wrap their tails).
    pub tail_samples: u64,
    pub is_loop: bool,
    /// Integrated loudness of the master before and after normalisation (LUFS).
    pub loudness_before_lufs: f64,
    pub loudness_lufs: f64,
    /// True peak of the final master (dBTP).
    pub true_peak_dbtp: f64,
    /// Gain applied by normalisation (or the score's frozen gain).
    pub applied_gain_db: f64,
    /// Loop cycles rendered before the captured cycle, so tails and effect state wrap.
    pub warmup_cycles: u32,
    pub notes_rendered: u64,
    /// Layers skipped because their class is on the avoid-list (FR-3 enforcement).
    pub blocked_layers: Vec<String>,
    pub render_seconds: f64,
    /// Non-fatal problems (an effect not built, a missing impulse response...). Never silent.
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderOutput {
    pub master: StereoBuffer,
    pub stems: Vec<Stem>,
    pub report: RenderReport,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RenderError {
    #[error("score is invalid: {0}")]
    InvalidScore(String),
    #[error(transparent)]
    Instrument(#[from] crate::instrument::InstrumentError),
    #[error("render failed: {0}")]
    Other(String),
}
