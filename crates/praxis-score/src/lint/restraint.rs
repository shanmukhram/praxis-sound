//! Restraint lints (owner rule 4): few elements, one emotion, no unrequested trailer effects.

use super::{LintContext, warn};
use crate::types::{GeneratorClass, Layer, Score};
use crate::validate::{Issue, IssueCode};
use std::collections::BTreeSet;

/// Most layers that may sound at the same time in one section ("few elements").
const MAX_ELEMENTS: usize = 4;

/// Most distinct emotions a score may carry ("one emotion").
const MAX_EMOTIONS: usize = 1;

/// Emotion words recognised in `meta.tags`. `meta.emotion` itself is split on separators, so any
/// word there counts; tags only count when they are one of these.
const EMOTION_WORDS: &[&str] = &[
    "aggressive",
    "angry",
    "anxious",
    "bittersweet",
    "calm",
    "creepy",
    "dark",
    "dreamy",
    "eerie",
    "energetic",
    "epic",
    "euphoric",
    "happy",
    "heroic",
    "hopeful",
    "intimate",
    "joyful",
    "lonely",
    "majestic",
    "melancholic",
    "melancholy",
    "mysterious",
    "nostalgic",
    "ominous",
    "peaceful",
    "playful",
    "reflective",
    "romantic",
    "sad",
    "scary",
    "serene",
    "somber",
    "sombre",
    "suspenseful",
    "tender",
    "tense",
    "tranquil",
    "triumphant",
    "uplifting",
    "warm",
    "whimsical",
    "wistful",
];

/// Classes the owner banned unless the brief asks for them by name (rule 4).
const EFFECT_FAMILY: &[GeneratorClass] = &[
    GeneratorClass::Riser,
    GeneratorClass::Sweep,
    GeneratorClass::Impact,
    GeneratorClass::Boom,
    GeneratorClass::Pad,
    GeneratorClass::Whoosh,
];

/// True when the layer sounds in `bar`: it is not muted and a clip covering the bar plays a
/// pattern with notes.
fn sounds_in(score: &Score, l: &Layer, bar: u32) -> bool {
    !l.muted
        && l.clips.iter().any(|c| {
            c.bars.start <= bar
                && bar <= c.bars.end
                && score
                    .pattern(&c.pattern)
                    .is_some_and(|p| !p.notes.is_empty())
        })
}

fn busiest_bar(score: &Score, first: u32, last: u32) -> (u32, usize) {
    let mut best = (first, 0);
    for bar in first..=last.min(score.bars) {
        let n = score
            .layers
            .iter()
            .filter(|l| sounds_in(score, l, bar))
            .count();
        if n > best.1 {
            best = (bar, n);
        }
    }
    best
}

/// Emotions named by the score: the parts of `meta.emotion` (split on `, / & + ; |` and the
/// words "and", "but", "with", "yet") plus emotion words in `meta.tags`.
fn emotions(score: &Score) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some(e) = &score.meta.emotion {
        let mut text = format!(" {} ", e.to_lowercase());
        for sep in [",", "/", "&", "+", ";", "|"] {
            text = text.replace(sep, " , ");
        }
        for word in [" and ", " but ", " with ", " yet "] {
            while text.contains(word) {
                text = text.replace(word, " , ");
            }
        }
        for part in text.split(',') {
            let part = part.split_whitespace().collect::<Vec<_>>().join(" ");
            if !part.is_empty() {
                out.insert(part);
            }
        }
    }
    for t in &score.meta.tags {
        let t = t.trim().to_lowercase();
        if EMOTION_WORDS.contains(&t.as_str()) {
            out.insert(t);
        }
    }
    out
}

/// `too_many_elements`: more than [`MAX_ELEMENTS`] layers sounding in the same bar of a section
/// (the whole score when there are no sections), or more than [`MAX_EMOTIONS`] emotion.
pub(super) fn too_many_elements(score: &Score, out: &mut Vec<Issue>) {
    if score.sections.is_empty() {
        let (bar, n) = busiest_bar(score, 1, score.bars);
        if n > MAX_ELEMENTS {
            warn(
                out,
                IssueCode::TooManyElements,
                "/layers".into(),
                format!(
                    "{n} layers sound together in bar {bar}; keep it to {MAX_ELEMENTS} or fewer"
                ),
            );
        }
    }
    for (i, s) in score.sections.iter().enumerate() {
        let (bar, n) = busiest_bar(score, s.bars.start, s.bars.end);
        if n > MAX_ELEMENTS {
            warn(
                out,
                IssueCode::TooManyElements,
                format!("/sections/{i}"),
                format!(
                    "section `{}`: {n} layers sound together in bar {bar}; keep it to {MAX_ELEMENTS} or fewer",
                    s.id
                ),
            );
        }
    }
    let found = emotions(score);
    if found.len() > MAX_EMOTIONS {
        let list: Vec<&str> = found.iter().map(String::as_str).collect();
        warn(
            out,
            IssueCode::TooManyElements,
            "/meta/emotion".into(),
            format!(
                "the score names {} emotions ({}); pick one",
                found.len(),
                list.join(", ")
            ),
        );
    }
}

/// `unrequested_effect_class`: a riser, sweep, impact, boom, pad or whoosh layer whose class is
/// not in `ctx.requested_classes`. "Cinematic" or "mysterious" are moods, not requests.
pub(super) fn unrequested_effect_class(score: &Score, ctx: &LintContext, out: &mut Vec<Issue>) {
    for (i, l) in score.layers.iter().enumerate() {
        if EFFECT_FAMILY.contains(&l.class) && !ctx.requested_classes.contains(&l.class) {
            warn(
                out,
                IssueCode::UnrequestedEffectClass,
                format!("/layers/{i}/class"),
                format!(
                    "layer `{}` is a {:?} layer but the brief did not ask for one",
                    l.id, l.class
                ),
            );
        }
    }
}
