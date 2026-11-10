//! Offline renderer (FR-27). OWNED BY WP-F (engine). The signature of [`render_offline`] is a
//! FROZEN CONTRACT; see docs/contracts/engine.md.
//!
//! Pipeline: notes from `timeline::expand_notes` -> one instrument per layer -> layer effects ->
//! layer automation -> equal-power gain/pan -> stem; stems summed in `score.layers` order -> master
//! effects (the *bus*) -> loudness normalisation or frozen gain -> true-peak limiter (`master.rs`).
//! Compressors keyed by another layer get that layer's instrument output (`sidechain.rs`); effect
//! latency is compensated so stems stay on the grid (`latency.rs`).
//!
//! Layer selection, in this order:
//! 1. Avoid-list (FR-3): a layer whose class, or whose preset's intrinsic class
//!    (`InstrumentFactory::describe`), is in `constraints.avoid` never renders and is listed in
//!    `report.blocked_layers`. The validator's `avoided_class` error is therefore not fatal here:
//!    the engine enforces the avoid-list itself instead of trusting the score.
//! 2. `opts.only_layers` non-empty: exactly those layers render (mute/solo are ignored, it is an
//!    explicit request); unknown ids are reported in `warnings`.
//! 3. Otherwise mute/solo: muted layers are skipped; if any remaining layer is solo, only solo
//!    layers render.
//!
//! Stems are returned for rendered layers only, in score order.

use crate::api::*;
use crate::automation::Lane;
use crate::effects::{BuiltinEffects, EffectBuilder, EffectChain};
use crate::graph::{Graph, LayerNode, ParamLane, ParamTarget};
use crate::instrument::{InstrumentContext, InstrumentFactory};
use crate::loopwrap::{self, CapturePlan};
use crate::master::{self, MasterBus};
use crate::seeding;
use crate::sequencer::Sequencer;
use praxis_dsp::StereoBuffer;
use praxis_dsp::meter::true_peak_dbtp;
use praxis_score::params::effect_params;
use praxis_score::timeline::{self, NoteEvent, TimeMap};
use praxis_score::validate::{self, IssueCode};
use praxis_score::{Layer, Playback, Score};

/// Engine-level settings beyond [`RenderOptions`].
pub struct RenderConfig<'a> {
    /// Where effect processors come from. Default: [`BuiltinEffects`] (`praxis_dsp::fx`).
    pub effects: &'a dyn EffectBuilder,
    /// Minimum loop warm-up cycles (the contract's `k` is raised to at least this). Default 0.
    /// Tests use it to show that the captured cycle is already in steady state.
    pub min_warmup_cycles: u32,
}

impl Default for RenderConfig<'_> {
    fn default() -> Self {
        RenderConfig {
            effects: &BuiltinEffects,
            min_warmup_cycles: 0,
        }
    }
}

/// Renders `score` offline. Deterministic for identical inputs.
pub fn render_offline(
    score: &Score,
    factory: &dyn InstrumentFactory,
    opts: &RenderOptions,
) -> Result<RenderOutput, RenderError> {
    render_offline_with(score, factory, opts, &RenderConfig::default())
}

