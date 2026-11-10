//! Property tests for patches over generated scores: exact inverse (rule 5), scope rejection
//! (rule 3), lock rejection (rule 2) and isolation (rule 6).
//!
//! Generated scores: 4-16 bars, 4/4, 3/4 or 6/8, tempo anywhere in 60-180 BPM (so float paths
//! are exercised), 1-3 patterns on any grid (1 or 2 bars long, micro-timing offsets), 1-3 layers
//! (motif, bass, drums, harmony; about a third locked) with non-overlapping clips at any phase,
//! transpose and velocity scale.

#[path = "patch_common.rs"]
mod common;

use common::*;
use praxis_score::apply::apply;
use praxis_score::patch::{LayerUpdate, Origin, PatchError, PatchOp, Scope, ScorePatch};
use praxis_score::*;
use proptest::collection::vec;
use proptest::prelude::*;
use std::cell::Cell;

const GRIDS: [Grid; 5] = [
    Grid::Quarter,
    Grid::Eighth,
    Grid::Sixteenth,
    Grid::EighthTriplet,
    Grid::SixteenthTriplet,
];
const TIME_SIGNATURES: [(u8, u8); 3] = [(4, 4), (3, 4), (6, 8)];
const CLASSES: [GeneratorClass; 4] = [
    GeneratorClass::Motif,
    GeneratorClass::Bass,
    GeneratorClass::Drums,
    GeneratorClass::Harmony,
];
const TRANSPOSE: [i8; 3] = [-12, 0, 12];

#[derive(Clone, Debug)]
struct RawNote {
    step: u32,
    pitch: usize,
    velocity: f32,
    length: f32,
    offset: i8,
}

#[derive(Clone, Debug)]
struct RawPattern {
    grid: usize,
    bars: u32,
    notes: Vec<RawNote>,
}

#[derive(Clone, Debug)]
struct RawClip {
    gap: u32,
    len: u32,
    pattern: usize,
    offset: u32,
    transpose: usize,
    scale: f32,
}

#[derive(Clone, Debug)]
struct RawLayer {
    class: usize,
    locked: bool,
    clips: Vec<RawClip>,
}

fn raw_note() -> impl Strategy<Value = RawNote> {
    (0u32..1000, 0usize..6, 0.1f32..1.0, 0.25f32..4.0, -2i8..=2).prop_map(
        |(step, pitch, velocity, length, offset)| RawNote {
            step,
            pitch,
            velocity,
            length,
            offset,
        },
    )
}

fn raw_pattern() -> impl Strategy<Value = RawPattern> {
    (0usize..5, 1u32..=2, vec(raw_note(), 0..6)).prop_map(|(grid, bars, notes)| RawPattern {
        grid,
        bars,
        notes,
    })
}

fn raw_layer() -> impl Strategy<Value = RawLayer> {
    let raw_clip = (
        0u32..3,
        1u32..5,
        0usize..3,
        0u32..1000,
        0usize..3,
        0.5f32..1.5,
    )
        .prop_map(|(gap, len, pattern, offset, transpose, scale)| RawClip {
            gap,
            len,
            pattern,
            offset,
            transpose,
            scale,
        });
    (0usize..4, prop::bool::weighted(0.35), vec(raw_clip, 0..4)).prop_map(
        |(class, locked, clips)| RawLayer {
            class,
            locked,
            clips,
        },
    )
}

fn build_pattern(id: &str, ts: TimeSignature, raw: &RawPattern) -> Pattern {
    let grid = GRIDS[raw.grid % GRIDS.len()];
    let spb = ts
        .steps_per_bar(grid)
        .expect("every grid fits the generated meters");
    let len = spb * raw.bars;
    Pattern {
        id: id.into(),
        grid,
        length_steps: len,
        notes: raw
            .notes
            .iter()
            .map(|n| Note {
                step: n.step % len,
                pitch: A_MINOR[n.pitch % A_MINOR.len()],
                velocity: n.velocity,
                accent: Accent::Normal,
                length: n.length,
                offset: n.offset as f32 * 0.1,
            })
            .collect(),
        spoken: None,
    }
}

