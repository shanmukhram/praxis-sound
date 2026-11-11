//! Offline edit rules: a deterministic reader for common edit phrasings, returning an
//! [`EditPlan`] as the `edit_score` tool input so the edit loop treats it like a model.
//!
//! It reads: bar phrases ("the last four bars", "bars 9-12", "second half", "from bar 9"),
//! keep clauses ("keep the first half"), intents (add / add a melody, remove / drop, softer /
//! louder, thin out / sparser, swap X for Y / change X to Y, tone: muffled / darker / brighter,
//! space: more room / drier / no reverb, and new notes: rewrite / vary / new motif / change the
//! melody), instruments and layer references (piano, kick, bells, bass, hats, melody, drums ...),
//! register and dynamics words for new material (subtle, soft, high, low), "a little" / "much"
//! amounts, grid words and a quoted spoken rhythm. An instruction with none of these intents is
//! `not_understood`: it never falls back to rewriting notes.

use super::plan::{
    ChangeAmount, DynamicsChange, EDIT_TOOL_NAME, EditIntent, EditParams, EditPlan, EditScope,
    SpaceChange, ToneChange,
};
use super::scope::{Phrase, find_phrase, words};
use crate::catalogue::Catalogue;
use crate::plan::{Dynamics, Register};
use crate::provider::{Provider, ProviderError, Role, ToolRequest};
use praxis_score::rhythm::looks_spoken;
use praxis_score::{GeneratorClass, Grid};

/// Instrument words, longest first, and the catalogue preset a new layer of that instrument
/// uses. The first word list entry is also the layer reference the plan names.
const INSTRUMENTS: &[(&[&str], &str)] = &[
    (&["felt", "piano"], "felt_piano"),
    (&["upright", "piano"], "upright_piano"),
    (&["electric", "piano"], "soft_keys"),
    (&["fm", "keys"], "fm_keys"),
    (&["soft", "keys"], "soft_keys"),
    (&["sub", "bass"], "sub_bass"),
    (&["soft", "kick"], "soft_kick"),
    (&["open", "hat"], "hat_open"),
    (&["hi", "-", "hat"], "hat_closed"),
    (&["hi", "-", "hats"], "hat_closed"),
    (&["hi", "hat"], "hat_closed"),
    (&["hi", "hats"], "hat_closed"),
    (&["piano"], "felt_piano"),
    (&["pianos"], "felt_piano"),
    (&["upright"], "upright_piano"),
    (&["bell"], "bell"),
    (&["bells"], "bell"),
    (&["glass"], "glass"),
    (&["mallet"], "mallet"),
    (&["mallets"], "mallet"),
    (&["marimba"], "mallet"),
    (&["vibraphone"], "mallet"),
    (&["keys"], "soft_keys"),
    (&["rhodes"], "soft_keys"),
    (&["pluck"], "pluck"),
    (&["plucks"], "pluck"),
    (&["arp"], "pluck"),
    (&["arpeggio"], "pluck"),
    (&["arpeggios"], "pluck"),
    (&["bass"], "warm_bass"),
    (&["bassline"], "warm_bass"),
    (&["kick"], "soft_kick"),
    (&["kicks"], "soft_kick"),
    (&["snare"], "snare"),
    (&["snares"], "snare"),
    (&["rim"], "rim"),
    (&["rimshot"], "rim"),
    (&["hat"], "hat_closed"),
    (&["hats"], "hat_closed"),
    (&["hihat"], "hat_closed"),
    (&["hihats"], "hat_closed"),
    (&["shaker"], "shaker"),
    (&["clap"], "clap"),
    (&["claps"], "clap"),
    (&["tom"], "tom_mid"),
    (&["toms"], "tom_mid"),
    (&["kit"], "kit"),
    (&["drone"], "drone"),
    (&["noise"], "noise_bed"),
    (&["tick"], "tick"),
    (&["ticks"], "tick"),
    (&["glitch"], "glitch"),
];

/// Role words that name layers by class.
const ROLES: &[(&str, &str)] = &[
    ("melody", "melody"),
    ("lead", "melody"),
    ("tune", "melody"),
    ("drums", "drums"),
    ("beat", "drums"),
    ("percussion", "percussion"),
    ("chords", "chords"),
    ("harmony", "harmony"),
    ("arps", "arp"),
];

