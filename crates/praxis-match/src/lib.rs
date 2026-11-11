//! praxis-match: "make something like this" (FR-14 to FR-18).
//! Maps a Sound Profile to a Score and refines it by analysis-by-synthesis until the
//! similarity stops improving. Phase 6; see docs/PLAN.md (WP-M) and the "Matching" section of
//! docs/contracts/sound-profile.md.
//!
//! - [`character`]: what the matcher reads from a profile.
//! - [`music`]: profile to ScorePlan to Score (the planner's own arranger).
//! - [`sfx`]: profile to a procedural effect family with its macros, or an ambience bed.
//! - [`optimize`]: deterministic coordinate descent.
//! - [`matcher`]: candidates, refinement, the final render and the originality guard.
//! - [`refset`]: the self-made reference set the Phase 6 exit is measured on.
//!
//! Wasm-safe: no threads, no clock, no files. The reference only ever enters as a
//! [`praxis_analyze::SoundProfile`]; its audio never reaches the engine.

#![forbid(unsafe_code)]

pub mod analysis;
pub mod character;
pub mod error;
pub mod keys;
pub mod matcher;
pub mod music;
pub mod optimize;
pub mod refset;
pub mod sfx;

pub use analysis::Originality;
pub use error::MatchError;
pub use keys::parse_key;
pub use matcher::{
    Iteration, MatchOptions, MatchOutcome, MatchRequest, ReferenceSimilarity, match_reference,
};

/// Written to `meta.provenance.matcher` of every matched Score.
pub const MATCHER_VERSION: &str = "praxis-match 1";
