//! Building a graph for real-time playback, off the audio thread.

use super::BLOCK;
use super::player::Cursor;
use crate::api::RenderError;
use crate::effects::{BuiltinEffects, EffectChain, build_effect};
use crate::graph::Graph;
use crate::instrument::InstrumentFactory;
use crate::master::{self, MAX_GAIN_DB, MIN_GAIN_DB};
use crate::render::{apply_avoid_list, build_nodes, validate_for_render};
use praxis_dsp::Processor;
use praxis_dsp::util::db_to_gain_f64;
use praxis_score::{Playback, Score, timeline};

/// Settings for [`prepare`].
#[derive(Clone, Debug, PartialEq)]
pub struct RealtimeOptions {
    /// The device's sample rate. A prepared graph only plays in an engine of the same rate.
    pub sample_rate: u32,
    /// Master gain (dB) when the score has no `master.frozen_gain_db`: the gain the last offline
    /// render applied (`RenderReport::applied_gain_db`). The real-time path never measures
    /// loudness. `None` plays at 0 dB and says so in [`PreparedGraph::warnings`].
    pub master_gain_db: Option<f64>,
}

impl Default for RealtimeOptions {
    fn default() -> Self {
        RealtimeOptions {
            sample_rate: 48_000,
            master_gain_db: None,
        }
    }
}

/// Live mix state of one graph node (layer gain trim, mute, solo).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LiveLayer {
    /// Linear trim on top of the score's layer gain (1 = 0 dB).
    pub trim: f32,
    pub muted: bool,
    pub solo: bool,
    /// Gain applied at the end of the last block (ramps towards the target).
    pub current: f32,
}

/// A score's graph, fully built and preallocated, ready to be moved into the
/// [`super::RealtimeEngine`] (boxed, through the command queue) and played. Everything that
/// allocates happened in [`prepare`].
pub struct PreparedGraph {
    pub(super) graph: Graph,
    pub(super) limiter: Box<dyn Processor>,
    /// Master gain (linear) and the target it ramps to after `SetMasterGain`.
    pub(super) gain: f32,
    pub(super) gain_target: f32,
    gain_db: f64,
    pub(super) sample_rate: u32,
    pub(super) master_latency: u64,
    pub(super) limiter_latency: u64,
    /// Musical length (the bars) in samples; the loop period for loops.
    pub(super) length: u64,
    /// One-shots: bars plus the score's tail (where the offline render ends). Loops: `length`.
    pub(super) end: u64,
    pub(super) is_loop: bool,
    layer_ids: Vec<String>,
    /// Score layer index -> graph node index (None for avoid-listed layers).
    pub(super) node_of_layer: Vec<Option<usize>>,
    /// Per graph node.
    pub(super) live: Vec<LiveLayer>,
    pub(super) any_solo: bool,
    blocked: Vec<String>,
    warnings: Vec<String>,
    pub(super) cursor: Cursor,
}

impl std::fmt::Debug for PreparedGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedGraph")
            .field("sample_rate", &self.sample_rate)
            .field("layers", &self.layer_ids)
            .field("gain_db", &self.gain_db)
            .field("length", &self.length)
            .field("is_loop", &self.is_loop)
            .finish_non_exhaustive()
    }
}

