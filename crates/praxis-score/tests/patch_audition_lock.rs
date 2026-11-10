//! A lock protects content, not auditioning: a user may mute/solo a locked layer; nobody may
//! change its sound, and a model may not even mute it.

use praxis_score::patch::{LayerUpdate, Origin, PatchError, PatchOp, ScorePatch};
use praxis_score::{apply::apply, from_json};

fn score() -> praxis_score::Score {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/acceptance1_bell.score.json"
    );
    from_json(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn patch(origin: Origin, update: LayerUpdate) -> ScorePatch {
    ScorePatch {
        description: String::new(),
        origin,
        scope: Default::default(),
        ops: vec![PatchOp::UpdateLayer {
            id: "piano".into(),
            update,
        }],
    }
}

#[test]
fn user_may_mute_and_solo_a_locked_layer_with_exact_undo() {
    let s = score();
    assert!(s.layer("piano").unwrap().locked);
    for u in [
        LayerUpdate {
            muted: Some(true),
            ..Default::default()
        },
        LayerUpdate {
            solo: Some(true),
            ..Default::default()
        },
        LayerUpdate {
            muted: Some(true),
            solo: Some(true),
            ..Default::default()
        },
    ] {
        let a = apply(&s, &patch(Origin::User, u)).expect("audition change allowed");
        let back = apply(&a.score, &a.inverse).expect("undo");
        assert_eq!(back.score, s);
    }
}

#[test]
fn locked_layer_sound_and_model_mutes_are_refused() {
    let s = score();
    let gain = LayerUpdate {
        gain_db: Some(-3.0),
        ..Default::default()
    };
    assert!(matches!(
        apply(&s, &patch(Origin::User, gain)),
        Err(PatchError::LockedLayer(_))
    ));
    let mute_and_gain = LayerUpdate {
        muted: Some(true),
        gain_db: Some(-3.0),
        ..Default::default()
    };
    assert!(matches!(
        apply(&s, &patch(Origin::User, mute_and_gain)),
        Err(PatchError::LockedLayer(_))
    ));
    let mute = LayerUpdate {
        muted: Some(true),
        ..Default::default()
    };
    assert!(matches!(
        apply(&s, &patch(Origin::Model, mute)),
        Err(PatchError::LockedLayer(_))
    ));
}