fn arb_score() -> impl Strategy<Value = Score> {
    (
        4u32..=16,
        0usize..3,
        60.0f64..180.0,
        vec(raw_pattern(), 1..=3),
        vec(raw_layer(), 1..=3),
        any::<bool>(),
    )
        .prop_map(|(bars, ts, bpm, pats, layers, looped)| {
            let mut s = Score::empty(bars, bpm);
            s.key = Key {
                tonic: PitchClass::A,
                mode: Mode::Minor,
            };
            let (numerator, denominator) = TIME_SIGNATURES[ts];
            s.time_signature = TimeSignature {
                numerator,
                denominator,
            };
            if looped {
                s.playback = Playback::Loop;
            }
            s.patterns = pats
                .iter()
                .enumerate()
                .map(|(i, p)| build_pattern(&format!("p{i}"), s.time_signature, p))
                .collect();
            for (li, rl) in layers.iter().enumerate() {
                let mut l = layer(&format!("l{li}"), CLASSES[rl.class], vec![]);
                l.locked = rl.locked;
                let mut cursor = 1;
                for rc in &rl.clips {
                    let start = cursor + rc.gap;
                    if start > bars {
                        break;
                    }
                    let end = (start + rc.len - 1).min(bars);
                    let p = &s.patterns[rc.pattern % s.patterns.len()];
                    l.clips.push(Clip {
                        bars: BarRange { start, end },
                        pattern: p.id.clone(),
                        pattern_offset_steps: rc.offset % p.length_steps,
                        transpose: TRANSPOSE[rc.transpose],
                        velocity_scale: rc.scale,
                    });
                    cursor = end + 1;
                }
                s.layers.push(l);
            }
            s
        })
        .prop_filter("generated score validates", |s| {
            validate::validate(s).is_valid()
        })
}

#[derive(Clone, Debug)]
enum RawOp {
    Write {
        layer: usize,
        a: u32,
        b: u32,
        pat: RawPattern,
        keep_id: bool,
    },
    Clear {
        layer: usize,
        a: u32,
        b: u32,
    },
    Place {
        layer: usize,
        a: u32,
        b: u32,
        pattern: usize,
        offset: u32,
    },
    SetClipsVia {
        layer: usize,
        a: u32,
        b: u32,
        pattern: usize,
        offset: u32,
    },
    Rename {
        layer: usize,
    },
    Gain {
        layer: usize,
        db: f32,
    },
    Lock {
        layer: usize,
        locked: bool,
    },
    AddPattern {
        pat: RawPattern,
    },
    ReplacePattern {
        pattern: usize,
        pat: RawPattern,
    },
    RemovePattern {
        pattern: usize,
    },
    AddLayer {
        class: usize,
        a: u32,
        b: u32,
        pattern: usize,
        locked: bool,
    },
    RemoveLayer {
        layer: usize,
    },
    Tempo {
        bpm: f64,
    },
    Key {
        tonic: usize,
        transpose: bool,
    },
}

fn raw_op() -> impl Strategy<Value = RawOp> {
    let ix = 0usize..8;
    let bar = 0u32..64;
    prop_oneof![
        3 => (ix.clone(), bar.clone(), bar.clone(), raw_pattern(), any::<bool>())
            .prop_map(|(layer, a, b, pat, keep_id)| RawOp::Write { layer, a, b, pat, keep_id }),
        2 => (ix.clone(), bar.clone(), bar.clone()).prop_map(|(layer, a, b)| RawOp::Clear { layer, a, b }),
        2 => (ix.clone(), bar.clone(), bar.clone(), ix.clone(), 0u32..1000)
            .prop_map(|(layer, a, b, pattern, offset)| RawOp::Place { layer, a, b, pattern, offset }),
        2 => (ix.clone(), bar.clone(), bar.clone(), ix.clone(), 0u32..1000)
            .prop_map(|(layer, a, b, pattern, offset)| RawOp::SetClipsVia { layer, a, b, pattern, offset }),
        1 => ix.clone().prop_map(|layer| RawOp::Rename { layer }),
        1 => (ix.clone(), -12.0f32..0.0).prop_map(|(layer, db)| RawOp::Gain { layer, db }),
        1 => (ix.clone(), any::<bool>()).prop_map(|(layer, locked)| RawOp::Lock { layer, locked }),
        1 => raw_pattern().prop_map(|pat| RawOp::AddPattern { pat }),
        1 => (ix.clone(), raw_pattern()).prop_map(|(pattern, pat)| RawOp::ReplacePattern { pattern, pat }),
        1 => ix.clone().prop_map(|pattern| RawOp::RemovePattern { pattern }),
        1 => (0usize..4, bar.clone(), bar.clone(), ix.clone(), any::<bool>())
            .prop_map(|(class, a, b, pattern, locked)| RawOp::AddLayer { class, a, b, pattern, locked }),
        1 => ix.clone().prop_map(|layer| RawOp::RemoveLayer { layer }),
        1 => (60.0f64..180.0).prop_map(|bpm| RawOp::Tempo { bpm }),
        1 => (0usize..12, any::<bool>()).prop_map(|(tonic, transpose)| RawOp::Key { tonic, transpose }),
    ]
}

