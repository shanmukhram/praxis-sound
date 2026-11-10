//! Deterministic seed derivation for rendering (docs/contracts/score.md, "Seeds").
//!
//! Every random draw in a render descends from `score.seed`:
//! - a layer's seed is `derive(score.seed, layer.seed_offset)` (it is also `InstrumentContext::seed`);
//! - a note's seed is `derive(layer seed, note.key)`, where `note.key` is the note's stable identity
//!   from `timeline::expand_notes` (layer id, bar, step, pitch).
//!
//! Because the note seed depends only on the note's own identity, editing one bar can never shift
//! the random draws of a note in another bar, and variations (FR-8) change only `seed_offset`.

use praxis_dsp::rng;

/// Seed of one layer: `derive(score_seed, seed_offset)`.
pub fn layer_seed(score_seed: u64, seed_offset: u64) -> u64 {
    rng::derive(score_seed, seed_offset)
}

/// Seed of one note: `derive(derive(score_seed, seed_offset), note_key)`.
pub fn note_seed(score_seed: u64, seed_offset: u64, note_key: u64) -> u64 {
    rng::derive(layer_seed(score_seed, seed_offset), note_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_seed_composes_layer_seed() {
        let l = layer_seed(1, 7);
        assert_eq!(note_seed(1, 7, 99), rng::derive(l, 99));
        assert_ne!(note_seed(1, 7, 99), note_seed(1, 8, 99));
        assert_ne!(note_seed(1, 7, 99), note_seed(1, 7, 100));
    }
}
