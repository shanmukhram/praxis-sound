//! `RuleProvider`: a deterministic brief parser (no network). It returns a [`ScorePlan`] as the
//! tool input, so the repair loop treats it exactly like a model.
//!
//! It reads: bar counts ("16 bars", "a 16-bar loop"), time signatures ("4/4", "waltz"), keys
//! ("A minor", "F# dorian", "in a minor key"), tempo ("128 BPM"), target durations ("30
//! seconds", "2 minutes", "1:30"), loop vs one-shot, one emotion word, instruments (felt piano,
//! kick, bells, bass, ...), "no X / without X" bans, quoted or inline spoken rhythms ("DON din
//! DON don TIN"), grid words (eighth, sixteenth, triplet), explicit chord lists ("chords Am F C
//! G"), and layering phrases: "bars 9-16: ... plus a soft muted kick", "add bells in the second
//! half", "from bar 9", "the last four bars", "bars 13-16: drop the bells".
//!
//! Layering rule: once a layer enters it keeps playing to the end unless a later phrase removes
//! it ("drop", "remove", "without", "only in bars ..."). Mood words never add instruments.

use crate::catalogue::Catalogue;
use crate::motifs::{
    CHORD_RELATIVE_MOTIF, default_degrees, default_spoken, follows_chords, motif_templates,
    rhythm_role,
};
use crate::plan::{
    Dynamics, Motif, PlanChord, PlanLayer, PlanPlayback, PlanSection, PresetChoice, Register,
    ScorePlan, TRAILER_CLASSES, diatonic_symbol, parse_chord,
};
use crate::provider::{Provider, ProviderError, Role, ToolRequest};
use praxis_score::rhythm::{SpokenStep, looks_spoken, parse_spoken};
use praxis_score::timeline::duration;
use praxis_score::{
    BarRange, GeneratorClass, Grid, Key, Mode, PitchClass, SectionRole, TimeSignature,
};
use std::collections::BTreeSet;

/// Deterministic offline provider. Used in CI and when no API key is configured.
#[derive(Clone, Debug, Default)]
pub struct RuleProvider {
    catalogue: Catalogue,
}

impl RuleProvider {
    pub fn new() -> RuleProvider {
        RuleProvider::default()
    }

    pub fn with_catalogue(catalogue: Catalogue) -> RuleProvider {
        RuleProvider { catalogue }
    }
}

impl Provider for RuleProvider {
    fn id(&self) -> String {
        "offline-rules".to_string()
    }

    fn call_tool(&self, req: &ToolRequest) -> Result<serde_json::Value, ProviderError> {
        // Edit requests (`edit_score`) go to the offline edit reader, so a RuleProvider answers
        // both tools wherever it is used (not only inside `plan_edit`).
        if req.tool_name == crate::edit::EDIT_TOOL_NAME {
            return crate::edit::EditRules::with_catalogue(&self.catalogue).call_tool(req);
        }
        let brief = req
            .messages
            .iter()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.as_str())
            .ok_or_else(|| ProviderError::Unsupported("the request has no brief".into()))?;
        if brief.trim().is_empty() {
            return Err(ProviderError::Unsupported("the brief is empty".into()));
        }
        let plan = parse_brief(brief, &self.catalogue, req.seed);
        serde_json::to_value(plan).map_err(|e| ProviderError::Request(e.to_string()))
    }
}

// ---------------------------------------------------------------------------------------------
// Lexing

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Word,
    Num(f64),
    Sym(char),
}

#[derive(Clone, Debug)]
struct Tok {
    text: String,
    kind: Kind,
    /// Byte offset in the brief.
    start: usize,
}

impl Tok {
    fn is_word(&self, w: &str) -> bool {
        self.kind == Kind::Word && self.text == w
    }
    fn word(&self) -> Option<&str> {
        (self.kind == Kind::Word).then_some(self.text.as_str())
    }
    fn is_sym(&self, c: char) -> bool {
        self.kind == Kind::Sym(c)
    }
}

/// Byte-length-preserving punctuation normalisation (curly quotes, dashes).
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let rep = match c {
            '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{00ab}' | '\u{00bb}' => Some('"'),
            '\u{2018}' | '\u{2019}' => Some('\''),
            '\u{2013}' | '\u{2014}' | '\u{2212}' => Some('-'),
            _ => None,
        };
        match rep {
            Some(r) => {
                out.push(r);
                for _ in 1..c.len_utf8() {
                    out.push(' ');
                }
            }
            None => out.push(c),
        }
    }
    out
}

fn lex(s: &str) -> Vec<Tok> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];
        if c == '\n' {
            out.push(Tok {
                text: "\n".into(),
                kind: Kind::Sym('\n'),
                start: pos,
            });
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() {
            let mut j = i;
            let mut text = String::new();
            while j < chars.len() && chars[j].1.is_ascii_digit() {
                text.push(chars[j].1);
                j += 1;
            }
            if j + 1 < chars.len() && chars[j].1 == '.' && chars[j + 1].1.is_ascii_digit() {
                text.push('.');
                j += 1;
                while j < chars.len() && chars[j].1.is_ascii_digit() {
                    text.push(chars[j].1);
                    j += 1;
                }
            }
            let mut letters = String::new();
            let mut k = j;
            while k < chars.len() && chars[k].1.is_alphabetic() {
                letters.push(chars[k].1);
                k += 1;
            }
            if ["th", "st", "nd", "rd"].contains(&letters.as_str()) {
                out.push(Tok {
                    text: format!("{text}{letters}"),
                    kind: Kind::Word,
                    start: pos,
                });
                i = k;
            } else {
                out.push(Tok {
                    kind: Kind::Num(text.parse().unwrap_or(0.0)),
                    text,
                    start: pos,
                });
                i = j;
            }
        } else if c.is_alphabetic() {
            let mut j = i;
            let mut text = String::new();
            while j < chars.len()
                && (chars[j].1.is_alphabetic()
                    || chars[j].1 == '\''
                    || ((chars[j].1 == '#' || chars[j].1 == '\u{266f}') && j > i))
            {
                text.push(chars[j].1);
                j += 1;
            }
            out.push(Tok {
                text,
                kind: Kind::Word,
                start: pos,
            });
            i = j;
        } else {
            out.push(Tok {
                text: c.to_string(),
                kind: Kind::Sym(c),
                start: pos,
            });
            i += 1;
        }
    }
    out
}

fn number_word(w: &str) -> Option<f64> {
    const UNITS: [&str; 21] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
    ];
    if let Some(i) = UNITS.iter().position(|u| *u == w) {
        return Some(i as f64);
    }
    Some(match w {
        "thirty" => 30.0,
        "forty" => 40.0,
        "fifty" => 50.0,
        "sixty" => 60.0,
        "seventy" => 70.0,
        "eighty" => 80.0,
        "ninety" => 90.0,
        _ => return None,
    })
}

/// A number at `i` (digits or words like "sixteen", "thirty-two"), and the index after it.
fn num_at(t: &[Tok], i: usize) -> Option<(f64, usize)> {
    let tok = t.get(i)?;
    match &tok.kind {
        Kind::Num(v) => Some((*v, i + 1)),
        Kind::Word => {
            let v = number_word(&tok.text)?;
            if v >= 20.0 && v % 10.0 == 0.0 {
                let mut j = i + 1;
                if t.get(j).is_some_and(|x| x.is_sym('-')) {
                    j += 1;
                }
                if let Some(u) = t.get(j).and_then(|x| x.word()).and_then(number_word)
                    && (1.0..10.0).contains(&u)
                {
                    return Some((v + u, j + 1));
                }
            }
            Some((v, i + 1))
        }
        Kind::Sym(_) => None,
    }
}

fn is_sentence_end(t: &Tok) -> bool {
    matches!(t.kind, Kind::Sym('.' | '!' | '?' | ';' | '\n'))
}

/// Matches a word phrase at `i` (a '-' may join the words). Returns the index after it.
fn phrase_at(t: &[Tok], i: usize, phrase: &[&str]) -> Option<usize> {
    let mut j = i;
    for (k, w) in phrase.iter().enumerate() {
        if k > 0 && t.get(j).is_some_and(|x| x.is_sym('-')) {
            j += 1;
        }
        if !t.get(j)?.is_word(w) {
            return None;
        }
        j += 1;
    }
    Some(j)
}

// ---------------------------------------------------------------------------------------------
// Lexicons

