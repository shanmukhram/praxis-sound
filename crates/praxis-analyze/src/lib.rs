//! praxis-analyze: measures audio. Used three ways with the same code:
//! 1. Reference analysis -> Sound Profile (FR-12, FR-13).
//! 2. The match loop compares renders with references (FR-15).
//! 3. Verification of our own renders against their Score (tempo, key, grid, loudness, seam).

#![forbid(unsafe_code)]

// ---- frozen contract (orchestrator) ----
pub mod profile;
pub mod verify;

// ---- WP-D analyze: implementation modules ----
pub mod analysis;
pub mod features;
pub mod hash;
pub mod originality;
pub mod similarity;

pub use analysis::{ProfileOptions, sound_profile};
pub use profile::SoundProfile;
