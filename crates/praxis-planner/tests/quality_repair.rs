//! Quality repair rounds for model providers (WP-P4): the arranged Score's lints and the brief's
//! restraint rules go back to the model as issues, within the repair-round limit; the offline
//! provider is never asked again; usage is added up per response.

mod common;

use praxis_planner::brief::{quality_issues, requested_classes};
use praxis_planner::catalogue::kind_name;
use praxis_planner::offline::{brief_instruments, parse_brief};
use praxis_planner::plan::{PlanContext, PlanIssueCode, validate_plan};
use praxis_planner::{Catalogue, Provider, ProviderError, ScorePlan, ToolRequest, plan_brief};
use praxis_score::validate::IssueCode;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Mutex;

fn preset(name: &str) -> Value {
    let cat = Catalogue::standard();
    let p = cat.get(name).expect("preset");
    json!({"kind": kind_name(p.kind), "name": name})
}

fn layer(id: &str, class: &str, preset_name: &str, spoken: &str, grid: &str) -> Value {
    json!({
        "id": id,
        "class": class,
        "preset": preset(preset_name),
        "dynamics": "soft",
        "motif": {"grid": grid, "spoken": spoken, "degrees": [0, 2, 4, 2]}
    })
}

fn plan(key: (&str, &str), bpm: f64, bars: u32, layers: Vec<Value>, sections: Value) -> Value {
    json!({
        "emotion": "calm",
        "key": {"tonic": key.0, "mode": key.1},
        "tempo_bpm": bpm,
        "bars": bars,
        "playback": "one_shot",
        "sections": sections,
        "layers": layers,
    })
}

fn one_section(bars: u32, ids: &[&str]) -> Value {
    json!([{"name": "Main", "bars": {"start": 1, "end": bars}, "role": "main",
            "intensity": 0.5, "layers": ids}])
}

fn issues_for(brief: &str, plan_json: &Value) -> Vec<(PlanIssueCode, String)> {
    let cat = Catalogue::standard();
    let requested = requested_classes(brief);
    let ctx = PlanContext {
        catalogue: &cat,
        requested: &requested,
    };
    let p: ScorePlan = serde_json::from_value(plan_json.clone()).expect("plan");
    assert!(
        validate_plan(&p, &ctx).is_empty(),
        "{:?}",
        validate_plan(&p, &ctx)
    );
    quality_issues(&p, &ctx, &brief_instruments(brief), 0)
        .into_iter()
        .map(|i| (i.code, i.path))
        .collect()
}

/// A provider that answers every call with the next plan in its list (the last one repeats)
/// and records the requests.
struct Scripted {
    id: String,
    answers: Vec<Value>,
    requests: Mutex<Vec<ToolRequest>>,
}

