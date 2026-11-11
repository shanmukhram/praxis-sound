//! Playback of one prepared graph: the cursor, hard starts, loop jumps, the per-block path
//! (layers, live mix, master chain, master gain, limiter) and pulling any number of frames.
//!
//! Everything here runs on the audio thread: no allocation, locks, I/O or panics, and every loop
//! is bounded by a block, a callback or the event list.
//!
//! **Block grid.** The graph is driven in blocks that end on multiples of [`BLOCK`] grid samples
//! (the offline renderer's grid), so automation is evaluated at the same points as offline. A
//! loop jump or a start in the middle of a block adds one short block to get back onto the grid.
//!
//! **Latency.** A hard start at musical time `s` starts the graph at the block boundary
//! `A <= s`, pre-rolls latency-compensated layers exactly like the offline renderer, drops the
//! first `s - A + master_latency` bus samples before the limiter and the limiter's latency after
//! it. The first output sample is then musical time `s`, and from `s = 0` the limiter sees
//! exactly the bus the offline master stage sees, in the same 128-sample chunks.

use super::BLOCK;
use super::prepare::PreparedGraph;
use crate::sequencer::Sequencer;
use praxis_dsp::util::sanitize;

/// A loop range in musical samples, `start < end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopRange {
    pub start: u64,
    pub end: u64,
}

/// Playback state of one graph.
pub(crate) struct Cursor {
    /// Grid time of the next graph block.
    grid_t: u64,
    /// The graph has processed audio since its last reset.
    dirty: bool,
    /// Bus samples to drop before the limiter (start alignment and master-chain latency).
    bus_skip: u64,
    /// Samples fed to the limiter since the start (its chunks are BLOCK long from there).
    lim_count: u64,
    /// Limiter output samples to drop (its latency).
    out_skip: u64,
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    buf_len: usize,
    buf_pos: usize,
    /// Musical time of the next output sample (within the period for loops).
    pub(super) out_pos: u64,
    /// A one-shot played past its end.
    pub(super) ended: bool,
}

impl Cursor {
    pub(super) fn new() -> Cursor {
        Cursor {
            grid_t: 0,
            dirty: false,
            bus_skip: 0,
            lim_count: 0,
            out_skip: 0,
            buf_l: vec![0.0; BLOCK],
            buf_r: vec![0.0; BLOCK],
            buf_len: 0,
            buf_pos: 0,
            out_pos: 0,
            ended: false,
        }
    }
}

/// Positions `seq` so its next event is the first at or after time `t`. Note-offs of notes that
/// started before `t` follow; instruments ignore offs for ids they do not hold.
fn seek_sequencer(seq: &mut Sequencer, t: u64) {
    match seq.period {
        None => {
            seq.cycle = 0;
            seq.index = seq.events.partition_point(|e| e.pos < t);
        }
        Some(p) => {
            seq.cycle = t / p;
            let pos = t % p;
            seq.index = seq.events.partition_point(|e| e.pos < pos);
        }
    }
}

impl PreparedGraph {
    pub(super) fn update_solo(&mut self) {
        self.any_solo = self.live.iter().any(|l| l.solo);
    }

    /// Gain node `i` ramps to: 0 when muted or not soloed while something is, else its trim.
    pub(super) fn live_target(&self, i: usize) -> f32 {
        let l = &self.live[i];
        if l.muted || (self.any_solo && !l.solo) {
            0.0
        } else {
            l.trim
        }
    }

    /// Musical time of grid time `t` (within the period for loops).
    fn musical(&self, t: u64) -> u64 {
        if self.is_loop {
            t % self.length.max(1)
        } else {
            t
        }
    }

    /// The loop range this graph honours: clamped to the loop period for loops, ignored when it
    /// is empty or (for loops) covers the whole period, which already repeats seamlessly.
    pub(super) fn effective_loop(&self, r: Option<LoopRange>) -> Option<LoopRange> {
        let r = r?;
        let (start, end) = if self.is_loop {
            (r.start.min(self.length), r.end.min(self.length))
        } else {
            (r.start, r.end)
        };
        if start >= end || (self.is_loop && start == 0 && end == self.length) {
            return None;
        }
        Some(LoopRange { start, end })
    }