/// [`render_offline`] with engine-level settings (effect builder, warm-up override).
pub fn render_offline_with(
    score: &Score,
    factory: &dyn InstrumentFactory,
    opts: &RenderOptions,
    config: &RenderConfig<'_>,
) -> Result<RenderOutput, RenderError> {
    #[cfg(not(target_arch = "wasm32"))]
    let started = std::time::Instant::now();

    if opts.sample_rate == 0 {
        return Err(RenderError::Other("sample_rate must be > 0".into()));
    }
    if opts.block_size == 0 {
        return Err(RenderError::Other("block_size must be > 0".into()));
    }
    let report = validate::validate(score);
    let fatal: Vec<_> = report
        .errors
        .iter()
        .filter(|e| e.code != IssueCode::AvoidedClass)
        .collect();
    if !fatal.is_empty() {
        return Err(RenderError::InvalidScore(format!("{fatal:?}")));
    }

    let sr = opts.sample_rate;
    let bs = opts.block_size;
    let mut warnings = Vec::new();
    let tm = TimeMap::new(score, sr);
    let len = timeline::score_length_samples(score, sr);
    let is_loop = matches!(score.playback, Playback::Loop);
    let period = if is_loop { Some(len) } else { None };

    // ---- layer selection
    let (render_idx, blocked) = select_layers(score, factory, opts, &mut warnings);

    // ---- build the graph (all allocation happens here, before the first block)
    let notes = timeline::expand_notes(score, sr);
    let bpm = score.tempo.bpm as f32;
    let keys = key_layers(score, &render_idx, &blocked);
    let mut nodes = Vec::with_capacity(render_idx.len() + keys.len());
    let mut max_tail = 0u64;
    let mut notes_rendered = 0u64;
    for (li, layer) in score.layers.iter().enumerate() {
        let rendered = render_idx.contains(&li);
        if !rendered && !keys.contains(&li) {
            continue;
        }
        let ctx = InstrumentContext {
            sample_rate: sr as f32,
            max_block: bs,
            key: score.key,
            bpm: score.tempo.bpm,
            seed: seeding::layer_seed(score.seed, layer.seed_offset),
        };
        let instrument = factory.create(&layer.instrument, &ctx)?;
        let layer_notes: Vec<&NoteEvent> = notes.iter().filter(|n| n.layer == li).collect();
        let seq = Sequencer::new(&layer_notes, period, score.seed, layer.seed_offset);
        let node = if rendered {
            notes_rendered += layer_notes.len() as u64;
            let owner = format!("layer `{}`", layer.id);
            let chain = EffectChain::build(
                &layer.effects,
                config.effects,
                sr as f32,
                bs,
                bpm,
                &owner,
                &mut warnings,
            );
            let (gain, pan, params) =
                build_lanes(layer, factory, &chain, &tm, period, &owner, &mut warnings);
            LayerNode::new(
                layer.id.clone(),
                instrument,
                seq,
                chain,
                gain,
                pan,
                params,
                bs,
            )
        } else {
            warnings.push(format!(
                "layer `{}` is not rendered but keys a sidechain; it runs as a key only (not mixed, no stem)",
                layer.id
            ));
            LayerNode::key_only(layer.id.clone(), instrument, seq, bs)
        };
        let layer_tail = node.sequencer().overhang(len) + node.tail_samples();
        max_tail = max_tail.max(layer_tail);
        nodes.push(node);
    }

    let master_chain = if opts.master {
        EffectChain::build(
            &score.master.effects,
            config.effects,
            sr as f32,
            bs,
            bpm,
            "master",
            &mut warnings,
        )
    } else {
        EffectChain::empty()
    };
    let mut master_bus = if opts.master {
        Some(MasterBus::new(&score.master, sr, bs, &mut warnings).map_err(RenderError::Other)?)
    } else {
        None
    };

    let mut graph = Graph::new(nodes, master_chain, bs, &mut warnings)
        .map_err(|c| RenderError::InvalidScore(format!("sidechain cycle: {c}")))?;
    max_tail = max_tail.saturating_add(graph.master_tail());

    let plan: CapturePlan = if is_loop {
        loopwrap::plan_loop(len, max_tail, sr, config.min_warmup_cycles)
    } else {
        loopwrap::plan_one_shot(len, timeline::tail_samples(score, sr))
    };
    let master_latency = graph.master_latency();
    let cap_len = plan.len as usize;

    let mut bus = StereoBuffer::silent(cap_len);
    let mut stems: Vec<StereoBuffer> = if opts.stems {
        (0..graph.layers().len())
            .map(|_| StereoBuffer::silent(cap_len))
            .collect()
    } else {
        Vec::new()
    };

    // ---- the block loop: no allocation from here on
    graph.preroll();
    let grid_end = plan.end() + master_latency;
    let master_start = plan.start + master_latency;
    let mut t = 0u64;
    while t < grid_end {
        let n = ((grid_end - t) as usize).min(bs);
        graph.process_block(t, n);
        let (ms, me) = (master_start, master_start + plan.len);
        copy_window(graph.master_output(), t, ms, me, &mut bus);
        if opts.stems {
            for (node, stem) in graph.layers().iter().zip(stems.iter_mut()) {
                if !node.is_key_only() {
                    copy_window(node.output(), t, plan.start, plan.end(), stem);
                }
            }
        }
        t += n as u64;
    }

    // ---- master: gain (frozen or measured) and the true-peak limiter
    let (master, loudness_before, loudness, true_peak, applied_gain_db) = match &mut master_bus {
        Some(mb) => {
            let r = mb.finish(&bus, is_loop, &mut warnings);
            (
                r.audio,
                r.loudness_before_lufs,
                r.loudness_lufs,
                r.true_peak_dbtp,
                r.applied_gain_db,
            )
        }
        None => {
            let l = master::measure_lufs(&bus, sr, is_loop);
            let tp = true_peak_dbtp(&bus.left, &bus.right, sr as f32);
            (bus, l, l, tp, 0.0)
        }
    };

    let stems = graph
        .layers()
        .iter()
        .zip(stems)
        .filter(|(node, _)| !node.is_key_only())
        .map(|(node, audio)| Stem {
            layer_id: node.layer_id().to_string(),
            audio,
        })
        .collect();

    #[cfg(not(target_arch = "wasm32"))]
    let render_seconds = started.elapsed().as_secs_f64();
    #[cfg(target_arch = "wasm32")]
    let render_seconds = 0.0;

    Ok(RenderOutput {
        master,
        stems,
        report: RenderReport {
            sample_rate: sr,
            length_samples: len,
            tail_samples: if is_loop {
                0
            } else {
                timeline::tail_samples(score, sr)
            },
            is_loop,
            loudness_before_lufs: loudness_before,
            loudness_lufs: loudness,
            true_peak_dbtp: true_peak,
            applied_gain_db,
            warmup_cycles: plan.warmup_cycles,
            notes_rendered,
            blocked_layers: blocked,
            render_seconds,
            warnings,
        },
    })
}