impl PreparedGraph {
    /// Index of layer `id` in `score.layers`, for the layer commands.
    pub fn layer_index(&self, id: &str) -> Option<usize> {
        self.layer_ids.iter().position(|l| l == id)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The master gain this graph plays at (frozen, applied or 0 dB), in dB.
    pub fn master_gain_db(&self) -> f64 {
        self.gain_db
    }

    /// Musical length in samples (the loop period for loops).
    pub fn length_samples(&self) -> u64 {
        self.length
    }

    /// Where one-shot playback stops by itself (bars plus tail), in samples.
    pub fn end_samples(&self) -> u64 {
        self.end
    }

    pub fn is_loop(&self) -> bool {
        self.is_loop
    }

    /// Output latency of the master stage (master effects plus the limiter lookahead), in
    /// samples. The engine compensates it: playback from `p` sounds musical time `p` first.
    pub fn latency_samples(&self) -> u64 {
        self.master_latency + self.limiter_latency
    }

    /// Layers left out by the avoid-list.
    pub fn blocked_layers(&self) -> &[String] {
        &self.blocked
    }

    /// Non-fatal problems found while building (effects not built, no master gain...).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

/// Builds `score` for real-time playback. Call off the audio thread: this allocates, loads
/// instruments and builds effects. Validation and the avoid-list are applied as in
/// [`crate::render_offline`]. Unlike the offline renderer every allowed layer is built (muted and
/// unsoloed ones too, silent), so mute and solo can change live without rebuilding.
pub fn prepare(
    score: &Score,
    factory: &dyn InstrumentFactory,
    opts: &RealtimeOptions,
) -> Result<PreparedGraph, RenderError> {
    let sr = opts.sample_rate;
    if sr == 0 {
        return Err(RenderError::Other("sample_rate must be > 0".into()));
    }
    validate_for_render(score)?;
    let mut warnings = Vec::new();
    let (allowed, blocked) = apply_avoid_list(score, factory, &mut warnings);
    let (nodes, _max_tail, _notes) = build_nodes(
        score,
        factory,
        sr,
        BLOCK,
        &allowed,
        &blocked,
        &BuiltinEffects,
        &mut warnings,
    )?;
    let bpm = score.tempo.bpm as f32;
    let master_chain = EffectChain::build(
        &score.master.effects,
        &BuiltinEffects,
        sr as f32,
        BLOCK,
        bpm,
        "master",
        &mut warnings,
    );
    let graph = Graph::new(nodes, master_chain, BLOCK, &mut warnings)
        .map_err(|c| RenderError::InvalidScore(format!("sidechain cycle: {c}")))?;

    // The same limiter the offline master stage uses (master.rs).
    let loudness = &score.master.loudness;
    let ceiling = master::ceiling_dbtp(loudness);
    if ceiling != loudness.true_peak_dbtp {
        warnings.push(format!(
            "master: true_peak_dbtp {} is outside the limiter's range; using {ceiling} dBTP",
            loudness.true_peak_dbtp
        ));
    }
    let limiter = build_effect(&master::limiter_spec(loudness), sr as f32, BLOCK, 120.0)
        .map_err(|e| RenderError::Other(format!("master limiter: {e}")))?;
    let limiter_latency = limiter.latency_samples() as u64;

    let gain_db = match (score.master.frozen_gain_db, opts.master_gain_db) {
        (Some(g), _) if g.is_finite() => g.clamp(MIN_GAIN_DB, MAX_GAIN_DB),
        (_, Some(g)) if g.is_finite() => g.clamp(MIN_GAIN_DB, MAX_GAIN_DB),
        _ => {
            warnings.push(
                "master: no frozen gain and no applied gain given; playing at 0 dB (real time never measures loudness)"
                    .into(),
            );
            0.0
        }
    };
    let gain = db_to_gain_f64(gain_db) as f32;

    let length = timeline::score_length_samples(score, sr);
    let is_loop = matches!(score.playback, Playback::Loop);
    let end = if is_loop {
        length
    } else {
        length + timeline::tail_samples(score, sr)
    };

    let mut node_of_layer = vec![None; score.layers.len()];
    let mut live = Vec::with_capacity(allowed.len());
    for (node, &li) in allowed.iter().enumerate() {
        node_of_layer[li] = Some(node);
        let l = &score.layers[li];
        live.push(LiveLayer {
            trim: 1.0,
            muted: l.muted,
            solo: l.solo,
            current: 0.0,
        });
    }
    let master_latency = graph.master_latency();
    let mut prepared = PreparedGraph {
        graph,
        limiter,
        gain,
        gain_target: gain,
        gain_db,
        sample_rate: sr,
        master_latency,
        limiter_latency,
        length,
        end,
        is_loop,
        layer_ids: score.layers.iter().map(|l| l.id.clone()).collect(),
        node_of_layer,
        live,
        any_solo: false,
        blocked,
        warnings,
        cursor: Cursor::new(),
    };
    prepared.update_solo();
    for i in 0..prepared.live.len() {
        prepared.live[i].current = prepared.live_target(i);
    }
    Ok(prepared)
}