/// Maps two raw numbers to an ordered bar range inside `lo..=hi`.
fn range_in(a: u32, b: u32, lo: u32, hi: u32) -> BarRange {
    let span = hi - lo + 1;
    let (x, y) = (lo + a % span, lo + b % span);
    BarRange {
        start: x.min(y),
        end: x.max(y),
    }
}

/// Builds a concrete op against `s`. `layers` are the candidate layer ids for layer ops and
/// `lo..=hi` bounds the bar ranges.
fn build_op(
    s: &Score,
    raw: &RawOp,
    layers: &[String],
    lo: u32,
    hi: u32,
    n: usize,
) -> Option<PatchOp> {
    let pick = |i: usize| layers.get(i % layers.len().max(1)).cloned();
    let pat_id = |i: usize| s.patterns[i % s.patterns.len()].id.clone();
    let place_clip = |a, b, pattern: usize, offset: u32| {
        let p = &s.patterns[pattern % s.patterns.len()];
        Clip {
            bars: range_in(a, b, lo, hi),
            pattern: p.id.clone(),
            pattern_offset_steps: offset % p.length_steps,
            transpose: 0,
            velocity_scale: 1.0,
        }
    };
    Some(match raw {
        RawOp::Write {
            layer,
            a,
            b,
            pat,
            keep_id,
        } => {
            let id = if *keep_id { pat_id(n) } else { format!("w{n}") };
            PatchOp::WriteBars {
                layer: pick(*layer)?,
                bars: range_in(*a, *b, lo, hi),
                pattern: build_pattern(&id, s.time_signature, pat),
            }
        }
        RawOp::Clear { layer, a, b } => PatchOp::ClearBars {
            layer: pick(*layer)?,
            bars: range_in(*a, *b, lo, hi),
        },
        RawOp::Place {
            layer,
            a,
            b,
            pattern,
            offset,
        } => PatchOp::PlaceClip {
            layer: pick(*layer)?,
            clip: place_clip(*a, *b, *pattern, *offset),
        },
        RawOp::SetClipsVia {
            layer,
            a,
            b,
            pattern,
            offset,
        } => {
            let id = pick(*layer)?;
            let mut via = patch(vec![PatchOp::PlaceClip {
                layer: id.clone(),
                clip: place_clip(*a, *b, *pattern, *offset),
            }]);
            via.origin = Origin::User;
            let placed = apply(s, &via).ok()?;
            PatchOp::SetClips {
                clips: placed.score.layer(&id)?.clips.clone(),
                layer: id,
            }
        }
        RawOp::Rename { layer } => PatchOp::UpdateLayer {
            id: pick(*layer)?,
            update: LayerUpdate {
                name: Some(format!("renamed {n}")),
                ..Default::default()
            },
        },
        RawOp::Gain { layer, db } => PatchOp::UpdateLayer {
            id: pick(*layer)?,
            update: LayerUpdate {
                gain_db: Some(*db),
                ..Default::default()
            },
        },
        RawOp::Lock { layer, locked } => PatchOp::SetLock {
            id: pick(*layer)?,
            locked: *locked,
        },
        RawOp::AddPattern { pat } => PatchOp::AddPattern {
            pattern: build_pattern(&format!("new{n}"), s.time_signature, pat),
        },
        RawOp::ReplacePattern { pattern, pat } => PatchOp::ReplacePattern {
            pattern: build_pattern(&pat_id(*pattern), s.time_signature, pat),
        },
        RawOp::RemovePattern { pattern } => PatchOp::RemovePattern {
            id: pat_id(*pattern),
        },
        RawOp::AddLayer {
            class,
            a,
            b,
            pattern,
            locked,
        } => {
            let mut l = layer(
                &format!("added{n}"),
                CLASSES[*class],
                vec![place_clip(*a, *b, *pattern, 0)],
            );
            l.locked = *locked;
            PatchOp::AddLayer { layer: l }
        }
        RawOp::RemoveLayer { layer } => PatchOp::RemoveLayer { id: pick(*layer)? },
        RawOp::Tempo { bpm } => PatchOp::SetTempo { bpm: *bpm },
        RawOp::Key { tonic, transpose } => PatchOp::SetKey {
            key: Key {
                tonic: PitchClass::ALL[*tonic],
                mode: Mode::Minor,
            },
            transpose_notes: *transpose,
        },
    })
}