const SOFT_WORDS: [&str; 14] = [
    "soft", "muted", "gentle", "quiet", "felt", "subtle", "light", "delicate", "warm", "mellow",
    "dull", "padded", "hushed", "faint",
];

const STRONG_WORDS: [&str; 6] = ["loud", "strong", "hard", "punchy", "heavy", "driving"];

const LOW_WORDS: [&str; 6] = ["underneath", "under", "beneath", "below", "low", "deep"];
const HIGH_WORDS: [&str; 4] = ["high", "above", "sparkling", "top"];

/// Instrument phrases: (words, family, preset). Longest phrases first.
const INSTRUMENTS: &[(&[&str], &str, &str)] = &[
    (&["open", "hi", "hat"], "openhat", "hat_open"),
    (&["felt", "piano"], "piano", "felt_piano"),
    (&["upright", "piano"], "piano", "upright_piano"),
    (&["grand", "piano"], "piano", "upright_piano"),
    (&["electric", "piano"], "keys", "soft_keys"),
    (&["fm", "keys"], "fmkeys", "fm_keys"),
    (&["fm", "piano"], "fmkeys", "fm_keys"),
    (&["music", "box"], "bells", "bell"),
    (&["bass", "drum"], "kick", "*kick"),
    (&["kick", "drum"], "kick", "*kick"),
    (&["sub", "bass"], "bass", "sub_bass"),
    (&["open", "hat"], "openhat", "hat_open"),
    (&["open", "hihat"], "openhat", "hat_open"),
    (&["hi", "hat"], "hats", "hat_closed"),
    (&["hi", "hats"], "hats", "hat_closed"),
    (&["drum", "kit"], "drums", "kit"),
    (&["piano"], "piano", "*piano"),
    (&["pianos"], "piano", "*piano"),
    (&["rhodes"], "keys", "soft_keys"),
    (&["keys"], "keys", "soft_keys"),
    (&["kick"], "kick", "*kick"),
    (&["kicks"], "kick", "*kick"),
    (&["bassline"], "bass", "warm_bass"),
    (&["bass"], "bass", "warm_bass"),
    (&["bells"], "bells", "bell"),
    (&["bell"], "bells", "bell"),
    (&["chimes"], "bells", "bell"),
    (&["glockenspiel"], "bells", "bell"),
    (&["celesta"], "bells", "bell"),
    (&["mallets"], "mallet", "mallet"),
    (&["mallet"], "mallet", "mallet"),
    (&["marimba"], "mallet", "mallet"),
    (&["vibraphone"], "mallet", "mallet"),
    (&["vibes"], "mallet", "mallet"),
    (&["xylophone"], "mallet", "mallet"),
    (&["glass"], "glass", "glass"),
    (&["plucks"], "pluck", "pluck"),
    (&["pluck"], "pluck", "pluck"),
    (&["plucked"], "pluck", "pluck"),
    (&["harp"], "pluck", "pluck"),
    (&["kalimba"], "pluck", "pluck"),
    (&["guitar"], "pluck", "pluck"),
    (&["arpeggio"], "arp", "pluck"),
    (&["arpeggios"], "arp", "pluck"),
    (&["arpeggiated"], "arp", "pluck"),
    (&["arp"], "arp", "pluck"),
    (&["arps"], "arp", "pluck"),
    (&["hihat"], "hats", "hat_closed"),
    (&["hihats"], "hats", "hat_closed"),
    (&["hats"], "hats", "hat_closed"),
    (&["hat"], "hats", "hat_closed"),
    (&["snare"], "snare", "snare"),
    (&["snares"], "snare", "snare"),
    (&["rimshot"], "rim", "rim"),
    (&["rim"], "rim", "rim"),
    (&["sidestick"], "rim", "rim"),
    (&["shaker"], "shaker", "shaker"),
    (&["shakers"], "shaker", "shaker"),
    (&["clap"], "clap", "clap"),
    (&["claps"], "clap", "clap"),
    (&["handclaps"], "clap", "clap"),
    (&["toms"], "toms", "tom_mid"),
    (&["tom"], "toms", "tom_mid"),
    (&["drums"], "drums", "kit"),
    (&["pad"], "pad", "granular_texture"),
    (&["pads"], "pad", "granular_texture"),
    (&["texture"], "texture", "noise_bed"),
    (&["noise"], "texture", "noise_bed"),
    (&["drone"], "drone", "drone"),
    (&["drones"], "drone", "drone"),
    (&["riser"], "riser", "riser"),
    (&["risers"], "riser", "riser"),
    (&["impact"], "impact", "impact"),
    (&["impacts"], "impact", "impact"),
    (&["whoosh"], "whoosh", "whoosh"),
    (&["whooshes"], "whoosh", "whoosh"),
    (&["glitch"], "glitch", "glitch"),
    (&["glitches"], "glitch", "glitch"),
    (&["ticking"], "tick", "tick"),
    (&["ticks"], "tick", "tick"),
    (&["tick"], "tick", "tick"),
    (&["clock"], "tick", "tick"),
];

/// Instruments people ask for that the catalogue does not have yet.
const UNMAPPED: [&str; 18] = [
    "strings",
    "string",
    "violin",
    "violins",
    "viola",
    "cello",
    "cellos",
    "choir",
    "vocals",
    "vocal",
    "brass",
    "horns",
    "trumpet",
    "flute",
    "sax",
    "saxophone",
    "organ",
    "accordion",
];

fn class_words(w: &str) -> Option<Vec<GeneratorClass>> {
    use GeneratorClass as C;
    let v = match w {
        "riser" | "risers" => vec![C::Riser],
        "sweep" | "sweeps" => vec![C::Sweep],
        "impact" | "impacts" => vec![C::Impact],
        "boom" | "booms" => vec![C::Boom],
        "pad" | "pads" => vec![C::Pad],
        "whoosh" | "whooshes" => vec![C::Whoosh],
        "fill" | "fills" => vec![C::Fill],
        "drone" | "drones" => vec![C::Drone],
        "glitch" | "glitches" => vec![C::Glitch],
        "tick" | "ticks" => vec![C::Tick],
        "drums" => vec![C::Drums],
        "percussion" => vec![C::Percussion],
        "melody" | "melodies" | "lead" => vec![C::Melody],
        "chords" => vec![C::Chords],
        "harmony" => vec![C::Harmony],
        "arp" | "arps" | "arpeggio" | "arpeggios" => vec![C::Arp],
        "bass" => vec![C::Bass],
        "texture" | "textures" => vec![C::Texture],
        "ambience" => vec![C::Ambience],
        "sfx" | "effects" | "fx" => {
            let mut v = TRAILER_CLASSES.to_vec();
            v.retain(|c| *c != C::Pad);
            v.push(C::Sfx);
            v
        }
        "trailer" => TRAILER_CLASSES.to_vec(),
        _ => return None,
    };
    Some(v)
}

/// Words that may continue a "no ..." list.
fn negation_continues(w: &str) -> bool {
    const CONT: [&str; 34] = [
        "or",
        "and",
        "nor",
        "any",
        "new",
        "extra",
        "additional",
        "more",
        "big",
        "loud",
        "trailer",
        "cinematic",
        "sound",
        "style",
        "type",
        "kind",
        "of",
        "the",
        "a",
        "an",
        "other",
        "added",
        "heavy",
        "sudden",
        "dramatic",
        "huge",
        "epic",
        "obvious",
        "kick",
        "hi",
        "sub",
        "open",
        "drum",
        "music",
    ];
    CONT.contains(&w)
        || class_words(w).is_some()
        || INSTRUMENTS.iter().any(|(ws, _, _)| ws.contains(&w))
        || SOFT_WORDS.contains(&w)
        || UNMAPPED.contains(&w)
}

fn is_negation(w: &str) -> bool {
    matches!(
        w,
        "no" | "without" | "avoid" | "never" | "nothing" | "zero" | "minus" | "not"
    )
}

const EMOTIONS: [&str; 28] = [
    "calm",
    "melancholy",
    "mysterious",
    "hopeful",
    "tense",
    "warm",
    "dreamy",
    "playful",
    "dark",
    "uplifting",
    "nostalgic",
    "serene",
    "sad",
    "happy",
    "eerie",
    "romantic",
    "peaceful",
    "somber",
    "ominous",
    "tender",
    "wistful",
    "joyful",
    "triumphant",
    "anxious",
    "lonely",
    "reflective",
    "bittersweet",
    "intimate",
];

