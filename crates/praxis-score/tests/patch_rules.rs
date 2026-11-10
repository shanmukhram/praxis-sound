//! Unit tests for every rule in docs/contracts/patch.md and every op.

#[path = "patch_common.rs"]
mod common;

use common::*;
use praxis_score::apply::apply;
use praxis_score::patch::{LayerUpdate, Origin, PatchError, PatchOp};
use praxis_score::*;

fn write(layer: &str, a: u32, b: u32, p: Pattern) -> PatchOp {
    PatchOp::WriteBars {
        layer: layer.into(),
        bars: bars(a, b),
        pattern: p,
    }
}

fn melody(id: &str) -> Pattern {
    pattern(
        id,
        Grid::Eighth,
        8,
        (0..8)
            .map(|i| note(i, A_MINOR[(i as usize + 2) % 6], 1.5))
            .collect(),
    )
}

// ---------------------------------------------------------------- rule 1: atomic, validated

#[test]
fn rule1_failing_op_discards_earlier_ops() {
    let s = base_score();
    let p = patch(vec![
        PatchOp::SetTitle {
            title: "changed".into(),
        },
        PatchOp::ClearBars {
            layer: "nope".into(),
            bars: bars(1, 2),
        },
    ]);
    let e = apply_err(&s, &p);
    assert!(
        matches!(e, PatchError::UnknownId { index: 1, ref what, .. } if what == "layer"),
        "{e:?}"
    );
}

#[test]
fn rule1_invalid_result_is_rejected() {
    let s = base_score();
    // C#4 is not in A minor.
    let bad = pattern("bad", Grid::Eighth, 8, vec![note(0, 61, 1.5)]);
    let e = apply_err(&s, &patch(vec![write("piano", 13, 16, bad)]));
    assert!(
        matches!(e, PatchError::Invalid(ref m) if m.contains("OffScale")),
        "{e:?}"
    );
    // Shrinking the score below existing clips is invalid too.
    let e = apply_err(&s, &patch(vec![PatchOp::SetBars { bars: 8 }]));
    assert!(matches!(e, PatchError::Invalid(_)), "{e:?}");
}

#[test]
fn empty_patch_is_identity() {
    let s = base_score();
    let a = apply_ok(&s, &patch(vec![]));
    assert_eq!(a.score, s);
    assert!(a.inverse.ops.is_empty());
}

// ---------------------------------------------------------------- rule 2: locks

#[test]
fn rule2_ops_on_locked_layer_fail() {
    let s = base_score();
    let locked = PatchError::LockedLayer("bass".into());
    let ops = vec![
        write(
            "bass",
            1,
            2,
            pattern("b2", Grid::Quarter, 4, vec![note(0, 45, 2.0)]),
        ),
        PatchOp::ClearBars {
            layer: "bass".into(),
            bars: bars(1, 2),
        },
        PatchOp::PlaceClip {
            layer: "bass".into(),
            clip: clip(9, 10, "bassline"),
        },
        PatchOp::SetClips {
            layer: "bass".into(),
            clips: vec![],
        },
        PatchOp::UpdateLayer {
            id: "bass".into(),
            update: LayerUpdate {
                gain_db: Some(-3.0),
                ..Default::default()
            },
        },
        PatchOp::RemoveLayer { id: "bass".into() },
        // A pattern played by a locked layer cannot be replaced or removed.
        PatchOp::ReplacePattern {
            pattern: pattern("bassline", Grid::Quarter, 4, vec![note(0, 45, 1.0)]),
        },
        PatchOp::RemovePattern {
            id: "bassline".into(),
        },
        // Transposing would move the locked bass.
        PatchOp::SetKey {
            key: Key {
                tonic: PitchClass::D,
                mode: Mode::Minor,
            },
            transpose_notes: true,
        },
    ];
    for op in ops {
        let name = op.name();
        assert_eq!(apply_err(&s, &patch(vec![op])), locked, "{name}");
    }
}

#[test]
fn rule2_only_user_may_unlock() {
    let s = base_score();
    let unlock = PatchOp::SetLock {
        id: "bass".into(),
        locked: false,
    };
    for origin in [Origin::Model, Origin::System] {
        let mut p = patch(vec![unlock.clone()]);
        p.origin = origin;
        assert_eq!(
            apply_err(&s, &p),
            PatchError::UnlockNotAllowed("bass".into())
        );
    }
    // A user unlock followed by an edit in the same patch works, and undoes exactly.
    let p = patch(vec![
        unlock,
        PatchOp::ClearBars {
            layer: "bass".into(),
            bars: bars(5, 8),
        },
    ]);
    let a = apply_ok(&s, &p);
    assert!(!a.score.layer("bass").unwrap().locked);
    assert_eq!(a.inverse.origin, Origin::User);
}

