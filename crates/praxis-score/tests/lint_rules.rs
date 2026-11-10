//! Quality lints: each one positive and negative, plus the acceptance fixture comes out clean.

#[path = "patch_common.rs"]
mod common;

use common::*;
use praxis_score::lint::{LintContext, lint};
use praxis_score::validate::IssueCode;
use praxis_score::*;
use std::path::PathBuf;

fn codes(s: &Score, ctx: &LintContext) -> Vec<IssueCode> {
    lint(s, ctx).into_iter().map(|i| i.code).collect()
}

fn has(s: &Score, ctx: &LintContext, code: IssueCode) -> bool {
    codes(s, ctx).contains(&code)
}

fn ctx() -> LintContext {
    LintContext::default()
}

/// One pitched layer `mel` playing `notes` (eighth grid, one bar) over bars 1-4.
fn one_layer(notes: Vec<Note>, class: GeneratorClass) -> Score {
    let mut s = Score::empty(4, 120.0);
    s.key = Key {
        tonic: PitchClass::A,
        mode: Mode::Minor,
    };
    s.patterns = vec![pattern("p", Grid::Eighth, 8, notes)];
    s.layers = vec![layer("mel", class, vec![clip(1, 4, "p")])];
    assert!(validate::validate(&s).is_valid());
    s
}

#[test]
fn acceptance_fixtures_are_clean() {
    for name in ["acceptance1_bell.score.json", "acceptance1.score.json"] {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name);
        let s = from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
        let ctx = LintContext {
            sustaining_layers: vec!["piano".into()],
            requested_classes: vec![],
        };
        assert_eq!(lint(&s, &ctx), vec![], "{name}");
    }
}

// ---------------------------------------------------------------- isolated_notes

#[test]
fn isolated_notes_flags_sparse_disconnected_notes() {
    // One short note per bar (an eighth), then 3.5 beats of silence: out of time.
    let s = one_layer(vec![note(0, 57, 1.0)], GeneratorClass::Melody);
    let issues = lint(&s, &ctx());
    let i = issues
        .iter()
        .find(|i| i.code == IssueCode::IsolatedNotes)
        .expect("flagged");
    assert_eq!(i.path, "/layers/0");
}

#[test]
fn isolated_notes_accepts_connected_motif() {
    // Eighth notes on every step with overlap: no gaps.
    let notes = (0..8)
        .map(|i| note(i, A_MINOR[i as usize % 6], 1.5))
        .collect();
    let s = one_layer(notes, GeneratorClass::Motif);
    assert!(!has(&s, &ctx(), IssueCode::IsolatedNotes));
}

#[test]
fn isolated_notes_sustain_bridges_gaps() {
    // Sparse short notes over a held bass note that rings through the whole bar.
    let s = one_layer(
        vec![note(0, 45, 8.0), note(2, 64, 0.5), note(6, 67, 0.5)],
        GeneratorClass::Harmony,
    );
    assert!(!has(&s, &ctx(), IssueCode::IsolatedNotes));
}

#[test]
fn isolated_notes_ignores_occasional_phrase_rest_and_unpitched_layers() {
    // Seven connected eighths, then the last step silent: one beat gap at most -> not flagged.
    let notes = (0..6)
        .map(|i| note(i, A_MINOR[i as usize % 6], 1.0))
        .collect();
    let s = one_layer(notes, GeneratorClass::Melody);
    assert!(!has(&s, &ctx(), IssueCode::IsolatedNotes));
    // Drums are allowed to be sparse.
    let s = one_layer(vec![note(0, 36, 1.0)], GeneratorClass::Drums);
    assert!(!has(&s, &ctx(), IssueCode::IsolatedNotes));
}

#[test]
fn isolated_notes_does_not_count_arrangement_gaps() {
    // A connected layer that drops out for bars 3-6 is arrangement, not isolated notes.
    let notes = (0..8)
        .map(|i| note(i, A_MINOR[i as usize % 6], 1.5))
        .collect();
    let mut s = one_layer(notes, GeneratorClass::Motif);
    s.bars = 8;
    s.layers[0].clips = vec![clip(1, 2, "p"), clip(7, 8, "p")];
    assert!(!has(&s, &ctx(), IssueCode::IsolatedNotes));
}

