//! Offline edit rules: a deterministic reader for common edit phrasings, returning an
//! [`EditPlan`] as the `edit_score` tool input so the edit loop treats it like a model.
//!
//! It reads: bar phrases ("the last four bars", "bars 9-12", "second half", "from bar 9"),
//! keep clauses ("keep the first half"), intents (add / add a melody, remove / drop, softer /
//! louder, thin out / sparser, swap X for Y / change X to Y, rewrite / vary / new motif),
//! instruments and layer references (piano, kick, bells, bass, hats, melody, drums ...),
//! register and dynamics words for new material (subtle, soft, high, low), grid words and a
//! quoted spoken rhythm.

use super::plan::{DynamicsChange, EDIT_TOOL_NAME, EditIntent, EditParams, EditPlan, EditScope};
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

/// Reads an edit instruction. Never fails: an instruction with no recognisable intent becomes
/// a `rewrite_motif` of whatever it names, and the planner's resolution explains what is
/// missing.
pub fn parse_edit(instruction: &str) -> EditPlan {
    let mut keep: Option<Phrase> = None;
    let mut scope: Option<Phrase> = None;
    let mut change_words: Vec<String> = Vec::new();
    for clause in clauses(instruction) {
        let w = words(&clause);
        let found = find_phrase(&w, 0).map(|(p, _, _)| p);
        if is_keep_clause(&w) && !has(&w, &["add", "change", "make", "remove", "drop"]) {
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
    let rewrites = has(
        &w,
        &[
            "rewrite",
            "vary",
            "variation",
            "different",
            "new",
            "change",
            "rework",
            "reimagine",
        ],
    );

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

    if adds && (melody_word || ms.is_empty()) && !removes {
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
    } else {
        plan.intent = EditIntent::RewriteMotif;
        plan.layers = named;
        if !rewrites {
            plan.notes
                .push("no edit verb recognised; rewriting the motif of what was named".into());
        }
        plan.params.spoken = quoted_spoken(instruction);
        plan.params.grid = grid_of(&w);
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
}
