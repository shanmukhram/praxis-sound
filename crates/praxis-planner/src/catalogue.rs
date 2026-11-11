//! The preset catalogue the planner may choose from.
//!
//! The planner cannot depend on praxis-instruments, so this is a copy of the stable preset names in
//! docs/contracts/instruments.md with the facts the planner needs (class, pitched, sustaining,
//! playable range, GM drum note). Where the contract lists a preset's class as "various" the class
//! here is the planner's reading of the name; see the "Plan schema" section of
//! docs/contracts/planner.md.
//!
//! Tone controls ([`ToneControls`]) copy the brightness parameters each preset declares in
//! praxis-instruments (`tone` 0..1 on every pitched synth, the sampled pianos and the drums;
//! `felt` 0..1 on `felt_piano`, see `sampler/presets.rs`), so a tone edit can change the sound
//! through the instrument instead of an effect. The procedural sound effects have none.

use praxis_score::{GeneratorClass, InstrumentKind};

/// One preset the planner may use.
#[derive(Clone, Debug, PartialEq)]
pub struct PresetSpec {
    pub kind: InstrumentKind,
    pub name: String,
    /// The preset's intrinsic class (the avoid-list keys on it).
    pub class: GeneratorClass,
    /// False for unpitched drums, noise and ticks.
    pub pitched: bool,
    /// True when notes ring past their length (piano, mallets): patterns should overlap.
    pub sustaining: bool,
    /// Playable MIDI range, inclusive.
    pub range: (u8, u8),
    /// General MIDI drum note for single-voice drum presets.
    pub gm_note: Option<u8>,
    /// One line for the model.
    pub description: String,
    /// The preset's brightness parameters, when it has any.
    pub tone: Option<ToneControls>,
}

/// A preset's brightness parameters (names and defaults as the preset declares them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneControls {
    /// Default of `tone` (0 dark ..= 1 bright).
    pub tone_default: f64,
    /// Default of `felt` (0..=1, more is softer and more muffled), `felt_piano` only.
    pub felt_default: Option<f64>,
}

impl ToneControls {
    /// Name of the brightness parameter.
    pub const TONE: &'static str = "tone";
    /// Name of the felt parameter.
    pub const FELT: &'static str = "felt";
}

/// Tone controls by preset name (praxis-instruments `PresetInfo::params`).
fn tone_controls(name: &str) -> Option<ToneControls> {
    let t = |tone_default: f64| {
        Some(ToneControls {
            tone_default,
            felt_default: None,
        })
    };
    match name {
        "felt_piano" => Some(ToneControls {
            tone_default: 0.5,
            felt_default: Some(0.7),
        }),
        "upright_piano" | "bell" => t(0.5),
        "soft_keys" | "fm_keys" | "warm_bass" | "granular_texture" => t(0.4),
        "pluck" | "mallet" | "glass" => t(0.45),
        "sub_bass" => t(0.3),
        "noise_bed" => t(0.35),
        "soft_kick" | "kick" | "snare" | "rim" | "hat_closed" | "hat_open" | "shaker"
        | "tom_low" | "tom_mid" | "tom_high" | "clap" | "kit" => t(0.5),
        _ => None,
    }
}

/// The presets the planner can choose from.
#[derive(Clone, Debug, PartialEq)]
pub struct Catalogue {
    pub presets: Vec<PresetSpec>,
}

impl Default for Catalogue {
    fn default() -> Self {
        Catalogue::standard()
    }
}

#[allow(clippy::too_many_arguments)]
fn spec(
    kind: InstrumentKind,
    name: &str,
    class: GeneratorClass,
    pitched: bool,
    sustaining: bool,
    range: (u8, u8),
    gm_note: Option<u8>,
    description: &str,
) -> PresetSpec {
    PresetSpec {
        kind,
        name: name.to_string(),
        class,
        pitched,
        sustaining,
        range,
        gm_note,
        description: description.to_string(),
        tone: tone_controls(name),
    }
}

