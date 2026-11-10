//! ClaudeProvider against recorded request/response fixtures. No network: the HTTP layer is a
//! replay transport. The fixtures are hand-written from the documented Messages API format.
#![cfg(feature = "claude")]

mod common;

use common::check_acceptance1;
use praxis_planner::claude::{
    API_URL, API_VERSION, ClaudeConfig, ClaudeProvider, FALLBACK_BETA, HttpResponse, Transport,
};
use praxis_planner::{Catalogue, PlanError, Provider, ProviderError, plan_brief};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const BRIEF: &str = "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt \
piano playing a repeating rhythmic motif on an eighth-note grid with accents and overlapping \
sustain. Bars 9-16: the identical piano part plus a soft muted kick underneath. No new melody, \
no fills, no risers, sweeps, impacts or pads.";

/// A placeholder, not a real key.
const FAKE_KEY: &str = "test-placeholder-not-a-key";

type Recorded = (String, Vec<(String, String)>, Value);

#[derive(Default)]
struct Replay {
    responses: Mutex<VecDeque<HttpResponse>>,
    requests: Mutex<Vec<Recorded>>,
}

struct ReplayTransport(Arc<Replay>);

impl Transport for ReplayTransport {
    fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<HttpResponse, String> {
        self.0.requests.lock().map_err(|e| e.to_string())?.push((
            url.to_string(),
            headers.to_vec(),
            body.clone(),
        ));
        self.0
            .responses
            .lock()
            .map_err(|e| e.to_string())?
            .pop_front()
            .ok_or_else(|| "no recorded response left".to_string())
    }
}

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("claude")
        .join(name);
    std::fs::read_to_string(path).expect("fixture")
}

fn ok(name: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        body: fixture(name),
        retry_after_s: None,
    }
}

fn provider(model: &str, responses: Vec<HttpResponse>) -> (ClaudeProvider, Arc<Replay>) {
    let replay = Arc::new(Replay::default());
    replay.responses.lock().expect("lock").extend(responses);
    let config = ClaudeConfig {
        model: model.to_string(),
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

fn header<'a>(h: &'a [(String, String)], name: &str) -> Option<&'a str> {
    h.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

fn contains_key(v: &Value, key: &str) -> bool {
    match v {
        Value::Object(m) => m.iter().any(|(k, x)| k == key || contains_key(x, key)),
        Value::Array(a) => a.iter().any(|x| contains_key(x, key)),
        _ => false,
    }
}

#[test]
fn plans_acceptance1_and_sends_the_documented_request() {
    let (p, replay) = provider("claude-opus-5-5", vec![ok("plan_response.json")]);
    assert_eq!(p.id(), "claude:claude-opus-5-5");
    let out = plan_brief(BRIEF, &p, &Catalogue::standard(), 1).expect("plan");
    check_acceptance1(&out.score).expect("acceptance 1");
    assert_eq!(out.attempts, 1);
    assert_eq!(out.provider_id, "claude:claude-opus-5-5");
    assert_eq!(
        out.score
            .meta
            .provenance
            .get("provider")
            .map(String::as_str),
        Some("claude:claude-opus-5-5")
    );

    let reqs = replay.requests.lock().expect("lock");
    assert_eq!(reqs.len(), 1);
    let (url, headers, body) = &reqs[0];
    assert_eq!(url, API_URL);
    assert_eq!(header(headers, "x-api-key"), Some(FAKE_KEY));
    assert_eq!(header(headers, "anthropic-version"), Some(API_VERSION));
    assert_eq!(header(headers, "content-type"), Some("application/json"));
    assert_eq!(header(headers, "anthropic-beta"), Some(FALLBACK_BETA));
    // The key travels only in its header.
    assert!(!body.to_string().contains(FAKE_KEY));

    // Shape: everything except the long system prompt and the schema matches the fixture.
    let mut skeleton = body.clone();
    let system = skeleton
        .as_object_mut()
        .and_then(|o| o.remove("system"))
        .expect("system");
    let schema = skeleton["tools"][0]
        .as_object_mut()
        .and_then(|o| o.remove("input_schema"))
        .expect("input_schema");
    let expected: Value = serde_json::from_str(&fixture("expected_request.json")).expect("json");
    assert_eq!(skeleton, expected);
    let system = system.as_str().expect("system text");
    assert!(system.contains("plan_score"));
    assert!(system.contains("felt_piano"));
    assert!(system.contains("Answer only by calling the `plan_score` tool"));
    // Strict tool schema: an object schema, closed objects, no unsupported constraints.
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], false);
    assert!(schema["properties"]["layers"].is_object());
    for k in ["minimum", "maximum", "format", "$schema", "default"] {
        assert!(!contains_key(&schema["$defs"], k), "{k} in $defs");
        assert!(schema.get(k).is_none(), "{k} at top level");
    }
    // Opus 5.5 rejects forced tool use, thinking budgets and sampling parameters.
    assert!(body.get("thinking").is_none());
    assert!(body.get("temperature").is_none());
    assert_eq!(body["tool_choice"]["type"], "auto");
}