/// Copies the part of block `[t, t + block.len())` that lies in `[start, end)` into `out`
/// (indexed from `start`).
fn copy_window(block: (&[f32], &[f32]), t: u64, start: u64, end: u64, out: &mut StereoBuffer) {
    let (l, r) = block;
    let b_end = t + l.len() as u64;
    let from = t.max(start);
    let to = b_end.min(end);
    if from >= to {
        return;
    }
    let src = (from - t) as usize;
    let dst = (from - start) as usize;
    let n = (to - from) as usize;
    out.left[dst..dst + n].copy_from_slice(&l[src..src + n]);
    out.right[dst..dst + n].copy_from_slice(&r[src..src + n]);
}

/// Indices of layers that are not rendered (and not blocked) but key a sidechain of a rendered
/// layer, in score order.
fn key_layers(score: &Score, render_idx: &[usize], blocked: &[String]) -> Vec<usize> {
    let mut out = Vec::new();
    for &li in render_idx {
        for e in &score.layers[li].effects {
            if e.bypass {
                continue;
            }
            let Some(k) = e.sidechain.as_deref() else {
                continue;
            };
            let Some(ki) = score.layers.iter().position(|l| l.id == k) else {
                continue;
            };
            if !render_idx.contains(&ki) && !blocked.iter().any(|b| b == k) && !out.contains(&ki) {
                out.push(ki);
            }
        }
    }
    out.sort_unstable();
    out
}