impl Catalogue {
    /// The preset names of docs/contracts/instruments.md.
    pub fn standard() -> Catalogue {
        use GeneratorClass as C;
        use InstrumentKind::{Drum, Sampler, Sfx, Synth};
        let presets = vec![
            spec(
                Sampler,
                "felt_piano",
                C::Motif,
                true,
                true,
                (21, 108),
                None,
                "sampled upright piano voiced soft with felt: intimate, rhythmic motifs",
            ),
            spec(
                Sampler,
                "upright_piano",
                C::Melody,
                true,
                true,
                (21, 108),
                None,
                "sampled upright piano, natural voicing",
            ),
            spec(
                Synth,
                "bell",
                C::Motif,
                true,
                true,
                (48, 108),
                None,
                "struck FM bell",
            ),
            spec(
                Synth,
                "soft_keys",
                C::Chords,
                true,
                true,
                (36, 96),
                None,
                "soft electric keys for chords",
            ),
            spec(
                Synth,
                "pluck",
                C::Arp,
                true,
                true,
                (36, 96),
                None,
                "plucked string model for arpeggios",
            ),
            spec(
                Synth,
                "mallet",
                C::Motif,
                true,
                true,
                (45, 96),
                None,
                "struck mallet (marimba / vibraphone-like)",
            ),
            spec(
                Synth,
                "sub_bass",
                C::Bass,
                true,
                false,
                (24, 55),
                None,
                "sine sub bass",
            ),
            spec(
                Synth,
                "warm_bass",
                C::Bass,
                true,
                false,
                (28, 60),
                None,
                "warm round synth bass",
            ),
            spec(
                Synth,
                "fm_keys",
                C::Melody,
                true,
                true,
                (36, 96),
                None,
                "FM electric piano for lead lines",
            ),
            spec(
                Synth,
                "glass",
                C::Motif,
                true,
                true,
                (48, 108),
                None,
                "glassy struck tone",
            ),
            spec(
                Synth,
                "granular_texture",
                C::Pad,
                true,
                true,
                (36, 96),
                None,
                "granular sustained pad (pad class: only when the brief asks for a pad)",
            ),
            spec(
                Synth,
                "noise_bed",
                C::Texture,
                false,
                true,
                (0, 127),
                None,
                "filtered noise bed",
            ),
            spec(
                Drum,
                "soft_kick",
                C::Drums,
                false,
                false,
                (35, 81),
                Some(36),
                "soft, muted kick: short sine sweep plus felt click, low-passed",
            ),
            spec(
                Drum,
                "kick",
                C::Drums,
                false,
                false,
                (35, 81),
                Some(36),
                "synthesised kick",
            ),
            spec(
                Drum,
                "snare",
                C::Drums,
                false,
                false,
                (35, 81),
                Some(38),
                "synthesised snare",
            ),
            spec(
                Drum,
                "rim",
                C::Drums,
                false,
                false,
                (35, 81),
                Some(37),
                "rim click",
            ),
            spec(
                Drum,
                "hat_closed",
                C::Drums,
                false,
                false,
                (35, 81),
                Some(42),
                "closed hi-hat",
            ),
            spec(
                Drum,
                "hat_open",
                C::Drums,
                false,
                false,
                (35, 81),
                Some(46),
                "open hi-hat",
            ),
            spec(
                Drum,
                "shaker",
                C::Percussion,
                false,
                false,
                (35, 81),
                Some(70),
                "shaker",
            ),
            spec(
                Drum,
                "tom_low",
                C::Drums,
                false,
                false,
                (35, 81),
                Some(41),
                "low tom",
            ),
            spec(
                Drum,
                "tom_mid",
                C::Drums,
                false,
                false,
                (35, 81),
                Some(45),
                "mid tom",
            ),
            spec(
                Drum,
                "tom_high",
                C::Drums,
                false,
                false,
                (35, 81),
                Some(48),
                "high tom",
            ),
            spec(
                Drum,
                "clap",
                C::Drums,
                false,
                false,
                (35, 81),
                Some(39),
                "hand clap",
            ),
            spec(
                Drum,
                "kit",
                C::Drums,
                false,
                false,
                (35, 81),
                None,
                "multi-voice kit keyed by the GM drum map (36 kick, 38 snare, 42 hat)",
            ),
            spec(
                Sfx,
                "whoosh",
                C::Whoosh,
                true,
                false,
                (24, 96),
                None,
                "procedural whoosh",
            ),
            spec(
                Sfx,
                "impact",
                C::Impact,
                true,
                false,
                (24, 96),
                None,
                "procedural impact",
            ),
            spec(
                Sfx,
                "riser",
                C::Riser,
                true,
                false,
                (24, 96),
                None,
                "procedural riser",
            ),
            spec(
                Sfx,
                "drone",
                C::Drone,
                true,
                true,
                (24, 84),
                None,
                "tuned drone",
            ),
            spec(
                Sfx,
                "glitch",
                C::Glitch,
                true,
                false,
                (24, 96),
                None,
                "glitch texture",
            ),
            spec(
                Sfx,
                "tick",
                C::Tick,
                false,
                false,
                (24, 96),
                None,
                "clock tick",
            ),
        ];
        Catalogue { presets }
    }

    /// Preset by name (names are unique across kinds in the standard catalogue).
    pub fn get(&self, name: &str) -> Option<&PresetSpec> {
        self.presets.iter().find(|p| p.name == name)
    }

    pub fn get_kind(&self, kind: InstrumentKind, name: &str) -> Option<&PresetSpec> {
        self.presets
            .iter()
            .find(|p| p.kind == kind && p.name == name)
    }

