//! Latency compensation (docs/contracts/engine.md "Latency compensation").
//!
//! Effects report `latency_samples` (the limiter's equals its lookahead). The engine keeps every
//! stem and the master sample-aligned with the grid:
//!
//! - **Layers** run ahead. A layer whose chain has latency `D` runs its instrument `D` samples
//!   ahead of the grid (pre-rolled over `[0, D)` before the first block), so the chain output at
//!   grid time `t` is the musical time `t`. Offline this costs nothing; the real-time engine can
//!   do the same because the score is known in advance.
//! - **Master effects**: the driver runs the graph `D_master` samples longer and captures the bus
//!   from `start + D_master` (render.rs).
//! - **The master limiter** is compensated in `master.rs`: one-shots feed `D_limiter` samples of
//!   silence after the bus and drop the first `D_limiter` output samples; loops feed the captured
//!   cycle periodically, so the output is a steady-state cycle aligned with the grid.
//! - **Sidechain keys** are read at the exact musical time the compressor processes. Inside a
//!   layer with chain latency `D_x`, the slot `p` sees the chain input delayed by the latency of
//!   the slots before it (`pre_p`), so at chain-input musical time `s` it processes musical time
//!   `s - pre_p`. The key layer's instrument has produced musical times up to `t + D_k` by grid
//!   time `t`; when `D_x - pre_p > D_k` the key would be needed before it exists, so it is read
//!   `lag = D_x - pre_p - D_k` samples late and the render reports a warning.

/// How a sidechained slot reads its key from the key layer's tap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyAlignment {
    /// Latency of the slots before the compressor in its own chain (`pre_p`).
    pub pre: u64,
    /// Samples the key is read late because the key layer does not run far enough ahead (0 when
    /// aligned).
    pub lag: u64,
}

impl KeyAlignment {
    /// Musical start time of the key for a chain-input block starting at musical time `start`.
    pub fn key_start(&self, start: u64) -> i64 {
        start as i64 - self.pre as i64 - self.lag as i64
    }
}

/// Alignment of a compressor in a layer with chain latency `layer_latency`, behind slots with
/// `pre_latency`, keyed by a layer with chain latency `key_latency`.
pub fn key_alignment(layer_latency: u64, pre_latency: u64, key_latency: u64) -> KeyAlignment {
    let lead = layer_latency.saturating_sub(pre_latency);
    KeyAlignment {
        pre: pre_latency,
        lag: lead.saturating_sub(key_latency),
    }
}

/// History a key tap must keep: at grid time `t` the newest key sample is at musical time
/// `t + key_latency + block - 1` and the oldest any reader needs is at `t` or later.
pub fn tap_capacity(key_latency: u64, max_block: usize) -> usize {
    key_latency as usize + 2 * max_block.max(1) + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment() {
        // No latency anywhere: read at the same time.
        let a = key_alignment(0, 0, 0);
        assert_eq!((a.pre, a.lag, a.key_start(100)), (0, 0, 100));
        // A limiter (144) after the compressor: the compressor sees the input 144 ahead of the
        // grid; the key layer runs only 0 ahead, so the key is 144 late.
        let a = key_alignment(144, 0, 144);
        assert_eq!(a.lag, 0);
        let a = key_alignment(144, 0, 0);
        assert_eq!(a.lag, 144);
        // A limiter before the compressor: the compressor sees grid time, no lag.
        let a = key_alignment(144, 144, 0);
        assert_eq!((a.lag, a.key_start(244)), (0, 100));
        assert!(tap_capacity(144, 128) > 144 + 128);
    }
}