/// Returns (indices of layers to render, ids of layers blocked by the avoid-list).
fn select_layers(
    score: &Score,
    factory: &dyn InstrumentFactory,
    opts: &RenderOptions,
    warnings: &mut Vec<String>,
) -> (Vec<usize>, Vec<String>) {
    let avoid = &score.constraints.avoid;
    let mut blocked = Vec::new();
    let mut allowed = Vec::new();
    for (i, l) in score.layers.iter().enumerate() {
        let intrinsic = factory
            .describe(l.instrument.kind, &l.instrument.preset)
            .map(|p| p.class);
        if avoid.contains(&l.class) {
            warnings.push(format!(
                "layer `{}` blocked: class {:?} is on the avoid-list",
                l.id, l.class
            ));
            blocked.push(l.id.clone());
        } else if let Some(c) = intrinsic.filter(|c| avoid.contains(c)) {
            warnings.push(format!(
                "layer `{}` blocked: preset `{}` is intrinsically {:?}, which is on the avoid-list",
                l.id, l.instrument.preset, c
            ));
            blocked.push(l.id.clone());
        } else {
            allowed.push(i);
        }
    }
    for id in &opts.only_layers {
        if score.layer(id).is_none() {
            warnings.push(format!("only_layers: no layer `{id}` in the score"));
        }
    }
    let render: Vec<usize> = if !opts.only_layers.is_empty() {
        allowed
            .into_iter()
            .filter(|&i| opts.only_layers.iter().any(|id| *id == score.layers[i].id))
            .collect()
    } else {
        let any_solo = allowed.iter().any(|&i| score.layers[i].solo);
        allowed
            .into_iter()
            .filter(|&i| {
                let l = &score.layers[i];
                !l.muted && (!any_solo || l.solo)
            })
            .collect()
    };
    (render, blocked)
}

/// Builds the gain/pan lanes and the instrument/effect parameter lanes of one layer.
fn build_lanes(
    layer: &Layer,
    factory: &dyn InstrumentFactory,
    chain: &EffectChain,
    tm: &TimeMap,
    period: Option<u64>,
    owner: &str,
    warnings: &mut Vec<String>,
) -> (Lane, Lane, Vec<ParamLane>) {
    let mut gain: Option<Lane> = None;
    let mut pan: Option<Lane> = None;
    let mut params: Vec<ParamLane> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    let info = factory.describe(layer.instrument.kind, &layer.instrument.preset);
    for au in &layer.automation {
        let t = au.target.as_str();
        if seen.contains(&t) {
            warnings.push(format!(
                "{owner}: duplicate automation of `{t}`; the first lane is used"
            ));
            continue;
        }
        seen.push(t);
        let lane = Lane::new(au, tm, period);
        if t == "gain_db" {
            gain = Some(lane);
        } else if t == "pan" {
            pan = Some(lane);
        } else if let Some(name) = t.strip_prefix("instrument.") {
            let found = info.as_ref().and_then(|i| {
                i.params
                    .iter()
                    .position(|p| p.name == name)
                    .map(|ix| (ix, i))
            });
            match found {
                Some((ix, i)) => params.push(ParamLane::new(
                    ParamTarget::Instrument(ix),
                    lane,
                    i.params[ix].min,
                    i.params[ix].max,
                )),
                None => warnings.push(format!(
                    "{owner}: automation target `{t}` is not a parameter of preset `{}`; skipped",
                    layer.instrument.preset
                )),
            }
        } else if let Some(rest) = t.strip_prefix("effects.") {
            let resolved = rest.split_once('.').and_then(|(eid, pname)| {
                let slot = chain.slot_of(eid)?;
                let kind = chain.slot_kind(slot)?;
                let index = effect_params(kind).iter().position(|s| s.name == pname)?;
                let spec = effect_params(kind)[index];
                Some((slot, index, spec.min, spec.max))
            });
            match resolved {
                Some((slot, index, min, max)) => params.push(ParamLane::new(
                    ParamTarget::Effect { slot, index },
                    lane,
                    min,
                    max,
                )),
                None => warnings.push(format!(
                    "{owner}: automation target `{t}` skipped: its effect was not built or has no such parameter"
                )),
            }
        } else {
            warnings.push(format!("{owner}: unknown automation target `{t}`; skipped"));
        }
    }
    (
        gain.unwrap_or_else(|| Lane::constant(layer.gain_db as f64)),
        pan.unwrap_or_else(|| Lane::constant(layer.pan as f64)),
        params,
    )
}
