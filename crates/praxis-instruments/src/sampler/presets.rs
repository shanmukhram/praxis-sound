//! The sampled presets: `felt_piano` and `upright_piano`, both from FreePats Upright Piano KW.
//!
//! The pack has two velocity layers per key zone (`vL` 0..=80 and `vH` 81..=127), 66 FLAC samples
//! (one zone per minor third), long sustain loops below A4 and a 0.6 s release. Both presets play
//! the same SFZ; the difference is the [`Voicing`] applied on top of it.
//!
//! ## `felt_piano` voicing
//!
//! A felt piano is an upright with a strip of felt between hammers and strings: the strike is
//! soft and round, highs are muted, and notes are allowed to ring and overlap. We imitate that:
//!
//! - **Soft layers first.** Layer choice uses `velocity x (1 - 0.45 felt)`, so ordinary notes play
//!   the soft `vL` recordings and only strong accents blend in the hard `vH` layer (equal-power
//!   crossfade across the 80/81 boundary instead of a timbre jump).
//! - **Felt-hammer low-pass that tracks velocity.** A gentle 2-pole low-pass (Q 0.6) whose cutoff
//!   is about 2.1 kHz at middle C and mid velocity (default `tone` and `felt`), moves +-1 octave
//!   from velocity 0 to 1 (darker when soft), follows half the keyboard, and never drops below
//!   three times the fundamental.
//! - **Bright accents** (`NoteOn.brightness` = 1, TIN) open the low-pass by an octave, lift the
//!   4.5 kHz shelf by 3 dB and push layer choice up by 0.25 so the hard layer blends in.
//! - **Less hammer noise.** A static high shelf at 4.5 kHz, down `2.5 x felt` dB.
//! - **Softened attack.** An attack ramp of `1.5 + 4 felt` ms plus up to `8 felt` ms more for soft
//!   notes, which rounds the hammer transient without smearing the rhythm.
//! - **Gentler dynamics.** `amp_veltrack` scaled by `1 - 0.35 felt`, so soft notes stay present.
//! - **Longer release.** `release_s` (default 1.2 s, the pack's own is 0.6 s): notes ring into the
//!   next one, which is how overlapping sustain sounds connected.
//!
//! Parameters: `tone` (0..1, brightness of the felt filter), `felt` (0..1, amount of all of the
//! above), `release_s` (0.05..4 s).
//!
//! ## `upright_piano` voicing
//!
//! The pack as recorded: both layers by velocity with a crossfade, full `amp_veltrack`, the
//! pack's 0.6 s release, no filter. `tone` below 0.5 adds a low-pass (darker), above 0.5 a
//! high-shelf lift (brighter); bright accents lift a 3 kHz shelf by 2.5 dB and push layer choice
//! up by 0.15.
//!
//! ## Tuning
//!
//! Both presets keep the recording's own tuning. Measured against equal temperament at each
//! sample's keycenter (ignored test `measure_kw_tuning`), A3..A6 sit within about +-8 cents, the
//! bass from F#1 to F#3 is 12..31 cents flat and the top octave 11..32 cents sharp: the stretch
//! of an aurally tuned upright. It is left as recorded because "correcting" an inharmonic bass to
//! equal-tempered fundamentals makes it sound sharp against the middle of the keyboard.

use super::engine::{SamplerInstrument, Tone, Voicing};
use super::program::Program;
use crate::packs::Packs;
use crate::registry::PresetEntry;
use praxis_engine::{Instrument, InstrumentContext, InstrumentError, PresetInfo, PresetParam};
use praxis_score::{GeneratorClass, InstrumentKind, InstrumentRef};
use std::sync::Arc;

/// Pack directory both presets load from.
pub const PIANO_PACK: &str = "upright-piano-kw";
/// Playable range of Upright Piano KW (A0..C8), checked against the SFZ by the tests.
pub const PIANO_RANGE: (u8, u8) = (21, 108);
/// Polyphony per instrument instance (each note may use two voices while crossfading layers).
pub const PIANO_VOICES: usize = 48;
const SOURCE: &str = "FreePats Upright Piano KW (CC0-1.0), pack `upright-piano-kw`";

fn param(name: &str, min: f64, max: f64, default: f64, unit: &str) -> PresetParam {
    PresetParam {
        name: name.to_string(),
        min,
        max,
        default,
        unit: unit.to_string(),
    }
}

fn felt_params() -> Vec<PresetParam> {
    vec![
        param("tone", 0.0, 1.0, 0.5, ""),
        param("felt", 0.0, 1.0, 0.7, ""),
        param("release_s", 0.05, 4.0, 1.2, "s"),
    ]
}

fn upright_params() -> Vec<PresetParam> {
    vec![
        param("tone", 0.0, 1.0, 0.5, ""),
        param("release_s", 0.05, 4.0, 0.6, "s"),
    ]
}

