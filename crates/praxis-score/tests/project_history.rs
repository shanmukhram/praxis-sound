//! Project history: commit, undo, redo, named versions, JSON (FR-31).

#[path = "patch_common.rs"]
mod common;

use common::*;
use praxis_score::patch::{Origin, PatchError, PatchOp};
use praxis_score::project::{PROJECT_FORMAT, Project, ProjectError};
use praxis_score::*;

fn retitle(t: &str) -> praxis_score::patch::ScorePatch {
    patch(vec![PatchOp::SetTitle { title: t.into() }])
}

fn clear(layer: &str, a: u32, b: u32) -> praxis_score::patch::ScorePatch {
    patch(vec![PatchOp::ClearBars {
        layer: layer.into(),
        bars: bars(a, b),
    }])
}

#[test]
fn commit_pushes_history_with_increasing_seq() {
    let s = base_score();
    let mut p = Project::new(s.clone());
    assert_eq!(p.format, PROJECT_FORMAT);
    let e1 = p.commit(retitle("one")).unwrap().clone();
    let e2 = p.commit(clear("piano", 13, 16)).unwrap().clone();
    assert_eq!((e1.seq, e2.seq), (1, 2));
    assert_eq!(p.next_seq, 3);
    assert_eq!(p.undo.len(), 2);
    assert_eq!(p.score.title, "one");
    assert_eq!(p.current_seq(), 2);
    // The stored inverse undoes exactly.
    let back = praxis_score::apply::apply(&p.score, &e2.inverse).unwrap();
    assert_eq!(back.score.layers, s.layers);
}

#[test]
fn rejected_commit_changes_nothing() {
    let mut p = Project::new(base_score());
    p.commit(retitle("one")).unwrap();
    let before = p.clone();
    let e = p.commit(clear("bass", 1, 2)).unwrap_err();
    assert_eq!(e, PatchError::LockedLayer("bass".into()));
    assert_eq!(p, before);
}

#[test]
fn undo_redo_round_trip() {
    let s = base_score();
    let mut p = Project::new(s.clone());
    p.commit(retitle("one")).unwrap();
    let after_one = p.score.clone();
    p.commit(clear("piano", 13, 16)).unwrap();
    let after_two = p.score.clone();

    assert_eq!(p.undo().unwrap().map(|e| e.seq), Some(2));
    assert_eq!(p.score, after_one);
    assert_eq!(p.undo().unwrap().map(|e| e.seq), Some(1));
    assert_eq!(p.score, s);
    assert!(p.undo().unwrap().is_none(), "nothing left to undo");
    assert!(!p.can_undo() && p.can_redo());
    assert_eq!(p.current_seq(), 0);

    assert_eq!(p.redo().unwrap().map(|e| e.seq), Some(1));
    assert_eq!(p.score, after_one);
    assert_eq!(p.redo().unwrap().map(|e| e.seq), Some(2));
    assert_eq!(p.score, after_two);
    assert!(p.redo().unwrap().is_none());
}

#[test]
fn commit_clears_redo() {
    let mut p = Project::new(base_score());
    p.commit(retitle("one")).unwrap();
    p.commit(retitle("two")).unwrap();
    p.undo().unwrap();
    assert_eq!(p.redo.len(), 1);
    let e = p.commit(retitle("three")).unwrap();
    assert_eq!(e.seq, 3, "seq keeps counting after an undo");
    assert!(p.redo.is_empty());
    assert_eq!(p.score.title, "three");
}

#[test]
fn undo_of_a_model_lock_works() {
    let mut p = Project::new(base_score());
    let mut lock = patch(vec![PatchOp::SetLock {
        id: "piano".into(),
        locked: true,
    }]);
    lock.origin = Origin::Model;
    p.commit(lock).unwrap();
    assert!(p.score.layer("piano").unwrap().locked);
    p.undo().unwrap();
    assert!(!p.score.layer("piano").unwrap().locked);
}

