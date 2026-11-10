//! Music theory primitives: pitch classes, scales, key membership, durations.

use crate::types::{Grid, Key, Mode, PitchClass, TimeSignature};

impl PitchClass {
    pub const ALL: [PitchClass; 12] = [
        PitchClass::C,
        PitchClass::Cs,
        PitchClass::D,
        PitchClass::Eb,
        PitchClass::E,
        PitchClass::F,
        PitchClass::Fs,
        PitchClass::G,
        PitchClass::Ab,
        PitchClass::A,
        PitchClass::Bb,
        PitchClass::B,
    ];

    /// Semitones above C, 0..12.
    pub fn semitone(self) -> u8 {
        PitchClass::ALL.iter().position(|p| *p == self).unwrap_or(0) as u8
    }

    pub fn from_semitone(s: i32) -> PitchClass {
        PitchClass::ALL[s.rem_euclid(12) as usize]
    }

    pub fn of_midi(pitch: u8) -> PitchClass {
        PitchClass::from_semitone(pitch as i32)
    }

    pub fn name(self) -> &'static str {
        [
            "C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
        ][self.semitone() as usize]
    }

    /// Parses "C", "c#", "Db", "F♯" etc.
    pub fn parse(s: &str) -> Option<PitchClass> {
        let s = s.trim();
        let mut chars = s.chars();
        let letter = chars.next()?.to_ascii_uppercase();
        let base: i32 = match letter {
            'C' => 0,
            'D' => 2,
            'E' => 4,
            'F' => 5,
            'G' => 7,
            'A' => 9,
            'B' => 11,
            _ => return None,
        };
        let mut acc = 0;
        for c in chars {
            match c {
                '#' | '♯' | 's' => acc += 1,
                'b' | '♭' => acc -= 1,
                _ => return None,
            }
        }
        Some(PitchClass::from_semitone(base + acc))
    }
}

impl Mode {
    /// Scale intervals in semitones above the tonic.
    pub fn intervals(self) -> &'static [u8] {
        match self {
            Mode::Major => &[0, 2, 4, 5, 7, 9, 11],
            Mode::Minor => &[0, 2, 3, 5, 7, 8, 10],
            Mode::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            Mode::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
            Mode::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Mode::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            Mode::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            Mode::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Mode::Locrian => &[0, 1, 3, 5, 6, 8, 10],
            Mode::MajorPentatonic => &[0, 2, 4, 7, 9],
            Mode::MinorPentatonic => &[0, 3, 5, 7, 10],
            Mode::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        }
    }
}

impl Key {
    /// True when the MIDI pitch belongs to this key's scale.
    pub fn contains(&self, pitch: u8) -> bool {
        let rel = (pitch as i32 - self.tonic.semitone() as i32).rem_euclid(12) as u8;
        self.mode.intervals().contains(&rel)
    }

    /// Pitch classes of the scale, tonic first.
    pub fn scale(&self) -> Vec<PitchClass> {
        self.mode
            .intervals()
            .iter()
            .map(|i| PitchClass::from_semitone(self.tonic.semitone() as i32 + *i as i32))
            .collect()
    }

    /// MIDI pitch of scale degree `degree` (0 = tonic, may be negative or exceed the scale)
    /// counted from the tonic in `octave` (MIDI octave numbering, C4 = 60 => octave 4).
    pub fn degree_to_midi(&self, degree: i32, octave: i32) -> i32 {
        let iv = self.mode.intervals();
        let n = iv.len() as i32;
        let oct = degree.div_euclid(n);
        let idx = degree.rem_euclid(n) as usize;
        (octave + 1) * 12 + self.tonic.semitone() as i32 + oct * 12 + iv[idx] as i32
    }

    /// Nearest in-key pitch to `pitch` (ties resolve downward).
    pub fn snap(&self, pitch: u8) -> u8 {
        if self.contains(pitch) {
            return pitch;
        }
        for d in 1..=6i32 {
            let down = pitch as i32 - d;
            if (0..=127).contains(&down) && self.contains(down as u8) {
                return down as u8;
            }
            let up = pitch as i32 + d;
            if (0..=127).contains(&up) && self.contains(up as u8) {
                return up as u8;
            }
        }
        pitch
    }

