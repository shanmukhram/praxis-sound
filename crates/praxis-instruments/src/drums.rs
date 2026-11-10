//! WP-C2 drums: synthesised kick, snare, hats, toms and percussion, and a GM-mapped kit (FR-22).
//!
//! Every drum is a [`spec::DrumSpec`] recipe (sine body with a pitch glide, filtered noise,
//! metallic square cluster, beater click) played by [`voice::DrumVoice`] on the shared
//! polyphonic voice framework ([`crate::synth::poly`]).
//!
//! | Preset | Class | Sound |
//! | --- | --- | --- |
//! | `soft_kick` | drums | soft, muted kick: sine gliding 110 -> 55 Hz in ~22 ms, felt thump, ~2.3 kHz low-pass, 0.34 s decay, no drive |
//! | `kick` | drums | punchier kick: 160 -> 50 Hz glide, beater click, gentle saturation |
//! | `snare` | drums | two membrane modes (185 Hz, x1.78) plus band-limited wire noise |
//! | `rim` | percussion | short high modes and a noise tick |
//! | `clap` | percussion | four band-passed noise bursts 11 ms apart, then a short room tail |
//! | `hat_closed` | drums | six-square metal cluster and noise above 7 kHz, 50 ms |
//! | `hat_open` | drums | same, 0.4 s; choked by `hat_closed` (and by itself) |
//! | `shaker` | percussion | soft-attack high noise |
//! | `tom_low`, `tom_mid`, `tom_high` | drums | gliding membrane (87 / 116 / 155 Hz) with a second mode |
//! | `kit` | drums | GM drum map: 35/36 kick, 37 rim, 38/40 snare, 39 clap, 42/44 closed hat, 46 open hat, 41/43 low tom, 45/47 mid tom, 48/50 high tom, 70 shaker |
//!
//! Drums are unpitched (`pitched: false`): the note number only selects the drum in `kit`, and is
//! ignored by the single-drum presets. Parameters (all presets): `tune` -12..12 st (0),
//! `decay` 0.5..2 x (1), `tone` 0..1 (0.5). Velocity sets level and opens the tone; bright accents
//! open it further.

use crate::registry::PresetEntry;
use crate::synth::poly;

pub mod spec;
pub mod voice;

use spec::{
    CLAP, HAT_CLOSED, HAT_OPEN, KICK, KIT, RIM, SHAKER, SNARE, SOFT_KICK, TOM_HIGH, TOM_LOW,
    TOM_MID,
};
use voice::Drum;

/// Presets this family provides.
pub fn presets() -> Vec<PresetEntry> {
    vec![
        poly::entry::<Drum<SOFT_KICK>>(),
        poly::entry::<Drum<KICK>>(),
        poly::entry::<Drum<SNARE>>(),
        poly::entry::<Drum<RIM>>(),
        poly::entry::<Drum<CLAP>>(),
        poly::entry::<Drum<HAT_CLOSED>>(),
        poly::entry::<Drum<HAT_OPEN>>(),
        poly::entry::<Drum<SHAKER>>(),
        poly::entry::<Drum<TOM_LOW>>(),
        poly::entry::<Drum<TOM_MID>>(),
        poly::entry::<Drum<TOM_HIGH>>(),
        poly::entry::<Drum<KIT>>(),
    ]
}
