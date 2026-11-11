//! ClaudeProvider for edits against replayed responses (no network): the `edit_score` tool
//! request, a repair round, and the never-widen rule on a model's answers.
#![cfg(feature = "claude")]

use praxis_planner::claude::{ClaudeConfig, ClaudeProvider, HttpResponse, Transport};
use praxis_planner::edit::EditIssueCode;
use praxis_planner::{Catalogue, RuleProvider, plan_brief, plan_edit};
use praxis_score::BarRange;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const BRIEF: &str = "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt \
piano playing a repeating rhythmic motif on an eighth-note grid with accents and overlapping \
sustain. Bars 9-16: the identical piano part plus a soft muted kick underneath. No new melody, \
no fills, no risers, sweeps, impacts or pads.";

/// A placeholder, not a real key.
const FAKE_KEY: &str = "test-placeholder-not-a-key";

#[derive(Default)]
struct Replay {
    responses: Mutex<VecDeque<HttpResponse>>,
    requests: Mutex<Vec<Value>>,
}

struct ReplayTransport(Arc<Replay>);

impl Transport for ReplayTransport {
    fn post_json(
        &self,
        _url: &str,
        _headers: &[(String, String)],
        body: &Value,
    ) -> Result<HttpResponse, String> {
        self.0
            .requests
            .lock()
            .map_err(|e| e.to_string())?
            .push(body.clone());
        self.0
            .responses
            .lock()
            .map_err(|e| e.to_string())?
            .pop_front()
            .ok_or_else(|| "no recorded response left".to_string())
    }
}

/// A Messages API response that calls `edit_score` with `input`.
fn tool_response(input: Value) -> HttpResponse {
    HttpResponse {
        status: 200,
        body: json!({
            "id": "msg_01EditFixture",
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-5-5",
            "content": [
                {"type": "text", "text": "Here is the edit."},
                {"type": "tool_use", "id": "toolu_01Edit", "name": "edit_score", "input": input}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 1200, "output_tokens": 90}
        })
        .to_string(),
        retry_after_s: None,
    }
}

fn provider(responses: Vec<HttpResponse>) -> (ClaudeProvider, Arc<Replay>) {
    let replay = Arc::new(Replay::default());
    replay.responses.lock().expect("lock").extend(responses);
    let config = ClaudeConfig {
        retry_base_ms: 0,
        ..ClaudeConfig::default()
    };
    let p = ClaudeProvider::with_parts(
        config,
        Box::new(ReplayTransport(replay.clone())),
        Box::new(|| Ok(FAKE_KEY.to_string())),
    );
    (p, replay)
}