    /// Clears every piece of state the graph built up while playing.
    fn reset_state(&mut self) {
        for node in &mut self.graph.layers {
            node.instrument.reset();
            node.chain.reset();
            for p in &mut node.params {
                p.last = None;
            }
            node.n = 0;
        }
        self.graph.master.reset();
    }

    /// Hard start at musical time `s`: fresh state, positioned, pre-rolled (see the module docs).
    pub(super) fn start_at(&mut self, s: u64) {
        let s = if self.is_loop {
            s % self.length.max(1)
        } else {
            s.min(self.end)
        };
        if self.cursor.dirty {
            self.reset_state();
        }
        // Offline resets its limiter before use (which also snaps the ceiling ramp).
        self.limiter.reset();
        let aligned = s - s % BLOCK as u64;
        for node in &mut self.graph.layers {
            seek_sequencer(&mut node.seq, aligned);
        }
        for tap in self.graph.taps.iter_mut().flatten() {
            tap.l.iter_mut().for_each(|v| *v = 0.0);
            tap.r.iter_mut().for_each(|v| *v = 0.0);
            tap.written = aligned;
        }
        self.graph.preroll_from(aligned);
        self.gain = self.gain_target;
        for i in 0..self.live.len() {
            self.live[i].current = self.live_target(i);
        }
        let c = &mut self.cursor;
        c.grid_t = aligned;
        c.bus_skip = (s - aligned) + self.master_latency;
        c.lim_count = 0;
        c.out_skip = self.limiter_latency;
        c.buf_len = 0;
        c.buf_pos = 0;
        c.out_pos = s;
        c.ended = !self.is_loop && s >= self.end;
        c.dirty = true;
    }

    /// Loop jump to grid time `t`: notes are released (their tails ring on), effect state is
    /// kept (reverb tails continue) and the sequencers continue from `t`.
    fn soft_jump(&mut self, t: u64) {
        let g = &mut self.graph;
        for (i, node) in g.layers.iter_mut().enumerate() {
            node.instrument.all_notes_off();
            // A layer running ahead by its chain latency jumps at its own instrument time.
            seek_sequencer(&mut node.seq, t + node.latency);
            if let Some(tap) = g.taps.get_mut(i).and_then(|t| t.as_mut()) {
                tap.written = t + node.latency;
            }
        }
        self.cursor.grid_t = t;
    }

