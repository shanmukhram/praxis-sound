//! Default motifs: spoken rhythms and scale-degree templates the offline provider and the
//! deterministic repairs reach for when a plan does not say. All are rhythm-first (a hit on every
//! downbeat, accents on the strong beats) and repeat.

use crate::catalogue::PresetSpec;
use praxis_score::{GeneratorClass, Grid, InstrumentKind, TimeSignature};

/// Grid steps per beat (the time signature's beat), or 2 when the grid does not divide it.
fn steps_per_beat(ts: TimeSignature, spb: u32) -> u32 {
    let beats = ts.numerator.max(1) as u32;
    if spb.is_multiple_of(beats) {
        (spb / beats).max(1)
    } else {
        2
    }
}

/// The rhythm family a preset plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RhythmRole {
    /// Pitched motif: every step a hit, strong downbeat, bright half-bar.
    Motif,
    /// Sparser struck motif (bells, glass, mallets): hits on beats, bright half-bar.
    Sparse,
    Kick,
    Backbeat,
    Hats,
    Shaker,
    /// Kit: DON = kick, TIN = snare, din = hat.
    Kit,
    /// Root on the downbeat and the half-bar, tied.
    Bass,
    /// One sustained hit per bar.
    Sustained,
    /// Every step, even weight (arpeggios).
    Arp,
    /// One hit at the start of the bar (effects).
    Single,
}

pub fn rhythm_role(preset: &PresetSpec, class: GeneratorClass) -> RhythmRole {
    use GeneratorClass as C;
    if preset.kind == InstrumentKind::Drum {
        return match preset.name.as_str() {
            "kick" | "soft_kick" | "tom_low" | "tom_mid" | "tom_high" => RhythmRole::Kick,
            "snare" | "clap" | "rim" => RhythmRole::Backbeat,
            "hat_closed" | "hat_open" => RhythmRole::Hats,
            "shaker" => RhythmRole::Shaker,
            _ => RhythmRole::Kit,
        };
    }
    match class {
        C::Bass => RhythmRole::Bass,
        C::Chords | C::Harmony | C::Pad | C::Drone | C::Texture | C::Ambience => {
            RhythmRole::Sustained
        }
        C::Arp => RhythmRole::Arp,
        C::Riser | C::Sweep | C::Impact | C::Boom | C::Whoosh | C::Glitch | C::Fill | C::Sfx => {
            RhythmRole::Single
        }
        C::Tick => RhythmRole::Hats,
        _ if matches!(preset.name.as_str(), "bell" | "glass" | "mallet") => RhythmRole::Sparse,
        _ => RhythmRole::Motif,
    }
}

/// One bar of spoken rhythm for a role on a grid. `None` when the grid does not divide the bar.
pub fn default_spoken(role: RhythmRole, ts: TimeSignature, grid: Grid) -> Option<String> {
    let spb = ts.steps_per_bar(grid)?;
    let beat = steps_per_beat(ts, spb);
    let half = if spb % 2 == 0 && spb >= 4 {
        Some(spb / 2)
    } else {
        None
    };
    let mut toks: Vec<&str> = Vec::with_capacity(spb as usize);
    for i in 0..spb {
        let on_beat = i % beat == 0;
        let beat_index = i / beat;
        let t = match role {
            RhythmRole::Motif => {
                if i == 0 {
                    "DON"
                } else if Some(i) == half {
                    "TIN"
                } else if on_beat {
                    "don"
                } else {
                    "din"
                }
            }
            RhythmRole::Sparse => {
                if i == 0 {
                    "DON"
                } else if Some(i) == half {
                    "TIN"
                } else if on_beat {
                    "don"
                } else {
                    "-"
                }
            }
            RhythmRole::Kick => {
                if i == 0 {
                    "DON"
                } else if Some(i) == half && on_beat {
                    "don"
                } else {
                    "-"
                }
            }
            RhythmRole::Backbeat => {
                if on_beat && beat_index % 2 == 1 {
                    "DON"
                } else {
                    "-"
                }
            }
            RhythmRole::Hats => {
                if on_beat {
                    "din"
                } else {
                    "don"
                }
            }
            RhythmRole::Shaker => "din",
            RhythmRole::Kit => {
                if i == 0 || (Some(i) == half && ts.numerator.is_multiple_of(2)) {
                    "DON"
                } else if on_beat && beat_index % 2 == 1 {
                    "TIN"
                } else if on_beat || i % 2 == 0 {
                    "din"
                } else {
                    "-"
                }
            }
            RhythmRole::Bass => {
                if i == 0 {
                    "DON"
                } else if Some(i) == half && on_beat {
                    "don"
                } else {
                    "~"
                }
            }
            RhythmRole::Sustained => {
                if i == 0 {
                    "DON"
                } else {
                    "~"
                }
            }
            RhythmRole::Arp => {
                if i == 0 {
                    "DON"
                } else if on_beat {
                    "don"
                } else {
                    "din"
                }
            }
            RhythmRole::Single => {
                if i == 0 {
                    "DON"
                } else {
                    "-"
                }
            }
        };
        toks.push(t);
    }
    Some(toks.join(" "))
}