    pub fn display(&self) -> String {
        format!("{} {:?}", self.tonic.name(), self.mode).to_lowercase()
    }
}

impl Grid {
    /// Duration of one step as a fraction of a whole note.
    pub fn step_whole_notes(self) -> f64 {
        match self {
            Grid::Quarter => 1.0 / 4.0,
            Grid::Eighth => 1.0 / 8.0,
            Grid::Sixteenth => 1.0 / 16.0,
            Grid::EighthTriplet => 1.0 / 12.0,
            Grid::SixteenthTriplet => 1.0 / 24.0,
        }
    }
}

impl TimeSignature {
    /// Bar length as a fraction of a whole note (4/4 = 1.0, 6/8 = 0.75).
    pub fn bar_whole_notes(self) -> f64 {
        self.numerator as f64 / self.denominator as f64
    }

    /// Bar length in quarter-note beats (the unit `Tempo::bpm` counts).
    pub fn bar_quarters(self) -> f64 {
        self.bar_whole_notes() * 4.0
    }

    /// Number of grid steps in one bar, if whole.
    pub fn steps_per_bar(self, grid: Grid) -> Option<u32> {
        let s = self.bar_whole_notes() / grid.step_whole_notes();
        let r = s.round();
        if (s - r).abs() < 1e-9 && r >= 1.0 {
            Some(r as u32)
        } else {
            None
        }
    }
}

pub fn midi_to_hz(pitch: f64) -> f64 {
    440.0 * 2f64.powf((pitch - 69.0) / 12.0)
}

pub fn hz_to_midi(hz: f64) -> f64 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// "A4" style name for a MIDI pitch.
pub fn midi_name(pitch: u8) -> String {
    format!(
        "{}{}",
        PitchClass::of_midi(pitch).name(),
        pitch as i32 / 12 - 1
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minor_membership() {
        let k = Key {
            tonic: PitchClass::A,
            mode: Mode::Minor,
        };
        for p in [57u8, 59, 60, 62, 64, 65, 67, 69] {
            assert!(k.contains(p), "{p}");
        }
        for p in [58u8, 61, 63, 66, 68] {
            assert!(!k.contains(p), "{p}");
        }
        assert_eq!(k.degree_to_midi(0, 4), 69);
        assert_eq!(k.degree_to_midi(2, 4), 72);
        assert_eq!(k.degree_to_midi(-1, 4), 67);
        assert_eq!(k.snap(61), 60);
    }

    #[test]
    fn parse_pitch_classes() {
        assert_eq!(PitchClass::parse("c#"), Some(PitchClass::Cs));
        assert_eq!(PitchClass::parse("Db"), Some(PitchClass::Cs));
        assert_eq!(PitchClass::parse("Bb"), Some(PitchClass::Bb));
        assert_eq!(PitchClass::parse("H"), None);
    }

    #[test]
    fn grid_and_bars() {
        let ts = TimeSignature {
            numerator: 4,
            denominator: 4,
        };
        assert_eq!(ts.steps_per_bar(Grid::Eighth), Some(8));
        assert_eq!(ts.steps_per_bar(Grid::EighthTriplet), Some(12));
        let ts68 = TimeSignature {
            numerator: 6,
            denominator: 8,
        };
        assert_eq!(ts68.steps_per_bar(Grid::Eighth), Some(6));
        assert_eq!(ts68.steps_per_bar(Grid::Quarter), Some(3));
        let ts58 = TimeSignature {
            numerator: 5,
            denominator: 8,
        };
        assert_eq!(ts58.steps_per_bar(Grid::Quarter), None);
        assert!((midi_to_hz(69.0) - 440.0).abs() < 1e-9);
        assert_eq!(midi_name(60), "C4");
    }
}
