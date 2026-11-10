//! Helpers shared by the patch, project and lint tests (included with `#[path]`; as its own test
//! target it contains no tests).
#![allow(dead_code)]

use praxis_score::apply::apply;
use praxis_score::patch::{Applied, Origin, PatchError, PatchOp, Scope, ScorePatch};
use praxis_score::timeline::{NoteEvent, expand_notes};
use praxis_score::*;

pub const SR: u32 = 48_000;

pub fn note(step: u32, pitch: u8, length: f32) -> Note {
    Note {
        step,
        pitch,
        velocity: 0.7,
        accent: Accent::Normal,
        length,
        offset: 0.0,
    }
}

pub fn pattern(id: &str, grid: Grid, length_steps: u32, notes: Vec<Note>) -> Pattern {
    Pattern {
        id: id.into(),
        grid,
        length_steps,
        notes,
        spoken: None,
    }
}

pub fn clip(start: u32, end: u32, pattern: &str) -> Clip {
    Clip {
        bars: BarRange { start, end },
        pattern: pattern.into(),
        pattern_offset_steps: 0,
        transpose: 0,
        velocity_scale: 1.0,
    }
}

pub fn layer(id: &str, class: GeneratorClass, clips: Vec<Clip>) -> Layer {
    let kind = if class == GeneratorClass::Drums {
        InstrumentKind::Drum
    } else {
        InstrumentKind::Synth
    };
    Layer {
        id: id.into(),
        name: String::new(),
        class,
        instrument: InstrumentRef {
            kind,
            preset: "bell".into(),
            params: Default::default(),
        },
        clips,
        locked: false,
        muted: false,
        solo: false,
        gain_db: 0.0,
        pan: 0.0,
        effects: vec![],
        automation: vec![],
        seed_offset: 0,
    }
}

pub fn bars(start: u32, end: u32) -> BarRange {
    BarRange { start, end }
}

/// A minor pitches used by the helpers: A3 C4 D4 E4 G4 A4.
pub const A_MINOR: [u8; 6] = [57, 60, 62, 64, 67, 69];

/// 16 bars of 4/4 at 128 BPM in A minor:
/// - `piano` (motif): 2-bar eighth pattern `motif` over bars 1-16, unlocked;
/// - `kick` (drums): 1-bar pattern `kick` over bars 9-16, unlocked;
/// - `bass` (bass, LOCKED): 1-bar quarter pattern `bassline` over bars 1-8.
pub fn base_score() -> Score {
    let mut s = Score::empty(16, 128.0);
    s.key = Key {
        tonic: PitchClass::A,
        mode: Mode::Minor,
    };
    s.title = "test".into();
    let motif_notes = (0..16)
        .map(|i| note(i, A_MINOR[(i as usize * 5) % 6], 1.5))
        .collect();
    s.patterns = vec![
        pattern("motif", Grid::Eighth, 16, motif_notes),
        pattern(
            "kick",
            Grid::Eighth,
            8,
            vec![note(0, 36, 1.0), note(4, 36, 1.0)],
        ),
        pattern(
            "bassline",
            Grid::Quarter,
            4,
            vec![note(0, 45, 2.0), note(2, 43, 2.0)],
        ),
    ];
    let mut bass = layer("bass", GeneratorClass::Bass, vec![clip(1, 8, "bassline")]);
    bass.locked = true;
    s.layers = vec![
        layer("piano", GeneratorClass::Motif, vec![clip(1, 16, "motif")]),
        layer("kick", GeneratorClass::Drums, vec![clip(9, 16, "kick")]),
        bass,
    ];
    assert!(
        validate::validate(&s).is_valid(),
        "{:?}",
        validate::validate(&s).errors
    );
    s
}

pub fn patch(ops: Vec<PatchOp>) -> ScorePatch {
    ScorePatch {
        description: "test".into(),
        origin: Origin::User,
        scope: Scope::default(),
        ops,
    }
}

pub fn scoped(range: Option<(u32, u32)>, layers: &[&str], ops: Vec<PatchOp>) -> ScorePatch {
    ScorePatch {
        description: "scoped test".into(),
        origin: Origin::Model,
        scope: Scope {
            bars: range.map(|(a, b)| bars(a, b)),
            layers: layers.iter().map(|l| l.to_string()).collect(),
        },
        ops,
    }
}

/// Applies `p` and checks the exact-inverse rule on the way.
pub fn apply_ok(s: &Score, p: &ScorePatch) -> Applied {
    let a = apply(s, p).unwrap_or_else(|e| panic!("patch rejected: {e}\n{p:#?}"));
    let back = apply(&a.score, &a.inverse)
        .unwrap_or_else(|e| panic!("inverse rejected: {e}\n{:#?}", a.inverse));
    assert_eq!(&back.score, s, "inverse is not exact");
    a
}

pub fn apply_err(s: &Score, p: &ScorePatch) -> PatchError {
    match apply(s, p) {
        Ok(_) => panic!("patch unexpectedly accepted: {p:#?}"),
        Err(e) => e,
    }
}

/// Note events of one layer by id, `layer` index normalised to 0.
pub fn layer_events(s: &Score, id: &str) -> Vec<NoteEvent> {
    let Some(li) = s.layers.iter().position(|l| l.id == id) else {
        return vec![];
    };
    expand_notes(s, SR)
        .into_iter()
        .filter(|e| e.layer == li)
        .map(|mut e| {
            e.layer = 0;
            e
        })
        .collect()
}

pub fn outside(events: &[NoteEvent], range: BarRange) -> Vec<NoteEvent> {
    events
        .iter()
        .filter(|e| e.bar < range.start || e.bar > range.end)
        .cloned()
        .collect()
}

/// Same events, exactly: every field including the `f64` seconds (`expand_notes` computes note
/// times from one integer step count, so a split clip reproduces them bit for bit).
pub fn assert_same_events(a: &[NoteEvent], b: &[NoteEvent], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: event count differs");
    for (x, y) in a.iter().zip(b) {
        assert_eq!(x, y, "{what}");
    }
}
