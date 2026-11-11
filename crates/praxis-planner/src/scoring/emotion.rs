//! Emotion presets (FR-9): each of the eight moods maps to a mode, a tempo range, a register,
//! instruments in the order they enter, a default form with its intensity shape, and chord
//! progressions (scale degrees: 0 = tonic chord, 5 = the chord on the sixth degree).

use crate::plan::Register;
use praxis_score::{Mode, PitchClass, SectionRole};

/// One emotion preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmotionPreset {
    /// The preset's name, also the plan's emotion word when a brief uses a synonym the offline
    /// planner does not know.
    pub name: &'static str,
    /// Mood words that select this preset.
    pub synonyms: &'static [&'static str],
    pub tonic: PitchClass,
    pub mode: Mode,
    /// Lowest, usual and highest tempo in BPM.
    pub tempo: (f64, f64, f64),
    /// Register of the lead motif.
    pub register: Register,
    /// Presets in the order they enter as the intensity rises (used when a brief names none).
    pub instruments: &'static [&'static str],
    /// Default form: role, weight and intensity of each part.
    pub shape: &'static [(SectionRole, f32, f32)],
    /// Chord progressions as scale degrees of the chord roots.
    pub progressions: &'static [&'static [i32]],
    /// Bars each chord lasts.
    pub bars_per_chord: u32,
}

use SectionRole::{Build, Intro, Main, Outro, Peak, Release, Verse};

/// The eight presets: calm, tense, hopeful, melancholy, playful, dark, warm and
/// epic-but-restrained.
pub const PRESETS: [EmotionPreset; 8] = [
    EmotionPreset {
        name: "calm",
        synonyms: &[
            "calm",
            "serene",
            "peaceful",
            "tranquil",
            "relaxed",
            "relaxing",
            "dreamy",
            "reflective",
            "gentle",
            "soothing",
        ],
        tonic: PitchClass::C,
        mode: Mode::Major,
        tempo: (72.0, 84.0, 96.0),
        register: Register::Mid,
        instruments: &["felt_piano", "soft_keys", "warm_bass"],
        shape: &[
            (Intro, 2.0, 0.3),
            (Main, 3.0, 0.5),
            (Peak, 3.0, 0.7),
            (Release, 2.0, 0.35),
        ],
        progressions: &[&[0, 4, 5, 3], &[0, 3, 0, 4], &[0, 5, 3, 4]],
        bars_per_chord: 1,
    },
    EmotionPreset {
        name: "tense",
        synonyms: &[
            "tense",
            "tension",
            "anxious",
            "suspense",
            "suspenseful",
            "uneasy",
            "nervous",
            "urgent",
        ],
        tonic: PitchClass::D,
        mode: Mode::Minor,
        tempo: (88.0, 100.0, 120.0),
        register: Register::Low,
        instruments: &["felt_piano", "sub_bass", "soft_kick"],
        shape: &[
            (Intro, 2.0, 0.35),
            (Build, 3.0, 0.55),
            (Peak, 3.0, 0.9),
            (Release, 2.0, 0.4),
        ],
        progressions: &[&[0, 0, 5, 4], &[0, 6, 5, 6], &[0, 5, 0, 4]],
        bars_per_chord: 1,
    },
    EmotionPreset {
        name: "hopeful",
        synonyms: &[
            "hopeful",
            "hope",
            "uplifting",
            "uplifted",
            "optimistic",
            "inspiring",
        ],
        tonic: PitchClass::C,
        mode: Mode::Major,
        tempo: (84.0, 96.0, 112.0),
        register: Register::High,
        instruments: &["upright_piano", "soft_keys", "warm_bass", "shaker"],
        shape: &[
            (Intro, 2.0, 0.3),
            (Build, 3.0, 0.55),
            (Peak, 3.0, 0.85),
            (Release, 2.0, 0.5),
        ],
        progressions: &[&[3, 0, 4, 5], &[0, 4, 5, 3], &[5, 3, 0, 4]],
        bars_per_chord: 1,
    },
    EmotionPreset {
        name: "melancholy",
        synonyms: &[
            "melancholy",
            "melancholic",
            "sad",
            "sorrowful",
            "somber",
            "sombre",
            "lonely",
            "wistful",
            "nostalgic",
            "bittersweet",
            "mournful",
        ],
        tonic: PitchClass::E,
        mode: Mode::Minor,
        tempo: (66.0, 76.0, 86.0),
        register: Register::Mid,
        instruments: &["felt_piano", "soft_keys", "warm_bass"],
        shape: &[
            (Intro, 2.0, 0.25),
            (Main, 3.0, 0.45),
            (Peak, 2.0, 0.65),
            (Release, 3.0, 0.3),
        ],
        progressions: &[&[0, 5, 2, 6], &[0, 3, 5, 4], &[0, 5, 3, 4]],
        bars_per_chord: 1,
    },
    EmotionPreset {
        name: "playful",
        synonyms: &[
            "playful",
            "happy",
            "cheerful",
            "whimsical",
            "joyful",
            "upbeat",
            "fun",
            "bouncy",
        ],
        tonic: PitchClass::F,
        mode: Mode::Major,
        tempo: (100.0, 112.0, 128.0),
        register: Register::Mid,
        instruments: &["pluck", "mallet", "warm_bass", "shaker"],
        shape: &[
            (Intro, 1.0, 0.4),
            (Verse, 3.0, 0.55),
            (Main, 3.0, 0.75),
            (Outro, 1.0, 0.45),
        ],
        progressions: &[&[0, 3, 4, 3], &[0, 1, 4, 0], &[0, 5, 1, 4]],
        bars_per_chord: 1,
    },
    EmotionPreset {
        name: "dark",
        synonyms: &[
            "dark",
            "ominous",
            "eerie",
            "brooding",
            "menacing",
            "grim",
            "foreboding",
            "sinister",
            "mysterious",
            "creepy",
        ],
        tonic: PitchClass::E,
        mode: Mode::Phrygian,
        tempo: (64.0, 76.0, 88.0),
        register: Register::Low,
        instruments: &["felt_piano", "sub_bass", "soft_kick"],
        shape: &[
            (Intro, 2.0, 0.3),
            (Build, 3.0, 0.5),
            (Peak, 2.0, 0.75),
            (Release, 3.0, 0.35),
        ],
        progressions: &[&[0, 1, 0, 6], &[0, 5, 1, 0], &[0, 3, 1, 0]],
        bars_per_chord: 1,
    },
    EmotionPreset {
        name: "warm",
        synonyms: &[
            "warm",
            "tender",
            "romantic",
            "cozy",
            "cosy",
            "intimate",
            "heartfelt",
            "loving",
        ],
        tonic: PitchClass::F,
        mode: Mode::Major,
        tempo: (72.0, 86.0, 100.0),
        register: Register::Mid,
        instruments: &["felt_piano", "soft_keys", "warm_bass", "soft_kick"],
        shape: &[
            (Intro, 2.0, 0.3),
            (Main, 3.0, 0.5),
            (Peak, 3.0, 0.7),
            (Release, 2.0, 0.4),
        ],
        progressions: &[&[0, 5, 3, 4], &[0, 2, 3, 0], &[0, 3, 5, 4]],
        bars_per_chord: 1,
    },
    EmotionPreset {
        name: "epic-but-restrained",
        synonyms: &[
            "epic-but-restrained",
            "epic",
            "heroic",
            "majestic",
            "triumphant",
        ],
        tonic: PitchClass::D,
        mode: Mode::Minor,
        tempo: (76.0, 90.0, 104.0),
        register: Register::Mid,
        instruments: &["upright_piano", "soft_keys", "sub_bass", "tom_low"],
        shape: &[
            (Intro, 2.0, 0.3),
            (Build, 3.0, 0.5),
            (Build, 2.0, 0.7),
            (Peak, 3.0, 0.9),
            (Release, 2.0, 0.45),
        ],
        progressions: &[&[0, 5, 2, 6], &[0, 6, 5, 6], &[0, 5, 6, 0]],
        bars_per_chord: 2,
    },
];