#[test]
fn rule2_model_may_lock_and_its_undo_unlocks_as_user() {
    let s = base_score();
    let mut p = patch(vec![PatchOp::SetLock {
        id: "piano".into(),
        locked: true,
    }]);
    p.origin = Origin::Model;
    let a = apply_ok(&s, &p);
    assert!(a.score.layer("piano").unwrap().locked);
    assert_eq!(a.inverse.origin, Origin::User);
    // Once locked, the model's later edit is rejected.
    let mut edit = patch(vec![PatchOp::ClearBars {
        layer: "piano".into(),
        bars: bars(1, 1),
    }]);
    edit.origin = Origin::Model;
    assert_eq!(
        apply_err(&a.score, &edit),
        PatchError::LockedLayer("piano".into())
    );
}

// ---------------------------------------------------------------- rule 3: scope

#[test]
fn rule3_global_ops_need_unscoped_patch() {
    let s = base_score();
    for (range, layers) in [(Some((13, 16)), vec![]), (None, vec!["piano"])] {
        let p = scoped(range, &layers, vec![PatchOp::SetTempo { bpm: 90.0 }]);
        assert!(
            matches!(apply_err(&s, &p), PatchError::OutOfScope { index: 0, .. }),
            "{range:?} {layers:?}"
        );
    }
    // Unscoped works.
    let a = apply_ok(&s, &patch(vec![PatchOp::SetTempo { bpm: 90.0 }]));
    assert_eq!(a.score.tempo.bpm, 90.0);
}

#[test]
fn rule3_bar_ops_outside_scope_are_rejected_not_clipped() {
    let s = base_score();
    for op in [
        write("piano", 12, 16, melody("m")),
        PatchOp::ClearBars {
            layer: "piano".into(),
            bars: bars(12, 13),
        },
        PatchOp::PlaceClip {
            layer: "piano".into(),
            clip: clip(1, 1, "motif"),
        },
    ] {
        let p = scoped(Some((13, 16)), &[], vec![op]);
        assert!(matches!(apply_err(&s, &p), PatchError::OutOfScope { .. }));
    }
    // Inside the scope it applies.
    apply_ok(
        &s,
        &scoped(
            Some((13, 16)),
            &[],
            vec![write("piano", 13, 16, melody("m"))],
        ),
    );
}

#[test]
fn rule3_layer_ops_must_name_scoped_layer() {
    let s = base_score();
    let p = scoped(
        Some((13, 16)),
        &["piano"],
        vec![PatchOp::ClearBars {
            layer: "kick".into(),
            bars: bars(13, 16),
        }],
    );
    assert!(matches!(apply_err(&s, &p), PatchError::OutOfScope { .. }));
    let p = scoped(
        None,
        &["piano"],
        vec![PatchOp::UpdateLayer {
            id: "kick".into(),
            update: LayerUpdate {
                gain_db: Some(-6.0),
                ..Default::default()
            },
        }],
    );
    assert!(matches!(apply_err(&s, &p), PatchError::OutOfScope { .. }));
    let p = scoped(
        None,
        &["piano"],
        vec![PatchOp::AddLayer {
            layer: layer("pad2", GeneratorClass::Harmony, vec![]),
        }],
    );
    assert!(matches!(apply_err(&s, &p), PatchError::OutOfScope { .. }));
}

#[test]
fn rule3_bar_scope_limits_whole_layer_changes() {
    let s = base_score();
    let gain = PatchOp::UpdateLayer {
        id: "piano".into(),
        update: LayerUpdate {
            gain_db: Some(-3.0),
            ..Default::default()
        },
    };
    assert!(matches!(
        apply_err(&s, &scoped(Some((13, 16)), &[], vec![gain])),
        PatchError::OutOfScope { .. }
    ));
    let rename = PatchOp::UpdateLayer {
        id: "piano".into(),
        update: LayerUpdate {
            name: Some("Felt".into()),
            ..Default::default()
        },
    };
    apply_ok(&s, &scoped(Some((13, 16)), &[], vec![rename]));
    // Removing a layer that plays outside the bars, or adding one that does, is out of scope.
    assert!(matches!(
        apply_err(
            &s,
            &scoped(
                Some((9, 16)),
                &[],
                vec![PatchOp::RemoveLayer { id: "piano".into() }]
            )
        ),
        PatchError::OutOfScope { .. }
    ));
    let mut extra = layer(
        "tick",
        GeneratorClass::Percussion,
        vec![clip(1, 16, "kick")],
    );
    assert!(matches!(
        apply_err(
            &s,
            &scoped(
                Some((9, 16)),
                &[],
                vec![PatchOp::AddLayer {
                    layer: extra.clone()
                }]
            )
        ),
        PatchError::OutOfScope { .. }
    ));
    extra.clips = vec![clip(9, 16, "kick")];
    apply_ok(
        &s,
        &scoped(Some((9, 16)), &[], vec![PatchOp::AddLayer { layer: extra }]),
    );
    // Kick plays only bars 9-16, so a 9-16 scope may remove it.
    apply_ok(
        &s,
        &scoped(
            Some((9, 16)),
            &[],
            vec![PatchOp::RemoveLayer { id: "kick".into() }],
        ),
    );
}

