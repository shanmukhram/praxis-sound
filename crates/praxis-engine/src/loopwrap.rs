//! Loop-wrap planning (FR-26, docs/contracts/engine.md "Loop wrap").
//!
//! For `playback: loop` with loop length `L`, the engine renders the arrangement continuously for
//! `k + 1` cycles, where `k = ceil(max_tail / L)` (at least 1), and returns the last cycle
//! `[k·L, (k+1)·L)`. Every earlier cycle's tails have then rung into the captured cycle exactly as
//! they would in a looping player, so sample `L-1` flows into sample `0`. No fades are applied.
//! `max_tail` is the longest layer tail (notes held past the loop end, instrument release, effect
//! tails) plus the master chain tail, capped at [`MAX_TAIL_SECONDS`].
//!
//! One-shots render once: `[0, L + tail_seconds)`.

/// Upper bound for the tail used to plan warm-up cycles.
pub const MAX_TAIL_SECONDS: f64 = 30.0;

/// The part of the continuous timeline the renderer returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapturePlan {
    /// Cycles rendered before the captured one (0 for one-shots).
    pub warmup_cycles: u32,
    /// First captured grid sample.
    pub start: u64,
    /// Captured length in samples.
    pub len: u64,
}

impl CapturePlan {
    pub fn end(&self) -> u64 {
        self.start + self.len
    }
}

/// Plans a loop capture. `min_warmup` raises `k` (used by tests to prove the captured cycle is
/// already in steady state).
pub fn plan_loop(loop_len: u64, max_tail: u64, sample_rate: u32, min_warmup: u32) -> CapturePlan {
    let cap = (MAX_TAIL_SECONDS * sample_rate as f64).round() as u64;
    let tail = max_tail.min(cap);
    let l = loop_len.max(1);
    let k = tail.div_ceil(l).max(1).max(min_warmup as u64);
    let k = k.min(u32::MAX as u64) as u32;
    CapturePlan {
        warmup_cycles: k,
        start: k as u64 * l,
        len: l,
    }
}

/// Plans a one-shot capture: the bars plus the score's tail.
pub fn plan_one_shot(len: u64, tail: u64) -> CapturePlan {
    CapturePlan {
        warmup_cycles: 0,
        start: 0,
        len: len + tail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warmup_cycles() {
        assert_eq!(plan_loop(1000, 0, 48_000, 0).warmup_cycles, 1);
        assert_eq!(plan_loop(1000, 1000, 48_000, 0).warmup_cycles, 1);
        assert_eq!(plan_loop(1000, 1001, 48_000, 0).warmup_cycles, 2);
        let p = plan_loop(1000, 2500, 48_000, 0);
        assert_eq!((p.warmup_cycles, p.start, p.len), (3, 3000, 1000));
        assert_eq!(plan_loop(1000, 10, 48_000, 4).warmup_cycles, 4);
        // Capped at 30 s.
        assert_eq!(plan_loop(48_000, u64::MAX, 48_000, 0).warmup_cycles, 30);
        assert_eq!(plan_one_shot(100, 50).end(), 150);
    }
}
