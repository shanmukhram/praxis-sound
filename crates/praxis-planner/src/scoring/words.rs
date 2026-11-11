//! What the offline planner reads from a brief for scoring depth: hit timecodes ("hits at 0:42,
//! 1:05 and 1:38"), form words (intro, build, peak, release, verse, chorus, A/B/A) and whether
//! the brief asks for a scored cue at all.
//!
//! A brief is scored only on clear words: hit points, two or more different form-part words, an
//! A/B form, "cue", "evolving", "progression", "tension curve", "score for" or "underscore".
//! One part word alone ("an 8-bar build", "a trailer-style intro") is not a form.

use super::emotion::{EmotionPreset, PRESETS};
use crate::plan::{FormPart, HitPoint, Transition};
use praxis_score::SectionRole;

/// What a brief says about scoring.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CueWords {
    /// The brief asks for a scored cue.
    pub scored: bool,
    /// Form parts named in the brief, in order (empty: none named).
    pub form: Vec<FormPart>,
    /// The brief asks for borrowed chords (which Score v1 cannot keep in key).
    pub borrowed: bool,
    /// The first emotion preset word in the brief, for briefs whose mood word the rule planner
    /// does not list ("epic", "heroic").
    pub preset: Option<&'static EmotionPreset>,
}

fn is_sentence_end(bytes: &[u8], i: usize) -> bool {
    match bytes[i] {
        b'!' | b'?' | b';' | b'\n' => true,
        b'.' => {
            !(i > 0
                && bytes[i - 1].is_ascii_digit()
                && bytes.get(i + 1).is_some_and(|c| c.is_ascii_digit()))
        }
        _ => false,
    }
}

/// A time at byte `i` of `s`: "m:ss", "m:ss.s", "N s", "N sec", "N seconds". Returns
/// (seconds, end byte).
fn time_at(s: &str, i: usize) -> Option<(f64, usize)> {
    let b = s.as_bytes();
    if !b[i].is_ascii_digit() || (i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'.')) {
        return None;
    }
    let mut j = i;
    while j < b.len() && b[j].is_ascii_digit() {
        j += 1;
    }
    let whole: f64 = s[i..j].parse().ok()?;
    if j < b.len() && b[j] == b':' {
        let k = j + 1;
        let mut e = k;
        while e < b.len() && b[e].is_ascii_digit() {
            e += 1;
        }
        if e - k != 2 {
            return None;
        }
        let mut secs: f64 = s[k..e].parse().ok()?;
        if secs >= 60.0 {
            return None;
        }
        if e + 1 < b.len() && b[e] == b'.' && b[e + 1].is_ascii_digit() {
            let mut f = e + 1;
            while f < b.len() && b[f].is_ascii_digit() {
                f += 1;
            }
            secs += s[e..f].parse::<f64>().ok()?;
            e = f;
        }
        return Some((whole * 60.0 + secs, e));
    }
    // N(.n) s / sec / seconds
    let mut e = j;
    let mut value = whole;
    if e + 1 < b.len() && b[e] == b'.' && b[e + 1].is_ascii_digit() {
        let mut f = e + 1;
        while f < b.len() && b[f].is_ascii_digit() {
            f += 1;
        }
        value = s[i..f].parse().ok()?;
        e = f;
    }
    let mut k = e;
    while k < b.len() && b[k] == b' ' {
        k += 1;
    }
    let mut w = k;
    while w < b.len() && b[w].is_ascii_alphabetic() {
        w += 1;
    }
    let unit = s[k..w].to_ascii_lowercase();
    if matches!(unit.as_str(), "s" | "sec" | "secs" | "second" | "seconds") {
        return Some((value, w));
    }
    None
}

const HIT_WORDS: [&str; 6] = ["hit", "hits", "hitpoint", "hitpoints", "marker", "markers"];

