//! praxis-dsp: oscillators, envelopes, filters, effects and meters.
//!
//! Rules for everything in this crate (docs/contracts/dsp.md):
//! - No I/O, no threads, no allocation inside `process`/`next` (allocate in `new`/`prepare`).
//! - No panics on valid input; denormals flushed; NaN never escapes a processor.
//! - Deterministic: the same inputs and seed give bit-identical output. Randomness only via [`rng`].
//! - Compiles for `wasm32-unknown-unknown`.

#![forbid(unsafe_code)]

// ---- frozen contract modules (orchestrator) ----
pub mod biquad;
pub mod buffer;
pub mod delayline;
pub mod rng;
pub mod traits;
pub mod util;

// ---- WP-B1 dsp-core ----
pub mod env;
pub mod filter;
pub mod interp;
pub mod noise;
pub mod osc;
pub mod smooth;

// ---- WP-B2 dsp-effects ----
pub mod fx;
pub mod meter;

pub use buffer::StereoBuffer;
pub use traits::Processor;