#[test]
fn claude_edit_with_one_repair_round() {
    let score = plan_brief(BRIEF, &RuleProvider::new(), &Catalogue::standard(), 1)
        .expect("plan")
        .score;
    let (p, replay) = provider(vec![
        // A preset the catalogue lacks; the scope (bars 13-16) is bound.
        tool_response(json!({
            "intent": "add_melody",
            "scope": {"phrase": "last 4 bars", "keep": "first half"},
            "params": {"preset": "violin", "dynamics": "soft"}
        })),
        // Widened to bars 9-16: sent back.
        tool_response(json!({
            "intent": "add_melody",
            "scope": {"phrase": "second half"},
            "params": {"preset": "bell", "dynamics": "soft"}
        })),
        tool_response(json!({
            "intent": "add_melody",
            "scope": {"bars": {"start": 13, "end": 16}, "keep": "first half"},
            "params": {"preset": "bell", "dynamics": "soft", "gain_db": -6}
        })),
    ]);
    let out = plan_edit(
        &score,
        "keep the first half, change the last four bars to add a subtle melody",
        &p,
        &Catalogue::standard(),
        3,
    )
    .expect("edit");
    assert_eq!(out.provider_id, "claude:claude-opus-5-5");
    assert_eq!(out.attempts, 3);
    assert_eq!(out.rounds[0][0].code, EditIssueCode::UnknownPreset);
    assert_eq!(out.rounds[1][0].code, EditIssueCode::Widened);
    assert_eq!(out.patch.scope.bars, Some(BarRange { start: 13, end: 16 }));
    let new = out.score.layer(&out.new_layers[0]).expect("new layer");
    assert_eq!(new.instrument.preset, "bell");

    let reqs = replay.requests.lock().expect("lock");
    assert_eq!(reqs.len(), 3);
    let first = &reqs[0];
    assert_eq!(first["tools"][0]["name"], "edit_score");
    // Not strict: the EditPlan grammar is too large for the API's strict mode (live 400).
    assert!(first["tools"][0].get("strict").is_none());
    assert_eq!(
        first["tools"][0]["input_schema"]["additionalProperties"],
        false
    );
    // Opus 5.5 rejects a forced tool choice: auto plus a named tool in the system prompt.
    assert_eq!(first["tool_choice"]["type"], "auto");
    let system = first["system"].as_str().expect("system");
    assert!(system.contains("edit_score"));
    assert!(
        system.contains("Layer `piano`") && system.contains("Layer `kick`"),
        "{system}"
    );
    // Repair rounds carry the issues back after the model's own call.
    let msgs = reqs[1]["messages"].as_array().expect("messages");
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[1]["role"], "assistant");
    let repair = msgs[2]["content"].as_str().expect("repair");
    assert!(
        repair.contains("unknown_preset") && repair.contains("violin"),
        "{repair}"
    );
    let repair = reqs[2]["messages"][4]["content"]
        .as_str()
        .expect("repair 2");
    assert!(repair.contains("widened"), "{repair}");
    // The key never appears in a request body.
    assert!(!reqs.iter().any(|r| r.to_string().contains(FAKE_KEY)));
}

/// A tone request through Claude: the request teaches `change_tone` (system prompt and schema),
/// the replayed answer changes the piano's sound and no note, and a `not_understood` answer
/// changes nothing.
#[test]
fn claude_tone_edit_and_not_understood() {
    use praxis_planner::edit::{EditIntent, ToneChange};
    use praxis_score::timeline::expand_notes;
    let score = plan_brief(BRIEF, &RuleProvider::new(), &Catalogue::standard(), 1)
        .expect("plan")
        .score;
    let (p, replay) = provider(vec![tool_response(json!({
        "intent": "change_tone",
        "layers": ["piano"],
        "params": {"tone": "darker", "amount": "normal"}
    }))]);
    let out = plan_edit(
        &score,
        "can you keep the piano but make it sound muffled?",
        &p,
        &Catalogue::standard(),
        3,
    )
    .expect("edit");
    assert_eq!(out.plan.intent, EditIntent::ChangeTone);
    assert_eq!(out.plan.params.tone, Some(ToneChange::Darker));
    assert_eq!(out.attempts, 1);
    assert_eq!(
        expand_notes(&score, 48_000),
        expand_notes(&out.score, 48_000)
    );
    let piano = out.score.layer("piano").expect("piano");
    assert_eq!(piano.instrument.params.get("tone"), Some(&0.2));
    let reqs = replay.requests.lock().expect("lock");
    let system = reqs[0]["system"].as_str().expect("system");
    assert!(
        system.contains("change_tone")
            && system.contains("change_space")
            && system.contains("not_understood")
            && system.contains("felt_piano (tone 0.50, felt 0.70)"),
        "{system}"
    );
    let schema = reqs[0]["tools"][0]["input_schema"].to_string();
    for word in [
        "change_tone",
        "change_space",
        "not_understood",
        "darker",
        "wetter",
        "a_little",
    ] {
        assert!(schema.contains(word), "schema lacks {word}");
    }
    drop(reqs);

    let (p, _) = provider(vec![tool_response(json!({
        "intent": "not_understood",
        "notes": ["\"make it pop\" does not say what to change"]
    }))]);
    let out = plan_edit(&score, "make it pop", &p, &Catalogue::standard(), 3).expect("edit");
    assert!(!out.changed() && !out.understood());
    assert_eq!(out.score, score);
    assert!(
        out.summary.starts_with("Nothing changed"),
        "{}",
        out.summary
    );
}