#[test]
fn isolated_notes_checks_loop_wrap() {
    // A 4-bar loop whose only notes are 8 connected eighths in bar 1: bars 2-4 are silent, and
    // the only place that shows is the wrap from the last note back to bar 1.
    let mut s = Score::empty(4, 120.0);
    s.key = Key {
        tonic: PitchClass::A,
        mode: Mode::Minor,
    };
    s.playback = Playback::Loop;
    let notes = (0..8)
        .map(|i| note(i, A_MINOR[i as usize % 6], 1.5))
        .collect();
    s.patterns = vec![pattern("p", Grid::Eighth, 32, notes)];
    s.layers = vec![layer("mel", GeneratorClass::Motif, vec![clip(1, 4, "p")])];
    // 8 notes -> 7 inner transitions without gaps + 1 wrap transition with a 3.25-bar gap:
    // 1 of 8 < 25 %, so not flagged ...
    assert!(!has(&s, &ctx(), IssueCode::IsolatedNotes));
    // ... but with only two notes, the wrap makes it 1 gap in 2 transitions.
    s.patterns[0].notes.truncate(2);
    assert!(has(&s, &ctx(), IssueCode::IsolatedNotes));
}

// ---------------------------------------------------------------- no_motif_repetition

#[test]
fn no_motif_repetition_flags_through_composed_rhythm() {
    // A 4-bar pattern whose bars all have different onset rhythms, played once.
    let mut s = Score::empty(4, 120.0);
    s.key = Key {
        tonic: PitchClass::A,
        mode: Mode::Minor,
    };
    let notes = vec![
        note(0, 57, 2.0),
        note(8, 60, 2.0),
        note(10, 62, 2.0),
        note(16, 64, 2.0),
        note(19, 67, 2.0),
        note(21, 69, 2.0),
        note(25, 57, 7.0),
    ];
    s.patterns = vec![pattern("p", Grid::Eighth, 32, notes)];
    s.layers = vec![layer("mel", GeneratorClass::Melody, vec![clip(1, 4, "p")])];
    let issues = lint(&s, &ctx());
    let i = issues
        .iter()
        .find(|i| i.code == IssueCode::NoMotifRepetition)
        .expect("flagged");
    assert_eq!(i.path, "/layers/0");
}

#[test]
fn no_motif_repetition_accepts_repeated_cell() {
    let notes = (0..8)
        .map(|i| note(i, A_MINOR[i as usize % 6], 1.5))
        .collect();
    let s = one_layer(notes, GeneratorClass::Motif);
    assert!(!has(&s, &ctx(), IssueCode::NoMotifRepetition));
    // A 2-bar pattern looped repeats its cells too (bars 1 and 3 match), even with an offset.
    let mut s = one_layer(vec![note(0, 57, 2.0)], GeneratorClass::Motif);
    s.patterns[0] = pattern(
        "p",
        Grid::Eighth,
        16,
        vec![note(0, 57, 2.0), note(11, 60, 2.0)],
    );
    s.layers[0].clips[0].pattern_offset_steps = 8;
    assert!(!has(&s, &ctx(), IssueCode::NoMotifRepetition));
    // Sound effects are one-off by nature.
    let mut s = Score::empty(2, 120.0);
    s.patterns = vec![pattern(
        "p",
        Grid::Eighth,
        16,
        vec![note(0, 60, 1.0), note(9, 60, 1.0)],
    )];
    s.layers = vec![layer("fx", GeneratorClass::Impact, vec![clip(1, 2, "p")])];
    assert!(!has(&s, &ctx(), IssueCode::NoMotifRepetition));
}

// ---------------------------------------------------------------- missing_sustain_overlap

#[test]
fn missing_sustain_overlap_only_for_sustaining_layers() {
    let notes = (0..8)
        .map(|i| note(i, A_MINOR[i as usize % 6], 1.0))
        .collect();
    let s = one_layer(notes, GeneratorClass::Motif);
    let sustaining = LintContext {
        sustaining_layers: vec!["mel".into()],
        ..Default::default()
    };
    let issues = lint(&s, &sustaining);
    let i = issues
        .iter()
        .find(|i| i.code == IssueCode::MissingSustainOverlap)
        .expect("flagged");
    assert_eq!(i.path, "/layers/0");
    // Not a sustaining instrument: no warning.
    assert!(!has(&s, &ctx(), IssueCode::MissingSustainOverlap));
    // With overlap: no warning.
    let notes = (0..8)
        .map(|i| note(i, A_MINOR[i as usize % 6], 1.6))
        .collect();
    let s = one_layer(notes, GeneratorClass::Motif);
    assert!(!has(&s, &sustaining, IssueCode::MissingSustainOverlap));
}