/// Grids to try, best first, when a plan's grid does not fit the bar.
pub const GRID_PREFERENCE: [Grid; 5] = [
    Grid::Eighth,
    Grid::Quarter,
    Grid::Sixteenth,
    Grid::EighthTriplet,
    Grid::SixteenthTriplet,
];

pub fn fitting_grid(ts: TimeSignature) -> Grid {
    GRID_PREFERENCE
        .into_iter()
        .find(|g| ts.steps_per_bar(*g).is_some())
        .unwrap_or(Grid::Quarter)
}

/// A degree template for a pitched motif and the chord-root progression (scale degrees, one per
/// bar) it outlines.
#[derive(Clone, Debug, PartialEq)]
pub struct MotifTemplate {
    pub length_bars: u32,
    pub degrees: Vec<i32>,
    pub progression: Vec<i32>,
}

/// Motif templates; the offline provider picks one by seed. Template 0 is the acceptance-test
/// motif: an arpeggio outlining i then VI (I then vi in major).
pub fn motif_templates() -> Vec<MotifTemplate> {
    vec![
        MotifTemplate {
            length_bars: 2,
            degrees: vec![0, 4, 7, 4, 9, 4, 7, 4, -2, 2, 5, 2, 7, 2, 6, 4],
            progression: vec![0, 5],
        },
        MotifTemplate {
            length_bars: 2,
            degrees: vec![0, 2, 4, 7, 4, 2, 4, 2, -2, 0, 2, 5, 2, 0, 2, 0],
            progression: vec![0, 5],
        },
        MotifTemplate {
            length_bars: 1,
            degrees: vec![0, 4, 2, 4, 7, 4, 2, 4],
            progression: vec![0],
        },
    ]
}

/// Chord-relative degrees for a motif that follows explicit chords.
pub const CHORD_RELATIVE_MOTIF: [i32; 8] = [0, 4, 7, 4, 9, 4, 7, 4];

/// Two-bar chord-relative motif for scored cues: the first bar outlines the chord, the second
/// answers it reaching up to the fifth above the octave (degrees are cycled over the hits, so
/// on an eighth grid each bar takes eight).
pub const SCORED_MOTIF: [i32; 16] = [0, 4, 7, 4, 9, 4, 7, 4, 0, 4, 7, 9, 11, 9, 7, 4];

/// Default degrees for a role (relative to the chord root for chord-following roles).
pub fn default_degrees(role: RhythmRole) -> Vec<i32> {
    match role {
        RhythmRole::Sparse => vec![7, 9, 11, 9],
        RhythmRole::Arp => vec![0, 2, 4, 7],
        RhythmRole::Motif => CHORD_RELATIVE_MOTIF.to_vec(),
        _ => vec![0],
    }
}

/// Whether a role follows the chord progression by default.
pub fn follows_chords(role: RhythmRole) -> bool {
    matches!(
        role,
        RhythmRole::Bass | RhythmRole::Sustained | RhythmRole::Arp
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Catalogue;

    const FOUR: TimeSignature = TimeSignature {
        numerator: 4,
        denominator: 4,
    };

    #[test]
    fn acceptance_rhythms() {
        assert_eq!(
            default_spoken(RhythmRole::Motif, FOUR, Grid::Eighth).unwrap(),
            "DON din don din TIN din don din"
        );
        assert_eq!(
            default_spoken(RhythmRole::Kick, FOUR, Grid::Eighth).unwrap(),
            "DON - - - don - - -"
        );
        assert_eq!(
            default_spoken(RhythmRole::Backbeat, FOUR, Grid::Eighth).unwrap(),
            "- - DON - - - DON -"
        );
        assert_eq!(
            default_spoken(RhythmRole::Bass, FOUR, Grid::Eighth).unwrap(),
            "DON ~ ~ ~ don ~ ~ ~"
        );
    }

    #[test]
    fn every_default_parses_on_every_grid() {
        let c = Catalogue::standard();
        for ts in [
            FOUR,
            TimeSignature {
                numerator: 3,
                denominator: 4,
            },
            TimeSignature {
                numerator: 6,
                denominator: 8,
            },
            TimeSignature {
                numerator: 5,
                denominator: 4,
            },
        ] {
            for g in GRID_PREFERENCE {
                for p in &c.presets {
                    let role = rhythm_role(p, p.class);
                    if let Some(s) = default_spoken(role, ts, g) {
                        let steps = praxis_score::rhythm::parse_spoken(&s).unwrap();
                        assert_eq!(steps.len() as u32, ts.steps_per_bar(g).unwrap());
                    }
                }
            }
        }
        assert_eq!(
            fitting_grid(TimeSignature {
                numerator: 5,
                denominator: 8
            }),
            Grid::Eighth
        );
    }
}
