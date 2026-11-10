//! The processing trait every effect implements. FROZEN CONTRACT.

/// A stereo audio processor (effect). Processes in place, block by block.
///
/// Implementations allocate only in their constructor or `prepare`. `process` must not allocate,
/// lock, perform I/O or panic.
pub trait Processor: Send {
    /// Called once before processing and whenever the sample rate or max block size changes.
    fn prepare(&mut self, sample_rate: f32, max_block: usize);

    /// Processes `left`/`right` (equal length, <= max_block) in place.
    fn process(&mut self, left: &mut [f32], right: &mut [f32]);

    /// Sets a parameter by index into the effect kind's spec table
    /// (`praxis_score::params::effect_params`). Values arrive clamped to the spec range.
    /// Implementations smooth changes that would otherwise click.
    fn set_param(&mut self, index: usize, value: f32);

    /// Clears all internal state (delay lines, envelopes) to silence.
    fn reset(&mut self);

    /// Samples of output that may follow silent input (reverb/delay tails). Used for loop wrap.
    fn tail_samples(&self) -> usize {
        0
    }

    /// Processing latency in samples (lookahead). The engine compensates.
    fn latency_samples(&self) -> usize {
        0
    }

    /// Feeds the sidechain key for the next `process` call. Only compressors use it.
    fn set_sidechain(&mut self, _left: &[f32], _right: &[f32]) {}
}