fn unscoped_patch(s: &Score, raws: &[RawOp]) -> ScorePatch {
    let ids: Vec<String> = s.layers.iter().map(|l| l.id.clone()).collect();
    let ops = raws
        .iter()
        .enumerate()
        .filter_map(|(n, r)| build_op(s, r, &ids, 1, s.bars, n))
        .collect();
    patch(ops)
}

/// Asserts the exact-inverse rule for an accepted patch.
fn check_inverse(
    s: &Score,
    p: &ScorePatch,
    a: &praxis_score::patch::Applied,
) -> Result<(), TestCaseError> {
    let back = apply(&a.score, &a.inverse);
    prop_assert!(
        back.is_ok(),
        "inverse rejected: {:?}\n{:#?}",
        back.err(),
        a.inverse
    );
    let back = back.unwrap();
    prop_assert_eq!(&back.score, s, "inverse is not exact for {:#?}", p);
    Ok(())
}

proptest! {
    // 1024 cases per property run in well under a second (scores are small); PROPTEST_CASES
    // overrides it for deeper runs (20 000 per property pass).
    #![proptest_config(ProptestConfig { cases: 1024, ..ProptestConfig::default() })]

    /// Rule 5 for arbitrary unscoped patches of 1-4 ops (any op kind, possibly conflicting).
    #[test]
    fn exact_inverse(s in arb_score(), raws in vec(raw_op(), 1..=4)) {
        let p = unscoped_patch(&s, &raws);
        if let Ok(a) = apply(&s, &p) {
            prop_assert!(validate::validate(&a.score).is_valid());
            check_inverse(&s, &p, &a)?;
            // Determinism: the same patch on the same score gives the same result.
            prop_assert_eq!(apply(&s, &p), Ok(a));
        }
    }

    /// Rule 6 (and 3, 4, 5) for patches built inside a random scope: they are all accepted, layers
    /// outside the scope are structurally identical, and every bar outside the scope expands to
    /// the same events.
    #[test]
    fn scoped_patches_are_isolated(
        s in arb_score(),
        raws in vec(raw_op(), 1..=4),
        (ra, rb) in (0u32..64, 0u32..64),
        mask in 0u8..8,
    ) {
        let range = range_in(ra, rb, 1, s.bars);
        let scope_layers: Vec<String> = s.layers.iter().enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, l)| l.id.clone())
            .collect();
        let targets: Vec<String> = s.layers.iter()
            .filter(|l| !l.locked && (scope_layers.is_empty() || scope_layers.contains(&l.id)))
            .map(|l| l.id.clone())
            .collect();
        let ops: Vec<PatchOp> = raws.iter().enumerate()
            .filter(|(_, r)| matches!(r, RawOp::Write { .. } | RawOp::Clear { .. } | RawOp::Place { .. }
                | RawOp::SetClipsVia { .. } | RawOp::Rename { .. } | RawOp::AddPattern { .. }))
            .filter_map(|(n, r)| build_op(&s, r, &targets, range.start, range.end, n))
            .collect();
        let p = ScorePatch {
            description: "scoped".into(),
            origin: Origin::Model,
            scope: Scope { bars: Some(range), layers: scope_layers.clone() },
            ops,
        };
        let a = apply(&s, &p);
        prop_assert!(a.is_ok(), "in-scope patch rejected: {:?}\n{:#?}", a.as_ref().err(), p);
        let a = a.unwrap();
        for l in &s.layers {
            let in_layers = scope_layers.is_empty() || scope_layers.contains(&l.id);
            if !in_layers {
                prop_assert_eq!(a.score.layer(&l.id), Some(l));
            }
            assert_same_events(
                &outside(&layer_events(&s, &l.id), range),
                &outside(&layer_events(&a.score, &l.id), range),
                &format!("layer {} outside bars {}-{}", l.id, range.start, range.end),
            );
        }
        check_inverse(&s, &p, &a)?;
    }

    /// Rule 3: a bar op reaching one bar past the scope, a layer op on a layer outside
    /// scope.layers, and any global op in a scoped patch are rejected with OutOfScope.
    #[test]
    fn out_of_scope_ops_are_rejected(
        s in arb_score(),
        (ra, rb) in (0u32..64, 0u32..64),
        which in 0usize..3,
        grow_left in any::<bool>(),
        pat in raw_pattern(),
    ) {
        // Cases without an unlocked layer, or whose scope covers the whole score, have nothing to
        // test; they return early instead of rejecting (too many rejects abort proptest).
        let unlocked: Vec<&Layer> = s.layers.iter().filter(|l| !l.locked).collect();
        let Some(first) = unlocked.first() else {
            return Ok(());
        };
        let target = first.id.clone();
        // A scope strictly inside the score, so one more bar on some side is still in the score.
        let range = range_in(ra, rb, 1, s.bars);
        if range.start == 1 && range.end == s.bars {
            return Ok(());
        }
        let wider = if (grow_left && range.start > 1) || range.end == s.bars {
            BarRange { start: range.start - 1, end: range.end }
        } else {
            BarRange { start: range.start, end: range.end + 1 }
        };
        let op = match which {
            0 => PatchOp::WriteBars {
                layer: target.clone(),
                bars: wider,
                pattern: build_pattern("x", s.time_signature, &pat),
            },
            1 => PatchOp::ClearBars { layer: target.clone(), bars: wider },
            _ => PatchOp::PlaceClip { layer: target.clone(), clip: Clip {
                bars: wider,
                pattern: s.patterns[0].id.clone(),
                pattern_offset_steps: 0,
                transpose: 0,
                velocity_scale: 1.0,
            }},
        };
        let p = scoped(Some((range.start, range.end)), &[], vec![op]);
        let is_scope_error = matches!(apply(&s, &p), Err(PatchError::OutOfScope { .. }));
        prop_assert!(is_scope_error);

        // Layer scope: name a different layer (it may be locked; locks are checked first).
        if let Some(other) = s.layers.iter().find(|l| l.id != target && !l.locked) {
            let p = scoped(None, &[&target], vec![PatchOp::ClearBars {
                layer: other.id.clone(),
                bars: BarRange { start: 1, end: 1 },
            }]);
            let is_scope_error = matches!(apply(&s, &p), Err(PatchError::OutOfScope { .. }));
            prop_assert!(is_scope_error);
        }

        // Global ops need an unscoped patch.
        let p = scoped(Some((range.start, range.end)), &[], vec![PatchOp::SetTitle { title: "t".into() }]);
        let is_scope_error = matches!(apply(&s, &p), Err(PatchError::OutOfScope { .. }));
        prop_assert!(is_scope_error);
    }

    /// Rule 2: every op that changes a locked layer, or a pattern it plays, fails with LockedLayer,
    /// whatever the origin and scope.
    #[test]
    fn locked_layers_reject_changes(
        s in arb_score(),
        raw in raw_op(),
        origin in prop::sample::select(vec![Origin::User, Origin::Model, Origin::System]),
    ) {
        let Some(locked) = s.layers.iter().find(|l| l.locked) else {
            return Ok(());
        };
        let ids = vec![locked.id.clone()];
        let layer_op = matches!(raw, RawOp::Write { .. } | RawOp::Clear { .. } | RawOp::Place { .. }
            | RawOp::SetClipsVia { .. } | RawOp::Rename { .. } | RawOp::Gain { .. } | RawOp::RemoveLayer { .. });
        if layer_op {
            // SetClipsVia needs an unlocked layer to compute its clips; use the layer's own clips.
            let op = match raw {
                RawOp::SetClipsVia { .. } => Some(PatchOp::SetClips { layer: locked.id.clone(), clips: vec![] }),
                _ => build_op(&s, &raw, &ids, 1, s.bars, 0),
            };
            if let Some(op) = op {
                let mut p = patch(vec![op]);
                p.origin = origin;
                prop_assert_eq!(apply(&s, &p).err(), Some(PatchError::LockedLayer(locked.id.clone())));
            }
        }
        // Replacing a pattern the locked layer plays.
        if let Some(c) = locked.clips.first() {
            let mut p = patch(vec![PatchOp::ReplacePattern {
                pattern: build_pattern(&c.pattern, s.time_signature, &RawPattern { grid: 1, bars: 1, notes: vec![] }),
            }]);
            p.origin = origin;
            let is_lock_error = matches!(apply(&s, &p), Err(PatchError::LockedLayer(_)));
            prop_assert!(is_lock_error);
        }
    }
}

/// The random-patch property is only meaningful when a good share of patches is accepted.
#[test]
fn generated_patches_are_mostly_accepted() {
    use proptest::test_runner::{Config, TestRunner};
    let mut runner = TestRunner::new(Config {
        cases: 300,
        ..Config::default()
    });
    let accepted = Cell::new(0u32);
    let total = Cell::new(0u32);
    runner
        .run(&(arb_score(), vec(raw_op(), 1..=2)), |(s, raws)| {
            total.set(total.get() + 1);
            if apply(&s, &unscoped_patch(&s, &raws)).is_ok() {
                accepted.set(accepted.get() + 1);
            }
            Ok(())
        })
        .expect("runner");
    let (a, t) = (accepted.get(), total.get());
    assert!(
        a * 3 >= t,
        "only {a} of {t} generated patches were accepted"
    );
}