#[test]
fn versions_save_and_restore_as_undoable_patch() {
    let s = base_score();
    let mut p = Project::new(s.clone());
    p.commit(retitle("draft")).unwrap();
    let v = p.save_version(" first ").unwrap().clone();
    assert_eq!((v.name.as_str(), v.seq), ("first", 1));
    assert!(matches!(
        p.save_version("first"),
        Err(ProjectError::DuplicateVersion(_))
    ));
    assert!(matches!(
        p.save_version("  "),
        Err(ProjectError::EmptyVersionName)
    ));

    // Diverge a lot: layer removed, pattern added, locked layer changed, tempo changed.
    p.commit(patch(vec![
        PatchOp::RemoveLayer { id: "piano".into() },
        PatchOp::AddPattern {
            index: None,
            pattern: pattern("extra", Grid::Quarter, 4, vec![note(0, 57, 2.0)]),
        },
        PatchOp::SetLock {
            id: "bass".into(),
            locked: false,
        },
        PatchOp::PlaceClip {
            layer: "bass".into(),
            clip: clip(9, 16, "extra"),
        },
        PatchOp::SetLock {
            id: "bass".into(),
            locked: true,
        },
        PatchOp::SetTempo { bpm: 140.0 },
        PatchOp::AddSection {
            index: None,
            section: Section {
                id: "s".into(),
                name: String::new(),
                bars: bars(1, 16),
                role: SectionRole::Main,
                intensity: 0.5,
            },
        },
    ]))
    .unwrap();
    let diverged = p.score.clone();
    assert_ne!(diverged, v.score);

    let entry = p.restore_version("first").unwrap().clone();
    assert_eq!(p.score, v.score);
    assert_eq!(entry.seq, 3);
    assert_eq!(entry.patch.origin, Origin::User);
    // The restore is undoable and redoable.
    p.undo().unwrap();
    assert_eq!(p.score, diverged);
    p.redo().unwrap();
    assert_eq!(p.score, v.score);

    // Restoring the current state is an empty, harmless patch.
    let same = p.restore_version("first").unwrap();
    assert!(same.patch.ops.is_empty());

    assert!(matches!(
        p.restore_version("nope"),
        Err(ProjectError::UnknownVersion(_))
    ));
    assert_eq!(p.delete_version("first").unwrap().name, "first");
    assert!(p.version("first").is_none());
}

#[test]
fn restore_back_to_the_start() {
    let s = base_score();
    let mut p = Project::new(s.clone());
    p.save_version("start").unwrap();
    p.commit(patch(vec![
        PatchOp::WriteBars {
            layer: "piano".into(),
            bars: bars(5, 8),
            pattern: pattern("lead", Grid::Sixteenth, 16, vec![note(0, 64, 4.0)]),
        },
        PatchOp::RemoveLayer { id: "kick".into() },
    ]))
    .unwrap();
    p.commit(clear("piano", 1, 1)).unwrap();
    p.restore_version("start").unwrap();
    assert_eq!(p.score, s);
}

#[test]
fn json_round_trip() {
    let mut p = Project::new(base_score());
    p.commit(retitle("one")).unwrap();
    p.commit(clear("piano", 2, 3)).unwrap();
    p.undo().unwrap();
    p.save_version("v").unwrap();
    p.references.push(praxis_score::project::ReferenceRef {
        name: "ref".into(),
        profile: "profiles/ref.json".into(),
    });
    let text = p.to_json().unwrap();
    let back = Project::from_json(&text).unwrap();
    assert_eq!(back, p);
    assert_eq!(back.to_json().unwrap(), text, "canonical form is stable");
}

#[test]
fn from_json_checks_format_version_and_score() {
    let p = Project::new(base_score());
    let mut v: serde_json::Value = serde_json::from_str(&p.to_json().unwrap()).unwrap();

    let mut wrong = v.clone();
    wrong["format"] = "something-else".into();
    assert!(matches!(
        Project::from_json(&wrong.to_string()),
        Err(ProjectError::Format(f)) if f == "something-else"
    ));
    let mut missing = v.clone();
    missing.as_object_mut().unwrap().remove("format");
    assert!(matches!(
        Project::from_json(&missing.to_string()),
        Err(ProjectError::Format(_))
    ));
    let mut newer = v.clone();
    newer["version"] = 2.into();
    assert!(matches!(
        Project::from_json(&newer.to_string()),
        Err(ProjectError::Version(_))
    ));
    assert!(matches!(
        Project::from_json("{not json"),
        Err(ProjectError::Json(_))
    ));
    // A score that does not validate (C#4 in A minor).
    v["score"]["patterns"][0]["notes"][0]["pitch"] = 61.into();
    assert!(matches!(
        Project::from_json(&v.to_string()),
        Err(ProjectError::InvalidScore(_))
    ));
}

#[test]
fn from_json_moves_next_seq_past_history() {
    let mut p = Project::new(base_score());
    p.commit(retitle("one")).unwrap();
    p.commit(retitle("two")).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&p.to_json().unwrap()).unwrap();
    v.as_object_mut().unwrap().remove("next_seq");
    let mut back = Project::from_json(&v.to_string()).unwrap();
    assert_eq!(back.next_seq, 3);
    assert_eq!(back.commit(retitle("three")).unwrap().seq, 3);
}

#[test]
fn restore_brings_back_the_seed() {
    let mut p = Project::new(base_score());
    p.save_version("v").unwrap();
    p.commit(patch(vec![PatchOp::SetSeed { seed: 99 }]))
        .unwrap();
    let entry = p.restore_version("v").unwrap();
    assert!(entry.patch.ops.contains(&PatchOp::SetSeed { seed: 0 }));
    assert_eq!(p.score.seed, 0);
    p.undo().unwrap();
    assert_eq!(p.score.seed, 99);
}

#[test]
fn restore_refuses_what_no_patch_can_change() {
    let mut p = Project::new(base_score());
    p.save_version("v").unwrap();
    p.score.schema_version = 2; // only possible by editing the struct directly
    assert!(matches!(
        p.restore_version("v"),
        Err(ProjectError::Restore { .. })
    ));
}
