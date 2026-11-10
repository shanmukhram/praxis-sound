//! Phase 4 exit (FR-7): on the acceptance-1 Score, "change the last four bars to add a subtle
//! melody" changes only bars 13-16. Every layer's expanded note events in bars 1-12 are identical
//! before and after, existing layers are structurally untouched, and the patch undoes exactly.

mod common;

use common::SR;
use praxis_planner::edit::{EditIntent, plan_edit};
use praxis_planner::{Catalogue, RuleProvider, plan_brief};
use praxis_score::apply::apply;
use praxis_score::patch::Origin;
use praxis_score::timeline::{NoteEvent, expand_notes};
use praxis_score::validate::validate;
use praxis_score::{BarRange, Score};

const BRIEF: &str = "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt \
piano playing a repeating rhythmic motif on an eighth-note grid with accents and overlapping \
sustain. Bars 9-16: the identical piano part plus a soft muted kick underneath. No new melody, \
no fills, no risers, sweeps, impacts or pads.";

fn acceptance1() -> Score {
    plan_brief(BRIEF, &RuleProvider::new(), &Catalogue::standard(), 1)
        .expect("plan")
        .score
}

/// Note events of layer `id` starting in bars `bars`.
fn events(score: &Score, id: &str, bars: BarRange) -> Vec<NoteEvent> {
    let li = score.layers.iter().position(|l| l.id == id).expect("layer");
    expand_notes(score, SR)
        .into_iter()
        .filter(|e| e.layer == li && e.bar >= bars.start && e.bar <= bars.end)
        .collect()
}

#[test]
fn last_four_bars_melody_leaves_bars_1_to_12_identical() {
    let before = acceptance1();
    let catalogue = Catalogue::standard();
    for instruction in [
        "change the last four bars to add a subtle melody",
        "Keep the first half, change the last four bars to add a subtle melody",
    ] {
        let out =
            plan_edit(&before, instruction, &RuleProvider::new(), &catalogue, 7).expect("edit");
        println!("{instruction:?} -> {}", out.summary);
        assert_eq!(out.plan.intent, EditIntent::AddMelody);
        assert_eq!(out.bars, BarRange { start: 13, end: 16 });
        assert_eq!(out.patch.origin, Origin::Model);
        assert_eq!(out.patch.scope.bars, Some(BarRange { start: 13, end: 16 }));
        assert_eq!(out.attempts, 1);
        assert_eq!(out.provider_id, "offline-rules");
        assert!(validate(&out.score).is_valid());
        let after = &out.score;

        // Every layer of the original: identical events in bars 1-12 (and, as nothing else was
        // in scope, structurally identical as a whole).
        let head = BarRange { start: 1, end: 12 };
        for l in &before.layers {
            let a = events(&before, &l.id, head);
            let b = events(after, &l.id, head);
            assert!(!a.is_empty() || l.id == "kick", "layer {} has notes", l.id);
            assert_eq!(a, b, "layer {} changed in bars 1-12", l.id);
            assert_eq!(Some(l), after.layer(&l.id), "layer {} changed", l.id);
        }
        // The new layer plays only in bars 13-16, has notes there, and is a soft melody.
        assert_eq!(out.new_layers.len(), 1);
        let id = &out.new_layers[0];
        let new = after.layer(id).expect("new layer");
        assert_eq!(new.class, praxis_score::GeneratorClass::Melody);
        assert!(
            new.clips
                .iter()
                .all(|c| c.bars.start >= 13 && c.bars.end <= 16)
        );
        assert!(events(after, id, head).is_empty());
        let tail = events(after, id, BarRange { start: 13, end: 16 });
        assert!(tail.len() >= 8, "melody has {} notes", tail.len());
        assert!(
            tail.iter().all(|e| after.key.contains(e.pitch)),
            "melody in key"
        );
        assert!(
            new.gain_db < 0.0,
            "subtle melody sits under the piano ({} dB)",
            new.gain_db
        );
        // The events of every layer in bars 1-12, as a whole render input, are identical.
        let all_before: Vec<NoteEvent> = expand_notes(&before, SR)
            .into_iter()
            .filter(|e| e.bar <= 12)
            .collect();
        let all_after: Vec<NoteEvent> = expand_notes(after, SR)
            .into_iter()
            .filter(|e| e.bar <= 12)
            .collect();
        assert_eq!(all_before, all_after);

        // Exact undo.
        let undone = apply(after, &out.inverse).expect("undo");
        assert_eq!(undone.score, before);
        // Deterministic.
        let again = plan_edit(&before, instruction, &RuleProvider::new(), &catalogue, 7)
            .expect("edit again");
        assert_eq!(again.score, out.score);
    }
}

#[test]
fn bars_9_to_12_change_only_those_bars() {
    let before = acceptance1();
    let catalogue = Catalogue::standard();
    let out = plan_edit(
        &before,
        "bars 9-12: give the piano a new motif",
        &RuleProvider::new(),
        &catalogue,
        3,
    )
    .expect("edit");
    println!("{}", out.summary);
    assert_eq!(out.plan.intent, EditIntent::RewriteMotif);
    assert_eq!(out.patch.scope.bars, Some(BarRange { start: 9, end: 12 }));
    assert_eq!(out.patch.scope.layers, ["piano"]);
    for r in [
        BarRange { start: 1, end: 8 },
        BarRange { start: 13, end: 16 },
    ] {
        assert_eq!(events(&before, "piano", r), events(&out.score, "piano", r));
    }
    assert_ne!(
        events(&before, "piano", BarRange { start: 9, end: 12 }),
        events(&out.score, "piano", BarRange { start: 9, end: 12 })
    );
    assert_eq!(before.layer("kick"), out.score.layer("kick"));
    assert_eq!(apply(&out.score, &out.inverse).expect("undo").score, before);
}
