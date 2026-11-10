//! Acceptance test 1 (PRD section 2) from one prompt through the offline provider, compared
//! structurally with the hand-written fixtures/acceptance1.score.json.

mod common;

use common::{active_bars, check_acceptance1};
use praxis_planner::{Catalogue, RuleProvider, plan_brief};
use std::path::PathBuf;

const BRIEF: &str = "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt \
piano playing a repeating rhythmic motif on an eighth-note grid with accents and overlapping \
sustain. Bars 9-16: the identical piano part plus a soft muted kick underneath. No new melody, \
no fills, no risers, sweeps, impacts or pads.";

fn fixture() -> praxis_score::Score {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/acceptance1.score.json");
    let text = std::fs::read_to_string(path).expect("fixture");
    praxis_score::from_json(&text).expect("fixture is valid")
}

#[test]
fn acceptance1_from_one_prompt_first_try() {
    let out = plan_brief(BRIEF, &RuleProvider::new(), &Catalogue::standard(), 1).expect("plan");
    check_acceptance1(&out.score).expect("acceptance 1");
    assert_eq!(out.attempts, 1);
    assert!(out.repairs.is_empty(), "{:?}", out.repairs);
    assert_eq!(out.provider_id, "offline-rules");
    let prov = &out.score.meta.provenance;
    assert_eq!(
        prov.get("planner").map(String::as_str),
        Some("praxis-planner plan-v1")
    );
    assert_eq!(prov.get("attempts").map(String::as_str), Some("1"));
    assert_eq!(out.score.meta.brief.as_deref(), Some(BRIEF));

    // Summary for the report.
    let s = &out.score;
    println!(
        "acceptance-1 score: {} bars, {}/{}, {}, {} BPM, {:?}, avoid {:?}",
        s.bars,
        s.time_signature.numerator,
        s.time_signature.denominator,
        s.key.display(),
        s.tempo.bpm,
        s.playback,
        s.constraints.avoid
    );
    for sec in &s.sections {
        println!(
            "  section {} `{}` bars {}-{} {:?} intensity {}",
            sec.id, sec.name, sec.bars.start, sec.bars.end, sec.role, sec.intensity
        );
    }
    for l in &s.layers {
        let p = s.pattern(&l.clips[0].pattern).expect("pattern");
        println!(
            "  layer {} {:?}/{} class {:?} gain {} dB clips {:?} pattern {} {:?} {} steps \
             spoken {:?} fx {:?}",
            l.id,
            l.instrument.kind,
            l.instrument.preset,
            l.class,
            l.gain_db,
            l.clips
                .iter()
                .map(|c| (c.bars.start, c.bars.end, c.pattern_offset_steps))
                .collect::<Vec<_>>(),
            p.id,
            p.grid,
            p.length_steps,
            p.spoken,
            l.effects
                .iter()
                .map(|e| (&e.id, e.kind))
                .collect::<Vec<_>>()
        );
        for n in &p.notes {
            println!(
                "    step {:>2} pitch {} vel {:.3} {:?} len {}",
                n.step,
                praxis_score::theory::midi_name(n.pitch),
                n.velocity,
                n.accent,
                n.length
            );
        }
    }
}

#[test]
fn structurally_equivalent_to_the_hand_written_fixture() {
    let out = plan_brief(BRIEF, &RuleProvider::new(), &Catalogue::standard(), 1).expect("plan");
    let ours = &out.score;
    let theirs = fixture();
    check_acceptance1(&theirs).expect("the fixture itself meets acceptance 1");
    assert_eq!(ours.bars, theirs.bars);
    assert_eq!(ours.tempo, theirs.tempo);
    assert_eq!(ours.time_signature, theirs.time_signature);
    assert_eq!(ours.key, theirs.key);
    assert_eq!(ours.playback, theirs.playback);
    assert!(theirs.constraints.avoid.is_subset(&ours.constraints.avoid));
    assert_eq!(ours.sections.len(), theirs.sections.len());
    for (a, b) in ours.sections.iter().zip(&theirs.sections) {
        assert_eq!(a.bars, b.bars);
        assert_eq!(a.role, b.role);
    }
    let presets = |s: &praxis_score::Score| {
        let mut v: Vec<(String, String)> = s
            .layers
            .iter()
            .map(|l| {
                (
                    l.instrument.preset.clone(),
                    format!("{:?}", active_bars(s, &l.id)),
                )
            })
            .collect();
        v.sort();
        v
    };
    assert_eq!(presets(ours), presets(&theirs));
    let grid = |s: &praxis_score::Score, preset: &str| {
        let l = s
            .layers
            .iter()
            .find(|l| l.instrument.preset == preset)
            .expect("layer");
        s.pattern(&l.clips[0].pattern).expect("pattern").grid
    };
    assert_eq!(grid(ours, "felt_piano"), grid(&theirs, "felt_piano"));
    assert_eq!(grid(ours, "soft_kick"), grid(&theirs, "soft_kick"));
    // Room reverb on the piano, low-passed EQ on the soft kick, master defaults.
    let fx = |s: &praxis_score::Score, preset: &str| {
        s.layers
            .iter()
            .find(|l| l.instrument.preset == preset)
            .map(|l| l.effects.iter().map(|e| e.kind).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    assert_eq!(fx(ours, "felt_piano"), fx(&theirs, "felt_piano"));
    assert_eq!(fx(ours, "soft_kick"), fx(&theirs, "soft_kick"));
    assert_eq!(ours.master.loudness, theirs.master.loudness);
    let kick = ours
        .layers
        .iter()
        .find(|l| l.instrument.preset == "soft_kick")
        .expect("kick");
    assert!(
        kick.effects[0]
            .params
            .get("lp_hz")
            .is_some_and(|v| *v <= 4000.0),
        "soft kick is low-passed"
    );
}

#[test]
fn same_brief_and_seed_give_the_same_score() {
    let c = Catalogue::standard();
    let a = plan_brief(BRIEF, &RuleProvider::new(), &c, 7).expect("plan");
    let b = plan_brief(BRIEF, &RuleProvider::new(), &c, 7).expect("plan");
    assert_eq!(
        praxis_score::to_json(&a.score),
        praxis_score::to_json(&b.score)
    );
}