/// Hit points named after a hit word, and the brief with their timecodes blanked (byte length
/// kept), so durations elsewhere in the brief read as before.
pub fn extract_hits(norm: &str) -> (Vec<HitPoint>, String) {
    let lower = norm.to_ascii_lowercase();
    let b = lower.as_bytes();
    let mut hits: Vec<HitPoint> = Vec::new();
    let mut blank: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_alphabetic() || (i > 0 && b[i - 1].is_ascii_alphabetic()) {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < b.len() && (b[j].is_ascii_alphabetic() || b[j] == b'-') {
            j += 1;
        }
        let word = lower[i..j].replace('-', "");
        if HIT_WORDS.contains(&word.as_str()) {
            // Times up to the end of the sentence.
            let mut k = j;
            while k < b.len() && !is_sentence_end(b, k) {
                if let Some((t, e)) = time_at(&lower, k) {
                    if !hits.iter().any(|h| (h.time_s - t).abs() < 1e-9) {
                        hits.push(HitPoint {
                            time_s: t,
                            label: String::new(),
                        });
                    }
                    blank.push((k, e));
                    k = e;
                } else {
                    k += 1;
                }
            }
            i = k.max(j);
        } else {
            i = j;
        }
    }
    let mut out: Vec<u8> = norm.as_bytes().to_vec();
    for (a, e) in blank {
        for c in &mut out[a..e] {
            *c = b' ';
        }
    }
    hits.sort_by(|a, b| a.time_s.total_cmp(&b.time_s));
    let text = String::from_utf8(out).unwrap_or_else(|_| norm.to_string());
    (hits, text)
}

fn role_of(word: &str) -> Option<(SectionRole, &'static str)> {
    use SectionRole::*;
    Some(match word {
        "intro" | "introduction" => (Intro, ""),
        "build" | "builds" | "building" | "buildup" => (Build, ""),
        "peak" | "peaks" | "climax" => (Peak, ""),
        "release" | "releases" | "resolve" | "resolves" | "resolution" => (Release, ""),
        "outro" | "ending" | "coda" => (Outro, ""),
        "verse" | "verses" => (Verse, ""),
        "chorus" | "choruses" => (Main, "Chorus"),
        "bridge" => (Breakdown, "Bridge"),
        "breakdown" => (Breakdown, ""),
        _ => return None,
    })
}

/// Usual weight of a part with this role.
fn role_weight(role: SectionRole) -> f32 {
    use SectionRole::*;
    match role {
        Intro | Release | Outro | Breakdown => 2.0,
        Transition => 1.0,
        _ => 3.0,
    }
}

fn part(role: SectionRole, name: &str, intensity: Option<f32>) -> FormPart {
    FormPart {
        role,
        name: name.to_string(),
        weight: role_weight(role),
        bars: None,
        intensity,
        layers: vec![],
        transition: Transition::Auto,
    }
}

const NEGATIONS: [&str; 4] = ["no", "without", "not", "never"];

/// Reads the scoring words of a brief (after hit timecodes are blanked).
pub fn read(text: &str) -> CueWords {
    let lower = text.to_ascii_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '/'))
        .filter(|w| !w.is_empty())
        .collect();
    let mut out = CueWords::default();
    let mut roles: Vec<(SectionRole, &'static str)> = Vec::new();
    let mut ab: Option<&str> = None;
    for (i, w) in words.iter().enumerate() {
        let negated = i > 0 && NEGATIONS.contains(&words[i - 1]);
        if let Some(r) = role_of(w)
            && !negated
            && roles.last() != Some(&r)
        {
            roles.push(r);
        }
        if matches!(
            *w,
            "a/b" | "aba" | "a/b/a" | "abab" | "a/b/a/b" | "aaba" | "a/a/b/a"
        ) {
            ab = ab.or(Some(*w));
        }
        if matches!(
            *w,
            "cue"
                | "cues"
                | "evolving"
                | "evolve"
                | "evolves"
                | "progression"
                | "progressions"
                | "underscore"
        ) {
            out.scored = true;
        }
        if matches!(*w, "borrowed" | "modal")
            && words
                .get(i + 1)
                .is_some_and(|n| n.starts_with("chord") || *n == "mixture")
        {
            out.borrowed = true;
        }
        if out.preset.is_none() && !negated {
            out.preset = PRESETS
                .iter()
                .find(|p| p.name == *w || p.synonyms.contains(w));
        }
    }
    if lower.contains("tension curve") || lower.contains("score for") {
        out.scored = true;
    }
    let distinct: std::collections::BTreeSet<String> =
        roles.iter().map(|(r, n)| format!("{r:?}{n}")).collect();
    if let Some(form) = ab {
        let letters: Vec<char> = form.chars().filter(|c| *c == 'a' || *c == 'b').collect();
        out.form = letters
            .iter()
            .map(|c| {
                if *c == 'a' {
                    part(SectionRole::Verse, "A", Some(0.45))
                } else {
                    part(SectionRole::Main, "B", Some(0.7))
                }
            })
            .collect();
        out.scored = true;
    } else if distinct.len() >= 2 {
        out.form = roles.iter().map(|(r, n)| part(*r, n, None)).collect();
        out.scored = true;
    }
    out
}