fn emotion_word(w: &str) -> Option<&'static str> {
    if let Some(e) = EMOTIONS.iter().find(|e| **e == w) {
        return Some(e);
    }
    Some(match w {
        "melancholic" | "melancholia" => "melancholy",
        "mystery" | "mystical" | "mysteriously" => "mysterious",
        "uplifted" => "uplifting",
        "nostalgia" => "nostalgic",
        "cheerful" | "upbeat" => "happy",
        "relaxed" | "relaxing" | "chill" | "calming" => "calm",
        "sombre" => "somber",
        "hope" => "hopeful",
        "tension" | "suspense" | "suspenseful" => "tense",
        "creepy" | "spooky" => "eerie",
        "sorrowful" | "sorrow" => "sad",
        "dreamlike" => "dreamy",
        _ => return None,
    })
}

fn minor_emotion(e: &str) -> bool {
    matches!(
        e,
        "melancholy"
            | "mysterious"
            | "tense"
            | "dark"
            | "sad"
            | "eerie"
            | "somber"
            | "ominous"
            | "wistful"
            | "anxious"
            | "lonely"
            | "bittersweet"
            | "nostalgic"
            | "reflective"
    )
}

fn emotion_tempo(e: &str) -> f64 {
    match e {
        "melancholy" | "sad" | "somber" | "lonely" | "wistful" => 76.0,
        "calm" | "serene" | "peaceful" | "dreamy" | "tender" | "intimate" | "reflective" => 84.0,
        "mysterious" | "eerie" | "dark" | "ominous" => 80.0,
        "tense" | "anxious" => 100.0,
        "playful" | "happy" | "joyful" | "uplifting" | "triumphant" => 110.0,
        _ => 90.0,
    }
}

fn mode_word(t: &[Tok], i: usize) -> Option<(Mode, usize)> {
    let w = t.get(i)?.word()?;
    let next = t.get(i + 1).and_then(|x| x.word());
    Some(match (w, next) {
        ("harmonic", Some("minor")) => (Mode::HarmonicMinor, i + 2),
        ("melodic", Some("minor")) => (Mode::MelodicMinor, i + 2),
        ("minor", Some("pentatonic")) => (Mode::MinorPentatonic, i + 2),
        ("major", Some("pentatonic")) => (Mode::MajorPentatonic, i + 2),
        ("major" | "ionian", _) => (Mode::Major, i + 1),
        ("minor" | "aeolian", _) => (Mode::Minor, i + 1),
        ("dorian", _) => (Mode::Dorian, i + 1),
        ("phrygian", _) => (Mode::Phrygian, i + 1),
        ("lydian", _) => (Mode::Lydian, i + 1),
        ("mixolydian", _) => (Mode::Mixolydian, i + 1),
        ("locrian", _) => (Mode::Locrian, i + 1),
        ("chromatic" | "atonal", _) => (Mode::Chromatic, i + 1),
        _ => return None,
    })
}

fn default_tonic(mode: Mode) -> PitchClass {
    match mode {
        Mode::Major | Mode::MajorPentatonic | Mode::Chromatic => PitchClass::C,
        Mode::Dorian => PitchClass::D,
        Mode::Phrygian => PitchClass::E,
        Mode::Lydian => PitchClass::F,
        Mode::Mixolydian => PitchClass::G,
        Mode::Locrian => PitchClass::B,
        _ => PitchClass::A,
    }
}

