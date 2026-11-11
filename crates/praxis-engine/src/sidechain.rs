//! Compressor sidechain keyed by another layer (docs/contracts/engine.md "Sidechain").
//!
//! A compressor with `sidechain: <layer id>` is keyed by that layer's *instrument output*, before
//! its effects and gain. Every layer that keys something writes its instrument output into a
//! [`KeyTap`], a ring buffer indexed by musical time (the instrument's own timeline, which is
//! ahead of the grid by the layer's latency, see `latency.rs`). Before a keyed layer runs its
//! chain, the graph reads the key for the exact musical time the compressor is processing and
//! hands it over with `Processor::set_sidechain`.
//!
//! Layers are processed in [`processing_order`]: every key layer before the layers it keys (a
//! stable topological order; ties keep score order). Sums stay in score order, so the order of
//! processing never changes the mix. A cycle (`a` keys `b` and `b` keys `a`, directly or through
//! others) is a render error, as the contract says. A layer keying itself is allowed: its own
//! instrument output is available before its chain runs.
//!
//! A key layer that is not rendered (muted, not solo, not in `only_layers`) still runs its
//! instrument as a key-only node: it keys the compressor but is not mixed and has no stem, so a
//! layer's stem sounds the same whether it is rendered alone or in the full mix. Avoid-listed
//! layers never run, so they cannot key anything (warning).

/// Ring buffer of one key layer's instrument output, indexed by musical time.
#[derive(Clone, Debug)]
pub struct KeyTap {
    pub(crate) l: Vec<f32>,
    pub(crate) r: Vec<f32>,
    /// Musical time one past the last written sample.
    pub(crate) written: u64,
}

impl KeyTap {
    /// `capacity` samples of history (allocated here, never on the audio path).
    pub fn new(capacity: usize) -> KeyTap {
        let c = capacity.max(1);
        KeyTap {
            l: vec![0.0; c],
            r: vec![0.0; c],
            written: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.l.len()
    }

    /// Musical time one past the last written sample.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// Appends samples for musical times `[start, start + len)`. Writes are contiguous; a gap is
    /// filled with silence.
    pub fn write(&mut self, start: u64, left: &[f32], right: &[f32]) {
        let cap = self.l.len() as u64;
        while self.written < start {
            let i = (self.written % cap) as usize;
            self.l[i] = 0.0;
            self.r[i] = 0.0;
            self.written += 1;
        }
        let n = left.len().min(right.len());
        for k in 0..n {
            let t = start + k as u64;
            if t < self.written {
                continue;
            }
            let i = (t % cap) as usize;
            self.l[i] = left[k];
            self.r[i] = right[k];
            self.written = t + 1;
        }
    }

    /// Reads musical times `[start, start + out.len())`; times before 0, not yet written, or
    /// older than the ring's history read as silence.
    pub fn read(&self, start: i64, out_l: &mut [f32], out_r: &mut [f32]) {
        let cap = self.l.len() as i64;
        let written = self.written as i64;
        let n = out_l.len().min(out_r.len());
        for k in 0..n {
            let t = start + k as i64;
            if t < 0 || t >= written || t < written - cap {
                out_l[k] = 0.0;
                out_r[k] = 0.0;
            } else {
                let i = (t % cap) as usize;
                out_l[k] = self.l[i];
                out_r[k] = self.r[i];
            }
        }
    }
}

/// A sidechain cycle among layers: ids along the cycle, each keyed by the next (the first is
/// repeated at the end). Displayed as `a <- b <- a` ("a is keyed by b, which is keyed by a").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cycle(pub Vec<String>);

impl std::fmt::Display for Cycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.join(" <- "))
    }
}

/// Processing order for nodes `0..ids.len()` where `keys[i]` lists the nodes that key node `i`.
/// Keys come first; among nodes that are ready, the lowest index goes first (deterministic and
/// equal to score order when nothing is keyed). Self-keys are ignored. Returns the cycle if there
/// is one.
pub fn processing_order(ids: &[String], keys: &[Vec<usize>]) -> Result<Vec<usize>, Cycle> {
    let n = ids.len();
    let deps: Vec<Vec<usize>> = (0..n)
        .map(|i| {
            let mut d: Vec<usize> = keys
                .get(i)
                .map(|k| k.iter().copied().filter(|&j| j != i && j < n).collect())
                .unwrap_or_default();
            d.sort_unstable();
            d.dedup();
            d
        })
        .collect();
    let mut done = vec![false; n];
    let mut order = Vec::with_capacity(n);
    while order.len() < n {
        let next = (0..n).find(|&i| !done[i] && deps[i].iter().all(|&j| done[j]));
        match next {
            Some(i) => {
                done[i] = true;
                order.push(i);
            }
            None => return Err(find_cycle(ids, &deps, &done)),
        }
    }
    Ok(order)
}

fn find_cycle(ids: &[String], deps: &[Vec<usize>], done: &[bool]) -> Cycle {
    // Every remaining node has a remaining dependency: walk until a node repeats.
    let start = (0..deps.len()).find(|&i| !done[i]).unwrap_or(0);
    let mut path = vec![start];
    let mut cur = start;
    while let Some(&next) = deps[cur].iter().find(|&&j| !done[j]) {
        if let Some(pos) = path.iter().position(|&p| p == next) {
            let mut cyc: Vec<String> = path[pos..].iter().map(|&i| ids[i].clone()).collect();
            cyc.push(ids[next].clone());
            return Cycle(cyc);
        }
        path.push(next);
        cur = next;
    }
    Cycle(path.iter().map(|&i| ids[i].clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_reads_what_was_written() {
        let mut t = KeyTap::new(8);
        t.write(0, &[1.0, 2.0, 3.0], &[-1.0, -2.0, -3.0]);
        t.write(3, &[4.0, 5.0], &[-4.0, -5.0]);
        let (mut l, mut r) = ([0.0; 4], [0.0; 4]);
        t.read(-1, &mut l, &mut r);
        assert_eq!(l, [0.0, 1.0, 2.0, 3.0]);
        assert_eq!(r, [0.0, -1.0, -2.0, -3.0]);
        t.read(3, &mut l, &mut r);
        assert_eq!(l, [4.0, 5.0, 0.0, 0.0], "unwritten reads silent");
        t.write(5, &[6.0; 8], &[6.0; 8]);
        t.read(0, &mut l, &mut r);
        assert_eq!(l, [0.0; 4], "overwritten history reads silent");
    }

    #[test]
    fn order_puts_keys_first_and_finds_cycles() {
        let ids: Vec<String> = ["piano", "kick", "bass"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // piano keyed by kick, bass keyed by kick and itself.
        let order = processing_order(&ids, &[vec![1], vec![], vec![1, 2]]).unwrap();
        assert_eq!(order, vec![1, 0, 2]);
        assert_eq!(
            processing_order(&ids, &[vec![], vec![], vec![]]).unwrap(),
            vec![0, 1, 2]
        );
        let err = processing_order(&ids, &[vec![2], vec![], vec![0]]).unwrap_err();
        assert_eq!(err.to_string(), "piano <- bass <- piano");
    }
}
