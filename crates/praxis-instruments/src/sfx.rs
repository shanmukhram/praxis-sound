//! WP-H sfx: procedural sound effects that sit under music (FR-16).
//!
//! Each preset is a [`crate::synth::poly::Patch`] played by the shared polyphonic voice
//! framework (preallocated voices, quietest-voice stealing with a fade, voices summed in note-on
//! order, per-note randomness only from `NoteOn.seed`). Every preset has the same four macros,
//! each `0..1` with default 0.5, chosen so the match loop (WP-M) can fit them to a reference
//! clip: each moves one measurable property monotonically (tested in `tests/sfx_*.rs`).
//!
//! | Macro | Moves | Measured as |
//! | --- | --- | --- |
//! | `length` | duration (seconds; the riser in beats, the drone's attack/release) | last sample, within 1% |
//! | `brightness` | filter centres, cut-offs and harmonic roll-off | spectral centroid |
//! | `weight` | a low layer (sub, body, rumble or knock) at 41..78 Hz or the impact body | energy below 150 Hz |
//! | `pitch` | tuned parts by whole scale steps of the key, noise colour continuously | fundamental, centroid |
//!
//! | Preset | Class | Pitched | Sound |
//! | --- | --- | --- | --- |
//! | `whoosh` | whoosh | yes | moving air: pink turbulence in three moving bands and a hiss, gusting, a distance envelope with air absorption, Doppler bend through the note, stereo pass-by, a room |
//! | `impact` | impact | yes | a struck tuned body: contact force, tonic fundamental relaxing into pitch, near-harmonic modes, a shell-mode cloud, crack, slap, debris, tonic sub, a room |
//! | `riser` | riser | yes | evolving filtered air with gusts and an accelerating gate, detuning unisons holding the note and gliding up to the key's tonic or fifth, a room; ends on the note's last sample |
//! | `drone` | drone | yes, sustaining | drifting, beating harmonics on the note, bowed noise and breath, a long room; loops exactly with its note length |
//! | `glitch` | glitch | yes | granular slices of crushed FM blips and comb-tuned noise bursts, stutters (some at half speed) that stay harmonic (in key) |
//! | `tick` | tick | no | a modal click (contact force on a small body) with a ping on the key's tonic or fifth, a tiny room; usable as a clock |
//!
//! The physical-modelling pieces they share (a per-voice room, a 1/f random control, modal
//! resonators and contact pulses) are in [`space`]; tuning, macros and envelopes in [`common`].
//! Randomness that makes each take a little different (gusts, mode tunings, grain choices)
//! comes only from the note seed.
//!
//! **In key.** Everything tuned is on the note (moved only by scale steps of `ctx.key`), on the
//! key's tonic or on its fifth. Low layers sit on the note's or the tonic's pitch class.
//!
//! **Restraint.** Moderate levels (peaks around 0.2-0.5 at velocity 0.8), nothing tuned above
//! 9 kHz and no noise band or body mode above 11 kHz, no DC, smooth starts (ramps of 0.4 ms or
//! longer) and every sound ends in exact silence (a `cos^2` fade applied after the room), so
//! nothing clicks. One-shots (`whoosh`,
//! `impact`, `glitch`, `tick`) ignore note-offs and `tail_samples` covers their full length;
//! `riser` ends with its note; `drone` releases over `length`.
//!
//! Velocity scales level (`v^1.5`); a bright accent adds 0.25 to `brightness`.

use crate::registry::PresetEntry;
use crate::synth::poly;

pub mod common;
pub mod drone;
pub mod glitch;
pub mod impact;
pub mod riser;
pub mod space;
pub mod tick;
pub mod whoosh;

/// Presets this family provides.
pub fn presets() -> Vec<PresetEntry> {
    vec![
        poly::entry::<whoosh::Whoosh>(),
        poly::entry::<impact::Impact>(),
        poly::entry::<riser::Riser>(),
        poly::entry::<drone::Drone>(),
        poly::entry::<glitch::Glitch>(),
        poly::entry::<tick::Tick>(),
    ]
}