/// `felt_piano` voicing from [tone, felt, release_s].
pub fn felt_voicing(p: &[f32]) -> Voicing {
    let tone = p.first().copied().unwrap_or(0.5).clamp(0.0, 1.0);
    let felt = p.get(1).copied().unwrap_or(0.7).clamp(0.0, 1.0);
    let release = p.get(2).copied().unwrap_or(1.2).clamp(0.05, 4.0);
    Voicing {
        layer_velocity_scale: 1.0 - 0.45 * felt,
        bright_layer_boost: 0.25,
        auto_xfade: true,
        veltrack_scale: 1.0 - 0.35 * felt,
        min_attack_s: 0.0015 + 0.004 * felt,
        soft_attack_s: 0.008 * felt,
        release_s: Some(release),
        min_release_s: 0.02,
        tone: Tone {
            lp_base_hz: 850.0 * 2f32.powf(3.0 * tone) * 2f32.powf(-(felt - 0.5)),
            lp_vel_octaves: 2.0,
            lp_key_track: 0.5,
            lp_bright_octaves: 1.0,
            lp_q: 0.6,
            shelf_hz: 4500.0,
            shelf_db: -2.5 * felt,
            bright_shelf_db: 3.0,
        },
        // Make up for the energy the felt filter removes.
        gain_db: 2.0 * felt,
        gain_jitter_db: 0.25,
        sample_end_fade_s: 0.005,
        steal_fade_s: 0.005,
    }
}

/// `upright_piano` voicing from [tone, release_s].
pub fn upright_voicing(p: &[f32]) -> Voicing {
    let tone = p.first().copied().unwrap_or(0.5).clamp(0.0, 1.0);
    let release = p.get(1).copied().unwrap_or(0.6).clamp(0.05, 4.0);
    let mut t = Tone::NATURAL;
    if tone < 0.5 {
        t.lp_base_hz = 16000.0 * 2f32.powf(-6.0 * (0.5 - tone));
        t.lp_vel_octaves = 1.0;
        t.lp_key_track = 0.5;
    } else {
        t.shelf_db = 8.0 * (tone - 0.5);
    }
    Voicing {
        bright_layer_boost: 0.15,
        release_s: Some(release),
        tone: t,
        gain_jitter_db: 0.15,
        ..Voicing::default()
    }
}

fn info(name: &str, class: GeneratorClass, params: Vec<PresetParam>) -> PresetInfo {
    PresetInfo {
        kind: InstrumentKind::Sampler,
        name: name.to_string(),
        class,
        pitched: true,
        sustaining: true,
        range: PIANO_RANGE,
        params,
        source: SOURCE.to_string(),
    }
}

/// Loads the program a pack's manifest maps `preset` to.
pub fn load_pack_preset(
    packs: &Packs,
    pack: &str,
    preset: &str,
) -> Result<Arc<Program>, InstrumentError> {
    let manifest = packs
        .manifest(pack)
        .map_err(|_| InstrumentError::PackMissing(pack.to_string()))?;
    let entry = manifest.presets.get(preset).ok_or_else(|| {
        InstrumentError::Load(format!("pack `{pack}` has no entry for preset `{preset}`"))
    })?;
    let program = Program::load(packs, &format!("{pack}/{entry}"))?;
    if program.regions.is_empty() {
        return Err(InstrumentError::Load(format!(
            "`{pack}/{entry}` has no regions"
        )));
    }
    Ok(Arc::new(program))
}

/// Parameter values in `defs` order: the spec's value (clamped) or the default.
fn param_values(spec: &InstrumentRef, defs: &[PresetParam]) -> Vec<f32> {
    defs.iter()
        .map(|d| {
            spec.params
                .get(&d.name)
                .copied()
                .filter(|v| v.is_finite())
                .unwrap_or(d.default)
                .clamp(d.min, d.max) as f32
        })
        .collect()
}

fn build(
    spec: &InstrumentRef,
    ctx: &InstrumentContext,
    packs: &Packs,
    preset: &str,
    defs: Vec<PresetParam>,
    voicing: super::engine::VoicingFn,
) -> Result<Box<dyn Instrument>, InstrumentError> {
    let program = load_pack_preset(packs, PIANO_PACK, preset)?;
    let values = param_values(spec, &defs);
    Ok(Box::new(SamplerInstrument::new(
        program,
        ctx.sample_rate,
        PIANO_VOICES,
        voicing(&values),
        Some(voicing),
        &values,
    )))
}

fn build_felt(
    spec: &InstrumentRef,
    ctx: &InstrumentContext,
    packs: &Packs,
) -> Result<Box<dyn Instrument>, InstrumentError> {
    build(spec, ctx, packs, "felt_piano", felt_params(), felt_voicing)
}

fn build_upright(
    spec: &InstrumentRef,
    ctx: &InstrumentContext,
    packs: &Packs,
) -> Result<Box<dyn Instrument>, InstrumentError> {
    build(
        spec,
        ctx,
        packs,
        "upright_piano",
        upright_params(),
        upright_voicing,
    )
}

pub fn entries() -> Vec<PresetEntry> {
    vec![
        PresetEntry {
            info: info("felt_piano", GeneratorClass::Motif, felt_params()),
            build: build_felt,
        },
        PresetEntry {
            info: info("upright_piano", GeneratorClass::Melody, upright_params()),
            build: build_upright,
        },
    ]
}
