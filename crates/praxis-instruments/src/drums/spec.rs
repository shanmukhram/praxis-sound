//! Drum voice recipes: one [`DrumSpec`] per synthesised drum, all played by
//! [`super::voice::DrumVoice`].
//!
//! A drum is up to five parts, each with its own one-shot envelope, mixed and sent through a
//! low-pass "tone" filter and an optional soft saturation:
//!
//! - **body**: a sine whose frequency glides from `body_hz * sweep` down to `body_hz` with time
//!   constant `sweep_s` (kick and tom pitch drop), plus an optional second membrane mode at
//!   `mode2_ratio` sharing the glide;
//! - **noise**: seeded white noise through a high-pass and a low-pass (snare wires, hat sizzle,
//!   clap, shaker), optionally retriggered `bursts` times `burst_gap_s` apart (clap);
//! - **metal**: six band-limited square waves at inharmonic hi-hat frequencies, high-passed;
//! - **click**: a short low-passed noise tick (felt or beater contact).
//!
//! Times are seconds; a zero level disables a part.

use praxis_score::GeneratorClass;

/// One synthesised drum.
#[derive(Clone, Copy, Debug)]
pub struct DrumSpec {
    pub name: &'static str,
    pub class: GeneratorClass,
    /// GM drum-map notes that play this drum in `kit` (0 = unused slot).
    pub gm: [u8; 3],
    /// Choke group this drum belongs to (0 = none) and bit mask of groups a hit silences.
    pub group: u8,
    pub chokes: u32,
    /// Stereo offset in the kit and as a single preset, `-1..=1` (small values).
    pub pan: f32,
    /// Output level at full velocity.
    pub gain: f32,

    pub body_level: f32,
    pub body_hz: f32,
    pub sweep: f32,
    pub sweep_s: f32,
    pub body_attack_s: f32,
    pub body_decay_s: f32,
    pub mode2_ratio: f32,
    pub mode2_level: f32,
    pub mode2_decay_s: f32,

    pub noise_level: f32,
    /// High-pass corner (0 = off).
    pub noise_hp: f32,
    /// Low-pass corner (0 = off).
    pub noise_lp: f32,
    pub noise_attack_s: f32,
    pub noise_decay_s: f32,
    /// Number of noise bursts (1 = one hit; a clap uses several).
    pub bursts: u8,
    pub burst_gap_s: f32,

    pub metal_level: f32,
    pub metal_hp: f32,
    pub metal_attack_s: f32,
    pub metal_decay_s: f32,

    pub click_level: f32,
    pub click_lp: f32,
    pub click_decay_s: f32,

    /// Output low-pass at a middle tone; tone 0..1 moves it from about 0.44x to 2.3x.
    pub lp_hz: f32,
    /// Soft-saturation drive (0 = clean).
    pub drive: f32,
}

/// Hi-hat square frequencies (Hz): the classic analogue six-oscillator metal cluster.
pub const METAL_HZ: [f32; 6] = [263.0, 400.0, 421.0, 474.0, 587.0, 845.0];

const BASE: DrumSpec = DrumSpec {
    name: "",
    class: GeneratorClass::Drums,
    gm: [0; 3],
    group: 0,
    chokes: 0,
    pan: 0.0,
    gain: 0.6,
    body_level: 0.0,
    body_hz: 100.0,
    sweep: 1.0,
    sweep_s: 0.02,
    body_attack_s: 0.001,
    body_decay_s: 0.2,
    mode2_ratio: 1.5,
    mode2_level: 0.0,
    mode2_decay_s: 0.1,
    noise_level: 0.0,
    noise_hp: 0.0,
    noise_lp: 0.0,
    noise_attack_s: 0.0008,
    noise_decay_s: 0.1,
    bursts: 1,
    burst_gap_s: 0.0,
    metal_level: 0.0,
    metal_hp: 7000.0,
    metal_attack_s: 0.0005,
    metal_decay_s: 0.05,
    click_level: 0.0,
    click_lp: 2000.0,
    click_decay_s: 0.008,
    lp_hz: 12_000.0,
    drive: 0.0,
};

