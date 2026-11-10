//! Variations (FR-8): N alternates of a selection, each with its own seed, differ from each other
//! and from the original only inside the selection, undo exactly and never touch locked layers.

mod common;

use common::SR;
use praxis_planner::{Catalogue, EditError, RuleProvider, plan_brief, variations};
use praxis_score::apply::apply;
use praxis_score::lint::lint;
use praxis_score::patch::{Origin, Scope};
use praxis_score::timeline::{NoteEvent, expand_notes};
use praxis_score::validate::validate;
use praxis_score::{BarRange, Score};
use std::collections::BTreeSet;

const BRIEF: &str = "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt \
piano playing a repeating rhythmic motif on an eighth-note grid with accents and overlapping \
sustain. Bars 9-16: the identical piano part plus a soft muted kick underneath. No new melody, \
no fills, no risers, sweeps, impacts or pads.";

fn acceptance1() -> Score {
    plan_brief(BRIEF, &RuleProvider::new(), &Catalogue::standard(), 1)
        .expect("plan")
        .score
}

fn events(score: &Score, id: &str, keep: impl Fn(u32) -> bool) -> Vec<NoteEvent> {
    let li = score.layers.iter().position(|l| l.id == id).expect("layer");
    expand_notes(score, SR)
        .into_iter()
        .filter(|e| e.layer == li && keep(e.bar))
        .collect()
}

fn check_set(s: &Score, scope: &Scope, n: usize, seed: u64) {
    let vs = variations(s, scope, n, seed).expect("variations");
    assert_eq!(vs.len(), n);
    let bars = scope.bars.unwrap_or(BarRange {
        start: 1,
        end: s.bars,
    });
    let inside = |b: u32| b >= bars.start && b <= bars.end;
    let seeds: BTreeSet<u64> = vs.iter().map(|v| v.seed).collect();
    assert_eq!(seeds.len(), n, "each variation has its own seed");
    let mut seen = Vec::new();
    for v in &vs {
        println!("variation {}: {}", v.index, v.summary);
        assert_eq!(v.patch.origin, Origin::Model);
        assert_eq!(v.patch.scope.bars, scope.bars);
        assert!(validate(&v.score).is_valid());
        let lints = lint(
            &v.score,
            &praxis_planner::brief::lint_context(
                &v.score,
                &praxis_planner::plan::PlanContext {
                    catalogue: &Catalogue::standard(),
                    requested: &BTreeSet::new(),
                },
            ),
        );
        assert!(lints.is_empty(), "{lints:?}");
        for l in &s.layers {
            let selected = scope.layers.is_empty() || scope.layers.contains(&l.id);
            if !selected {
                assert_eq!(v.score.layer(&l.id), Some(l), "unselected {} changed", l.id);
                continue;
            }
            // Identical outside the selection.
            assert_eq!(
                events(s, &l.id, |b| !inside(b)),
                events(&v.score, &l.id, |b| !inside(b)),
                "{} changed outside the selection",
                l.id
            );
        }
        // Different inside it, from the original and from every other variation.
        let sel: Vec<Vec<NoteEvent>> = scope
            .layers
            .iter()
            .map(|id| events(&v.score, id, inside))
            .collect();
        let orig: Vec<Vec<NoteEvent>> = scope
            .layers
            .iter()
            .map(|id| events(s, id, inside))
            .collect();
        assert_ne!(sel, orig, "variation {} equals the original", v.index);
        assert!(
            !seen.contains(&sel),
            "variation {} repeats another",
            v.index
        );
        seen.push(sel);
        assert_eq!(apply(&v.score, &v.inverse).expect("undo").score, *s);
    }
    // Deterministic.
    assert_eq!(variations(s, scope, n, seed).expect("again"), vs);
}

#[test]
fn piano_variations_in_the_last_four_bars() {
    let s = acceptance1();
    let scope = Scope {
        bars: Some(BarRange { start: 13, end: 16 }),
        layers: vec!["piano".into()],
    };
    check_set(&s, &scope, 4, 11);
}

#[test]
fn kick_variations_in_the_second_half() {
    let s = acceptance1();
    let scope = Scope {
        bars: Some(BarRange { start: 9, end: 16 }),
        layers: vec!["kick".into()],
    };
    check_set(&s, &scope, 3, 2);
}

#[test]
fn whole_score_variations_get_new_seed_offsets() {
    let s = acceptance1();
    let scope = Scope {
        bars: None,
        layers: vec!["piano".into()],
    };
    check_set(&s, &scope, 2, 5);
    let vs = variations(&s, &scope, 2, 5).expect("variations");
    let offsets: BTreeSet<u64> = vs
        .iter()
        .map(|v| v.score.layer("piano").expect("piano").seed_offset)
        .collect();
    assert_eq!(offsets.len(), 2);
    assert!(!offsets.contains(&s.layer("piano").expect("piano").seed_offset));
}

#[test]
fn locked_layers_are_never_varied() {
    let mut s = acceptance1();
    s.layers[0].locked = true; // piano
    let scope = Scope {
        bars: Some(BarRange { start: 9, end: 16 }),
        layers: vec!["piano".into()],
    };
    assert!(matches!(
        variations(&s, &scope, 2, 0),
        Err(EditError::Invalid { .. })
    ));
    // With no layers named, only the unlocked kick varies.
    let scope = Scope {
        bars: Some(BarRange { start: 9, end: 16 }),
        layers: vec![],
    };
    let vs = variations(&s, &scope, 2, 0).expect("variations");
    for v in &vs {
        assert_eq!(v.patch.scope.layers, ["kick"]);
        assert_eq!(v.score.layer("piano"), s.layer("piano"));
        assert!(v.summary.contains("locked"), "{}", v.summary);
    }
    assert!(variations(&s, &scope, 0, 0).expect("none").is_empty());
}
