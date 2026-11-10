//! praxis-audio-io: file decoding and encoding, MIDI export and real-time device output.
//! The only crate besides the CLI/app that touches files and devices.

#![forbid(unsafe_code)]

// ---- frozen contract (orchestrator) ----
pub mod wav;

// ---- WP-E audio-io ----
/// Decode WAV, FLAC, OGG, MP3 natively; M4A/MP4/MOV/WebM through an ffmpeg subprocess.
pub mod decode;
/// Real-time output through cpal (feature `device`). Phase 3.
#[cfg(feature = "device")]
pub mod device;
/// FLAC export and per-layer stems.
pub mod encode;
/// Standard MIDI file export from a Score.
pub mod midi;

pub use wav::{WavFormat, write_wav};
