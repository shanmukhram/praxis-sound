//! WP-C2 synth: synthesised pitched presets (FR-21).
//!
//! Every preset is a [`poly::Patch`] (per-instrument settings, read by its voices) plus a
//! [`poly::Voice`] type, played by the shared polyphonic voice framework [`poly::PolyInstrument`]:
//! a preallocated voice pool, quietest-voice stealing with a short fade, voices summed in note-on
//! order and per-voice seeded variation from `NoteOn.seed`. `NoteOn.brightness` (bright accents)
//! and velocity open each preset's tone.
//!
//! | Preset | Class | Model |
//! | --- | --- | --- |
//! | `bell` | motif | struck bell: inharmonic modal partials (hum, prime, octave, 2.76, 4.07 ...), beating doublets, strike shimmer, long ring |
//! | `soft_keys` | melody | FM electric piano: 1:1 body pair with a decaying index plus a short 1:14 tine |
//! | `pluck` | melody | Karplus-Strong string: linear-phase damping loop, all-pass fine tuning, pick-position comb |
//! | `mallet` | motif | modal struck bar: marimba (1 : 3.93 : 9.2) to vibraphone (1 : 4 : 10) partials, felt-mallet knock |
//! | `sub_bass` | bass | sine with a slight 2nd harmonic |
//! | `warm_bass` | bass | two slightly detuned band-limited saws into a saturating ladder low-pass with an envelope |
//! | `fm_keys` | harmony | sustaining two-carrier FM keys for chords |
//! | `glass` | motif | morphing wavetable (bright to dark as it rings) plus a beating octave |
//! | `granular_texture` | texture | granular cloud of Hann-windowed grains read from an internal wavetable |
//! | `noise_bed` | ambience | decorrelated stereo pink noise through drifting band-pass filters (unpitched) |
//! | `warm_pad` | pad | three detuned saws through a low-pass that opens with the attack (blocked unless asked) |
//!
//! Rules every preset keeps (tested in `tests/synth_*.rs`): the fundamental (for `bell`, the
//! reference partial at ratio 1.0) is within a few cents of the MIDI pitch; attacks and releases
//! are ramps, so nothing clicks; voices reach exact silence and `tail_samples` covers the
//! longest release; output is a pure function of the note events and their seeds, independent
//! of block size and voice slot; nothing allocates after `create`.

use crate::registry::PresetEntry;

pub mod common;
pub mod poly;

pub mod bell;

/// Presets this family provides.
pub fn presets() -> Vec<PresetEntry> {
    vec![poly::entry::<bell::Bell>()]
}