    /// The preset the planner reaches for when a layer of `class` names no usable preset.
    pub fn default_for_class(&self, class: GeneratorClass) -> Option<&PresetSpec> {
        use GeneratorClass as C;
        let name = match class {
            C::Motif => "felt_piano",
            C::Melody => "upright_piano",
            C::Harmony | C::Chords => "soft_keys",
            C::Arp => "pluck",
            C::Bass => "warm_bass",
            C::Drums => "soft_kick",
            C::Percussion => "shaker",
            C::Pad => "granular_texture",
            C::Texture | C::Ambience => "noise_bed",
            C::Drone => "drone",
            C::Riser => "riser",
            C::Impact | C::Boom => "impact",
            C::Whoosh | C::Sweep => "whoosh",
            C::Glitch => "glitch",
            C::Tick => "tick",
            C::Fill | C::Sfx => return None,
        };
        self.get(name)
    }

    /// The catalogue as JSON for agents: `{"presets": [{kind, name, class, pitched, sustaining,
    /// range: [lo, hi], gm_note, description}], "trailer_classes": [...]}`.
    pub fn to_json(&self) -> serde_json::Value {
        let presets: Vec<serde_json::Value> = self
            .presets
            .iter()
            .map(|p| {
                serde_json::json!({
                    "kind": kind_name(p.kind),
                    "name": p.name,
                    "class": class_name(p.class),
                    "pitched": p.pitched,
                    "sustaining": p.sustaining,
                    "range": [p.range.0, p.range.1],
                    "gm_note": p.gm_note,
                    "description": p.description,
                    "tone_params": p.tone.map(|t| {
                        let mut m = serde_json::Map::new();
                        m.insert(ToneControls::TONE.into(), t.tone_default.into());
                        if let Some(f) = t.felt_default {
                            m.insert(ToneControls::FELT.into(), f.into());
                        }
                        serde_json::Value::Object(m)
                    }),
                })
            })
            .collect();
        let trailer: Vec<String> = crate::plan::TRAILER_CLASSES
            .iter()
            .map(|c| class_name(*c))
            .collect();
        serde_json::json!({
            "presets": presets,
            "trailer_classes": trailer,
            "note": "trailer classes are blocked unless the plan lists them in `requested`",
        })
    }

    /// Text listing for the model's system prompt, one preset per line.
    pub fn prompt_listing(&self) -> String {
        let mut s = String::new();
        for p in &self.presets {
            s.push_str(&format!(
                "- {} (kind {}, class {}, {}{}, MIDI {}-{}): {}\n",
                p.name,
                kind_name(p.kind),
                class_name(p.class),
                if p.pitched { "pitched" } else { "unpitched" },
                if p.sustaining { ", sustaining" } else { "" },
                p.range.0,
                p.range.1,
                p.description
            ));
        }
        s
    }
}

/// snake_case name of an instrument kind, as in the Score JSON.
pub fn kind_name(kind: InstrumentKind) -> &'static str {
    match kind {
        InstrumentKind::Sampler => "sampler",
        InstrumentKind::Synth => "synth",
        InstrumentKind::Drum => "drum",
        InstrumentKind::Sfx => "sfx",
    }
}

/// snake_case name of a generator class, as in the Score JSON.
pub fn class_name(class: GeneratorClass) -> String {
    serde_json::to_value(class)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_defaults_exist() {
        let c = Catalogue::standard();
        let mut names: Vec<&str> = c.presets.iter().map(|p| p.name.as_str()).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len());
        assert_eq!(
            c.get("felt_piano").map(|p| p.class),
            Some(GeneratorClass::Motif)
        );
        assert_eq!(c.get("soft_kick").and_then(|p| p.gm_note), Some(36));
        for p in &c.presets {
            if let Some(d) = c.default_for_class(p.class) {
                assert_eq!(d.class, p.class, "{}", d.name);
            }
        }
        assert_eq!(class_name(GeneratorClass::Riser), "riser");
        assert!(c.prompt_listing().contains("felt_piano (kind sampler"));
    }

    #[test]
    fn tone_controls_follow_the_presets() {
        let c = Catalogue::standard();
        let felt = c.get("felt_piano").and_then(|p| p.tone).expect("felt tone");
        assert_eq!(felt.tone_default, 0.5);
        assert_eq!(felt.felt_default, Some(0.7));
        assert_eq!(
            c.get("upright_piano")
                .and_then(|p| p.tone)
                .map(|t| t.felt_default),
            Some(None)
        );
        for p in &c.presets {
            let sfx = p.kind == InstrumentKind::Sfx;
            assert_eq!(p.tone.is_none(), sfx, "{}", p.name);
        }
        assert_eq!(
            c.to_json()["presets"][0]["tone_params"],
            serde_json::json!({"tone": 0.5, "felt": 0.7})
        );
    }
}