/// Index of each drum in [`DRUMS`].
pub const SOFT_KICK: usize = 0;
pub const KICK: usize = 1;
pub const SNARE: usize = 2;
pub const RIM: usize = 3;
pub const CLAP: usize = 4;
pub const HAT_CLOSED: usize = 5;
pub const HAT_OPEN: usize = 6;
pub const SHAKER: usize = 7;
pub const TOM_LOW: usize = 8;
pub const TOM_MID: usize = 9;
pub const TOM_HIGH: usize = 10;
/// Not a drum: the preset index of `kit`.
pub const KIT: usize = 11;

/// Choke group of the hi-hats.
const HATS: u8 = 1;

/// Every synthesised drum, in preset order.
pub const DRUMS: [DrumSpec; 11] = [
    // Soft, muted kick: a short sine glide settling on A1 (55 Hz), a felt-like thump, everything
    // under a ~2.3 kHz low-pass, no saturation.
    DrumSpec {
        name: "soft_kick",
        gm: [0; 3],
        gain: 0.78,
        body_level: 1.0,
        body_hz: 55.0,
        sweep: 2.0,
        sweep_s: 0.022,
        body_attack_s: 0.002,
        body_decay_s: 0.34,
        click_level: 0.10,
        click_lp: 1100.0,
        click_decay_s: 0.014,
        lp_hz: 3200.0,
        ..BASE
    },
    DrumSpec {
        name: "kick",
        gm: [36, 35, 0],
        gain: 0.8,
        body_level: 1.0,
        body_hz: 50.0,
        sweep: 3.2,
        sweep_s: 0.03,
        body_attack_s: 0.0008,
        body_decay_s: 0.45,
        click_level: 0.22,
        click_lp: 4500.0,
        click_decay_s: 0.006,
        lp_hz: 7000.0,
        drive: 1.6,
        ..BASE
    },
    DrumSpec {
        name: "snare",
        gm: [38, 40, 0],
        gain: 0.55,
        body_level: 0.55,
        body_hz: 185.0,
        sweep: 1.35,
        sweep_s: 0.015,
        body_attack_s: 0.0008,
        body_decay_s: 0.14,
        mode2_ratio: 1.78,
        mode2_level: 0.45,
        mode2_decay_s: 0.09,
        noise_level: 0.75,
        noise_hp: 1200.0,
        noise_lp: 9000.0,
        noise_decay_s: 0.2,
        click_level: 0.1,
        click_lp: 5000.0,
        click_decay_s: 0.004,
        lp_hz: 12_000.0,
        drive: 0.8,
        ..BASE
    },
    DrumSpec {
        name: "rim",
        class: GeneratorClass::Percussion,
        gm: [37, 0, 0],
        gain: 0.45,
        body_level: 0.7,
        body_hz: 520.0,
        body_attack_s: 0.0003,
        body_decay_s: 0.035,
        mode2_ratio: 3.2,
        mode2_level: 0.5,
        mode2_decay_s: 0.02,
        noise_level: 0.3,
        noise_hp: 2000.0,
        noise_lp: 7000.0,
        noise_attack_s: 0.0003,
        noise_decay_s: 0.012,
        lp_hz: 10_000.0,
        ..BASE
    },
    DrumSpec {
        name: "clap",
        class: GeneratorClass::Percussion,
        gm: [39, 0, 0],
        gain: 0.9,
        noise_level: 1.0,
        noise_hp: 900.0,
        noise_lp: 2800.0,
        noise_attack_s: 0.0005,
        noise_decay_s: 0.22,
        bursts: 4,
        burst_gap_s: 0.011,
        lp_hz: 9000.0,
        ..BASE
    },
    DrumSpec {
        name: "hat_closed",
        gm: [42, 44, 0],
        group: HATS,
        chokes: 1 << HATS,
        pan: 0.15,
        gain: 0.5,
        noise_level: 0.6,
        noise_hp: 7500.0,
        noise_attack_s: 0.0004,
        noise_decay_s: 0.045,
        metal_level: 0.55,
        metal_hp: 7000.0,
        metal_attack_s: 0.0004,
        metal_decay_s: 0.05,
        lp_hz: 16_000.0,
        ..BASE
    },
    DrumSpec {
        name: "hat_open",
        gm: [46, 0, 0],
        group: HATS,
        chokes: 1 << HATS,
        pan: 0.15,
        gain: 0.42,
        noise_level: 0.6,
        noise_hp: 7500.0,
        noise_attack_s: 0.0005,
        noise_decay_s: 0.38,
        metal_level: 0.55,
        metal_hp: 7000.0,
        metal_attack_s: 0.0005,
        metal_decay_s: 0.42,
        lp_hz: 16_000.0,
        ..BASE
    },
    DrumSpec {
        name: "shaker",
        class: GeneratorClass::Percussion,
        gm: [70, 0, 0],
        pan: -0.15,
        gain: 0.3,
        noise_level: 1.0,
        noise_hp: 5500.0,
        noise_lp: 12_000.0,
        noise_attack_s: 0.012,
        noise_decay_s: 0.09,
        lp_hz: 16_000.0,
        ..BASE
    },
    DrumSpec {
        name: "tom_low",
        gm: [41, 43, 0],
        pan: -0.2,
        gain: 0.65,
        body_level: 1.0,
        body_hz: 87.0,
        sweep: 1.5,
        sweep_s: 0.05,
        body_attack_s: 0.0008,
        body_decay_s: 0.55,
        mode2_ratio: 1.58,
        mode2_level: 0.22,
        mode2_decay_s: 0.25,
        noise_level: 0.08,
        noise_hp: 400.0,
        noise_lp: 4000.0,
        noise_decay_s: 0.08,
        click_level: 0.12,
        click_lp: 3000.0,
        click_decay_s: 0.006,
        lp_hz: 6000.0,
        ..BASE
    },
    DrumSpec {
        name: "tom_mid",
        gm: [45, 47, 0],
        gain: 0.62,
        body_level: 1.0,
        body_hz: 116.0,
        sweep: 1.5,
        sweep_s: 0.045,
        body_attack_s: 0.0008,
        body_decay_s: 0.45,
        mode2_ratio: 1.58,
        mode2_level: 0.22,
        mode2_decay_s: 0.2,
        noise_level: 0.08,
        noise_hp: 500.0,
        noise_lp: 4500.0,
        noise_decay_s: 0.07,
        click_level: 0.12,
        click_lp: 3200.0,
        click_decay_s: 0.006,
        lp_hz: 6500.0,
        ..BASE
    },
    DrumSpec {
        name: "tom_high",
        gm: [48, 50, 0],
        pan: 0.2,
        gain: 0.6,
        body_level: 1.0,
        body_hz: 155.0,
        sweep: 1.5,
        sweep_s: 0.04,
        body_attack_s: 0.0008,
        body_decay_s: 0.38,
        mode2_ratio: 1.58,
        mode2_level: 0.22,
        mode2_decay_s: 0.17,
        noise_level: 0.08,
        noise_hp: 600.0,
        noise_lp: 5000.0,
        noise_decay_s: 0.06,
        click_level: 0.12,
        click_lp: 3500.0,
        click_decay_s: 0.006,
        lp_hz: 7000.0,
        ..BASE
    },
];

/// The drum `kit` plays for a GM note, if any.
pub fn kit_drum(pitch: u8) -> Option<usize> {
    if pitch == 0 {
        return None;
    }
    DRUMS.iter().position(|d| d.gm.contains(&pitch))
}