// ---------------------------------------------------------------- too_many_elements

fn crowded(n: usize, sections: bool) -> Score {
    let mut s = Score::empty(8, 120.0);
    s.key = Key {
        tonic: PitchClass::A,
        mode: Mode::Minor,
    };
    let notes = (0..8)
        .map(|i| note(i, A_MINOR[i as usize % 6], 1.5))
        .collect();
    s.patterns = vec![pattern("p", Grid::Eighth, 8, notes)];
    s.layers = (0..n)
        .map(|i| {
            layer(
                &format!("l{i}"),
                GeneratorClass::Motif,
                vec![clip(5, 8, "p")],
            )
        })
        .collect();
    if sections {
        s.sections = vec![
            Section {
                id: "a".into(),
                name: String::new(),
                bars: bars(1, 4),
                role: SectionRole::Intro,
                intensity: 0.3,
            },
            Section {
                id: "b".into(),
                name: String::new(),
                bars: bars(5, 8),
                role: SectionRole::Main,
                intensity: 0.6,
            },
        ];
    }
    s
}

#[test]
fn too_many_elements_per_section() {
    let s = crowded(5, true);
    let issues: Vec<_> = lint(&s, &ctx())
        .into_iter()
        .filter(|i| i.code == IssueCode::TooManyElements)
        .collect();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].path, "/sections/1");
    assert!(!has(&crowded(4, true), &ctx(), IssueCode::TooManyElements));
    // Without sections the whole score counts.
    assert!(has(&crowded(5, false), &ctx(), IssueCode::TooManyElements));
    // Muted layers do not sound.
    let mut s = crowded(5, true);
    s.layers[0].muted = true;
    assert!(!has(&s, &ctx(), IssueCode::TooManyElements));
    // Five layers that never overlap are fine.
    let mut s = crowded(5, false);
    for (i, l) in s.layers.iter_mut().enumerate() {
        let b = 1 + i as u32;
        l.clips = vec![clip(b, b, "p")];
    }
    assert!(!has(&s, &ctx(), IssueCode::TooManyElements));
}

#[test]
fn too_many_elements_more_than_one_emotion() {
    let mut s = crowded(1, false);
    s.meta.emotion = Some("calm".into());
    assert!(!has(&s, &ctx(), IssueCode::TooManyElements));
    s.meta.tags = vec!["loop".into(), "calm".into()];
    assert!(
        !has(&s, &ctx(), IssueCode::TooManyElements),
        "same emotion twice is one"
    );
    for e in ["dark and hopeful", "sad, triumphant", "tense/eerie"] {
        s.meta.emotion = Some(e.into());
        let issues = lint(&s, &ctx());
        let i = issues
            .iter()
            .find(|i| i.code == IssueCode::TooManyElements)
            .unwrap_or_else(|| panic!("{e}"));
        assert_eq!(i.path, "/meta/emotion");
    }
    s.meta.emotion = Some("calm".into());
    s.meta.tags = vec!["epic".into()];
    assert!(
        has(&s, &ctx(), IssueCode::TooManyElements),
        "emotion tag adds a second emotion"
    );
}

// ---------------------------------------------------------------- unrequested_effect_class

#[test]
fn unrequested_effect_class_flags_effect_layers() {
    for class in [
        GeneratorClass::Riser,
        GeneratorClass::Sweep,
        GeneratorClass::Impact,
        GeneratorClass::Boom,
        GeneratorClass::Pad,
        GeneratorClass::Whoosh,
    ] {
        let mut s = crowded(1, false);
        s.layers[0].class = class;
        let issues = lint(&s, &ctx());
        let i = issues
            .iter()
            .find(|i| i.code == IssueCode::UnrequestedEffectClass)
            .unwrap_or_else(|| panic!("{class:?}"));
        assert_eq!(i.path, "/layers/0/class");
        // Requested: fine.
        let asked = LintContext {
            requested_classes: vec![class],
            ..Default::default()
        };
        assert!(
            !has(&s, &asked, IssueCode::UnrequestedEffectClass),
            "{class:?}"
        );
    }
    // Musical classes are never flagged.
    let s = crowded(1, false);
    assert!(!has(&s, &ctx(), IssueCode::UnrequestedEffectClass));
}
