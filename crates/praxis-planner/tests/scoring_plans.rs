//! Scored plans (WP-P3): a ScorePlan written directly with a form, hit points and a Roman
//! numeral progression composes into a valid Score; the plan a run returns arranges to the same
//! Score again; a model's scored plan passes the quality rounds first time (a form says when
//! layers enter, so `layer_bars` does not apply).

mod common;

use common::check_scoring;
use praxis_planner::plan::TOOL_NAME;
use praxis_planner::{Catalogue, Provider, ProviderError, ToolRequest, compose_plan, plan_brief};
use praxis_score::lint::{LintContext, lint};
use praxis_score::validate::validate;
use serde_json::{Value, json};
use std::sync::Mutex;

fn scored_plan() -> Value {
    json!({
        "title": "Reveal",
        "emotion": "hopeful",
        "key": {"tonic": "C", "mode": "major"},
        "tempo_bpm": 96.0,
        "duration_s": 90.0,
        "playback": "one_shot",
        "layers": [
            {"id": "piano", "class": "motif",
             "preset": {"kind": "sampler", "name": "upright_piano"},
             "motif": {"grid": "eighth", "length_bars": 2,
                       "spoken": "DON din don din TIN din don din",
                       "degrees": [0, 4, 7, 4, 9, 4, 7, 4, 0, 4, 7, 9, 11, 9, 7, 4],
                       "follow_chords": true, "sustain_steps": 1.6}},
            {"id": "keys", "class": "chords",
             "preset": {"kind": "synth", "name": "soft_keys"},
             "motif": {"grid": "eighth", "spoken": "DON ~ ~ ~ ~ ~ ~ ~", "degrees": [0],
                       "follow_chords": true, "sustain_steps": 1.6}},
            {"id": "bass", "class": "bass",
             "preset": {"kind": "synth", "name": "warm_bass"},
             "motif": {"grid": "eighth", "spoken": "DON ~ ~ ~ don ~ ~ ~", "degrees": [0],
                       "follow_chords": true}}
        ],
        "form": [
            {"role": "intro", "weight": 2},
            {"role": "build", "weight": 3},
            {"role": "peak", "weight": 3, "transition": "drop"},
            {"role": "release", "weight": 2}
        ],
        "hits": [{"time_s": 31.5, "label": "title"}, {"time_s": 58.0, "label": "reveal"}],
        "progression": {"chords": ["IV", "I", "V", "vi"], "bars_per_chord": 1}
    })
}

#[test]
fn a_hand_written_scored_plan_composes_and_recomposes() {
    let c = Catalogue::standard();
    let out = compose_plan(&scored_plan(), &c, 7).expect("compose");
    assert!(out.repairs.is_empty(), "{:?}", out.repairs);
    let score = &out.score;
    assert!(validate(score).is_valid(), "{:?}", validate(score).errors);
    let ctx = LintContext {
        sustaining_layers: score
            .layers
            .iter()
            .filter(|l| {
                c.get_kind(l.instrument.kind, &l.instrument.preset)
                    .is_some_and(|p| p.sustaining)
            })
            .map(|l| l.id.clone())
            .collect(),
        requested_classes: score.layers.iter().map(|l| l.class).collect(),
    };
    assert_eq!(lint(score, &ctx), vec![]);
    check_scoring(
        score,
        &json!({
            "section_roles": ["intro", "build", "peak", "release"],
            "hits": 2, "hits_within_ms": 1.0,
            "bpm_range": [92.16, 99.84],
            "duration_within": [90.0, 1.5],
            "layers_enter_in_order": true,
            "progression_start": ["F", "C", "G", "Am"],
            "voice_leading_max": 5,
            "bass_passing_tones": true,
            "hold_before": "release"
        }),
    )
    .expect("scoring checks");
    // The hit at 0:58 falls late in the bar before the peak, so that bar keeps its hit accent
    // instead of the drop (a transition never takes a bar that holds a hit), and the peak
    // starts on the next bar line, just after the hit.
    let peak = score
        .sections
        .iter()
        .find(|s| s.role == praxis_score::SectionRole::Peak)
        .expect("peak");
    let keys = score.layer("keys").expect("keys");
    assert!(
        keys.clips
            .iter()
            .any(|c| c.bars.start == peak.bars.start - 1 && c.pattern.ends_with("_hit2"))
    );
    // A one-shot ends on the tonic chord.
    assert_eq!(score.chords.last().map(|c| c.symbol.as_str()), Some("C"));
    // The returned plan is realised (sections, tempo, chords) and arranges to the same Score.
    assert!(!out.plan.sections.is_empty());
    let again =
        compose_plan(&serde_json::to_value(&out.plan).expect("json"), &c, 7).expect("recompose");
    assert_eq!(again.score.patterns, score.patterns);
    assert_eq!(again.score.layers, score.layers);
    assert_eq!(again.score.markers, score.markers);
    assert_eq!(again.score.tempo, score.tempo);
    // Deterministic.
    let twice = compose_plan(&scored_plan(), &c, 7).expect("compose");
    assert_eq!(twice.score, out.score);
    // The summary names the form and the hits.
    let s = out.summary();
    assert!(s.contains("Form: intro"), "{s}");
    assert!(s.contains("Hits land at 0:31.50"), "{s}");
}

#[test]
fn borrowed_chords_are_refused_with_the_chords_of_the_key() {
    let mut p = scored_plan();
    p["progression"]["chords"] = json!(["I", "bVII", "IV"]);
    let c = Catalogue::standard();
    // compose_plan repairs it deterministically (drops the borrowed chord) and says so.
    let out = compose_plan(&p, &c, 0).expect("compose");
    assert!(
        out.rounds
            .iter()
            .flatten()
            .any(|i| i.message.contains("borrowed") && i.message.contains("C, Dm, Em")),
        "{:?}",
        out.rounds
    );
    assert_eq!(
        out.plan.progression.as_ref().map(|x| x.chords.clone()),
        Some(vec!["I".to_string(), "IV".to_string()])
    );
}

/// A model stand-in that answers with the scored plan.
struct Model {
    calls: Mutex<u32>,
}

impl Provider for Model {
    fn id(&self) -> String {
        "model-test".into()
    }
    fn call_tool(&self, req: &ToolRequest) -> Result<Value, ProviderError> {
        assert_eq!(req.tool_name, TOOL_NAME);
        if let Ok(mut n) = self.calls.lock() {
            *n += 1;
        }
        Ok(scored_plan())
    }
}

#[test]
fn a_models_scored_plan_needs_no_repair_round() {
    let p = Model {
        calls: Mutex::new(0),
    };
    let c = Catalogue::standard();
    let out = plan_brief(
        "A hopeful 90-second cue on upright piano, soft keys and a warm bass: intro, build, peak \
         and release, with hits at 0:31.5 and 0:58.",
        &p,
        &c,
        0,
    )
    .expect("plan");
    assert_eq!(out.attempts, 1, "rounds {:?}", out.rounds);
    assert!(out.rounds.is_empty(), "{:?}", out.rounds);
    assert_eq!(*p.calls.lock().expect("lock"), 1);
    assert_eq!(out.score.markers.len(), 2);
    assert!(out.lint.is_empty(), "{:?}", out.lint);
}
