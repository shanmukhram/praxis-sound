//! The per-block render graph: one [`LayerNode`] per rendered layer (sequencer -> instrument ->
//! effect chain -> gain/pan) summed in score order into the master bus, then the master chain.
//! Master gain and the true-peak limiter run after the graph, in `master.rs`, because loudness
//! normalisation must measure the bus first.
//!
//! The graph is built and preallocated completely before the first block; [`Graph::process_block`]
//! performs no allocation, locking or I/O, so the same code can later be driven by the real-time
//! callback (WP-R). The offline renderer (`render.rs`) drives it over a timeline and copies out the
//! captured window.
//!
//! Timing rules:
//! - Blocks are fixed: the driver calls `process_block` at multiples of the block size from time
//!   zero. Automation is evaluated at these fixed boundaries only.
//! - Inside a block each layer splits its *instrument* processing at its own note events, so every
//!   `note_on`/`note_off` lands on its exact sample and instruments receive `note_on` at the start
//!   of a `process` call. Splits depend only on that layer's own events: editing one layer never
//!   changes how another layer is processed.
//! - Latency compensation: a layer whose chain reports `latency_samples = d` runs its instrument
//!   `d` samples ahead of the grid (pre-rolled before the first block), so its output is aligned
//!   with the grid. The master chain's latency is compensated by the driver (see `latency.rs`).
//! - Sidechain (see `sidechain.rs`): layers are processed in an order that puts key layers first;
//!   each key layer writes its instrument output (pre effects) into a [`KeyTap`] and keyed
//!   compressors read it at the musical time they process. Sums are always in score order.

use crate::automation::Lane;
use crate::effects::EffectChain;
use crate::instrument::Instrument;
use crate::latency::{KeyAlignment, key_alignment, tap_capacity};
use crate::mixer;
use crate::sequencer::{Delivered, Sequencer};
use crate::sidechain::{Cycle, KeyTap, processing_order};

/// One sidechained compressor of a layer: chain slot, key node, alignment.
#[derive(Clone, Copy, Debug)]
struct SidechainFeed {
    slot: usize,
    key: usize,
    align: KeyAlignment,
}

/// What a parameter lane drives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamTarget {
    /// Instrument parameter by index into `PresetInfo::params`.
    Instrument(usize),
    /// Effect parameter: built chain slot and index into the kind's spec table.
    Effect { slot: usize, index: usize },
}

/// An automation lane for an instrument or effect parameter, set once per block.
#[derive(Clone, Debug)]
pub struct ParamLane {
    pub target: ParamTarget,
    pub lane: Lane,
    pub min: f64,
    pub max: f64,
    pub(crate) last: Option<f32>,
}

impl ParamLane {
    pub fn new(target: ParamTarget, lane: Lane, min: f64, max: f64) -> ParamLane {
        ParamLane {
            target,
            lane,
            min,
            max,
            last: None,
        }
    }
}

/// One layer of the graph.
pub struct LayerNode {
    layer_id: String,
    pub(crate) instrument: Box<dyn Instrument>,
    pub(crate) seq: Sequencer,
    pub(crate) chain: EffectChain,
    gain_db: Lane,
    pan: Lane,
    pub(crate) params: Vec<ParamLane>,
    pub(crate) latency: u64,
    key_only: bool,
    feeds: Vec<SidechainFeed>,
    sc_l: Vec<f32>,
    sc_r: Vec<f32>,
    out_l: Vec<f32>,
    out_r: Vec<f32>,
    pub(crate) n: usize,
}

impl LayerNode {
    /// Builds a layer node; allocates its block buffers (`max_block` samples per channel).
    pub fn new(
        layer_id: String,
        instrument: Box<dyn Instrument>,
        seq: Sequencer,
        chain: EffectChain,
        gain_db: Lane,
        pan: Lane,
        params: Vec<ParamLane>,
        max_block: usize,
    ) -> LayerNode {
        let latency = chain.latency_samples() as u64;
        LayerNode {
            layer_id,
            instrument,
            seq,
            chain,
            gain_db,
            pan,
            params,
            latency,
            key_only: false,
            feeds: Vec::new(),
            sc_l: Vec::new(),
            sc_r: Vec::new(),
            out_l: vec![0.0; max_block.max(1)],
            out_r: vec![0.0; max_block.max(1)],
            n: 0,
        }
    }

    /// A node that only runs its instrument to key other layers' compressors: it is not mixed
    /// and has no stem (its layer is not rendered, see `sidechain.rs`).
    pub fn key_only(
        layer_id: String,
        instrument: Box<dyn Instrument>,
        seq: Sequencer,
        max_block: usize,
    ) -> LayerNode {
        let mut n = LayerNode::new(
            layer_id,
            instrument,
            seq,
            EffectChain::empty(),
            Lane::constant(0.0),
            Lane::constant(0.0),
            Vec::new(),
            max_block,
        );
        n.key_only = true;
        n
    }

    pub fn layer_id(&self) -> &str {
        &self.layer_id
    }

    pub fn is_key_only(&self) -> bool {
        self.key_only
    }