/// The preset an emotion word selects, if any (the name or a synonym, case-insensitive).
pub fn find(word: &str) -> Option<&'static EmotionPreset> {
    let w = word.trim().to_lowercase();
    PRESETS
        .iter()
        .find(|p| p.name == w || p.synonyms.contains(&w.as_str()))
}

/// The preset for an emotion word, `calm` when none matches.
pub fn preset_for(word: &str) -> &'static EmotionPreset {
    find(word).unwrap_or(&PRESETS[0])
}

/// The usual intensity of a part with this role (an emotion preset's shape wins for its roles).
pub fn role_intensity(role: SectionRole) -> f32 {
    match role {
        Intro => 0.3,
        Verse => 0.45,
        Main => 0.55,
        Build => 0.6,
        Peak => 0.85,
        SectionRole::Breakdown => 0.35,
        Release => 0.4,
        SectionRole::Transition => 0.5,
        Outro => 0.3,
    }
}

/// One line per preset for the model's system prompt.
pub fn prompt_listing() -> String {
    PRESETS
        .iter()
        .map(|p| {
            let shape: Vec<String> = p
                .shape
                .iter()
                .map(|(r, _, i)| format!("{} {i}", role_word(*r)))
                .collect();
            format!(
                "- {}: {} {}, {}-{} BPM, {} register, instruments {}, form {}",
                p.name,
                p.tonic.name(),
                format!("{:?}", p.mode).to_lowercase(),
                p.tempo.0,
                p.tempo.2,
                match p.register {
                    Register::Low => "low",
                    Register::Mid => "mid",
                    Register::High => "high",
                },
                p.instruments.join(", "),
                shape.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// snake_case name of a section role, as in the plan JSON.
pub fn role_word(role: SectionRole) -> &'static str {
    match role {
        Intro => "intro",
        Main => "main",
        Verse => "verse",
        Build => "build",
        Peak => "peak",
        SectionRole::Breakdown => "breakdown",
        Release => "release",
        SectionRole::Transition => "transition",
        Outro => "outro",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Catalogue;

    #[test]
    fn eight_presets_with_valid_instruments_and_progressions() {
        let names: Vec<&str> = PRESETS.iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            [
                "calm",
                "tense",
                "hopeful",
                "melancholy",
                "playful",
                "dark",
                "warm",
                "epic-but-restrained"
            ]
        );
        let c = Catalogue::standard();
        for p in &PRESETS {
            assert!(p.tempo.0 < p.tempo.1 && p.tempo.1 < p.tempo.2, "{}", p.name);
            for i in p.instruments {
                assert!(c.get(i).is_some(), "{} uses unknown preset {i}", p.name);
            }
            for prog in p.progressions {
                assert_eq!(prog.len(), 4);
                assert!(prog.iter().all(|d| (0..7).contains(d)));
            }
            assert!(p.shape.len() >= 4);
        }
        assert_eq!(find("Ominous").map(|p| p.name), Some("dark"));
        assert_eq!(find("epic").map(|p| p.name), Some("epic-but-restrained"));
        assert_eq!(preset_for("unknown").name, "calm");
    }
}