impl Scripted {
    fn new(id: &str, answers: Vec<Value>) -> Scripted {
        Scripted {
            id: id.into(),
            answers,
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl Provider for Scripted {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn call_tool(&self, req: &ToolRequest) -> Result<Value, ProviderError> {
        let mut r = self
            .requests
            .lock()
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        r.push(req.clone());
        let i = (r.len() - 1).min(self.answers.len() - 1);
        Ok(self.answers[i].clone())
    }
}

const BASS_BRIEF: &str = "8 bars in G major at 100 BPM, felt piano with a warm bass";

fn piano() -> Value {
    json!({
        "id": "piano", "class": "motif", "preset": preset("felt_piano"), "dynamics": "soft",
        "motif": {"grid": "eighth", "length_bars": 2, "spoken": "DON din don din TIN din don din",
                  "degrees": [0, 4, 7, 4, 9, 4, 7, 4, -2, 2, 5, 2, 7, 2, 6, 4], "sustain_steps": 1.6}
    })
}

/// The live failure: a model-written bass with one short note per bar and three beats of silence.
fn gappy_bass_plan() -> Value {
    let bass = layer("bass", "bass", "warm_bass", "DON - - -", "quarter");
    plan(
        ("G", "major"),
        100.0,
        8,
        vec![piano(), bass],
        one_section(8, &["piano", "bass"]),
    )
}

fn connected_bass_plan() -> Value {
    let bass = layer("bass", "bass", "warm_bass", "DON ~ ~ don", "quarter");
    plan(
        ("G", "major"),
        100.0,
        8,
        vec![piano(), bass],
        one_section(8, &["piano", "bass"]),
    )
}

#[test]
fn an_isolated_bass_goes_back_to_the_model_and_the_repaired_answer_wins() {
    let issues = issues_for(BASS_BRIEF, &gappy_bass_plan());
    assert_eq!(
        issues,
        vec![(PlanIssueCode::IsolatedNotes, "/layers/1/motif".to_string())]
    );
    assert!(issues_for(BASS_BRIEF, &connected_bass_plan()).is_empty());

    let p = Scripted::new(
        "claude:test",
        vec![gappy_bass_plan(), connected_bass_plan()],
    );
    let out = plan_brief(BASS_BRIEF, &p, &Catalogue::standard(), 0).expect("plan");
    assert_eq!(out.attempts, 2);
    assert!(out.repairs.is_empty());
    assert_eq!(out.rounds.len(), 1);
    assert_eq!(out.rounds[0][0].code, PlanIssueCode::IsolatedNotes);
    assert!(out.lint.is_empty(), "{:?}", out.lint);
    let pattern = out.score.pattern("bass_motif").expect("bass pattern");
    assert_eq!(pattern.spoken.as_deref(), Some("DON ~ ~ don"));
    // The repair message names the problem, points into the plan and says what to do.
    let reqs = p.requests.lock().expect("lock");
    let last = &reqs[1].messages.last().expect("message").content;
    assert!(last.contains("isolated_notes"), "{last}");
    assert!(last.contains("/layers/1/motif"), "{last}");
    assert!(last.contains("let sustain"), "{last}");
    assert!(!last.contains("  "), "{last}");
}

#[test]
fn a_model_that_keeps_its_plan_keeps_a_valid_plan_with_the_warning() {
    let p = Scripted::new("claude:test", vec![gappy_bass_plan()]);
    let out = plan_brief(BASS_BRIEF, &p, &Catalogue::standard(), 0).expect("plan");
    // The same answer twice: not asked a third time, and the valid plan is kept as it was.
    assert_eq!(out.attempts, 2);
    assert!(out.repairs.is_empty());
    assert!(out.lint.iter().any(|w| w.code == IssueCode::IsolatedNotes));
}

#[test]
fn quality_rounds_stay_within_the_round_limit() {
    // A model that answers with a different gappy plan every time: one call plus three rounds.
    let answers: Vec<Value> = (0..6)
        .map(|i| {
            let mut v = gappy_bass_plan();
            v["title"] = json!(format!("take {i}"));
            v
        })
        .collect();
    let p = Scripted::new("claude:test", answers);
    let out = plan_brief(BASS_BRIEF, &p, &Catalogue::standard(), 0).expect("plan");
    assert_eq!(out.attempts, 1 + praxis_planner::repair::MAX_REPAIR_ROUNDS);
    assert_eq!(out.rounds.len() as u32, out.attempts);
}

#[test]
fn the_offline_provider_is_never_asked_for_quality() {
    let p = Scripted::new(
        "offline-rules",
        vec![gappy_bass_plan(), connected_bass_plan()],
    );
    let out = plan_brief(BASS_BRIEF, &p, &Catalogue::standard(), 0).expect("plan");
    assert_eq!(out.attempts, 1);
    assert!(out.rounds.is_empty());
    assert!(out.lint.iter().any(|w| w.code == IssueCode::IsolatedNotes));
}

#[test]
fn unrequested_layers_are_reported_where_the_brief_is_explicit() {
    // ban_list: a glass motif, and the model added soft keys.
    let brief = "Dreamy glass motif in E major, 8 bars at 84 BPM, no drums, no bass, no pads";
    let glass = layer(
        "glass",
        "motif",
        "glass",
        "DON din don din TIN din don din",
        "eighth",
    );
    let keys = layer(
        "keys",
        "chords",
        "soft_keys",
        "DON ~ ~ ~ don ~ ~ ~",
        "eighth",
    );
    let p = plan(
        ("E", "major"),
        84.0,
        8,
        vec![glass.clone(), keys],
        one_section(8, &["glass", "keys"]),
    );
    assert_eq!(
        issues_for(brief, &p),
        vec![(PlanIssueCode::UnrequestedLayer, "/layers/1".to_string())]
    );
    let only = plan(
        ("E", "major"),
        84.0,
        8,
        vec![glass],
        one_section(8, &["glass"]),
    );
    assert!(issues_for(brief, &only).is_empty());

    // unmapped_strings: piano named, strings not in the catalogue; keys and bass were added.
    let brief = "Melancholy strings and piano in D minor, 8 bars at 72 bpm";
    let pno = layer(
        "piano",
        "motif",
        "felt_piano",
        "DON din don din TIN din don din",
        "eighth",
    );
    let keys = layer(
        "keys",
        "chords",
        "soft_keys",
        "DON ~ ~ ~ don ~ ~ ~",
        "eighth",
    );
    let bass = layer("bass", "bass", "sub_bass", "DON ~ ~ ~ don ~ ~ ~", "eighth");
    let p = plan(
        ("D", "minor"),
        72.0,
        8,
        vec![pno, keys, bass],
        one_section(8, &["piano", "keys", "bass"]),
    );
    let got = issues_for(brief, &p);
    assert_eq!(
        got,
        vec![
            (PlanIssueCode::UnrequestedLayer, "/layers/1".to_string()),
            (PlanIssueCode::UnrequestedLayer, "/layers/2".to_string()),
        ]
    );
}

#[test]
fn a_layer_that_enters_late_is_reported_when_the_brief_gives_no_bars() {
    let brief = "6/8 nostalgic mallet loop in A minor, 8 bars at 70 bpm with a soft kick";
    let mallet = layer(
        "mallet",
        "motif",
        "mallet",
        "DON din don TIN din don",
        "eighth",
    );
    let kick = json!({"id": "kick", "class": "drums", "preset": preset("soft_kick"),
                      "dynamics": "soft", "motif": {"grid": "eighth", "spoken": "DON - - don - -"}});
    let mut p = plan(
        ("A", "minor"),
        70.0,
        8,
        vec![mallet, kick],
        json!([
            {"name": "A", "bars": {"start": 1, "end": 2}, "role": "intro", "intensity": 0.3,
             "layers": ["mallet"]},
            {"name": "B", "bars": {"start": 3, "end": 8}, "role": "main", "intensity": 0.5,
             "layers": ["mallet", "kick"]}
        ]),
    );
    p["time_signature"] = json!({"numerator": 6, "denominator": 8});
    assert_eq!(
        issues_for(brief, &p),
        vec![(PlanIssueCode::LayerBars, "/sections".to_string())]
    );
    // With bars in the brief the same plan is what was asked for.
    let timed = "6/8 nostalgic mallet loop in A minor, 8 bars at 70 bpm; from bar 3 a soft kick";
    assert!(issues_for(timed, &p).is_empty());
}

fn prompts() -> Vec<(String, Value)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("prompts");
    let mut out: Vec<(String, Value)> = std::fs::read_dir(dir)
        .expect("prompts dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json")
                && !p.to_string_lossy().ends_with(".plan.json")
        })
        .map(|p| {
            let v: Value =
                serde_json::from_str(&std::fs::read_to_string(&p).expect("read")).expect("json");
            (
                p.file_stem().expect("stem").to_string_lossy().to_string(),
                v,
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The offline planner's plans are the reference answers of the suite: the quality checks a
/// model gets must find nothing in them (no false restraint or timing reports on any brief).
#[test]
fn the_reference_plans_of_every_prompt_have_no_quality_issues() {
    let cat = Catalogue::standard();
    for (name, case) in prompts() {
        let brief = case["brief"].as_str().expect("brief");
        let seed = case["seed"].as_u64().unwrap_or(0);
        let requested = requested_classes(brief);
        let ctx = PlanContext {
            catalogue: &cat,
            requested: &requested,
        };
        let plan = parse_brief(brief, &cat, seed);
        if !validate_plan(&plan, &ctx).is_empty() {
            continue;
        }
        let issues = quality_issues(&plan, &ctx, &brief_instruments(brief), seed);
        assert!(issues.is_empty(), "{name}: {issues:?}");
    }
}

/// Every layer a case expects is an instrument its brief names (the restraint check would
/// never ask a model to remove an expected layer).
#[test]
fn every_expected_layer_is_named_by_its_brief() {
    let cat = Catalogue::standard();
    for (name, case) in prompts() {
        let brief = case["brief"].as_str().expect("brief");
        let named = brief_instruments(brief);
        // A brief that names no instrument gets the planner's choice (one felt piano, or a
        // cue's emotion preset); the restraint check does not judge it.
        if !named.names_any() {
            continue;
        }
        for want in case["expect"]["layers"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            let p = want["preset"].as_str().expect("preset");
            let class = cat.get(p).expect("catalogue preset").class;
            assert!(
                named.covers(p, class),
                "{name}: `{p}` not named in {named:?}"
            );
        }
    }
}

#[cfg(feature = "claude")]
#[test]
fn claude_usage_is_added_up_and_priced() {
    use praxis_planner::claude::{ClaudeConfig, ClaudeProvider, HttpResponse, Transport, Usage};
    struct Two(Mutex<Vec<HttpResponse>>);
    impl Transport for Two {
        fn post_json(
            &self,
            _: &str,
            _: &[(String, String)],
            _: &Value,
        ) -> Result<HttpResponse, String> {
            self.0
                .lock()
                .map_err(|e| e.to_string())?
                .pop()
                .ok_or("empty".into())
        }
    }
    let resp = |input: Value, usage: Value| {
        HttpResponse {
        status: 200,
        body: json!({
            "id": "msg_q", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
            "content": [{"type": "tool_use", "id": "toolu_q", "name": "plan_score", "input": input}],
            "stop_reason": "tool_use", "usage": usage
        })
        .to_string(),
        retry_after_s: None,
    }
    };
    // Popped from the end: the gappy plan first, then the repaired one.
    let t = Two(Mutex::new(vec![
        resp(
            connected_bass_plan(),
            json!({"input_tokens": 300, "cache_read_input_tokens": 9000, "output_tokens": 2000}),
        ),
        resp(
            gappy_bass_plan(),
            json!({"input_tokens": 300, "cache_creation_input_tokens": 9000, "output_tokens": 3000}),
        ),
    ]));
    let config = ClaudeConfig {
        retry_base_ms: 0,
        ..ClaudeConfig::default()
    };
    let p = ClaudeProvider::with_parts(config, Box::new(t), Box::new(|| Ok("placeholder".into())));
    let out = plan_brief(BASS_BRIEF, &p, &Catalogue::standard(), 0).expect("plan");
    assert_eq!(out.attempts, 2);
    assert_eq!(out.rounds[0][0].code, PlanIssueCode::IsolatedNotes);
    let u = p.usage();
    assert_eq!(
        u,
        Usage {
            requests: 2,
            input_tokens: 600,
            output_tokens: 5000,
            cache_creation_input_tokens: 9000,
            cache_read_input_tokens: 9000,
        }
    );
    // 600 x $4 + 5000 x $20 + 9000 x $5 + 9000 x $0.20 per million.
    let cost = u.cost_usd("claude-opus-5-5").expect("price");
    assert!((cost - 0.1492).abs() < 1e-9, "{cost}");
    assert_eq!(u.since(&u), Usage::default());
}

/// The suites' fair comparisons stay strict about what a listener hears.
#[test]
fn spoken_rhythms_and_emotions_are_compared_fairly_but_strictly() {
    use common::{emotion_matches, same_spoken};
    // Trailing rests and exact repetition are the same rhythm.
    assert!(same_spoken(
        "DON din DON don TIN",
        "DON din DON don TIN - - -"
    ));
    assert!(same_spoken(
        "DON - din DON ~ din TIN -",
        "DON - din DON ~ din TIN - - - - - - - - -"
    ));
    assert!(same_spoken(
        "DON din DON don TIN",
        "DON din DON don TIN - - - DON din DON don TIN - - -"
    ));
    // A different hit, accent, tie or inner rest is not.
    assert!(!same_spoken("DON din DON don TIN", "DON din DON din TIN"));
    assert!(!same_spoken("DON din DON don TIN", "DON din - DON don TIN"));
    assert!(!same_spoken(
        "DON - din DON ~ din TIN -",
        "DON - din DON - din TIN -"
    ));
    assert!(!same_spoken("DON din DON don TIN", "- DON din DON don TIN"));
    // Close synonyms only.
    assert!(emotion_matches("dark", "ominous"));
    assert!(emotion_matches("tense", "Suspenseful"));
    assert!(emotion_matches("calm", "calm"));
    assert!(!emotion_matches("dark", "happy"));
    assert!(!emotion_matches("tense", "calm"));
}