    pub fn chain(&self) -> &EffectChain {
        &self.chain
    }

    /// Instrument release tail plus the chain's effect tails, in samples.
    pub fn tail_samples(&self) -> u64 {
        self.instrument.tail_samples() as u64 + self.chain.tail_samples() as u64
    }

    pub fn latency_samples(&self) -> u64 {
        self.latency
    }

    pub fn sequencer(&self) -> &Sequencer {
        &self.seq
    }

    /// The last processed block (post gain/pan).
    pub fn output(&self) -> (&[f32], &[f32]) {
        (&self.out_l[..self.n], &self.out_r[..self.n])
    }

    /// Runs the instrument for `n` samples starting at instrument time `t0`, splitting at events.
    fn run_instrument(&mut self, t0: u64, n: usize) {
        let mut pos = 0usize;
        while pos < n {
            let now = t0 + pos as u64;
            while let Some(t) = self.seq.peek_time() {
                if t > now {
                    break;
                }
                match self.seq.pop() {
                    Some(Delivered::On(note)) => self.instrument.note_on(&note),
                    Some(Delivered::Off(id)) => self.instrument.note_off(id),
                    None => break,
                }
            }
            let chunk = match self.seq.peek_time() {
                Some(t) => ((t - now) as usize).min(n - pos),
                None => n - pos,
            }
            .max(1);
            let end = pos + chunk;
            self.instrument
                .process(&mut self.out_l[pos..end], &mut self.out_r[pos..end]);
            pos = end;
        }
    }

    /// Reads the sidechain keys for a chain-input block at musical time `start` and hands them to
    /// the compressors.
    fn feed_sidechains(&mut self, start: u64, n: usize, taps: &[Option<KeyTap>]) {
        for f in &self.feeds {
            if let Some(tap) = taps.get(f.key).and_then(|t| t.as_ref()) {
                tap.read(
                    f.align.key_start(start),
                    &mut self.sc_l[..n],
                    &mut self.sc_r[..n],
                );
                self.chain
                    .set_sidechain(f.slot, &self.sc_l[..n], &self.sc_r[..n]);
            }
        }
    }

    fn set_block_params(&mut self, grid_t: u64) {
        for p in &mut self.params {
            let v = p.lane.value_at(grid_t).clamp(p.min, p.max) as f32;
            if p.last == Some(v) {
                continue;
            }
            p.last = Some(v);
            match p.target {
                ParamTarget::Instrument(i) => self.instrument.set_param(i, v),
                ParamTarget::Effect { slot, index } => self.chain.set_param(slot, index, v),
            }
        }
    }

    /// First half of grid block `[grid_t, grid_t + n)`: parameters and instrument. Afterwards
    /// the output buffers hold the instrument output (pre effects), the sidechain key.
    fn front(&mut self, grid_t: u64, n: usize) {
        let n = n.min(self.out_l.len());
        self.n = n;
        if n == 0 {
            return;
        }
        self.set_block_params(grid_t);
        self.run_instrument(grid_t + self.latency, n);
    }

    /// Second half: sidechain keys, effect chain, gain/pan.
    fn back(&mut self, grid_t: u64, taps: &[Option<KeyTap>]) {
        let n = self.n;
        if n == 0 || self.key_only {
            return;
        }
        self.feed_sidechains(grid_t + self.latency, n, taps);
        self.chain
            .process(&mut self.out_l[..n], &mut self.out_r[..n]);
        let at = |gain: &Lane, pan: &Lane, t: u64| {
            mixer::layer_gains(
                gain.value_at(t).clamp(-60.0, 24.0) as f32,
                pan.value_at(t).clamp(-1.0, 1.0) as f32,
            )
        };
        let g0 = at(&self.gain_db, &self.pan, grid_t);
        let g1 = at(&self.gain_db, &self.pan, grid_t + n as u64);
        mixer::apply_gain_ramp(&mut self.out_l[..n], &mut self.out_r[..n], g0, g1);
    }
}

/// All layers plus the master bus (sum and master effects; gain and limiter are in `master.rs`).
pub struct Graph {
    pub(crate) layers: Vec<LayerNode>,
    order: Vec<usize>,
    pub(crate) taps: Vec<Option<KeyTap>>,
    pub(crate) master: EffectChain,
    pub(crate) mix_l: Vec<f32>,
    pub(crate) mix_r: Vec<f32>,
    max_block: usize,
    n: usize,
}