/// A mention of an instrument or role in the instruction.
#[derive(Clone, Debug, PartialEq)]
struct Mention {
    /// Word index where it starts.
    at: usize,
    /// Layer reference text ("piano", "kick", "melody").
    reference: String,
    /// Preset a new layer of this instrument would use.
    preset: Option<&'static str>,
}

fn mentions(w: &[String]) -> Vec<Mention> {
    let mut out = Vec::new();
    let mut i = 0;
    'outer: while i < w.len() {
        for (seq, preset) in INSTRUMENTS {
            if w.len() >= i + seq.len() && seq.iter().zip(&w[i..]).all(|(a, b)| a == b) {
                let reference = match *preset {
                    // A named felt or upright piano refers to that preset; "piano" to any.
                    "felt_piano" | "upright_piano"
                        if seq.len() == 1 && seq[0].starts_with("piano") =>
                    {
                        "piano".to_string()
                    }
                    p if seq.len() > 1 => p.to_string(),
                    _ => seq[0].trim_end_matches('s').to_string(),
                };
                let reference = if reference.is_empty() || reference == "ba" {
                    seq[0].to_string()
                } else {
                    reference
                };
                out.push(Mention {
                    at: i,
                    reference,
                    preset: Some(preset),
                });
                i += seq.len();
                continue 'outer;
            }
        }
        if let Some((_, role)) = ROLES.iter().find(|(word, _)| *word == w[i]) {
            out.push(Mention {
                at: i,
                reference: role.to_string(),
                preset: None,
            });
        }
        i += 1;
    }
    out
}

fn has(w: &[String], any: &[&str]) -> bool {
    w.iter().any(|x| any.contains(&x.as_str()))
}

fn has_seq(w: &[String], seq: &[&str]) -> bool {
    w.windows(seq.len())
        .any(|win| win.iter().zip(seq).all(|(a, b)| a == b))
}

