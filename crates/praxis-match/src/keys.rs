//! Reading a key the user typed: `"A minor"`, `"Am"`, `"F#"`, `"Eb dorian"`, `"c#m"`.

use praxis_score::{Key, Mode, PitchClass};

/// Parses a key. A bare tonic is major; `m` or `min` after the tonic is minor.
pub fn parse_key(text: &str) -> Option<Key> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let (tonic_text, rest) = split_tonic(t)?;
    let tonic = PitchClass::parse(&tonic_text)?;
    let rest = rest.trim().to_ascii_lowercase().replace(['-', '_'], " ");
    let rest = rest.trim();
    let mode = match rest {
        "" | "maj" | "major" | "ionian" => Mode::Major,
        "m" | "min" | "minor" | "aeolian" => Mode::Minor,
        "harmonic minor" => Mode::HarmonicMinor,
        "melodic minor" => Mode::MelodicMinor,
        "dorian" => Mode::Dorian,
        "phrygian" => Mode::Phrygian,
        "lydian" => Mode::Lydian,
        "mixolydian" => Mode::Mixolydian,
        "locrian" => Mode::Locrian,
        "major pentatonic" => Mode::MajorPentatonic,
        "minor pentatonic" | "m pentatonic" => Mode::MinorPentatonic,
        _ => return None,
    };
    Some(Key { tonic, mode })
}

/// Splits `"F#m"` into `("F#", "m")` and `"Bb minor"` into `("Bb", " minor")`.
fn split_tonic(t: &str) -> Option<(String, &str)> {
    let mut chars = t.char_indices();
    let (_, letter) = chars.next()?;
    if !letter.is_ascii_alphabetic() {
        return None;
    }
    let mut end = letter.len_utf8();
    // A `b` right after the letter is always a flat ("Bb", "Ebm"); "bm" is B minor because its
    // second character is `m`.
    if let Some((i, c)) = chars.next()
        && matches!(c, '#' | 'b' | '♯' | '♭')
    {
        end = i + c.len_utf8();
    }
    let mut tonic: String = t[..end].to_string();
    tonic = tonic.replace('♯', "#").replace('♭', "b");
    let mut cs = tonic.chars();
    let first = cs.next()?.to_ascii_uppercase();
    let tonic = std::iter::once(first).chain(cs).collect::<String>();
    Some((tonic, &t[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(t: PitchClass, m: Mode) -> Option<Key> {
        Some(Key { tonic: t, mode: m })
    }

    #[test]
    fn reads_common_spellings() {
        assert_eq!(parse_key("A minor"), k(PitchClass::A, Mode::Minor));
        assert_eq!(parse_key("Am"), k(PitchClass::A, Mode::Minor));
        assert_eq!(parse_key("am"), k(PitchClass::A, Mode::Minor));
        assert_eq!(parse_key("F#"), k(PitchClass::Fs, Mode::Major));
        assert_eq!(parse_key("c#m"), k(PitchClass::Cs, Mode::Minor));
        assert_eq!(parse_key("Eb dorian"), k(PitchClass::Eb, Mode::Dorian));
        assert_eq!(parse_key("Bb minor"), k(PitchClass::Bb, Mode::Minor));
        assert_eq!(parse_key("bm"), k(PitchClass::B, Mode::Minor));
        assert_eq!(parse_key("D major"), k(PitchClass::D, Mode::Major));
        assert_eq!(parse_key("G harmonic minor"), k(PitchClass::G, Mode::HarmonicMinor));
    }

    #[test]
    fn rejects_nonsense() {
        assert_eq!(parse_key(""), None);
        assert_eq!(parse_key("H minor"), None);
        assert_eq!(parse_key("A sideways"), None);
        assert_eq!(parse_key("#"), None);
    }
}