impl Graph {
    /// Wires the sidechains (keys by layer id among `layers`) and allocates every buffer.
    /// Problems (a key layer that is not in the graph, a key read late because of latency) go to
    /// `warnings`; a sidechain cycle is an error.
    pub fn new(
        mut layers: Vec<LayerNode>,
        master: EffectChain,
        max_block: usize,
        warnings: &mut Vec<String>,
    ) -> Result<Graph, Cycle> {
        let max_block = max_block.max(1);
        let ids: Vec<String> = layers.iter().map(|l| l.layer_id.clone()).collect();
        let lat: Vec<u64> = layers.iter().map(|l| l.latency).collect();
        let mut keys: Vec<Vec<usize>> = vec![Vec::new(); layers.len()];
        for (i, node) in layers.iter_mut().enumerate() {
            let mut feeds = Vec::new();
            for (slot, key_id) in node.chain.sidechains() {
                let Some(k) = ids.iter().position(|id| *id == key_id) else {
                    warnings.push(format!(
                        "layer `{}`: sidechain key layer `{key_id}` is not rendered (avoid-list); the compressor runs unkeyed",
                        node.layer_id
                    ));
                    continue;
                };
                let pre = node.chain.latency_before(slot) as u64;
                let align = key_alignment(node.latency, pre, lat[k]);
                if align.lag > 0 {
                    warnings.push(format!(
                        "layer `{}`: sidechain key from `{key_id}` arrives {} samples late (latency of the effects after the compressor)",
                        node.layer_id, align.lag
                    ));
                }
                keys[i].push(k);
                feeds.push(SidechainFeed {
                    slot,
                    key: k,
                    align,
                });
            }
            if !feeds.is_empty() {
                node.sc_l = vec![0.0; max_block];
                node.sc_r = vec![0.0; max_block];
            }
            node.feeds = feeds;
        }
        let mut taps: Vec<Option<KeyTap>> = (0..layers.len()).map(|_| None).collect();
        for &j in keys.iter().flatten() {
            if taps[j].is_none() {
                taps[j] = Some(KeyTap::new(tap_capacity(lat[j], max_block)));
            }
        }
        let order = processing_order(&ids, &keys)?;
        Ok(Graph {
            layers,
            order,
            taps,
            master,
            mix_l: vec![0.0; max_block],
            mix_r: vec![0.0; max_block],
            max_block,
            n: 0,
        })
    }

    pub fn max_block(&self) -> usize {
        self.max_block
    }

    /// Every node in score order, including key-only nodes (see [`LayerNode::is_key_only`]).
    pub fn layers(&self) -> &[LayerNode] {
        &self.layers
    }

    pub fn master_latency(&self) -> u64 {
        self.master.latency_samples() as u64
    }

    pub fn master_tail(&self) -> u64 {
        self.master.tail_samples() as u64
    }

    /// Pre-rolls every latency-compensated layer over `[0, latency)`, keys first. Call once
    /// before the first block.
    pub fn preroll(&mut self) {
        self.preroll_from(0);
    }

    /// [`Self::preroll`] for a graph that starts at grid time `start` instead of zero: every
    /// latency-compensated layer runs its instrument over `[start, start + latency)`. The
    /// sequencers must already be positioned at `start` (the real-time engine's seek).
    pub(crate) fn preroll_from(&mut self, start: u64) {
        for oi in 0..self.order.len() {
            let i = self.order[oi];
            let end = start + self.layers[i].latency;
            let max = self.layers[i].out_l.len();
            let mut t = start;
            while t < end {
                let n = ((end - t) as usize).min(max);
                let node = &mut self.layers[i];
                node.run_instrument(t, n);
                if let Some(tap) = self.taps[i].as_mut() {
                    tap.write(t, &node.out_l[..n], &node.out_r[..n]);
                }
                node.feed_sidechains(t, n, &self.taps);
                node.chain
                    .process(&mut node.out_l[..n], &mut node.out_r[..n]);
                t += n as u64;
            }
            self.layers[i].n = 0;
        }
    }

    /// Renders grid block `[grid_t, grid_t + n)`; `n <= max_block`.
    pub fn process_block(&mut self, grid_t: u64, n: usize) {
        let n = self.process_layers(grid_t, n);
        self.mix_l[..n].fill(0.0);
        self.mix_r[..n].fill(0.0);
        for l in &self.layers {
            if l.key_only {
                continue;
            }
            let (sl, sr) = l.output();
            mixer::sum_into(&mut self.mix_l[..n], &mut self.mix_r[..n], sl, sr);
        }
        self.process_master_chain();
    }

    /// First part of [`Self::process_block`]: every layer (keys first) into its own output
    /// buffer. Returns the block length actually processed. The real-time engine sums the layers
    /// itself (live gain, mute and solo) and then calls [`Self::process_master_chain`].
    pub(crate) fn process_layers(&mut self, grid_t: u64, n: usize) -> usize {
        let n = n.min(self.max_block);
        self.n = n;
        for oi in 0..self.order.len() {
            let i = self.order[oi];
            let node = &mut self.layers[i];
            node.front(grid_t, n);
            if let Some(tap) = self.taps[i].as_mut() {
                let m = node.n;
                tap.write(grid_t + node.latency, &node.out_l[..m], &node.out_r[..m]);
            }
            node.back(grid_t, &self.taps);
        }
        n
    }

    /// Last part of [`Self::process_block`]: the master chain over the summed bus block.
    pub(crate) fn process_master_chain(&mut self) {
        let n = self.n;
        self.master
            .process(&mut self.mix_l[..n], &mut self.mix_r[..n]);
    }

    /// The last processed master bus block (after the master chain, before gain and limiter).
    pub fn master_output(&self) -> (&[f32], &[f32]) {
        (&self.mix_l[..self.n], &self.mix_r[..self.n])
    }
}
