//! The per-block render graph: one [`LayerNode`] per rendered layer (sequencer -> instrument ->
//! effect chain -> gain/pan) summed in score order into the master bus, then the master chain and
//! master gain.
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
//!   with the grid. The master chain's latency is compensated by the driver.

use crate::automation::Lane;
use crate::effects::EffectChain;
use crate::instrument::Instrument;
use crate::mixer;
use crate::sequencer::{Delivered, Sequencer};

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
    last: Option<f32>,
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
    instrument: Box<dyn Instrument>,
    seq: Sequencer,
    chain: EffectChain,
    gain_db: Lane,
    pan: Lane,
    params: Vec<ParamLane>,
    latency: u64,
    out_l: Vec<f32>,
    out_r: Vec<f32>,
    n: usize,
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
            out_l: vec![0.0; max_block.max(1)],
            out_r: vec![0.0; max_block.max(1)],
            n: 0,
        }
    }

    pub fn layer_id(&self) -> &str {
        &self.layer_id
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

    /// Runs the instrument and chain over `[0, latency)` so the output lines up with the grid.
    pub fn preroll(&mut self) {
        let max = self.out_l.len();
        let mut t = 0u64;
        while t < self.latency {
            let n = ((self.latency - t) as usize).min(max);
            self.run_instrument(t, n);
            self.chain
                .process(&mut self.out_l[..n], &mut self.out_r[..n]);
            t += n as u64;
        }
        self.n = 0;
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

    /// Processes the grid block `[grid_t, grid_t + n)`.
    pub fn process(&mut self, grid_t: u64, n: usize) {
        let n = n.min(self.out_l.len());
        self.n = n;
        if n == 0 {
            return;
        }
        self.set_block_params(grid_t);
        self.run_instrument(grid_t + self.latency, n);
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

/// All layers plus the master bus.
pub struct Graph {
    layers: Vec<LayerNode>,
    master: EffectChain,
    master_gain: f32,
    mix_l: Vec<f32>,
    mix_r: Vec<f32>,
    max_block: usize,
    n: usize,
}

impl Graph {
    /// `master_gain` is linear (frozen gain or, later, normalisation gain).
    pub fn new(
        layers: Vec<LayerNode>,
        master: EffectChain,
        master_gain: f32,
        max_block: usize,
    ) -> Graph {
        let max_block = max_block.max(1);
        Graph {
            layers,
            master,
            master_gain,
            mix_l: vec![0.0; max_block],
            mix_r: vec![0.0; max_block],
            max_block,
            n: 0,
        }
    }

    pub fn max_block(&self) -> usize {
        self.max_block
    }

    pub fn layers(&self) -> &[LayerNode] {
        &self.layers
    }

    pub fn master_latency(&self) -> u64 {
        self.master.latency_samples() as u64
    }

    pub fn master_tail(&self) -> u64 {
        self.master.tail_samples() as u64
    }

    /// Pre-rolls every latency-compensated layer. Call once before the first block.
    pub fn preroll(&mut self) {
        for l in &mut self.layers {
            l.preroll();
        }
    }

    /// Renders grid block `[grid_t, grid_t + n)`; `n <= max_block`.
    pub fn process_block(&mut self, grid_t: u64, n: usize) {
        let n = n.min(self.max_block);
        self.n = n;
        self.mix_l[..n].fill(0.0);
        self.mix_r[..n].fill(0.0);
        for l in &mut self.layers {
            l.process(grid_t, n);
            let (sl, sr) = l.output();
            mixer::sum_into(&mut self.mix_l[..n], &mut self.mix_r[..n], sl, sr);
        }
        self.master
            .process(&mut self.mix_l[..n], &mut self.mix_r[..n]);
        mixer::apply_gain(&mut self.mix_l[..n], &mut self.mix_r[..n], self.master_gain);
    }

    /// The last processed master block (after the master chain and gain).
    pub fn master_output(&self) -> (&[f32], &[f32]) {
        (&self.mix_l[..self.n], &self.mix_r[..self.n])
    }
}