fn grid_word(t: &[Tok], i: usize) -> Option<Grid> {
    let w = t.get(i)?.word()?;
    // The next two words, skipping hyphens ("eighth-note triplets").
    let following: Vec<&str> = t[i + 1..]
        .iter()
        .take(4)
        .filter(|x| !x.is_sym('-'))
        .map_while(|x| x.word())
        .take(2)
        .collect();
    let next = following.first().copied();
    let next2 = following.get(1).copied();
    let triplet = |n: Option<&str>| matches!(n, Some("triplet" | "triplets"));
    Some(match w {
        "sixteenth" | "sixteenths" | "16th" | "16ths" | "semiquaver" | "semiquavers" => {
            if triplet(next) || triplet(next2) {
                Grid::SixteenthTriplet
            } else {
                Grid::Sixteenth
            }
        }
        "eighth" | "eighths" | "8th" | "8ths" | "quaver" | "quavers" => {
            if triplet(next) || triplet(next2) {
                Grid::EighthTriplet
            } else {
                Grid::Eighth
            }
        }
        "triplet" | "triplets" => {
            let prev_is_grid = i > 0
                && t[i - 1]
                    .word()
                    .is_some_and(|p| matches!(p, "sixteenth" | "16th" | "eighth" | "8th" | "note"));
            if prev_is_grid {
                return None;
            }
            Grid::EighthTriplet
        }
        "quarter" | "quarters" | "crotchet" | "crotchets" => Grid::Quarter,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// Analysis

/// A spoken rhythm found in the brief.
#[derive(Clone, Debug, PartialEq)]
struct SpokenFound {
    text: String,
    pos: usize,
}

/// Finds quoted and inline spoken rhythms; returns them and the text with them blanked out.
fn extract_spoken(norm: &str) -> (Vec<SpokenFound>, String) {
    let mut found = Vec::new();
    let mut blank: Vec<(usize, usize)> = Vec::new();
    // Quoted.
    for q in ['"', '\'', '`'] {
        let idx: Vec<usize> = norm.match_indices(q).map(|(i, _)| i).collect();
        let mut k = 0;
        while k + 1 < idx.len() {
            let (a, b) = (idx[k], idx[k + 1]);
            let inner = &norm[a + 1..b];
            if looks_spoken(inner, 2) {
                found.push(SpokenFound {
                    text: inner.trim().to_string(),
                    pos: a,
                });
                blank.push((a, b + 1));
                k += 2;
            } else if q == '"' {
                k += 2;
            } else {
                k += 1;
            }
        }
    }
    // Inline runs of spoken tokens (at least three hits).
    let words: Vec<(usize, &str)> = norm
        .split_whitespace()
        .map(|w| (w.as_ptr() as usize - norm.as_ptr() as usize, w))
        .collect();
    let is_tok = |w: &str| {
        let w = w.trim_end_matches([',', ';', ':']);
        !w.is_empty() && (w == "~" || parse_spoken(w).is_ok())
    };
    let mut i = 0;
    while i < words.len() {
        if !is_tok(words[i].1)
            || blank
                .iter()
                .any(|(a, b)| words[i].0 >= *a && words[i].0 < *b)
        {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < words.len() && is_tok(words[j].1) {
            j += 1;
        }
        let start = words[i].0;
        let end = words[j - 1].0 + words[j - 1].1.len();
        let text: String = words[i..j]
            .iter()
            .map(|(_, w)| w.trim_end_matches([',', ';', ':']))
            .collect::<Vec<_>>()
            .join(" ");
        let hits = parse_spoken(&text)
            .map(|s| {
                s.iter()
                    .filter(|x| matches!(x, SpokenStep::Hit { .. }))
                    .count()
            })
            .unwrap_or(0);
        if hits >= 3 {
            found.push(SpokenFound { text, pos: start });
            blank.push((start, end));
        }
        i = j;
    }
    found.sort_by_key(|f| f.pos);
    let mut bytes = norm.as_bytes().to_vec();
    for (a, b) in blank {
        for x in bytes.iter_mut().take(b).skip(a) {
            *x = b' ';
        }
    }
    // Blanking whole quoted spans of ASCII-only spoken tokens keeps UTF-8 valid; fall back to the
    // original if anything went wrong.
    let text = String::from_utf8(bytes).unwrap_or_else(|_| norm.to_string());
    (found, text)
}

/// Lower-cases ASCII and turns "don't" / "do not" into "no", keeping byte positions.
fn lower_and_negate(s: &str) -> String {
    let mut out = s.to_ascii_lowercase();
    for (pat, rep) in [
        ("don't", "no   "),
        ("do not", "no    "),
        ("doesn't", "no     "),
        ("shouldn't", "no       "),
    ] {
        out = out.replace(pat, rep);
    }
    out
}

/// Explicit chord lists after "chords" / "progression" (case preserved).
fn explicit_chords(norm: &str) -> Vec<String> {
    let lower = norm.to_ascii_lowercase();
    for kw in ["progression", "chords", "changes"] {
        if let Some(i) = lower.find(kw) {
            let rest = &norm[i + kw.len()..];
            let mut out = Vec::new();
            for item in rest.split(|c: char| c.is_whitespace() || matches!(c, ',' | '|' | ':')) {
                let item =
                    item.trim_matches(|c: char| c == '-' || c == '.' || c == '(' || c == ')');
                if item.is_empty() || item == "-" {
                    continue;
                }
                if matches!(
                    item.to_ascii_lowercase().as_str(),
                    "of" | "is" | "like" | "then"
                ) && out.is_empty()
                {
                    continue;
                }
                // A bare "A" only counts inside a list ("chords A D E"), not as an article.
                if parse_chord(item).is_some() && (item != "A" || !out.is_empty()) {
                    out.push(item.to_string());
                } else {
                    break;
                }
            }
            if !out.is_empty() {
                return out;
            }
        }
    }
    Vec::new()
}

#[derive(Clone, Debug)]
struct Mention {
    family: String,
    /// Catalogue preset, or "*piano" / "*kick" to resolve from modifiers.
    preset: String,
    tok: usize,
    pos: usize,
    sentence: usize,
    negated: bool,
    modifiers: Vec<String>,
    after: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Marker {
    start: usize,
    end: usize,
    range: (u32, u32),
    colon: bool,
}

struct Analysis {
    toks: Vec<Tok>,
    sentence_of: Vec<usize>,
    sentences: Vec<(usize, usize)>,
    negated: Vec<bool>,
    /// Negation scope contained new/extra/more before the word ("no new melody").
    restraint_only: Vec<bool>,
}

impl Analysis {
    fn new(work: &str) -> Analysis {
        let toks = lex(work);
        let mut sentences = Vec::new();
        let mut sentence_of = vec![0; toks.len()];
        let mut start = 0;
        for (i, t) in toks.iter().enumerate() {
            sentence_of[i] = sentences.len();
            if is_sentence_end(t) {
                sentences.push((start, i + 1));
                start = i + 1;
            }
        }
        if start < toks.len() {
            sentences.push((start, toks.len()));
        }
        let n = toks.len();
        let mut negated = vec![false; n];
        let mut restraint_only = vec![false; n];
        let mut i = 0;
        while i < n {
            if toks[i].word().is_some_and(is_negation) {
                let mut j = i + 1;
                let mut newish = false;
                while j < n {
                    let t = &toks[j];
                    let cont = match &t.kind {
                        Kind::Word => negation_continues(&t.text),
                        Kind::Sym(c) => matches!(c, ',' | '/' | '-' | '&'),
                        Kind::Num(_) => false,
                    };
                    if !cont || is_sentence_end(t) {
                        break;
                    }
                    if t.word()
                        .is_some_and(|w| matches!(w, "new" | "extra" | "additional" | "more"))
                    {
                        newish = true;
                    }
                    negated[j] = true;
                    restraint_only[j] = newish;
                    j += 1;
                }
                i = j.max(i + 1);
            } else {
                i += 1;
            }
        }
        Analysis {
            toks,
            sentence_of,
            sentences,
            negated,
            restraint_only,
        }
    }

    fn words_before(&self, i: usize, n: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut j = i;
        while j > 0 && out.len() < n {
            j -= 1;
            let t = &self.toks[j];
            match &t.kind {
                Kind::Word => out.push(t.text.clone()),
                Kind::Sym('-') => {}
                _ => break,
            }
        }
        out
    }

    fn words_after(&self, i: usize, n: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut j = i;
        while j < self.toks.len() && out.len() < n {
            let t = &self.toks[j];
            match &t.kind {
                Kind::Word => out.push(t.text.clone()),
                Kind::Sym('-') => {}
                _ => break,
            }
            j += 1;
        }
        out
    }

    fn mentions(&self, notes: &mut Vec<String>) -> Vec<Mention> {
        let t = &self.toks;
        let mut out = Vec::new();
        let mut i = 0;
        while i < t.len() {
            if t[i].kind != Kind::Word {
                i += 1;
                continue;
            }
            let mut matched = false;
            for (words, family, preset) in INSTRUMENTS {
                if let Some(end) = phrase_at(t, i, words) {
                    out.push(Mention {
                        family: family.to_string(),
                        preset: preset.to_string(),
                        tok: i,
                        pos: t[i].start,
                        sentence: self.sentence_of[i],
                        negated: self.negated[i],
                        modifiers: self.words_before(i, 3),
                        after: self.words_after(end, 3),
                    });
                    i = end;
                    matched = true;
                    break;
                }
            }
            if !matched {
                if UNMAPPED.contains(&t[i].text.as_str()) && !self.negated[i] {
                    let note = format!(
                        "`{}` is not in the instrument catalogue yet, so it was left out",
                        t[i].text
                    );
                    if !notes.contains(&note) {
                        notes.push(note);
                    }
                }
                i += 1;
            }
        }
        out
    }

    /// Bar-range markers in sentence `s`, given the total bar count.
    fn markers(&self, s: (usize, usize), total: u32) -> Vec<Marker> {
        let t = &self.toks;
        let mut out = Vec::new();
        let mut i = s.0;
        let colon_at = |j: usize| t.get(j).is_some_and(|x| x.is_sym(':'));
        while i < s.1 {
            let w = t[i].word().unwrap_or("");
            // bars N-M / bars N to M / bar N onwards / from bar N
            if matches!(w, "bar" | "bars" | "measure" | "measures")
                && let Some((a, j)) = num_at(t, i + 1)
            {
                let a = a as u32;
                let mut k = j;
                let mut range = None;
                if (t.get(k).is_some_and(|x| x.is_sym('-'))
                    || t.get(k)
                        .and_then(|x| x.word())
                        .is_some_and(|x| matches!(x, "to" | "through" | "thru" | "until" | "till")))
                    && let Some((b, k2)) = num_at(t, k + 1)
                {
                    range = Some((a, b as u32));
                    k = k2;
                }
                let prev = if i > s.0 { t[i - 1].word() } else { None };
                let prev2 = if i > s.0 + 1 { t[i - 2].word() } else { None };
                if range.is_none() {
                    let next = t.get(k).and_then(|x| x.word());
                    range = match (prev, next) {
                        (_, Some("onwards" | "onward" | "on")) => {
                            if next.is_some() {
                                k += 1;
                            }
                            Some((a, total))
                        }
                        (Some("after"), _) => Some((a + 1, total)),
                        (Some("from" | "at"), _) => Some((a, total)),
                        (Some("until" | "till"), _) => Some((1, a)),
                        (Some("before"), _) => Some((1, a.saturating_sub(1))),
                        (Some("to"), _) if prev2 == Some("up") => Some((1, a)),
                        (Some("in" | "on" | "for"), _) => Some((a, a)),
                        _ => None,
                    };
                }
                if let Some((a, b)) = range
                    && a >= 1
                    && b >= a
                {
                    let start = if matches!(prev, Some("from" | "after" | "at" | "until" | "in")) {
                        i - 1
                    } else {
                        i
                    };
                    out.push(Marker {
                        start,
                        end: k,
                        range: (a, b.min(total.max(a))),
                        colon: colon_at(k),
                    });
                    i = k;
                    continue;
                }
            }
            // second half / first half / last N bars / first N bars
            let half = total / 2;
            if let Some(end) = phrase_at(t, i, &["second", "half"])
                .or_else(|| phrase_at(t, i, &["last", "half"]))
                .or_else(|| phrase_at(t, i, &["back", "half"]))
                .or_else(|| phrase_at(t, i, &["halfway"]))
                .or_else(|| phrase_at(t, i, &["midway"]))
            {
                if half >= 1 {
                    out.push(Marker {
                        start: i,
                        end,
                        range: (half + 1, total),
                        colon: colon_at(end),
                    });
                }
                i = end;
                continue;
            }
            if let Some(end) = phrase_at(t, i, &["first", "half"]) {
                if half >= 1 {
                    out.push(Marker {
                        start: i,
                        end,
                        range: (1, half),
                        colon: colon_at(end),
                    });
                }
                i = end;
                continue;
            }
            if matches!(w, "last" | "final" | "closing" | "first" | "opening")
                && let Some((n, j)) = num_at(t, i + 1)
                && t.get(j)
                    .and_then(|x| x.word())
                    .is_some_and(|x| matches!(x, "bar" | "bars" | "measures"))
            {
                let n = (n as u32).clamp(1, total.max(1));
                let range = if matches!(w, "first" | "opening") {
                    (1, n)
                } else {
                    (total - n + 1, total)
                };
                out.push(Marker {
                    start: i,
                    end: j + 1,
                    range,
                    colon: colon_at(j + 1),
                });
                i = j + 1;
                continue;
            }
            i += 1;
        }
        out
    }
}

/// Classes the brief mentions: `requested` (named without a negation) and `banned`
/// ("no risers", "without pads"). Mood words never count as a request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClassMentions {
    pub requested: BTreeSet<GeneratorClass>,
    pub banned: BTreeSet<GeneratorClass>,
}

pub fn class_mentions(brief: &str) -> ClassMentions {
    let norm = normalize(brief);
    let (_, blanked) = extract_spoken(&norm);
    let a = Analysis::new(&lower_and_negate(&blanked));
    let mut m = ClassMentions::default();
    for (i, t) in a.toks.iter().enumerate() {
        let Some(w) = t.word() else { continue };
        let Some(classes) = class_words(w) else {
            continue;
        };
        // "hit points", "sound effects library" etc. are not requests for trailer effects:
        // only the bare class words count, and "effects"/"trailer" only when negated.
        let generic = matches!(w, "effects" | "fx" | "sfx" | "trailer");
        if a.negated[i] {
            if !a.restraint_only[i] || classes.iter().any(|c| TRAILER_CLASSES.contains(c)) {
                m.banned.extend(classes);
            }
        } else if !generic {
            m.requested.extend(classes);
        }
    }
    m
}

// ---------------------------------------------------------------------------------------------
// Brief -> plan

fn find_time_signature(t: &[Tok], notes: &mut Vec<String>) -> TimeSignature {
    for i in 0..t.len() {
        if let (Kind::Num(n), Some(s), Some(Kind::Num(d))) =
            (&t[i].kind, t.get(i + 1), t.get(i + 2).map(|x| &x.kind))
            && s.is_sym('/')
        {
            let (n, d) = (*n as u32, *d as u32);
            if (1..=32).contains(&n) && [2, 4, 8, 16].contains(&d) {
                return TimeSignature {
                    numerator: n as u8,
                    denominator: d as u8,
                };
            }
        }
        if t[i].is_word("waltz") {
            notes.push("\"waltz\" read as 3/4".into());
            return TimeSignature {
                numerator: 3,
                denominator: 4,
            };
        }
    }
    TimeSignature {
        numerator: 4,
        denominator: 4,
    }
}

/// (total bars, motif bars).
fn find_bars(t: &[Tok]) -> (Option<u32>, Option<u32>) {
    let mut total = None;
    let mut motif = None;
    for i in 0..t.len() {
        let Some((n, j)) = num_at(t, i) else { continue };
        if n < 1.0 || n.fract() != 0.0 {
            continue;
        }
        let mut k = j;
        let hyphen = t.get(k).is_some_and(|x| x.is_sym('-'));
        if hyphen {
            k += 1;
        }
        if !t
            .get(k)
            .and_then(|x| x.word())
            .is_some_and(|w| matches!(w, "bar" | "bars" | "measure" | "measures"))
        {
            continue;
        }
        let prev = if i > 0 { t[i - 1].word() } else { None };
        if matches!(
            prev,
            Some(
                "bar"
                    | "bars"
                    | "last"
                    | "final"
                    | "first"
                    | "opening"
                    | "closing"
                    | "next"
                    | "every"
                    | "each"
                    | "measure"
                    | "measures"
            )
        ) {
            continue;
        }
        // A range like "bars 9-16" ends in a number preceded by '-'.
        if i > 0 && t[i - 1].is_sym('-') {
            continue;
        }
        let next = t.get(k + 1).and_then(|x| x.word());
        let is_motif = matches!(
            next,
            Some(
                "motif"
                    | "phrase"
                    | "pattern"
                    | "figure"
                    | "riff"
                    | "ostinato"
                    | "cell"
                    | "cycle"
                    | "melody"
            )
        );
        if is_motif {
            motif.get_or_insert(n as u32);
        } else {
            total.get_or_insert(n as u32);
        }
    }
    (total, motif)
}

fn find_tempo(t: &[Tok]) -> Option<f64> {
    for i in 0..t.len() {
        if let Kind::Num(n) = t[i].kind
            && t.get(i + 1).is_some_and(|x| x.is_word("bpm"))
        {
            return Some(n);
        }
        if t[i].is_word("tempo")
            && let Some((n, _)) = num_at(t, i + 1).or_else(|| num_at(t, i + 2))
            && (20.0..=400.0).contains(&n)
        {
            return Some(n);
        }
    }
    None
}

fn find_duration(t: &[Tok]) -> Option<f64> {
    for i in 0..t.len() {
        // m:ss
        if let (Kind::Num(m), Some(c), Some(Kind::Num(s))) =
            (&t[i].kind, t.get(i + 1), t.get(i + 2).map(|x| &x.kind))
            && c.is_sym(':')
            && *s < 60.0
            && t[i + 2].text.len() == 2
        {
            return Some(m * 60.0 + s);
        }
        let (n, j) = match num_at(t, i) {
            Some(x) => x,
            None => {
                // "a minute", "half a minute"
                if t[i].is_word("half") && phrase_at(t, i + 1, &["a", "minute"]).is_some() {
                    return Some(30.0);
                }
                if (t[i].is_word("a") || t[i].is_word("one"))
                    && t.get(i + 1).is_some_and(|x| x.is_word("minute"))
                    && !(i > 0 && t[i - 1].is_word("half"))
                {
                    return Some(60.0);
                }
                continue;
            }
        };
        let mut k = j;
        if t.get(k).is_some_and(|x| x.is_sym('-')) {
            k += 1;
        }
        match t.get(k).and_then(|x| x.word()) {
            Some("seconds" | "second" | "secs" | "sec" | "s") => return Some(n),
            Some("minutes" | "minute" | "mins" | "min") => return Some(n * 60.0),
            _ => {}
        }
    }
    None
}

fn find_key(t: &[Tok], emotion: &str, notes: &mut Vec<String>) -> Key {
    for i in 0..t.len() {
        let Some(w) = t[i].word() else { continue };
        let first = w.chars().next().unwrap_or(' ');
        if !('a'..='g').contains(&first) {
            continue;
        }
        let mut name = w.to_string();
        let mut j = i + 1;
        if name.chars().count() == 1 {
            match t.get(j).map(|x| &x.kind) {
                Some(Kind::Sym('#' | '\u{266f}')) => {
                    name.push('#');
                    j += 1;
                }
                Some(Kind::Word) if t[j].text == "sharp" => {
                    name.push('#');
                    j += 1;
                }
                Some(Kind::Word) if t[j].text == "flat" => {
                    name.push('b');
                    j += 1;
                }
                _ => {}
            }
        }
        let valid_name = name.len() == 1
            || (name.len() == 2 && (name.ends_with('#') || name.ends_with('b')))
            || name.ends_with('\u{266f}');
        if !valid_name {
            continue;
        }
        let Some(tonic) = PitchClass::parse(&name) else {
            continue;
        };
        if let Some((mode, _)) = mode_word(t, j) {
            return Key { tonic, mode };
        }
    }
    // Mode without a tonic: "in a minor key" is handled above; "minor key", "in minor".
    for i in 0..t.len() {
        if let Some((mode, _)) = mode_word(t, i) {
            let tonic = default_tonic(mode);
            notes.push(format!(
                "no tonic given; used {} {}",
                tonic.name(),
                format!("{mode:?}").to_lowercase()
            ));
            return Key { tonic, mode };
        }
    }
    let key = if minor_emotion(emotion) {
        Key {
            tonic: PitchClass::A,
            mode: Mode::Minor,
        }
    } else {
        Key {
            tonic: PitchClass::C,
            mode: Mode::Major,
        }
    };
    notes.push(format!(
        "no key given; chose {} for a {emotion} mood",
        key.display()
    ));
    key
}

fn find_playback(t: &[Tok], neg: &[bool]) -> PlanPlayback {
    let mut looped = false;
    let mut one_shot = false;
    for (i, tok) in t.iter().enumerate() {
        let Some(w) = tok.word() else { continue };
        if matches!(
            w,
            "loop" | "loops" | "looping" | "loopable" | "looped" | "seamless" | "seamlessly"
        ) && !neg[i]
        {
            looped = true;
        }
        if matches!(w, "oneshot" | "sting" | "stinger")
            || phrase_at(t, i, &["one", "shot"]).is_some()
        {
            one_shot = true;
        }
    }
    // An explicit one-shot wins ("a one-shot sting, not a loop"); one-shot is also the default.
    if looped && !one_shot {
        PlanPlayback::Loop
    } else {
        PlanPlayback::OneShot
    }
}

fn find_emotion(t: &[Tok], neg: &[bool]) -> Option<&'static str> {
    t.iter()
        .enumerate()
        .filter(|(i, _)| !neg[*i])
        .find_map(|(_, x)| x.word().and_then(emotion_word))
}

fn find_intensity(t: &[Tok]) -> f32 {
    let soft = t.iter().any(|x| {
        x.word().is_some_and(|w| {
            SOFT_WORDS.contains(&w) || matches!(w, "sparse" | "minimal" | "restrained")
        })
    });
    let strong = t.iter().any(|x| {
        x.word().is_some_and(|w| {
            STRONG_WORDS.contains(&w) || matches!(w, "intense" | "energetic" | "big" | "powerful")
        })
    });
    match (soft, strong) {
        (true, false) => 0.3,
        (false, true) => 0.7,
        _ => 0.5,
    }
}

fn find_tempo_word(t: &[Tok]) -> Option<f64> {
    t.iter().find_map(|x| match x.word()? {
        "slow" | "slowly" => Some(72.0),
        "fast" | "upbeat" | "energetic" => Some(120.0),
        "moderate" | "medium" | "midtempo" => Some(100.0),
        _ => None,
    })
}

#[derive(Clone, Debug)]
struct Family {
    family: String,
    presets: Vec<String>,
    mentions: Vec<usize>,
    first_pos: usize,
    global: bool,
    entries: Vec<(u32, u32, bool)>,
    removals: Vec<(u32, u32)>,
    excluded: bool,
}

fn resolve_preset(f: &Family, mentions: &[Mention]) -> String {
    // A specific preset anywhere in the family wins; otherwise resolve the generic.
    if let Some(p) = f.presets.iter().find(|p| !p.starts_with('*')) {
        return p.clone();
    }
    let soft = f.mentions.iter().any(|&m| {
        mentions[m]
            .modifiers
            .iter()
            .any(|w| SOFT_WORDS.contains(&w.as_str()))
    });
    match f.presets.first().map(String::as_str) {
        Some("*kick") => {
            if soft {
                "soft_kick".into()
            } else {
                "kick".into()
            }
        }
        _ => "felt_piano".into(),
    }
}

/// Parses a brief into a plan. Pure function of (brief, catalogue, seed).
pub fn parse_brief(brief: &str, catalogue: &Catalogue, seed: u64) -> ScorePlan {
    let mut notes: Vec<String> = Vec::new();
    let norm = normalize(brief);
    let (spoken, blanked) = extract_spoken(&norm);
    let chords_explicit = explicit_chords(&blanked);
    let work = lower_and_negate(&blanked);
    let a = Analysis::new(&work);
    let t = &a.toks;

    let ts = find_time_signature(t, &mut notes);
    let (bars_total, motif_bars) = find_bars(t);
    let tempo = find_tempo(t);
    let duration_s = find_duration(t);
    let emotion = match find_emotion(t, &a.negated) {
        Some(e) => e.to_string(),
        None => {
            notes.push("no emotion word found; used \"calm\"".into());
            "calm".to_string()
        }
    };
    let key = find_key(t, &emotion, &mut notes);
    let playback = find_playback(t, &a.negated);
    let intensity = find_intensity(t);
    let global_grid = (0..t.len()).find_map(|i| grid_word(t, i));

    // Timing (the parser needs the total to read "second half").
    let mut tempo_bpm = tempo;
    let mut bars = bars_total;
    if tempo_bpm.is_none() && !(bars.is_some() && duration_s.is_some()) {
        let bpm = find_tempo_word(t).unwrap_or_else(|| emotion_tempo(&emotion));
        notes.push(format!("no tempo given; used {bpm} BPM"));
        tempo_bpm = Some(bpm);
    }
    let total = match (bars, tempo_bpm, duration_s) {
        (Some(b), _, _) => b,
        (None, Some(bpm), Some(s)) => duration::solve_bars(bpm, ts, s).0,
        _ => {
            // Largest bar number named in a range, else 8.
            let mut max_bar = 0;
            for i in 0..t.len() {
                if t[i].word().is_some_and(|w| matches!(w, "bar" | "bars"))
                    && let Some((n, j)) = num_at(t, i + 1)
                {
                    max_bar = max_bar.max(n as u32);
                    if t.get(j).is_some_and(|x| x.is_sym('-'))
                        && let Some((m, _)) = num_at(t, j + 1)
                    {
                        max_bar = max_bar.max(m as u32);
                    }
                }
            }
            let b = if max_bar > 0 { max_bar } else { 8 };
            if max_bar == 0 {
                notes.push("no length given; used 8 bars".into());
            }
            bars = Some(b);
            b
        }
    }
    .clamp(1, crate::plan::MAX_BARS);
    if bars.is_none() && duration_s.is_none() {
        bars = Some(total);
    }

    // Mentions, scopes and layering.
    let mentions = a.mentions(&mut notes);
    let mention_toks: Vec<usize> = mentions.iter().map(|m| m.tok).collect();
    let mut assign: Vec<Option<(u32, u32)>> = vec![None; t.len()];
    let mut only_scope: Vec<bool> = vec![false; t.len()];
    let mut removal_scope: Vec<bool> = vec![false; t.len()];
    for &(s0, s1) in &a.sentences {
        let markers = a.markers((s0, s1), total);
        let mut split = s0;
        for (k, m) in markers.iter().enumerate() {
            let (from, to) = if m.colon {
                let next = markers.get(k + 1).map(|n| n.start).unwrap_or(s1);
                (m.start, next)
            } else {
                let mut b = split;
                for j in split..m.start {
                    let tk = &t[j];
                    let boundary = tk.is_sym(',')
                        || tk.word().is_some_and(|w| {
                            matches!(
                                w,
                                "with"
                                    | "plus"
                                    | "then"
                                    | "while"
                                    | "but"
                                    | "add"
                                    | "adding"
                                    | "adds"
                                    | "bring"
                                    | "brings"
                                    | "introduce"
                                    | "introduces"
                            )
                        });
                    if boundary {
                        b = j;
                    }
                }
                let from = b.max(split);
                // "bars 9-16 add a kick": nothing named before the marker, so it scopes forward.
                if mention_toks.iter().any(|&j| j >= from && j < m.start) {
                    (from, m.end)
                } else {
                    let next = markers.get(k + 1).map(|n| n.start).unwrap_or(s1);
                    (m.start, next)
                }
            };
            let scope_words: Vec<&str> = (from..to).filter_map(|j| t[j].word()).collect();
            let only = scope_words
                .iter()
                .any(|w| matches!(*w, "only" | "just" | "solely"));
            let removal = scope_words.iter().any(|w| {
                matches!(
                    *w,
                    "drop"
                        | "drops"
                        | "remove"
                        | "removes"
                        | "mute"
                        | "mutes"
                        | "stop"
                        | "stops"
                        | "cut"
                        | "cuts"
                        | "out"
                        | "silence"
                )
            });
            for j in from..to.min(t.len()) {
                assign[j] = Some(m.range);
                only_scope[j] = only;
                removal_scope[j] = removal;
            }
            split = to;
        }
    }

    let mut families: Vec<Family> = Vec::new();
    for (mi, m) in mentions.iter().enumerate() {
        let idx = match families.iter().position(|f| f.family == m.family) {
            Some(i) => i,
            None => {
                families.push(Family {
                    family: m.family.clone(),
                    presets: vec![],
                    mentions: vec![],
                    first_pos: m.pos,
                    global: false,
                    entries: vec![],
                    removals: vec![],
                    excluded: false,
                });
                families.len() - 1
            }
        };
        let f = &mut families[idx];
        f.mentions.push(mi);
        match assign[m.tok] {
            Some(range) => {
                if m.negated || removal_scope[m.tok] {
                    f.removals.push(range);
                } else {
                    f.presets.push(m.preset.clone());
                    f.entries.push((range.0, range.1, only_scope[m.tok]));
                }
            }
            None => {
                if m.negated {
                    if !a.restraint_only[m.tok] {
                        f.excluded = true;
                    }
                } else {
                    f.presets.push(m.preset.clone());
                    f.global = true;
                }
            }
        }
    }

    let classes = {
        let mut c = ClassMentions::default();
        for (i, tok) in t.iter().enumerate() {
            let Some(w) = tok.word() else { continue };
            let Some(cl) = class_words(w) else { continue };
            if a.negated[i] && assign[i].is_none() {
                if !a.restraint_only[i] || cl.iter().any(|x| TRAILER_CLASSES.contains(x)) {
                    c.banned.extend(cl);
                }
            } else if !a.negated[i] && !matches!(w, "effects" | "fx" | "sfx" | "trailer") {
                c.requested.extend(cl);
            }
        }
        c
    };

    // Active bars per family.
    let mut layer_specs: Vec<(String, String, BTreeSet<u32>, Vec<usize>)> = Vec::new();
    families.sort_by_key(|f| f.first_pos);
    for f in &families {
        if f.excluded || (!f.global && f.entries.is_empty()) {
            continue;
        }
        let preset_name = resolve_preset(f, &mentions);
        let Some(preset) = catalogue.get(&preset_name) else {
            continue;
        };
        if classes.banned.contains(&preset.class) {
            notes.push(format!(
                "left out `{}`: the brief bans {}",
                preset.name,
                crate::catalogue::class_name(preset.class)
            ));
            continue;
        }
        let mut active = BTreeSet::new();
        if f.global {
            active.extend(1..=total);
        } else {
            for &(s, e, only) in &f.entries {
                let end = if only { e } else { total };
                active.extend(s.max(1)..=end.min(total));
            }
        }
        for &(s, e) in &f.removals {
            for b in s..=e {
                active.remove(&b);
            }
        }
        if active.is_empty() {
            continue;
        }
        layer_specs.push((f.family.clone(), preset_name, active, f.mentions.clone()));
    }
    if layer_specs.is_empty() {
        notes.push("no instrument named; used one felt piano (restraint)".into());
        layer_specs.push((
            "piano".into(),
            "felt_piano".into(),
            (1..=total).collect(),
            vec![],
        ));
    }

    // Chord progression.
    let templates = motif_templates();
    let template = &templates[(seed % templates.len() as u64) as usize];
    let mut chords: Vec<PlanChord> = Vec::new();
    let symbols: Vec<String> = if !chords_explicit.is_empty() {
        chords_explicit.clone()
    } else {
        template
            .progression
            .iter()
            .map(|d| diatonic_symbol(&key, *d))
            .collect()
    };
    if symbols.len() == 1 {
        chords.push(PlanChord {
            bars: BarRange {
                start: 1,
                end: total,
            },
            symbol: symbols[0].clone(),
        });
    } else {
        for b in 1..=total {
            chords.push(PlanChord {
                bars: BarRange { start: b, end: b },
                symbol: symbols[((b - 1) as usize) % symbols.len()].clone(),
            });
        }
    }

    // Layers.
    let mut layers = Vec::new();
    let mut used_ids = BTreeSet::new();
    let mut spoken_used = vec![false; spoken.len()];
    for (family, preset_name, _, mention_ids) in &layer_specs {
        let Some(preset) = catalogue.get(preset_name) else {
            continue;
        };
        let class = preset.class;
        let role = rhythm_role(preset, class);
        let mods: Vec<String> = mention_ids
            .iter()
            .flat_map(|&m| mentions[m].modifiers.clone())
            .collect();
        let after: Vec<String> = mention_ids
            .iter()
            .flat_map(|&m| mentions[m].after.clone())
            .collect();
        let has = |set: &[&str], words: &[String]| words.iter().any(|w| set.contains(&w.as_str()));
        let register = if has(&LOW_WORDS, &mods) || has(&LOW_WORDS, &after) {
            Register::Low
        } else if has(&HIGH_WORDS, &mods) {
            Register::High
        } else {
            Register::Mid
        };
        let dynamics = if has(&SOFT_WORDS, &mods) {
            Dynamics::Soft
        } else if has(&STRONG_WORDS, &mods) {
            Dynamics::Strong
        } else if intensity < 0.4 {
            Dynamics::Soft
        } else {
            Dynamics::Normal
        };
        // Grid: a grid word right before the instrument, else the brief's grid.
        let attached = mention_ids.iter().find_map(|&m| {
            let tok = mentions[m].tok;
            (tok.saturating_sub(3)..tok).find_map(|j| grid_word(t, j))
        });
        let grid = attached.or(global_grid).unwrap_or(Grid::Eighth);
        let grid = if ts.steps_per_bar(grid).is_some() {
            grid
        } else {
            crate::motifs::fitting_grid(ts)
        };
        let spb = ts.steps_per_bar(grid).unwrap_or(8);

        // Spoken rhythm: the nearest quoted rhythm after this instrument's mention in the same
        // sentence, else the nearest before; the first pitched motif layer takes any leftover.
        let mut chosen: Option<usize> = None;
        for &m in mention_ids {
            let mm = &mentions[m];
            let sentence_range = a.sentences.get(mm.sentence).copied().unwrap_or((0, 0));
            let s_start = t.get(sentence_range.0).map(|x| x.start).unwrap_or(0);
            let s_end = t
                .get(sentence_range.1)
                .map(|x| x.start)
                .unwrap_or(usize::MAX);
            let candidates: Vec<usize> = (0..spoken.len())
                .filter(|&k| !spoken_used[k] && spoken[k].pos >= s_start && spoken[k].pos < s_end)
                .collect();
            if let Some(&k) = candidates
                .iter()
                .filter(|&&k| spoken[k].pos > mm.pos)
                .min_by_key(|&&k| spoken[k].pos - mm.pos)
                .or_else(|| {
                    candidates
                        .iter()
                        .filter(|&&k| spoken[k].pos < mm.pos)
                        .min_by_key(|&&k| mm.pos - spoken[k].pos)
                })
            {
                chosen = Some(k);
                break;
            }
        }
        if let Some(k) = chosen {
            spoken_used[k] = true;
        }
        let spoken_text = chosen.map(|k| spoken[k].text.clone());

        let motif_like = role == crate::motifs::RhythmRole::Motif;
        let (mut length_bars, degrees, follow) = if motif_like && chords_explicit.is_empty() {
            (template.length_bars, template.degrees.clone(), false)
        } else if motif_like {
            (1, CHORD_RELATIVE_MOTIF.to_vec(), true)
        } else {
            (1, default_degrees(role), follows_chords(role))
        };
        if let Some(mb) = motif_bars
            && motif_like
        {
            length_bars = mb.clamp(1, 8);
        }
        let spoken_text = spoken_text
            .unwrap_or_else(|| default_spoken(role, ts, grid).unwrap_or_else(|| "DON".into()));
        let slen = parse_spoken(&spoken_text)
            .map(|s| s.len() as u32)
            .unwrap_or(0);
        if slen > length_bars * spb {
            length_bars = slen.div_ceil(spb).min(8);
        }
        length_bars = length_bars.min(total).max(1);
        let degrees = if preset.pitched { degrees } else { vec![] };

        let mut id = family.clone();
        let mut n = 2;
        while used_ids.contains(&id) {
            id = format!("{family}-{n}");
            n += 1;
        }
        used_ids.insert(id.clone());
        layers.push(PlanLayer {
            id,
            name: crate::arrange::display_name(&preset.name),
            class,
            preset: PresetChoice {
                kind: preset.kind,
                name: preset.name.clone(),
            },
            register,
            dynamics,
            gain_db: None,
            motif: Motif {
                grid,
                length_bars,
                spoken: spoken_text,
                degrees,
                octave: None,
                sustain_steps: if preset.sustaining {
                    Some(crate::arrange::DEFAULT_SUSTAIN)
                } else {
                    None
                },
                follow_chords: follow,
                notes: vec![],
            },
        });
    }
    // Any unassigned spoken rhythm goes to the first pitched motif layer without its own rhythm.
    for (k, used) in spoken_used.iter().enumerate() {
        if *used {
            continue;
        }
        if let Some(l) = layers.iter_mut().find(|l| {
            catalogue.get(&l.preset.name).is_some_and(|p| {
                p.pitched && rhythm_role(p, p.class) == crate::motifs::RhythmRole::Motif
            })
        }) {
            let spb = ts.steps_per_bar(l.motif.grid).unwrap_or(8);
            l.motif.spoken = spoken[k].text.clone();
            let slen = parse_spoken(&l.motif.spoken)
                .map(|s| s.len() as u32)
                .unwrap_or(0);
            if slen > l.motif.length_bars * spb {
                l.motif.length_bars = slen.div_ceil(spb).clamp(1, 8).min(total);
            }
            break;
        }
    }

    // Sections from layer entry and exit points.
    let mut bounds: BTreeSet<u32> = BTreeSet::new();
    bounds.insert(1);
    bounds.insert(total + 1);
    for (_, _, active, _) in &layer_specs {
        for &b in active {
            if !active.contains(&(b.wrapping_sub(1))) || b == 1 {
                bounds.insert(b);
            }
            if !active.contains(&(b + 1)) {
                bounds.insert(b + 1);
            }
        }
    }
    let bounds: Vec<u32> = bounds.into_iter().filter(|b| *b <= total + 1).collect();
    let mut sections = Vec::new();
    let base = intensity;
    let n_secs = bounds.len().saturating_sub(1);
    let mut prev_count = 0usize;
    let max_count = layer_specs.len();
    for (si, w) in bounds.windows(2).enumerate() {
        let (s, e) = (w[0], w[1] - 1);
        let active_ids: Vec<String> = layer_specs
            .iter()
            .zip(&layers)
            .filter(|((_, _, act, _), _)| act.contains(&s))
            .map(|(_, l)| l.id.clone())
            .collect();
        let names: Vec<String> = layer_specs
            .iter()
            .zip(&layers)
            .filter(|((_, _, act, _), _)| act.contains(&s))
            .map(|(_, l)| l.name.to_lowercase())
            .collect();
        let name = match names.len() {
            0 => "Silence".to_string(),
            1 if n_secs > 1 => format!("{} alone", crate::arrange::display_name(&names[0])),
            1 => crate::arrange::display_name(&names[0]),
            _ => {
                let (last, rest) = names.split_last().unwrap_or((&names[0], &[]));
                crate::arrange::display_name(&format!("{} and {last}", rest.join(", ")))
            }
        };
        let count = active_ids.len();
        let role = if n_secs == 1 {
            SectionRole::Main
        } else if si == 0 && count < max_count {
            SectionRole::Intro
        } else if si + 1 == n_secs && si > 0 && count < prev_count {
            SectionRole::Outro
        } else {
            SectionRole::Main
        };
        let inten = (base + 0.1 * count.saturating_sub(1) as f32).clamp(0.0, 1.0);
        sections.push(PlanSection {
            name,
            bars: BarRange { start: s, end: e },
            role,
            intensity: (inten * 100.0).round() / 100.0,
            layers: active_ids,
        });
        prev_count = count;
    }

    let title = {
        let first = layers
            .first()
            .map(|l| l.name.to_lowercase())
            .unwrap_or_else(|| "piece".into());
        let kind = match playback {
            PlanPlayback::Loop => " loop",
            PlanPlayback::OneShot => "",
        };
        crate::arrange::display_name(&format!("{emotion} {first}{kind}"))
    };

    let mut avoid: Vec<GeneratorClass> = classes.banned.iter().copied().collect();
    avoid.sort();
    let requested: Vec<GeneratorClass> = classes
        .requested
        .iter()
        .copied()
        .filter(|c| TRAILER_CLASSES.contains(c))
        .collect();

    let mut plan_notes = notes;
    plan_notes.dedup();
    ScorePlan {
        title,
        emotion,
        key,
        time_signature: ts,
        tempo_bpm,
        duration_s,
        bars,
        playback,
        avoid,
        sections,
        layers,
        chords,
        notes: plan_notes,
        requested,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(brief: &str) -> ScorePlan {
        parse_brief(brief, &Catalogue::standard(), 0)
    }

    #[test]
    fn answers_the_edit_tool_too() {
        let score = crate::plan_brief(
            "16 bars, A minor, soft felt piano on an eighth grid, seamless loop",
            &RuleProvider::new(),
            &Catalogue::standard(),
            1,
        )
        .expect("plan")
        .score;
        let req = crate::edit::edit_request(
            "change the last four bars to add a subtle melody",
            &score,
            &Catalogue::standard(),
            1,
        );
        let v = RuleProvider::new().call_tool(&req).expect("edit plan");
        let plan: crate::edit::EditPlan = serde_json::from_value(v).expect("an EditPlan");
        assert_eq!(plan.intent, crate::edit::EditIntent::AddMelody);
    }

    #[test]
    fn lexing_numbers_and_words() {
        let t = lex("16-bar loop at 128bpm, 4/4, 8th notes, c# minor, 2.5 seconds");
        let texts: Vec<&str> = t.iter().map(|x| x.text.as_str()).collect();
        assert!(texts.contains(&"128"));
        assert!(texts.contains(&"bpm"));
        assert!(texts.contains(&"8th"));
        assert!(texts.contains(&"c#"));
        assert!(texts.contains(&"2.5"));
        assert_eq!(num_at(&lex("thirty-two bars"), 0).map(|x| x.0), Some(32.0));
    }

    #[test]
    fn acceptance_brief() {
        let p = plan(
            "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt piano \
             playing a repeating rhythmic motif on an eighth-note grid with accents and \
             overlapping sustain. Bars 9-16: the identical piano part plus a soft muted kick \
             underneath. No new melody, no fills, no risers, sweeps, impacts or pads.",
        );
        assert_eq!(p.bars, Some(16));
        assert_eq!(p.duration_s, Some(30.0));
        assert_eq!(p.tempo_bpm, None);
        assert_eq!(
            p.key,
            Key {
                tonic: PitchClass::A,
                mode: Mode::Minor
            }
        );
        assert_eq!(p.playback, PlanPlayback::Loop);
        let names: Vec<&str> = p.layers.iter().map(|l| l.preset.name.as_str()).collect();
        assert_eq!(names, vec!["felt_piano", "soft_kick"]);
        assert_eq!(p.sections.len(), 2);
        assert_eq!(p.sections[0].layers, vec!["piano"]);
        assert_eq!(p.sections[1].layers, vec!["piano", "kick"]);
        for c in [
            GeneratorClass::Fill,
            GeneratorClass::Riser,
            GeneratorClass::Sweep,
            GeneratorClass::Impact,
            GeneratorClass::Pad,
        ] {
            assert!(p.avoid.contains(&c), "{c:?}");
        }
        assert!(!p.avoid.contains(&GeneratorClass::Melody));
        assert_eq!(p.layers[0].motif.grid, Grid::Eighth);
    }

    #[test]
    fn mood_words_add_nothing() {
        let p = plan("cinematic mysterious piano loop");
        assert_eq!(p.layers.len(), 1);
        assert_eq!(p.emotion, "mysterious");
        assert!(p.requested.is_empty());
        let m = class_mentions("cinematic mysterious piano loop, no risers");
        assert!(m.banned.contains(&GeneratorClass::Riser));
        assert!(m.requested.is_empty());
    }

    #[test]
    fn spoken_and_layering() {
        let p = plan(
            "A calm bell motif \"DON din DON don TIN\" in D dorian at 90 BPM, 8 bars; add a \
             soft kick in the second half",
        );
        assert_eq!(p.layers[0].motif.spoken, "DON din DON don TIN");
        assert_eq!(p.key.mode, Mode::Dorian);
        assert_eq!(p.sections.len(), 2);
        assert_eq!(p.sections[1].bars, BarRange { start: 5, end: 8 });
        assert_eq!(p.layers[1].preset.name, "soft_kick");
    }

    #[test]
    fn durations_and_keys() {
        let t = lex("1:30 long");
        assert_eq!(find_duration(&t), Some(90.0));
        assert_eq!(find_duration(&lex("two minutes")), Some(120.0));
        assert_eq!(find_duration(&lex("half a minute")), Some(30.0));
        assert_eq!(find_duration(&lex("a 30-second cue")), Some(30.0));
        let mut n = vec![];
        assert_eq!(
            find_key(&lex("in f sharp minor"), "calm", &mut n),
            Key {
                tonic: PitchClass::Fs,
                mode: Mode::Minor
            }
        );
        assert_eq!(
            find_key(&lex("eb major"), "calm", &mut n).tonic,
            PitchClass::Eb
        );
        assert_eq!(
            explicit_chords("chords Am F C G, slow"),
            vec!["Am", "F", "C", "G"]
        );
    }

    #[test]
    fn is_deterministic() {
        let b = "warm bells and bass in C major, 12 bars at 100 bpm";
        assert_eq!(plan(b), plan(b));
    }
}