    /// Renders the next graph block into the cursor buffer (after gain and limiter).
    fn render_block(&mut self, lp: Option<LoopRange>) {
        let t = self.cursor.grid_t;
        let mut n = BLOCK - (t % BLOCK as u64) as usize;
        let mut jump = None;
        if let Some(r) = lp {
            let m = self.musical(t);
            if m < r.end && r.end - m <= n as u64 {
                n = (r.end - m) as usize;
                jump = Some(t + n as u64 - (r.end - r.start));
            }
        }
        let n = self.graph.process_layers(t, n);

        // Live mix in score order (all nodes are mixed: prepare builds no key-only nodes).
        let g = &mut self.graph;
        g.mix_l[..n].fill(0.0);
        g.mix_r[..n].fill(0.0);
        let step = n as f32 / super::ramp_samples(self.sample_rate);
        for (i, node) in g.layers.iter().enumerate() {
            let l = &self.live[i];
            let target = if l.muted || (self.any_solo && !l.solo) {
                0.0
            } else {
                l.trim
            };
            let g0 = l.current;
            let max = step * g0.abs().max(target.abs()).max(1.0);
            let g1 = if (target - g0).abs() <= max {
                target
            } else if target > g0 {
                g0 + max
            } else {
                g0 - max
            };
            self.live[i].current = g1;
            let (sl, sr) = node.output();
            let (ml, mr) = (&mut g.mix_l[..n], &mut g.mix_r[..n]);
            if g0 == g1 {
                if g0 == 0.0 {
                    continue;
                }
                if g0 == 1.0 {
                    crate::mixer::sum_into(ml, mr, sl, sr);
                    continue;
                }
            }
            let inv = 1.0 / n.max(1) as f32;
            for k in 0..n.min(sl.len()) {
                let w = g0 + (g1 - g0) * (k as f32 * inv);
                ml[k] += sl[k] * w;
                mr[k] += sr[k] * w;
            }
        }
        g.process_master_chain();

        // Master gain (exactly `x * g` as offline while it is fixed), then the limiter.
        let c = &mut self.cursor;
        let mut from = 0usize;
        if c.bus_skip > 0 {
            let d = (c.bus_skip as usize).min(n);
            from = d;
            c.bus_skip -= d as u64;
        }
        let len = n - from;
        let (bl, br) = (&mut c.buf_l, &mut c.buf_r);
        let (ml, mr) = (&g.mix_l[from..n], &g.mix_r[from..n]);
        if self.gain == self.gain_target {
            let gain = self.gain;
            for k in 0..len {
                bl[k] = ml[k] * gain;
                br[k] = mr[k] * gain;
            }
        } else {
            // Linear ramp to the new gain over `ramp_samples`.
            let step =
                self.gain.abs().max(self.gain_target.abs()) / super::ramp_samples(self.sample_rate);
            for k in 0..len {
                let d = self.gain_target - self.gain;
                self.gain = if d.abs() <= step {
                    self.gain_target
                } else {
                    self.gain + step.copysign(d)
                };
                bl[k] = sanitize(ml[k] * self.gain);
                br[k] = sanitize(mr[k] * self.gain);
            }
        }
        let mut p = 0usize;
        while p < len {
            let off = (c.lim_count % BLOCK as u64) as usize;
            let m = (BLOCK - off).min(len - p);
            self.limiter.process(&mut bl[p..p + m], &mut br[p..p + m]);
            p += m;
            c.lim_count += m as u64;
        }
        c.buf_len = len;
        c.buf_pos = 0;
        c.grid_t = t + n as u64;
        if let Some(to) = jump {
            self.soft_jump(to);
        }
    }

    /// Fills `left`/`right` with the next output samples. After a one-shot's end the rest is
    /// silence and `cursor.ended` is set.
    pub(super) fn pull(&mut self, left: &mut [f32], right: &mut [f32], lp: Option<LoopRange>) {
        let lp = self.effective_loop(lp);
        let total = left.len().min(right.len());
        let mut i = 0usize;
        // Bounded: every pass either writes samples, drops skipped ones or renders a block, and
        // the skips are finite.
        while i < total {
            if !self.is_loop && self.cursor.out_pos >= self.end {
                self.cursor.ended = true;
                left[i..total].fill(0.0);
                right[i..total].fill(0.0);
                return;
            }
            if self.cursor.buf_pos == self.cursor.buf_len {
                self.render_block(lp);
                continue;
            }
            let c = &mut self.cursor;
            let avail = c.buf_len - c.buf_pos;
            if c.out_skip > 0 {
                let d = (c.out_skip as usize).min(avail);
                c.buf_pos += d;
                c.out_skip -= d as u64;
                continue;
            }
            let mut n = avail.min(total - i);
            let pos = c.out_pos;
            let wrap_at = match lp {
                Some(r) if pos < r.end => Some((r.end, r.start)),
                _ if self.is_loop => Some((self.length, 0)),
                _ => None,
            };
            if let Some((at, _)) = wrap_at {
                n = n.min((at - pos) as usize);
            } else {
                n = n.min((self.end - pos) as usize);
            }
            let n = n.max(1).min(avail).min(total - i);
            left[i..i + n].copy_from_slice(&c.buf_l[c.buf_pos..c.buf_pos + n]);
            right[i..i + n].copy_from_slice(&c.buf_r[c.buf_pos..c.buf_pos + n]);
            c.buf_pos += n;
            i += n;
            c.out_pos = pos + n as u64;
            if let Some((at, to)) = wrap_at
                && c.out_pos >= at
            {
                c.out_pos = to;
            }
        }
    }
}