#[test]
fn rule3_replace_pattern_needs_every_user_in_scope() {
    let s = base_score();
    let motif2 = pattern("motif", Grid::Eighth, 16, vec![note(0, 57, 2.0)]);
    let p = scoped(
        Some((13, 16)),
        &[],
        vec![PatchOp::ReplacePattern {
            pattern: motif2.clone(),
        }],
    );
    assert!(matches!(apply_err(&s, &p), PatchError::OutOfScope { .. }));
    // Unscoped, every clip is in scope.
    let a = apply_ok(
        &s,
        &patch(vec![PatchOp::ReplacePattern {
            pattern: motif2.clone(),
        }]),
    );
    assert_eq!(a.score.pattern("motif"), Some(&motif2));
    // Layer-scoped: a pattern shared with a layer outside the scope cannot be replaced.
    let mut shared = base_score();
    shared.layers[1].clips = vec![clip(9, 16, "motif")];
    shared.layers[1].class = GeneratorClass::Harmony;
    let p = scoped(
        None,
        &["piano"],
        vec![PatchOp::ReplacePattern { pattern: motif2 }],
    );
    assert!(matches!(
        apply_err(&shared, &p),
        PatchError::OutOfScope { .. }
    ));
}

#[test]
fn rule3_set_clips_checked_outside_scope() {
    let s = base_score();
    // Changing bars 13-16 only: accepted.
    let mut clips = vec![clip(1, 12, "motif")];
    clips.push(clip(13, 16, "kick"));
    let ok = scoped(
        Some((13, 16)),
        &["piano"],
        vec![PatchOp::SetClips {
            layer: "piano".into(),
            clips,
        }],
    );
    let a = apply(&s, &ok).expect("in-scope set_clips");
    assert_same_events(
        &outside(&layer_events(&s, "piano"), bars(13, 16)),
        &outside(&layer_events(&a.score, "piano"), bars(13, 16)),
        "bars 1-12",
    );
    // Touching bar 12 as well: rejected.
    let bad = scoped(
        Some((13, 16)),
        &["piano"],
        vec![PatchOp::SetClips {
            layer: "piano".into(),
            clips: vec![clip(1, 11, "motif")],
        }],
    );
    assert!(matches!(apply_err(&s, &bad), PatchError::OutOfScope { .. }));
}

// ---------------------------------------------------------------- rule 4: copy-on-write

#[test]
fn rule4_write_bars_splits_and_keeps_other_bars() {
    let s = base_score();
    let a = apply_ok(&s, &patch(vec![write("piano", 13, 16, melody("lead"))]));
    let piano = a.score.layer("piano").unwrap();
    assert_eq!(
        piano.clips,
        vec![clip(1, 12, "motif"), clip(13, 16, "lead")]
    );
    assert_eq!(
        a.score.pattern("motif"),
        s.pattern("motif"),
        "shared pattern untouched"
    );
    assert_eq!(a.score.pattern("lead"), Some(&melody("lead")));
    assert_eq!(
        outside(&layer_events(&a.score, "piano"), bars(13, 16)),
        outside(&layer_events(&s, "piano"), bars(13, 16)),
    );
}

#[test]
fn rule4_split_in_the_middle_keeps_phase() {
    let s = base_score();
    // Bars 6-7 of a 2-bar pattern: the right-hand piece starts at bar 8, which is the second
    // half of the pattern (step 8).
    let a = apply_ok(&s, &patch(vec![write("piano", 6, 7, melody("lead"))]));
    let piano = a.score.layer("piano").unwrap();
    let mut right = clip(8, 16, "motif");
    right.pattern_offset_steps = 8;
    assert_eq!(
        piano.clips,
        vec![clip(1, 5, "motif"), clip(6, 7, "lead"), right]
    );
    // At 128 BPM every split position is exact, so events are bit-identical.
    assert_eq!(
        outside(&layer_events(&a.score, "piano"), bars(6, 7)),
        outside(&layer_events(&s, "piano"), bars(6, 7)),
    );
}