#[test]
fn older_models_get_a_forced_tool_call() {
    let (p, _) = provider("claude-opus-5", vec![]);
    let req = praxis_planner::brief::tool_request(BRIEF, &Catalogue::standard(), 0);
    let body = p.request_body(&req);
    assert_eq!(body["tool_choice"]["type"], "tool");
    assert_eq!(body["tool_choice"]["name"], "plan_score");
    assert!(
        !body["system"]
            .as_str()
            .unwrap_or("")
            .contains("Answer only by calling")
    );
    // Haiku 5.5 does not take `fallbacks: "default"`.
    let (h, _) = provider("claude-haiku-5-5", vec![]);
    assert!(h.request_body(&req).get("fallbacks").is_none());
}

#[test]
fn a_text_answer_gets_one_nudge() {
    let (p, replay) = provider(
        "claude-opus-5-5",
        vec![ok("text_only_response.json"), ok("plan_response.json")],
    );
    let out = plan_brief(BRIEF, &p, &Catalogue::standard(), 1).expect("plan");
    check_acceptance1(&out.score).expect("acceptance 1");
    let reqs = replay.requests.lock().expect("lock");
    assert_eq!(reqs.len(), 2);
    let msgs = reqs[1].2["messages"].as_array().expect("messages");
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[1]["role"], "assistant");
    assert!(
        msgs[2]["content"]
            .as_str()
            .unwrap_or("")
            .contains("Call the `plan_score` tool now")
    );
}

#[test]
fn validation_issues_go_back_to_the_model() {
    let (p, replay) = provider(
        "claude-opus-5-5",
        vec![ok("offscale_plan_response.json"), ok("plan_response.json")],
    );
    let out = plan_brief(BRIEF, &p, &Catalogue::standard(), 1).expect("plan");
    assert_eq!(out.attempts, 2);
    assert!(out.repairs.is_empty());
    assert_eq!(out.rounds.len(), 1);
    check_acceptance1(&out.score).expect("acceptance 1");
    let reqs = replay.requests.lock().expect("lock");
    let msgs = reqs[1].2["messages"].as_array().expect("messages");
    let last = msgs.last().expect("last")["content"].as_str().unwrap_or("");
    assert!(last.contains("off_scale"), "{last}");
    assert!(last.contains("/layers/0/motif/notes/1/pitch"), "{last}");
    assert!(last.contains("C#4 is not in a minor"), "{last}");
}

#[test]
fn errors_are_typed_and_never_contain_the_key() {
    let (p, _) = provider("claude-opus-5-5", vec![ok("refusal_response.json")]);
    let e = plan_brief(BRIEF, &p, &Catalogue::standard(), 1).unwrap_err();
    assert!(matches!(&e, PlanError::Provider(ProviderError::Request(m)) if m.contains("declined")));

    let (p, _) = provider("claude-opus-5-5", vec![ok("max_tokens_response.json")]);
    let e = plan_brief(BRIEF, &p, &Catalogue::standard(), 1).unwrap_err();
    assert!(e.to_string().contains("max_tokens"));

    let (p, _) = provider(
        "claude-opus-5-5",
        vec![HttpResponse {
            status: 400,
            body: fixture("error_400.json"),
            retry_after_s: None,
        }],
    );
    let e = plan_brief(BRIEF, &p, &Catalogue::standard(), 1).unwrap_err();
    let text = e.to_string();
    assert!(text.contains("HTTP 400 invalid_request_error"), "{text}");
    assert!(!text.contains(FAKE_KEY));

    // 529 is retried, then succeeds.
    let (p, replay) = provider(
        "claude-opus-5-5",
        vec![
            HttpResponse {
                status: 529,
                body: fixture("error_529.json"),
                retry_after_s: Some(0),
            },
            ok("plan_response.json"),
        ],
    );
    plan_brief(BRIEF, &p, &Catalogue::standard(), 1).expect("retried");
    assert_eq!(replay.requests.lock().expect("lock").len(), 2);

    // No key: no request is made.
    let replay = Arc::new(Replay::default());
    let p = ClaudeProvider::with_parts(
        ClaudeConfig::default(),
        Box::new(ReplayTransport(replay.clone())),
        Box::new(|| Err(ProviderError::NoApiKey)),
    );
    let e = plan_brief(BRIEF, &p, &Catalogue::standard(), 1).unwrap_err();
    assert_eq!(e, PlanError::Provider(ProviderError::NoApiKey));
    assert!(replay.requests.lock().expect("lock").is_empty());
}

#[test]
fn debug_output_redacts_the_key() {
    let (p, _) = provider("claude-opus-5-5", vec![]);
    let d = format!("{p:?}");
    assert!(d.contains("<redacted>"));
    assert!(!d.contains(FAKE_KEY));
}