/// Splits the instruction into clauses at punctuation and joining words.
fn clauses(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if matches!(c, ',' | ';' | '.' | ':' | '\n') {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    let mut split = Vec::new();
    for c in out {
        let lower = c.to_lowercase();
        let mut rest = lower.as_str();
        // "keep X and/but/then change Y" -> two clauses.
        while let Some(pos) = [" but ", " then ", " and then ", " while "]
            .iter()
            .filter_map(|j| rest.find(j).map(|p| (p, j.len())))
            .min()
        {
            split.push(rest[..pos.0].to_string());
            rest = &rest[pos.0 + pos.1..];
        }
        split.push(rest.to_string());
    }
    split.into_iter().filter(|c| !c.trim().is_empty()).collect()
}

fn is_keep_clause(w: &[String]) -> bool {
    has(w, &["keep", "preserve", "leave"])
        || has_seq(w, &["don't", "touch"])
        || has_seq(w, &["do", "not", "touch"])
}

/// A quoted spoken rhythm in the instruction.
fn quoted_spoken(text: &str) -> Option<String> {
    let norm: String = text
        .chars()
        .map(|c| match c {
            '\u{201c}' | '\u{201d}' => '"',
            c => c,
        })
        .collect();
    let mut parts = norm.split('"');
    parts.next();
    while let Some(inside) = parts.next() {
        if looks_spoken(inside, 1) {
            return Some(inside.trim().to_string());
        }
        parts.next();
    }
    None
}

fn grid_of(w: &[String]) -> Option<Grid> {
    if has(w, &["triplet", "triplets"]) {
        return Some(if has(w, &["sixteenth", "sixteenths", "16th", "16ths"]) {
            Grid::SixteenthTriplet
        } else {
            Grid::EighthTriplet
        });
    }
    if has(w, &["sixteenth", "sixteenths", "16th", "16ths"]) {
        Some(Grid::Sixteenth)
    } else if has(w, &["eighth", "eighths", "8th", "8ths"]) {
        Some(Grid::Eighth)
    } else if has(w, &["quarter", "quarters"]) {
        Some(Grid::Quarter)
    } else {
        None
    }
}

const SOFTER: [&str; 12] = [
    "softer", "quieter", "gentler", "lower", "subtler", "calmer", "softly", "quietly", "down",
    "back", "less", "tamer",
];
const LOUDER: [&str; 8] = [
    "louder", "stronger", "harder", "punchier", "up", "bigger", "boost", "more",
];

/// Tone qualities: darker ones, brighter ones. "less"/"not as" before one flips it.
const DARKER: &[&str] = &[
    "muffled", "muffle", "muffles", "muffling", "muted", "darker", "dark", "darken", "warmer",
    "warm", "warmth", "duller", "mellow", "mellower", "rounder", "smoother",
];
const BRIGHTER: &[&str] = &[
    "brighter", "bright", "brighten", "clearer", "crisper", "crisp", "airier", "airy", "sparkly",
    "shinier", "sharper",
];
/// Complaints that ask for the opposite quality, with or without "less" / "too".
const TOO_BRIGHT: &[&str] = &["harsh", "harsher", "brittle", "shrill", "piercing", "tinny"];
const TOO_DARK: &[&str] = &["muddy", "murky", "boomy"];
/// Words that name the high end ("cut the highs" is darker, "more treble" brighter).
const HIGHS: &[&str] = &["highs", "treble", "air", "sparkle"];
/// Reverb nouns.
const REVERB: &[&str] = &[
    "reverb",
    "reverberation",
    "verb",
    "room",
    "space",
    "ambience",
    "ambiance",
    "echo",
];
const WETTER: &[&str] = &[
    "wetter",
    "wet",
    "roomier",
    "spacious",
    "spacier",
    "bigger",
    "huge",
    "vast",
    "distant",
    "further",
    "farther",
    "washy",
    "lush",
    "hall",
    "cathedral",
];
const DRIER: &[&str] = &["drier", "dryer", "closer", "intimate", "upfront"];
const LESS: &[&str] = &[
    "less", "reduce", "lower", "fewer", "smaller", "down", "tame",
];
const MORE: &[&str] = &[
    "more", "add", "increase", "boost", "up", "extra", "lots", "bigger",
];
const NO: &[&str] = &[
    "no", "without", "remove", "drop", "cut", "kill", "lose", "off",
];
const A_LITTLE: &[&str] = &[
    "little", "slightly", "bit", "touch", "tad", "subtly", "somewhat", "hint", "gently",
];
const MUCH: &[&str] = &[
    "much",
    "lot",
    "lots",
    "way",
    "very",
    "really",
    "far",
    "heavily",
    "completely",
    "totally",
    "hall",
    "cathedral",
];
/// Things a request for new notes names.
const NOTE_WORDS: &[&str] = &[
    "melody",
    "melodies",
    "motif",
    "motifs",
    "rhythm",
    "rhythms",
    "notes",
    "note",
    "pattern",
    "patterns",
    "tune",
    "riff",
    "line",
    "part",
    "phrase",
    "phrases",
    "groove",
    "chords",
    "harmony",
    "progression",
];

/// True when word `i` follows "less", "not as", "not so" or "no longer" within two words.
fn negated(w: &[String], i: usize) -> bool {
    let lo = i.saturating_sub(2);
    w[lo..i]
        .iter()
        .any(|x| matches!(x.as_str(), "less" | "not" | "no" | "never"))
}

/// Size of a tone or space change.
fn amount_of(w: &[String]) -> Option<ChangeAmount> {
    if has(w, MUCH) {
        Some(ChangeAmount::Much)
    } else if has(w, A_LITTLE) {
        Some(ChangeAmount::ALittle)
    } else {
        None
    }
}

/// The tone change an instruction asks for, if any.
fn tone_of(w: &[String]) -> Option<ToneChange> {
    // "tone down" is a dynamics phrase.
    let tone_word = w.iter().enumerate().any(|(i, x)| {
        matches!(x.as_str(), "tone" | "timbre") && w.get(i + 1).is_none_or(|n| n != "down")
    });
    for (i, x) in w.iter().enumerate() {
        let x = x.as_str();
        if DARKER.contains(&x) {
            return Some(if negated(w, i) {
                ToneChange::Brighter
            } else {
                ToneChange::Darker
            });
        }
        if BRIGHTER.contains(&x) {
            return Some(if negated(w, i) {
                ToneChange::Darker
            } else {
                ToneChange::Brighter
            });
        }
        if TOO_BRIGHT.contains(&x) {
            return Some(ToneChange::Darker);
        }
        if TOO_DARK.contains(&x) {
            return Some(ToneChange::Brighter);
        }
        if HIGHS.contains(&x)
            || (x == "end" && i > 0 && matches!(w[i - 1].as_str(), "high" | "top"))
        {
            let before = &w[..i];
            if before
                .iter()
                .any(|b| LESS.contains(&b.as_str()) || NO.contains(&b.as_str()) || b == "roll")
            {
                return Some(ToneChange::Darker);
            }
            if before.iter().any(|b| MORE.contains(&b.as_str())) {
                return Some(ToneChange::Brighter);
            }
        }
        // "a softer tone", "a gentler timbre".
        if tone_word && matches!(x, "softer" | "soft" | "gentler" | "gentle" | "rounder") {
            return Some(ToneChange::Darker);
        }
        if tone_word && matches!(x, "harder" | "edgier" | "brighter") {
            return Some(ToneChange::Brighter);
        }
    }
    None
}

/// The space change an instruction asks for, if any.
fn space_of(w: &[String]) -> Option<SpaceChange> {
    // "too much reverb", "the room is too big": less of it.
    let too_much = w.windows(2).any(|p| {
        p[0] == "too" && (p[1] == "much" || p[1] == "big" || WETTER.contains(&p[1].as_str()))
    });
    let any = |list: &[&str]| w.iter().any(|x| list.contains(&x.as_str()));
    if let Some(i) = w.iter().position(|x| REVERB.contains(&x.as_str())) {
        let near = &w[i.saturating_sub(3)..i];
        if any(NO) || (has(w, &["dry"]) && !near.iter().any(|b| b == "more")) {
            return Some(SpaceChange::Dry);
        }
        if too_much || any(LESS) || any(DRIER) {
            return Some(SpaceChange::Drier);
        }
        // "more room", "give it some space", "add reverb", "turn the reverb up": more.
        return Some(SpaceChange::Wetter);
    }
    if has(w, &["dry"]) {
        return Some(SpaceChange::Dry);
    }
    if too_much || any(DRIER) || has_seq(w, &["up", "front"]) {
        return Some(SpaceChange::Drier);
    }
    if any(WETTER) || has_seq(w, &["far", "away"]) {
        return Some(SpaceChange::Wetter);
    }
    None
}

/// Reads an edit instruction. Never fails: an instruction with no recognisable intent becomes
/// a `not_understood` plan (no change), never a rewrite of notes.
pub fn parse_edit(instruction: &str) -> EditPlan {
    let mut keep: Option<Phrase> = None;
    let mut scope: Option<Phrase> = None;
    let mut change_words: Vec<String> = Vec::new();
    // Layers a keep clause names without bars ("keep the piano but make it muffled"): a tone or
    // space edit, which keeps the notes, is about them.
    let mut kept_layers: Vec<String> = Vec::new();
    for clause in clauses(instruction) {
        let w = words(&clause);
        let found = find_phrase(&w, 0).map(|(p, _, _)| p);
        if is_keep_clause(&w) && !has(&w, &["add", "change", "make", "remove", "drop"]) {
            if found.is_none() {
                kept_layers.extend(mentions(&w).into_iter().map(|m| m.reference));
            }
            if keep.is_none() {
                keep = found;
            }
            continue;
        }
        if scope.is_none() {
            scope = found;
        }
        change_words.extend(w);
    }
    let w = change_words;
    let ms = mentions(&w);

    let melody_word = has(&w, &["melody", "tune", "lead", "topline", "countermelody"])
        || has_seq(&w, &["top", "line"]);
    let adds =
        has(&w, &["add", "adding", "introduce", "plus", "layer"]) || has_seq(&w, &["bring", "in"]);
    let removes = has(
        &w,
        &["remove", "drop", "delete", "without", "lose", "cut", "mute"],
    ) || has_seq(&w, &["take", "out"])
        || has_seq(&w, &["get", "rid"]);
    let swaps = has(&w, &["swap", "switch", "replace", "instead"])
        || ((has(&w, &["change", "turn", "make"]) || has_seq(&w, &["play", "the"]))
            && has(&w, &["to", "into", "on", "with", "for"])
            && ms.iter().filter(|m| m.preset.is_some()).count() >= 2);
    let thins = has(
        &w,
        &[
            "thin", "thinner", "sparser", "sparse", "simpler", "simplify", "fewer", "busy", "strip",
        ],
    );
    let softer = has(&w, &SOFTER) || has_seq(&w, &["tone", "down"]);
    let louder = has(&w, &LOUDER) || has_seq(&w, &["bring", "up"]);
    // New notes only when asked for: a rewrite verb, "change/redo the melody", a new rhythm.
    let rewrites = has(
        &w,
        &[
            "rewrite",
            "rewrites",
            "vary",
            "variation",
            "variations",
            "different",
            "new",
            "rework",
            "reimagine",
            "recompose",
            "reharmonise",
            "reharmonize",
        ],
    ) || (has(&w, &["change", "alter", "redo", "rewrite", "mix", "switch"])
        && has(&w, NOTE_WORDS))
        || quoted_spoken(instruction).is_some()
        || (grid_of(&w).is_some() && !ms.is_empty());
    let tone = tone_of(&w);
    let space = space_of(&w);
    // "add a bell with lots of reverb" adds a layer; "add reverb" / "add some warmth" do not.
    let adds_instrument = adds && (ms.iter().any(|m| m.preset.is_some()) || melody_word);

    let mut plan = EditPlan {
        intent: EditIntent::RewriteMotif,
        scope: EditScope {
            bars: None,
            phrase: scope.map(Phrase::describe),
            keep: keep.map(Phrase::describe),
        },
        layers: vec![],
        params: EditParams::default(),
        notes: vec![],
    };
    let named: Vec<String> = ms.iter().map(|m| m.reference.clone()).collect();
    let sound_targets = if named.is_empty() {
        kept_layers
    } else {
        named.clone()
    };

    if let Some(space) = space.filter(|_| !adds_instrument) {
        plan.intent = EditIntent::ChangeSpace;
        plan.layers = sound_targets;
        plan.params.space = Some(space);
        plan.params.amount = amount_of(&w);
    } else if let Some(tone) = tone.filter(|_| !adds_instrument && !(swaps && ms.len() >= 2)) {
        plan.intent = EditIntent::ChangeTone;
        plan.layers = sound_targets;
        plan.params.tone = Some(tone);
        plan.params.amount = amount_of(&w);
    } else if adds && (melody_word || ms.is_empty()) && !removes {
        plan.intent = EditIntent::AddMelody;
        // An instrument named for the melody ("add a bell melody").
        if let Some(m) = ms.iter().find(|m| m.preset.is_some()) {
            plan.params.preset = m.preset.map(str::to_string);
        }
        new_material_params(&w, instruction, &mut plan.params, true);
    } else if adds && !removes {
        plan.intent = EditIntent::AddLayer;
        match ms.iter().rev().find(|m| m.preset.is_some()) {
            Some(m) => plan.params.preset = m.preset.map(str::to_string),
            None => {
                if let Some(m) = ms.last() {
                    plan.params.class = role_class(&m.reference);
                }
            }
        }
        new_material_params(&w, instruction, &mut plan.params, false);
    } else if removes {
        plan.intent = EditIntent::RemoveLayer;
        plan.layers = named;
    } else if swaps && ms.len() >= 2 {
        plan.intent = EditIntent::ChangeInstrument;
        // "use bells instead of the piano": the layer follows "instead of".
        let instead = w.iter().position(|x| x == "instead");
        let (target, new) = match instead {
            Some(p) => {
                let after = ms.iter().find(|m| m.at > p);
                let before = ms.iter().rev().find(|m| m.at < p);
                (after.or(ms.first()), before.or(ms.last()))
            }
            None => (ms.first(), ms.last()),
        };
        if let Some(t) = target {
            plan.layers = vec![t.reference.clone()];
        }
        plan.params.preset = new.and_then(|m| m.preset).map(str::to_string);
    } else if thins {
        plan.intent = EditIntent::ThinOut;
        plan.layers = named;
    } else if softer || louder {
        plan.intent = EditIntent::ChangeDynamics;
        plan.layers = named;
        plan.params.change = Some(if louder && !softer {
            DynamicsChange::Louder
        } else {
            DynamicsChange::Softer
        });
        if has(&w, &["much", "lot", "way"]) {
            plan.params.amount_db = Some(if louder && !softer { 6.0 } else { 8.0 });
        } else if has(&w, &["slightly", "little", "bit", "touch", "tad"]) {
            plan.params.amount_db = Some(2.0);
        }
    } else if rewrites {
        plan.intent = EditIntent::RewriteMotif;
        plan.layers = named;
        plan.params.spoken = quoted_spoken(instruction);
        plan.params.grid = grid_of(&w);
    } else {
        plan.intent = EditIntent::NotUnderstood;
        plan.layers = named;
        plan.notes.push(format!(
            "\"{}\" names no change I recognise",
            instruction.trim()
        ));
    }
    plan
}

fn role_class(reference: &str) -> Option<GeneratorClass> {
    serde_json::from_value(serde_json::Value::String(reference.to_string())).ok()
}

/// Register, dynamics, gain, grid and rhythm words for a new layer.
fn new_material_params(w: &[String], instruction: &str, p: &mut EditParams, melody: bool) {
    let subtle = has(
        w,
        &[
            "subtle",
            "soft",
            "gentle",
            "quiet",
            "delicate",
            "faint",
            "light",
            "understated",
        ],
    );
    let strong = has(w, &["strong", "loud", "bold", "punchy", "driving"]);
    p.dynamics = if subtle {
        Some(Dynamics::Soft)
    } else if strong {
        Some(Dynamics::Strong)
    } else {
        None
    };
    if subtle {
        p.gain_db = Some(if melody { -6.0 } else { -8.0 });
    }
    p.register = if has(w, &["high", "above", "top", "sparkling", "bright"]) {
        Some(Register::High)
    } else if has(
        w,
        &["low", "under", "underneath", "beneath", "below", "deep"],
    ) {
        Some(Register::Low)
    } else if melody {
        Some(Register::High)
    } else {
        None
    };
    p.grid = grid_of(w);
    p.spoken = quoted_spoken(instruction);
}

/// The offline edit provider: answers an `edit_score` tool request by reading the instruction
/// (the first user message) with [`parse_edit`]. Its id is `offline-rules`, the same as
/// [`crate::RuleProvider`], whose edit requests the edit loop routes here.
#[derive(Clone, Debug, Default)]
pub struct EditRules;

impl EditRules {
    pub fn new() -> EditRules {
        EditRules
    }

    /// Unused catalogue parameter kept for symmetry with `RuleProvider::with_catalogue`.
    pub fn with_catalogue(_catalogue: &Catalogue) -> EditRules {
        EditRules
    }
}

impl Provider for EditRules {
    fn id(&self) -> String {
        super::RULE_PROVIDER_ID.to_string()
    }

    fn call_tool(&self, req: &ToolRequest) -> Result<serde_json::Value, ProviderError> {
        if req.tool_name != EDIT_TOOL_NAME {
            return Err(ProviderError::Unsupported(format!(
                "the offline edit rules answer only `{}`",
                EDIT_TOOL_NAME
            )));
        }
        let instruction = req
            .messages
            .iter()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.as_str())
            .ok_or_else(|| ProviderError::Unsupported("the request has no instruction".into()))?;
        if instruction.trim().is_empty() {
            return Err(ProviderError::Unsupported(
                "the instruction is empty".into(),
            ));
        }
        serde_json::to_value(parse_edit(instruction))
            .map_err(|e| ProviderError::Request(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_acceptance_edit() {
        let p = parse_edit("Keep the first half, change the last four bars to add a subtle melody");
        assert_eq!(p.intent, EditIntent::AddMelody);
        assert_eq!(p.scope.phrase.as_deref(), Some("the last 4 bars"));
        assert_eq!(p.scope.keep.as_deref(), Some("the first half"));
        assert_eq!(p.params.dynamics, Some(Dynamics::Soft));
        assert_eq!(p.params.register, Some(Register::High));
    }

    #[test]
    fn reads_common_phrasings() {
        let p = parse_edit("make the kick softer");
        assert_eq!(p.intent, EditIntent::ChangeDynamics);
        assert_eq!(p.layers, ["kick"]);
        assert_eq!(p.params.change, Some(DynamicsChange::Softer));
        assert_eq!(p.scope.phrase, None);

        let p = parse_edit("remove the kick");
        assert_eq!(p.intent, EditIntent::RemoveLayer);
        assert_eq!(p.layers, ["kick"]);

        let p = parse_edit("bars 9-12: thin out the piano");
        assert_eq!(p.intent, EditIntent::ThinOut);
        assert_eq!(p.scope.phrase.as_deref(), Some("bars 9-12"));
        assert_eq!(p.layers, ["piano"]);

        let p = parse_edit("swap the piano for bells in the second half");
        assert_eq!(p.intent, EditIntent::ChangeInstrument);
        assert_eq!(p.layers, ["piano"]);
        assert_eq!(p.params.preset.as_deref(), Some("bell"));
        assert_eq!(p.scope.phrase.as_deref(), Some("the second half"));

        let p = parse_edit("use bells instead of the piano");
        assert_eq!(p.intent, EditIntent::ChangeInstrument);
        assert_eq!(p.layers, ["piano"]);
        assert_eq!(p.params.preset.as_deref(), Some("bell"));

        let p = parse_edit("keep the first half, add a shaker");
        assert_eq!(p.intent, EditIntent::AddLayer);
        assert_eq!(p.params.preset.as_deref(), Some("shaker"));
        assert_eq!(p.scope.phrase, None);
        assert_eq!(p.scope.keep.as_deref(), Some("the first half"));

        let p = parse_edit("give the piano a new motif in bars 13-16");
        assert_eq!(p.intent, EditIntent::RewriteMotif);
        assert_eq!(p.layers, ["piano"]);

        let p = parse_edit("make the last four bars much louder");
        assert_eq!(p.intent, EditIntent::ChangeDynamics);
        assert!(p.layers.is_empty());
        assert_eq!(p.params.change, Some(DynamicsChange::Louder));
        assert_eq!(p.params.amount_db, Some(6.0));
    }

    fn tone(
        text: &str,
    ) -> (
        EditIntent,
        Option<ToneChange>,
        Option<ChangeAmount>,
        Vec<String>,
    ) {
        let p = parse_edit(text);
        (p.intent, p.params.tone, p.params.amount, p.layers)
    }

    fn space(text: &str) -> (EditIntent, Option<SpaceChange>, Option<ChangeAmount>) {
        let p = parse_edit(text);
        (p.intent, p.params.space, p.params.amount)
    }

    #[test]
    fn reads_tone_requests() {
        use ChangeAmount::*;
        use EditIntent::ChangeTone as T;
        use ToneChange::*;
        let piano = vec!["piano".to_string()];
        assert_eq!(
            tone("make the piano muffled"),
            (T, Some(Darker), None, piano.clone())
        );
        // The owner's question: the kept piano is the layer whose sound changes.
        assert_eq!(
            tone("can you keep the piano but make it sound muffled?"),
            (T, Some(Darker), None, piano.clone())
        );
        assert_eq!(
            tone("make it a little darker"),
            (T, Some(Darker), Some(ALittle), vec![])
        );
        assert_eq!(
            tone("Make it a little brighter"),
            (T, Some(Brighter), Some(ALittle), vec![])
        );
        assert_eq!(tone("much warmer piano").1, Some(Darker));
        assert_eq!(tone("much warmer piano").2, Some(Much));
        for t in [
            "muted piano",
            "duller",
            "mellow it out",
            "give the piano a softer tone",
        ] {
            assert_eq!(tone(t).0, T, "{t}");
            assert_eq!(tone(t).1, Some(Darker), "{t}");
        }
        for t in [
            "clearer please",
            "crisper hats",
            "more air on the bells",
            "less muffled",
        ] {
            assert_eq!(tone(t).0, T, "{t}");
            assert_eq!(tone(t).1, Some(Brighter), "{t}");
        }
        assert_eq!(tone("it's too harsh").1, Some(Darker));
        assert_eq!(tone("less bright").1, Some(Darker));
        let p = parse_edit("muffle the piano in the last four bars");
        assert_eq!(p.intent, T);
        assert_eq!(p.scope.phrase.as_deref(), Some("the last 4 bars"));
        // "tone down" stays a dynamics phrase; "make the kick softer" too.
        assert_eq!(
            parse_edit("tone down the kick").intent,
            EditIntent::ChangeDynamics
        );
        assert_eq!(
            parse_edit("make the kick softer").intent,
            EditIntent::ChangeDynamics
        );
        // Adding a darker melody is still an add.
        assert_eq!(
            parse_edit("add a dark bell melody").intent,
            EditIntent::AddMelody
        );
    }

    #[test]
    fn reads_space_requests() {
        use ChangeAmount::*;
        use EditIntent::ChangeSpace as S;
        use SpaceChange::*;
        assert_eq!(space("Give it more room"), (S, Some(Wetter), None));
        assert_eq!(space("Make it drier"), (S, Some(Drier), None));
        assert_eq!(space("less reverb on the piano"), (S, Some(Drier), None));
        assert_eq!(
            space("put the piano in a hall"),
            (S, Some(Wetter), Some(Much))
        );
        assert_eq!(
            space("make it a bit wetter"),
            (S, Some(Wetter), Some(ALittle))
        );
        assert_eq!(space("more space"), (S, Some(Wetter), None));
        assert_eq!(space("make it bigger"), (S, Some(Wetter), None));
        assert_eq!(space("bring the piano closer"), (S, Some(Drier), None));
        assert_eq!(space("remove the reverb"), (S, Some(Dry), None));
        assert_eq!(space("make it completely dry"), (S, Some(Dry), Some(Much)));
        assert_eq!(space("no reverb on the kick").1, Some(Dry));
        assert_eq!(space("add some reverb").1, Some(Wetter));
        assert_eq!(space("turn the reverb down").1, Some(Drier));
        assert_eq!(space("turn the reverb up").1, Some(Wetter));
        assert_eq!(space("there is too much reverb").1, Some(Drier));
        assert_eq!(space("take the reverb off the piano").1, Some(Dry));
        // An instrument with reverb is still a new layer.
        assert_eq!(
            parse_edit("add a bell with lots of reverb").intent,
            EditIntent::AddLayer
        );
    }

    #[test]
    fn new_notes_only_when_asked_and_never_as_a_fallback() {
        for t in [
            "change the melody",
            "a different rhythm for the piano",
            "new motif in bars 9-12",
            "vary it",
            "play the hats in sixteenths",
        ] {
            assert_eq!(parse_edit(t).intent, EditIntent::RewriteMotif, "{t}");
        }
        for t in [
            "make it pop",
            "piano",
            "do something cool",
            "change the piano",
        ] {
            let p = parse_edit(t);
            assert_eq!(p.intent, EditIntent::NotUnderstood, "{t}");
            assert!(p.notes[0].contains(t), "{:?}", p.notes);
        }
    }

    /// The Feel sliders' phrases (app/src/ui/views/overview.ts `MACROS`).
    #[test]
    fn reads_every_feel_slider_phrase() {
        use EditIntent::*;
        let cases: [(&str, EditIntent); 8] = [
            ("Make the mood calmer", ChangeDynamics),
            ("Make the mood more tense", ChangeDynamics),
            ("Make it a little darker", ChangeTone),
            ("Make it a little brighter", ChangeTone),
            ("Make it drier", ChangeSpace),
            ("Give it more room", ChangeSpace),
            ("Calmer, less energy", ChangeDynamics),
            ("More energy", ChangeDynamics),
        ];
        for (text, intent) in cases {
            let p = parse_edit(text);
            assert_eq!(p.intent, intent, "{text}");
            assert!(p.layers.is_empty(), "{text}: {:?}", p.layers);
            assert_eq!(p.scope, EditScope::default(), "{text}");
        }
        assert_eq!(
            parse_edit("Make the mood calmer").params.change,
            Some(DynamicsChange::Softer)
        );
        assert_eq!(
            parse_edit("Make the mood more tense").params.change,
            Some(DynamicsChange::Louder)
        );
        assert_eq!(
            parse_edit("Calmer, less energy").params.change,
            Some(DynamicsChange::Softer)
        );
        assert_eq!(
            parse_edit("More energy").params.change,
            Some(DynamicsChange::Louder)
        );
    }
}