#[test]
fn rule4_split_keeps_existing_offset_and_transpose() {
    let mut s = base_score();
    let mut c = clip(1, 16, "motif");
    c.pattern_offset_steps = 8;
    c.transpose = 12;
    c.velocity_scale = 0.8;
    s.layers[0].clips = vec![c.clone()];
    let a = apply_ok(
        &s,
        &patch(vec![PatchOp::ClearBars {
            layer: "piano".into(),
            bars: bars(3, 3),
        }]),
    );
    let clips = &a.score.layer("piano").unwrap().clips;
    assert_eq!(clips.len(), 2);
    assert_eq!(clips[0].bars, bars(1, 2));
    assert_eq!(clips[0].pattern_offset_steps, 8);
    assert_eq!(clips[1].bars, bars(4, 16));
    assert_eq!(clips[1].pattern_offset_steps, (8 + 3 * 8) % 16);
    assert_eq!((clips[1].transpose, clips[1].velocity_scale), (12, 0.8));
    assert_eq!(
        outside(&layer_events(&a.score, "piano"), bars(3, 3)),
        outside(&layer_events(&s, "piano"), bars(3, 3)),
    );
}

#[test]
fn rule4_exact_neighbours_merge_back() {
    let s = base_score();
    let cleared = apply_ok(
        &s,
        &patch(vec![PatchOp::ClearBars {
            layer: "piano".into(),
            bars: bars(5, 8),
        }]),
    );
    assert_eq!(cleared.score.layers[0].clips.len(), 2);
    assert!(
        layer_events(&cleared.score, "piano")
            .iter()
            .all(|e| !(5..=8).contains(&e.bar))
    );
    // Placing the motif back at its own phase (bar 5 = step 0 of the 2-bar pattern) merges into
    // one clip again: the canonical form is the original score.
    let placed = apply_ok(
        &cleared.score,
        &patch(vec![PatchOp::PlaceClip {
            layer: "piano".into(),
            clip: clip(5, 8, "motif"),
        }]),
    );
    assert_eq!(placed.score, s);
    // At the wrong phase it stays split.
    let mut off = clip(5, 8, "motif");
    off.pattern_offset_steps = 8;
    let placed = apply_ok(
        &cleared.score,
        &patch(vec![PatchOp::PlaceClip {
            layer: "piano".into(),
            clip: off,
        }]),
    );
    assert_eq!(placed.score.layers[0].clips.len(), 3);
}

#[test]
fn rule4_no_merge_when_scale_differs() {
    let s = base_score();
    let mut c = clip(13, 16, "motif");
    c.velocity_scale = 0.5;
    let a = apply_ok(
        &s,
        &patch(vec![PatchOp::PlaceClip {
            layer: "piano".into(),
            clip: c.clone(),
        }]),
    );
    assert_eq!(a.score.layers[0].clips, vec![clip(1, 12, "motif"), c]);
}

#[test]
fn rule4_other_layers_sharing_a_pattern_are_untouched() {
    let mut s = base_score();
    s.layers[1].class = GeneratorClass::Harmony;
    s.layers[1].clips = vec![clip(1, 16, "motif")];
    let a = apply_ok(&s, &patch(vec![write("piano", 1, 4, melody("lead"))]));
    assert_eq!(a.score.layers[1], s.layers[1]);
    assert_eq!(a.score.pattern("motif"), s.pattern("motif"));
}

#[test]
fn rule4_clips_are_sorted_after_bar_ops() {
    let mut s = base_score();
    s.layers[0].clips = vec![clip(9, 16, "motif"), clip(1, 8, "motif")];
    let a = apply_ok(
        &s,
        &patch(vec![PatchOp::ClearBars {
            layer: "piano".into(),
            bars: bars(16, 16),
        }]),
    );
    assert_eq!(
        a.score.layers[0].clips,
        vec![clip(1, 8, "motif"), clip(9, 15, "motif")]
    );
}

#[test]
fn rule4_place_clip_needs_known_pattern() {
    let s = base_score();
    let e = apply_err(
        &s,
        &patch(vec![PatchOp::PlaceClip {
            layer: "piano".into(),
            clip: clip(1, 2, "ghost"),
        }]),
    );
    assert!(matches!(e, PatchError::UnknownId { ref what, .. } if what == "pattern"));
}

/// Regression found by `patch_props::exact_inverse`: a set_clips naming a pattern that an earlier
/// op of the same patch removed, followed by a write_bars that re-creates that id, used to be
/// accepted, and its inverse then failed. Clips must name existing patterns at every op.
#[test]
fn clips_must_name_existing_patterns_at_every_op() {
    let s = base_score();
    let mut spare = s.clone();
    spare
        .patterns
        .push(pattern("spare", Grid::Eighth, 8, vec![]));
    let p = patch(vec![
        PatchOp::RemovePattern { id: "spare".into() },
        PatchOp::SetClips {
            layer: "piano".into(),
            clips: vec![clip(1, 1, "spare")],
        },
        write("piano", 2, 2, pattern("spare", Grid::Eighth, 8, vec![])),
    ]);
    let e = apply_err(&spare, &p);
    assert!(
        matches!(e, PatchError::UnknownId { index: 1, ref id, .. } if id == "spare"),
        "{e:?}"
    );
    let e = apply_err(
        &s,
        &patch(vec![PatchOp::AddLayer {
            layer: layer("x", GeneratorClass::Harmony, vec![clip(1, 1, "ghost")]),
        }]),
    );
    assert!(matches!(e, PatchError::UnknownId { ref what, .. } if what == "pattern"));
}