/// Roman numerals after "progression" ("the progression i VI III VII"), as written.
pub fn roman_progression(text: &str) -> Vec<String> {
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '-' | ':' | '.'))
        .filter(|w| !w.is_empty())
        .collect();
    for (i, w) in words.iter().enumerate() {
        if !w.to_ascii_lowercase().starts_with("progression") {
            continue;
        }
        let mut out = Vec::new();
        // Skip a few filler words ("of", "is", "like").
        let mut j = i + 1;
        while j < words.len() && j <= i + 3 && super::harmony::roman_degree(words[j]).is_none() {
            j += 1;
        }
        while j < words.len()
            && let Some(Ok(_)) = super::harmony::roman_degree(words[j])
        {
            out.push(words[j].to_string());
            j += 1;
        }
        if out.len() >= 2 {
            return out;
        }
    }
    vec![]
}

/// Form parts of an emotion preset's default shape.
pub fn preset_form(preset: &EmotionPreset) -> Vec<FormPart> {
    preset
        .shape
        .iter()
        .map(|(role, weight, intensity)| FormPart {
            role: *role,
            name: String::new(),
            weight: *weight,
            bars: None,
            intensity: Some(*intensity),
            layers: vec![],
            transition: Transition::Auto,
        })
        .collect()
}

/// Intensities for parts named in a brief: the preset's value for the same role, else the
/// role's usual level.
pub fn fill_intensities(form: &mut [FormPart], preset: &EmotionPreset) {
    for p in form.iter_mut() {
        if p.intensity.is_none() {
            p.intensity = Some(
                preset
                    .shape
                    .iter()
                    .find(|(r, _, _)| *r == p.role)
                    .map(|(_, _, x)| *x)
                    .unwrap_or_else(|| super::emotion::role_intensity(p.role)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_timecodes_are_read_and_blanked() {
        let s = "A 2-minute tense cue: intro, build, peak and release, with hits at 0:42, 1:05 \
                 and 1:38. Felt piano.";
        let (hits, blanked) = extract_hits(s);
        let t: Vec<f64> = hits.iter().map(|h| h.time_s).collect();
        assert_eq!(t, vec![42.0, 65.0, 98.0]);
        assert_eq!(blanked.len(), s.len());
        assert!(!blanked.contains("0:42") && blanked.contains("2-minute"));
        let (hits, _) = extract_hits("hit points at 30 s and 52.5 seconds; 90 seconds long");
        let t: Vec<f64> = hits.iter().map(|h| h.time_s).collect();
        assert_eq!(t, vec![30.0, 52.5]);
        assert!(extract_hits("a 1:30 piano piece").0.is_empty());
    }

    #[test]
    fn form_words_and_triggers() {
        let w = read("intro, build, peak and release");
        assert!(w.scored);
        let roles: Vec<SectionRole> = w.form.iter().map(|p| p.role).collect();
        assert_eq!(
            roles,
            [
                SectionRole::Intro,
                SectionRole::Build,
                SectionRole::Peak,
                SectionRole::Release
            ]
        );
        assert!(
            !read("Tense 8-bar build in D minor at 120 bpm with a felt piano and a riser").scored
        );
        assert!(
            !read("an epic cinematic trailer-style intro with a dark felt piano, 8 bars").scored
        );
        assert!(read("a calm 90 second cue on felt piano").scored);
        let ab = read("an A/B/A form for felt piano");
        assert_eq!(ab.form.len(), 3);
        assert_eq!(ab.form[1].name, "B");
        assert_eq!(
            read("an epic but restrained cue").preset.map(|p| p.name),
            Some("epic-but-restrained")
        );
        assert!(read("a cue with borrowed chords").borrowed);
        assert_eq!(
            roman_progression("with the chord progression I V vi IV on warm keys"),
            vec!["I", "V", "vi", "IV"]
        );
        assert!(roman_progression("a progression on piano").is_empty());
    }
}