#[test]
fn bar_ops_outside_the_score_are_invalid() {
    let s = base_score();
    let e = apply_err(
        &s,
        &patch(vec![PatchOp::ClearBars {
            layer: "piano".into(),
            bars: bars(15, 17),
        }]),
    );
    assert!(matches!(e, PatchError::Invalid(_)), "{e:?}");
}

// ---------------------------------------------------------------- rule 5: exact inverse

#[test]
fn rule5_inverse_of_every_op_is_exact() {
    let s = base_score();
    let mut tagged = base_score();
    tagged.sections = vec![
        Section {
            id: "a".into(),
            name: String::new(),
            bars: bars(1, 8),
            role: SectionRole::Intro,
            intensity: 0.3,
        },
        Section {
            id: "b".into(),
            name: String::new(),
            bars: bars(9, 16),
            role: SectionRole::Main,
            intensity: 0.5,
        },
    ];
    let marker = |id: &str, t: f64| Marker {
        id: id.into(),
        time_s: t,
        label: String::new(),
        kind: MarkerKind::Hit,
    };
    tagged.markers = vec![marker("m1", 1.0), marker("m2", 2.0), marker("m3", 3.0)];
    let cases: Vec<(Score, Vec<PatchOp>)> = vec![
        (s.clone(), vec![PatchOp::SetTitle { title: "t".into() }]),
        (
            s.clone(),
            vec![PatchOp::SetMeta {
                meta: Meta {
                    emotion: Some("calm".into()),
                    ..Default::default()
                },
            }],
        ),
        (s.clone(), vec![PatchOp::SetTempo { bpm: 97.3 }]),
        (
            s.clone(),
            vec![PatchOp::SetTimeSignature {
                time_signature: TimeSignature {
                    numerator: 2,
                    denominator: 4,
                },
            }],
        ),
        (s.clone(), vec![PatchOp::SetBars { bars: 32 }]),
        (
            s.clone(),
            vec![PatchOp::SetPlayback {
                playback: Playback::Loop,
            }],
        ),
        (
            s.clone(),
            vec![PatchOp::SetConstraints {
                constraints: Constraints {
                    avoid: [GeneratorClass::Riser].into_iter().collect(),
                },
            }],
        ),
        (
            s.clone(),
            vec![PatchOp::SetMaster {
                master: Master {
                    frozen_gain_db: Some(-2.5),
                    ..Default::default()
                },
            }],
        ),
        (
            s.clone(),
            vec![PatchOp::SetChords {
                chords: vec![ChordSpan {
                    bars: bars(1, 16),
                    beat: 0.0,
                    symbol: "Am".into(),
                }],
            }],
        ),
        (
            s.clone(),
            vec![PatchOp::AddSection {
                section: Section {
                    id: "c".into(),
                    name: String::new(),
                    bars: bars(1, 1),
                    role: SectionRole::Peak,
                    intensity: 1.0,
                },
            }],
        ),
        (
            tagged.clone(),
            vec![PatchOp::UpdateSection {
                section: Section {
                    id: "a".into(),
                    name: "intro".into(),
                    bars: bars(1, 4),
                    role: SectionRole::Intro,
                    intensity: 0.1,
                },
            }],
        ),
        // Removing the first of two sections / the middle marker needs the reorder chain.
        (
            tagged.clone(),
            vec![PatchOp::RemoveSection { id: "a".into() }],
        ),
        (
            tagged.clone(),
            vec![PatchOp::RemoveMarker { id: "m2".into() }],
        ),
        (
            tagged.clone(),
            vec![PatchOp::AddMarker {
                marker: marker("m4", 4.0),
            }],
        ),
        (
            s.clone(),
            vec![PatchOp::AddPattern {
                pattern: melody("spare"),
            }],
        ),
        (
            s.clone(),
            vec![PatchOp::ReplacePattern {
                pattern: melody("kick"),
            }],
        ),
        (
            s.clone(),
            vec![PatchOp::AddLayer {
                layer: layer("hats", GeneratorClass::Percussion, vec![clip(1, 4, "kick")]),
            }],
        ),
        (
            s.clone(),
            vec![PatchOp::AddLayer {
                layer: {
                    let mut l = layer("held", GeneratorClass::Harmony, vec![clip(1, 4, "motif")]);
                    l.locked = true;
                    l
                },
            }],
        ),
        // Removing the first layer: the later layers (one locked) are re-added in order.
        (s.clone(), vec![PatchOp::RemoveLayer { id: "piano".into() }]),
        (
            s.clone(),
            vec![PatchOp::UpdateLayer {
                id: "kick".into(),
                update: LayerUpdate {
                    name: Some("Kick".into()),
                    gain_db: Some(-8.0),
                    pan: Some(0.25),
                    muted: Some(true),
                    seed_offset: Some(7),
                    ..Default::default()
                },
            }],
        ),
        (
            s.clone(),
            vec![PatchOp::SetLock {
                id: "kick".into(),
                locked: true,
            }],
        ),
        (
            s.clone(),
            vec![PatchOp::SetLock {
                id: "bass".into(),
                locked: false,
            }],
        ),
        (s.clone(), vec![write("piano", 3, 6, melody("motif"))]),
        (
            s.clone(),
            vec![PatchOp::ClearBars {
                layer: "kick".into(),
                bars: bars(10, 12),
            }],
        ),
        (
            s.clone(),
            vec![PatchOp::PlaceClip {
                layer: "kick".into(),
                clip: clip(1, 2, "kick"),
            }],
        ),
        (
            s.clone(),
            vec![PatchOp::SetClips {
                layer: "piano".into(),
                clips: vec![clip(2, 3, "motif"), clip(1, 1, "kick")],
            }],
        ),
    ];
    for (score, ops) in cases {
        let name = ops[0].name();
        let a = apply(&score, &patch(ops)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let back = apply(&a.score, &a.inverse).unwrap_or_else(|e| panic!("{name} inverse: {e}"));
        assert_eq!(back.score, score, "{name}");
    }
}

#[test]
fn rule5_remove_pattern_that_is_not_last() {
    let mut s = base_score();
    // An unused pattern at the front: removing it shifts every played pattern, including the
    // locked bass's; the inverse must restore the order exactly.
    s.patterns.insert(0, melody("unused"));
    let a = apply_ok(
        &s,
        &patch(vec![PatchOp::RemovePattern {
            id: "unused".into(),
        }]),
    );
    assert_eq!(a.score.patterns.len(), 3);
    assert_eq!(a.inverse.origin, Origin::User);
    // A pattern still in use cannot be removed.
    let e = apply_err(
        &s,
        &patch(vec![PatchOp::RemovePattern { id: "motif".into() }]),
    );
    assert!(matches!(e, PatchError::Invalid(_)), "{e:?}");
}

#[test]
fn rule5_multi_op_patch_inverse() {
    let s = base_score();
    let p = patch(vec![
        PatchOp::SetLock {
            id: "bass".into(),
            locked: false,
        },
        write(
            "bass",
            1,
            2,
            pattern("b2", Grid::Quarter, 4, vec![note(0, 45, 3.0)]),
        ),
        PatchOp::SetLock {
            id: "bass".into(),
            locked: true,
        },
        write("piano", 13, 16, melody("lead")),
        PatchOp::ClearBars {
            layer: "piano".into(),
            bars: bars(14, 14),
        },
        PatchOp::AddLayer {
            layer: layer("perc", GeneratorClass::Percussion, vec![clip(5, 8, "kick")]),
        },
        PatchOp::SetTempo { bpm: 101.5 },
        PatchOp::RemoveLayer { id: "kick".into() },
    ]);
    apply_ok(&s, &p);
}

// ---------------------------------------------------------------- rule 6: isolation (exact)

#[test]
fn rule6_scoped_edit_isolates_bars_and_layers() {
    let s = base_score();
    let p = scoped(
        Some((13, 16)),
        &["piano"],
        vec![
            write("piano", 13, 14, melody("lead")),
            PatchOp::ClearBars {
                layer: "piano".into(),
                bars: bars(16, 16),
            },
        ],
    );
    let a = apply_ok(&s, &p);
    for (orig, new) in s.layers.iter().zip(&a.score.layers) {
        if orig.id != "piano" {
            assert_eq!(orig, new, "layer {} outside scope", orig.id);
        }
        // 128 BPM: exact equality, not just within tolerance.
        assert_eq!(
            outside(&layer_events(&s, &orig.id), bars(13, 16)),
            outside(&layer_events(&a.score, &orig.id), bars(13, 16)),
            "layer {}",
            orig.id
        );
    }
}

// ---------------------------------------------------------------- rule 7: deterministic ids

#[test]
fn rule7_derived_pattern_ids() {
    let s = base_score();
    // The id `motif` is taken by a different pattern: the new one gets `piano_b13-16`.
    let a = apply_ok(&s, &patch(vec![write("piano", 13, 16, melody("motif"))]));
    assert!(a.score.pattern("piano_b13-16").is_some());
    assert_eq!(a.score.pattern("motif"), s.pattern("motif"));
    assert_eq!(a.score.layers[0].clips[1].pattern, "piano_b13-16");
    // A second, different pattern for the same bars: numeric suffix.
    let mut other = melody("motif");
    other.notes[0].pitch = 69;
    let b = apply_ok(
        &a.score,
        &patch(vec![write("piano", 13, 16, other.clone())]),
    );
    assert_eq!(b.score.layers[0].clips[1].pattern, "piano_b13-16_2");
    // The same content again reuses the existing derived pattern instead of copying it.
    let c = apply_ok(&b.score, &patch(vec![write("piano", 13, 16, other)]));
    assert_eq!(c.score.patterns.len(), b.score.patterns.len());
    // Determinism: same input, same output.
    assert_eq!(
        apply(&s, &patch(vec![write("piano", 13, 16, melody("motif"))])).unwrap(),
        a
    );
    // An identical pattern under its own id is reused, not duplicated.
    let same = apply_ok(
        &s,
        &patch(vec![write("piano", 13, 16, s.patterns[0].clone())]),
    );
    assert_eq!(
        same.score, s,
        "writing the same motif at its own phase is a no-op"
    );
    // An empty id is derived too.
    let d = apply_ok(&s, &patch(vec![write("kick", 9, 9, melody(""))]));
    assert!(d.score.pattern("kick_b9-9").is_some());
}

// ---------------------------------------------------------------- set_key

fn d_minor() -> Key {
    Key {
        tonic: PitchClass::D,
        mode: Mode::Minor,
    }
}

#[test]
fn set_key_transposes_pitched_patterns_only() {
    let mut s = base_score();
    s.layers[2].locked = false;
    let a = apply_ok(
        &s,
        &patch(vec![PatchOp::SetKey {
            key: d_minor(),
            transpose_notes: true,
        }]),
    );
    // A -> D: +5 semitones (smaller than -7).
    let up = |id: &str| -> Vec<u8> {
        a.score
            .pattern(id)
            .unwrap()
            .notes
            .iter()
            .map(|n| n.pitch)
            .collect()
    };
    let was = |id: &str| -> Vec<u8> {
        s.pattern(id)
            .unwrap()
            .notes
            .iter()
            .map(|n| n.pitch)
            .collect()
    };
    assert_eq!(
        up("motif"),
        was("motif").iter().map(|p| p + 5).collect::<Vec<_>>()
    );
    assert_eq!(
        up("bassline"),
        was("bassline").iter().map(|p| p + 5).collect::<Vec<_>>()
    );
    assert_eq!(up("kick"), was("kick"), "drums untouched");
    assert_eq!(a.score.key, d_minor());
    // Without transposition the notes stay (the test pitches A C D E G are all in D minor).
    let b = apply_ok(
        &s,
        &patch(vec![PatchOp::SetKey {
            key: d_minor(),
            transpose_notes: false,
        }]),
    );
    assert_eq!(b.score.patterns, s.patterns);
    // ... and notes that leave the new key make the patch invalid (F# minor has no C, D or G).
    let fs_minor = Key {
        tonic: PitchClass::Fs,
        mode: Mode::Minor,
    };
    let e = apply_err(
        &s,
        &patch(vec![PatchOp::SetKey {
            key: fs_minor,
            transpose_notes: false,
        }]),
    );
    assert!(
        matches!(e, PatchError::Invalid(ref m) if m.contains("OffScale")),
        "{e:?}"
    );
}

#[test]
fn set_key_smallest_interval_goes_down_and_tritone_round_trips() {
    let mut s = base_score();
    s.layers[2].locked = false;
    // A -> E: -5 (smaller than +7).
    let e_minor = Key {
        tonic: PitchClass::E,
        mode: Mode::Minor,
    };
    let a = apply_ok(
        &s,
        &patch(vec![PatchOp::SetKey {
            key: e_minor,
            transpose_notes: true,
        }]),
    );
    assert_eq!(
        a.score.patterns[0].notes[0].pitch + 5,
        s.patterns[0].notes[0].pitch
    );
    // A -> Eb: tritone, +6; the inverse restores the exact notes (not an octave off).
    let eb_minor = Key {
        tonic: PitchClass::Eb,
        mode: Mode::Minor,
    };
    let b = apply_ok(
        &s,
        &patch(vec![PatchOp::SetKey {
            key: eb_minor,
            transpose_notes: true,
        }]),
    );
    assert_eq!(
        b.score.patterns[0].notes[0].pitch,
        s.patterns[0].notes[0].pitch + 6
    );
}

#[test]
fn set_key_pattern_shared_with_drums_uses_clip_transpose() {
    let mut s = base_score();
    s.layers[2].locked = false;
    // The drum layer and a pitched layer share the pattern `kick` (A2 = 45, in key).
    s.patterns[1].notes.iter_mut().for_each(|n| n.pitch = 45);
    s.layers.push(layer(
        "pluck",
        GeneratorClass::Arp,
        vec![clip(1, 4, "kick")],
    ));
    let a = apply_ok(
        &s,
        &patch(vec![PatchOp::SetKey {
            key: d_minor(),
            transpose_notes: true,
        }]),
    );
    assert_eq!(
        a.score.pattern("kick"),
        s.pattern("kick"),
        "drum pattern unchanged"
    );
    assert_eq!(a.score.layer("pluck").unwrap().clips[0].transpose, 5);
    assert_eq!(a.score.layer("kick").unwrap(), s.layer("kick").unwrap());
}

// ---------------------------------------------------------------- misc op checks

#[test]
fn duplicate_and_unknown_ids() {
    let s = base_score();
    let dup = |op: PatchOp| apply_err(&s, &patch(vec![op]));
    assert!(matches!(
        dup(PatchOp::AddPattern { pattern: melody("motif") }),
        PatchError::DuplicateId { ref what, .. } if what == "pattern"
    ));
    assert!(matches!(
        dup(PatchOp::AddLayer { layer: layer("piano", GeneratorClass::Motif, vec![]) }),
        PatchError::DuplicateId { ref what, .. } if what == "layer"
    ));
    assert!(matches!(
        dup(PatchOp::RemoveSection { id: "x".into() }),
        PatchError::UnknownId { ref what, .. } if what == "section"
    ));
    assert!(matches!(
        dup(PatchOp::RemoveMarker { id: "x".into() }),
        PatchError::UnknownId { ref what, .. } if what == "marker"
    ));
    assert!(matches!(
        dup(PatchOp::ReplacePattern { pattern: melody("x") }),
        PatchError::UnknownId { ref what, .. } if what == "pattern"
    ));
    assert!(matches!(
        dup(PatchOp::SetLock { id: "x".into(), locked: true }),
        PatchError::UnknownId { ref what, .. } if what == "layer"
    ));
}

#[test]
fn sections_and_markers_respect_bar_scope() {
    let mut s = base_score();
    s.sections = vec![Section {
        id: "a".into(),
        name: String::new(),
        bars: bars(1, 8),
        role: SectionRole::Intro,
        intensity: 0.5,
    }];
    let sec = |id: &str, a: u32, b: u32| Section {
        id: id.into(),
        name: String::new(),
        bars: bars(a, b),
        role: SectionRole::Main,
        intensity: 0.5,
    };
    assert!(matches!(
        apply_err(
            &s,
            &scoped(
                Some((9, 16)),
                &[],
                vec![PatchOp::RemoveSection { id: "a".into() }]
            )
        ),
        PatchError::OutOfScope { .. }
    ));
    apply_ok(
        &s,
        &scoped(
            Some((9, 16)),
            &[],
            vec![PatchOp::AddSection {
                section: sec("b", 9, 16),
            }],
        ),
    );
    assert!(matches!(
        apply_err(
            &s,
            &scoped(
                Some((9, 16)),
                &[],
                vec![PatchOp::AddSection {
                    section: sec("b", 8, 16)
                }]
            )
        ),
        PatchError::OutOfScope { .. }
    ));
    // Bar 9 starts at 8 * 1.875 s = 15 s at 128 BPM.
    let m = |t: f64| PatchOp::AddMarker {
        marker: Marker {
            id: "hit".into(),
            time_s: t,
            label: String::new(),
            kind: MarkerKind::Hit,
        },
    };
    apply_ok(&s, &scoped(Some((9, 16)), &[], vec![m(15.0)]));
    assert!(matches!(
        apply_err(&s, &scoped(Some((9, 16)), &[], vec![m(14.9)])),
        PatchError::OutOfScope { .. }
    ));
    // Chords: spans outside the scope must stay.
    let chord = |a: u32, b: u32, sym: &str| ChordSpan {
        bars: bars(a, b),
        beat: 0.0,
        symbol: sym.into(),
    };
    s.chords = vec![chord(1, 8, "Am"), chord(9, 16, "F")];
    apply_ok(
        &s,
        &scoped(
            Some((9, 16)),
            &[],
            vec![PatchOp::SetChords {
                chords: vec![chord(1, 8, "Am"), chord(9, 16, "G")],
            }],
        ),
    );
    assert!(matches!(
        apply_err(
            &s,
            &scoped(
                Some((9, 16)),
                &[],
                vec![PatchOp::SetChords {
                    chords: vec![chord(9, 16, "G")]
                }]
            )
        ),
        PatchError::OutOfScope { .. }
    ));
}
