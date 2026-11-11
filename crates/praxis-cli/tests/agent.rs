//! The sound assistant (`praxis_cli::agent`) through `ops::ask_with`, driven by a scripted model
//! (no key, no network) and by replayed API streams through the real HTTP client. The tools,
//! the draft, the renders, the take history and `conversation.json` are all real.
//!
//! - `a_conversation_new_sound_muffled_then_brighter`: a new sound; "make the piano muffled"
//!   (instrument parameters only: every note event identical, the piano's centroid drops);
//!   "a bit brighter than before" in the same conversation (the history is sent unchanged and
//!   the change partly undoes the previous one).
//! - `scoped_edit_keeps_bars_1_to_12_and_recovers_from_a_tool_error`: a change outside the
//!   request's scope is refused to the model, which then makes the scoped edit; bars 4-12 of the
//!   master stay bit-identical (bars 1-3 hold the new layer's tail wrapped round the loop).
//! - `a_locked_layer_is_refused_out_loud`, `the_step_budget_is_reached_and_explained`,
//!   `ask_user_ends_the_turn_and_the_answer_continues_it`, `refusals_and_api_failures`.
//! - `replayed_streams_through_the_http_client`: SSE fixtures through `HttpClient` (headers,
//!   stream assembly), and the key never reaching a file or a message.
//!
//! Every request is checked against the API rules: no forced tool choice, no `thinking` field
//! and no `budget_tokens`, explicit effort, strict tools, cache markers on the stable prefix,
//! and the history appended unchanged.
//!
//! Tests that render need the Upright Piano KW pack (`PRAXIS_PACKS`); without it they print why
//! and return.

use praxis_cli::ErrorCode;
use praxis_cli::agent::client::{HttpClient, PostResponse, Poster};
use praxis_cli::agent::testing::{Reply, ScriptedModel, message, text, thinking, tool_use};
use praxis_cli::agent::{AgentConfig, AgentEvent, ApiRequest, ModelClient, ModelError};
use praxis_cli::ops::{self, AskOptions, TakeScope};
use praxis_score::timeline::expand_notes;
use praxis_score::{BarRange, Score};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const PLAN: &str = include_str!("../../praxis-planner/tests/prompts/acceptance1.plan.json");
/// A placeholder, not a real key.
const FAKE_KEY: &str = "test-placeholder-not-a-key-7f3a";

fn have_piano(test: &str) -> bool {
    let ok = praxis_instruments::Packs::discover().has_pack("upright-piano-kw");
    if !ok {
        println!("SKIP {test}: the upright-piano-kw pack is not installed (set PRAXIS_PACKS)");
    }
    ok
}

fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("praxis-agent-{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::path::absolute(dir).unwrap()
}

fn ask(sound: &Path, text: &str) -> ops::AskRequest {
    ops::AskRequest {
        sound: sound.display().to_string(),
        text: text.into(),
        who: None,
        scope: None,
        patch: None,
        title: None,
        provider: None,
        model: None,
        seed: None,
        skip_missing: true,
        include_score: false,
        reference: None,
    }
}

type Events = Arc<Mutex<Vec<AgentEvent>>>;

fn options(model: Arc<dyn ModelClient>, events: &Events) -> AskOptions {
    let sink = events.clone();
    AskOptions {
        client: Some(model),
        agent: AgentConfig::default(),
        progress: Some(Arc::new(move |e: &AgentEvent| {
            sink.lock().unwrap().push(e.clone())
        })),
    }
}

fn events() -> Events {
    Arc::new(Mutex::new(Vec::new()))
}

fn step_words(events: &Events) -> Vec<String> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            AgentEvent::Step { words, .. } => Some(words.clone()),
            AgentEvent::StepDone { words, .. } if words.starts_with("Checking") => {
                Some(words.clone())
            }
            _ => None,
        })
        .collect()
}

fn read_json(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// The rules every request must keep.
fn assert_api_rules(r: &ApiRequest) {
    let b = &r.body;
    assert_eq!(b["model"], "claude-opus-5-5");
    assert_eq!(
        b["tool_choice"],
        json!({"type": "auto"}),
        "no forced tool choice"
    );
    assert!(
        b.get("thinking").is_none(),
        "thinking is left at the model default"
    );
    let text = b.to_string();
    assert!(!text.contains("budget_tokens"));
    assert!(b.get("temperature").is_none() && b.get("top_p").is_none());
    assert!(b["output_config"]["effort"].is_string(), "explicit effort");
    assert_eq!(b["stream"], true);
    let tools = b["tools"].as_array().unwrap();
    assert!(!tools.is_empty() && tools.len() <= 20);
    for t in tools {
        assert_eq!(t["strict"], true, "{}", t["name"]);
        assert_eq!(t["input_schema"]["additionalProperties"], false);
    }
    assert_eq!(b["system"][0]["cache_control"]["type"], "ephemeral");
    assert_eq!(b["cache_control"]["type"], "ephemeral");
    assert!(r.betas.iter().any(|x| x == "compact-2026-01-12"));
    assert_eq!(
        b["context_management"]["edits"][0]["type"],
        "compact_20260112"
    );
    assert_eq!(b["fallbacks"], "default");
    assert!(!text.contains(FAKE_KEY));
}

/// Each request repeats the one before it unchanged, then the reply it got (exactly as
/// returned), then the next user message.
fn assert_history_appended(reqs: &[ApiRequest], replies: &[Value]) {
    for (i, w) in reqs.windows(2).enumerate() {
        let (a, b) = (&w[0].body, &w[1].body);
        assert_eq!(a["system"], b["system"], "system prompt changed");
        assert_eq!(a["tools"], b["tools"], "tools changed");
        let ma = a["messages"].as_array().unwrap();
        let mb = b["messages"].as_array().unwrap();
        assert!(mb.len() > ma.len());
        assert_eq!(
            &mb[..ma.len()],
            &ma[..],
            "request {} rewrote history",
            i + 2
        );
        assert_eq!(
            mb[ma.len()],
            json!({"role": "assistant", "content": replies[i]["content"]}),
            "the reply is replayed exactly as returned"
        );
    }
}

/// The tool results sent in request `r` (its last user message).
fn last_results(r: &ApiRequest) -> Vec<Value> {
    let msgs = r.body["messages"].as_array().unwrap();
    let last = msgs.last().unwrap();
    assert_eq!(last["role"], "user");
    last["content"].as_array().unwrap().clone()
}

fn take_scores(sound: &Path) -> Vec<Score> {
    let v = ops::takes(&ops::TakesRequest {
        sound: sound.display().to_string(),
        include_scores: true,
    })
    .unwrap();
    v["takes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| serde_json::from_value(t["score"].clone()).unwrap())
        .collect()
}

fn piano_tone(score: &Score) -> f64 {
    score
        .layer("piano")
        .unwrap()
        .instrument
        .params
        .get("tone")
        .copied()
        .unwrap_or(0.5)
}

/// A scripted turn that composes the acceptance-1 loop.
fn compose_script() -> Vec<Value> {
    vec![
        message(
            vec![
                thinking("sig-compose"),
                text("I'll write the loop."),
                tool_use("c1", "compose_sound", json!({"plan_json": PLAN})),
            ],
            "tool_use",
        ),
        message(vec![tool_use("c2", "render_check", json!({}))], "tool_use"),
        message(
            vec![text(
                "A calm 16-bar felt piano loop in A minor at 128 BPM, with a soft kick from bar 9.",
            )],
            "end_turn",
        ),
    ]
}

fn new_sound(sound: &Path) -> Value {
    let model = ScriptedModel::new(compose_script());
    let ev = events();
    let out = ops::ask_with(
        &ask(sound, "a calm felt piano loop"),
        &options(model.clone(), &ev),
    )
    .expect("take 1");
    assert_eq!(model.left(), 0);
    out
}

#[test]
fn a_conversation_new_sound_muffled_then_brighter() {
    if !have_piano("a_conversation_new_sound_muffled_then_brighter") {
        return;
    }
    let sound = scratch("conversation").join("loop");

    // Turn 1: a new sound.
    let script = compose_script();
    let model = ScriptedModel::new(script.clone());
    let ev = events();
    let one = ops::ask_with(
        &ask(&sound, "a calm felt piano loop, 16 bars, A minor"),
        &options(model.clone(), &ev),
    )
    .expect("take 1");
    let t1 = &one["take"];
    assert_eq!(t1["n"], 1);
    assert_eq!(t1["who"], "Agent · Assistant");
    assert_eq!(t1["provider"], "agent:claude-opus-5-5");
    assert_eq!(t1["patch"], Value::Null);
    assert!(t1["checks"]["passed"] == true, "{:#}", t1["checks"]);
    assert_eq!(one["agent"]["outcome"], "take");
    assert_eq!(one["agent"]["steps"].as_array().unwrap().len(), 2);
    assert!(t1["say"].as_str().unwrap().starts_with("A calm 16-bar"));
    let reqs1 = model.requests();
    assert_eq!(reqs1.len(), 3);
    reqs1.iter().for_each(assert_api_rules);
    assert_history_appended(&reqs1, &script);
    let first_user = reqs1[0].body["messages"][0]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(first_user.contains("This sound is new"), "{first_user}");
    assert!(first_user.ends_with("a calm felt piano loop, 16 bars, A minor"));
    let words = step_words(&ev);
    println!("turn 1 progress: {words:?}");
    assert_eq!(words[..2], ["Writing a new sound", "Rendering"]);
    assert!(words[2].starts_with("Checking: "), "{words:?}");
    let conv = read_json(&sound.join("conversation.json"));
    assert_eq!(conv["format"], "praxis-conversation");
    assert_eq!(conv["turns"][0]["take"], 1);
    assert_eq!(conv["messages"].as_array().unwrap().len(), 6);

    // Turn 2: "make the piano muffled" changes the instrument, not the notes.
    let muffled = vec![
        message(
            vec![
                thinking("sig-m1"),
                tool_use(
                    "m1",
                    "read_sound",
                    json!({"part": "layer", "layer": "piano", "take": 0}),
                ),
            ],
            "tool_use",
        ),
        message(
            vec![tool_use(
                "m2",
                "set_params",
                json!({"layer": "piano", "params": [
                    {"name": "tone", "value": 0.2}, {"name": "felt", "value": 0.9}
                ]}),
            )],
            "tool_use",
        ),
        message(vec![tool_use("m3", "render_check", json!({}))], "tool_use"),
        // The reply, and the take's title through finish_turn (no further request).
        message(
            vec![
                text(
                    "Muffled the felt piano by lowering its tone to 0.2 and raising the felt to \
                     0.9; every note is the same.",
                ),
                tool_use(
                    "m4",
                    "finish_turn",
                    json!({"title": "Muffled felt piano", "no_take": ""}),
                ),
            ],
            "tool_use",
        ),
    ];
    let model = ScriptedModel::new(muffled.clone());
    let ev = events();
    let two = ops::ask_with(
        &ask(&sound, "make the piano muffled"),
        &options(model.clone(), &ev),
    )
    .expect("take 2");
    let t2 = &two["take"];
    assert_eq!(t2["n"], 2);
    assert_eq!(t2["from"], 1);
    assert_eq!(
        t2["title"], "Muffled felt piano",
        "the assistant's own title"
    );
    assert!(
        t2["say"]
            .as_str()
            .unwrap()
            .starts_with("Muffled the felt piano by")
    );
    assert_eq!(
        two["agent"]["steps"].as_array().unwrap().len(),
        3,
        "finish_turn is not a step"
    );
    let reqs2 = model.requests();
    assert_eq!(reqs2.len(), 4);
    reqs2.iter().for_each(assert_api_rules);
    assert_history_appended(&reqs2, &muffled);
    // The whole first conversation comes first, unchanged.
    let before: Vec<Value> = conv["messages"].as_array().unwrap().clone();
    let sent = reqs2[0].body["messages"].as_array().unwrap();
    assert_eq!(&sent[..before.len()], &before[..]);
    let state = sent[before.len()]["content"][0]["text"].as_str().unwrap();
    assert!(state.contains("Current take: 1 of 1"), "{state}");
    assert!(
        state.contains("`piano`") && state.contains("tone 0.5"),
        "{state}"
    );
    let words = step_words(&ev);
    println!("turn 2 progress: {words:?}");
    assert_eq!(
        words[..3],
        [
            "Reading the Felt piano",
            "Changing the Felt piano's tone and felt",
            "Rendering",
        ]
    );
    assert!(words[3].starts_with("Checking: "), "{words:?}");
    // Notes identical, tone and felt changed, the piano darker by measurement.
    let scores = take_scores(&sound);
    assert_eq!(
        expand_notes(&scores[0], 48_000),
        expand_notes(&scores[1], 48_000)
    );
    assert_eq!(piano_tone(&scores[1]), 0.2);
    assert_eq!(
        scores[1]
            .layer("piano")
            .unwrap()
            .instrument
            .params
            .get("felt"),
        Some(&0.9)
    );
    assert_eq!(
        t2["changed"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["piano"]
    );
    let measured: Value =
        serde_json::from_str(last_results(&reqs2[3])[0]["content"].as_str().unwrap()).unwrap();
    let piano = measured["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["layer"] == "piano")
        .unwrap()
        .clone();
    println!("muffled piano: {piano}");
    let muffled_centroid = piano["centroid_hz"].as_f64().unwrap();
    let original_centroid = piano["before"]["centroid_hz"].as_f64().unwrap();
    // The default felt piano is already soft; tone 0.5 -> 0.2 and felt 0.7 -> 0.9 move the
    // centroid by about 9 % and the share above 2 kHz by about 3 dB.
    assert!(
        muffled_centroid < 0.95 * original_centroid,
        "the piano's centroid drops by at least 5 %: {piano}"
    );
    assert!(
        piano["change"]["high_band_db"].as_f64().unwrap() < -2.0,
        "the piano loses its highs: {piano}"
    );

    // Turn 3: "a bit brighter than before" uses the history and partly undoes turn 2.
    let brighter = vec![
        message(
            vec![tool_use(
                "b1",
                "set_params",
                json!({"layer": "piano", "params": [{"name": "tone", "value": 0.35}]}),
            )],
            "tool_use",
        ),
        message(vec![tool_use("b2", "render_check", json!({}))], "tool_use"),
        message(
            vec![text(
                "Opened the piano up a little: tone 0.35, between the original and the muffled take.",
            )],
            "end_turn",
        ),
    ];
    let model = ScriptedModel::new(brighter.clone());
    let three = ops::ask_with(
        &ask(&sound, "make it a bit brighter than before"),
        &options(model.clone(), &events()),
    )
    .expect("take 3");
    assert_eq!(three["take"]["n"], 3);
    // No title from the assistant: a summary of what changed, never the reply clipped.
    let title3 = three["take"]["title"].as_str().unwrap();
    println!("take 3 title: {title3}");
    assert!(
        title3.ends_with(" sound") && !title3.starts_with("Opened"),
        "{title3}"
    );
    let reqs3 = model.requests();
    reqs3.iter().for_each(assert_api_rules);
    assert_history_appended(&reqs3, &brighter);
    // The earlier set_params call (tone 0.2) is in the history the model reads.
    let history = reqs3[0].body["messages"].to_string();
    assert!(history.contains("\"value\":0.2") && history.contains("sig-m1"));
    // finish_turn's result waits in the last user message; the new request joins it.
    let last = reqs3[0].body["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["content"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(last[0]["type"], "tool_result");
    assert_eq!(last[0]["tool_use_id"], "m4");
    let state = last[1]["text"].as_str().unwrap().to_string();
    assert!(state.contains("tone 0.2"), "{state}");
    let scores = take_scores(&sound);
    assert_eq!(piano_tone(&scores[2]), 0.35);
    assert_eq!(
        expand_notes(&scores[1], 48_000),
        expand_notes(&scores[2], 48_000)
    );
    let measured: Value =
        serde_json::from_str(last_results(&reqs3[2])[0]["content"].as_str().unwrap()).unwrap();
    let piano = measured["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["layer"] == "piano")
        .unwrap()
        .clone();
    println!("brighter piano: {piano}");
    // Brighter than take 2, still darker than take 1.
    let c3 = piano["centroid_hz"].as_f64().unwrap();
    assert_eq!(
        piano["before"]["centroid_hz"].as_f64(),
        Some(muffled_centroid)
    );
    assert!(
        muffled_centroid < c3 && c3 < original_centroid,
        "{muffled_centroid} < {c3} < {original_centroid}: {piano}"
    );
    let conv = read_json(&sound.join("conversation.json"));
    assert_eq!(conv["turns"].as_array().unwrap().len(), 3);
    assert_eq!(conv["turns"][2]["take"], 3);
    assert_eq!(conv["turns"][1]["steps"][1]["tool"], "set_params");
}

#[test]
fn scoped_edit_keeps_bars_1_to_12_and_recovers_from_a_tool_error() {
    if !have_piano("scoped_edit_keeps_bars_1_to_12_and_recovers_from_a_tool_error") {
        return;
    }
    let sound = scratch("scoped").join("loop");
    let one = new_sound(&sound);
    let mix1 = one["take"]["render"]["path"].as_str().unwrap().to_string();

    let edit_plan = json!({
        "intent": "add_melody",
        "scope": {"bars": {"start": 13, "end": 16}},
        "layers": [],
        "params": {"preset": "upright_piano", "register": "high", "dynamics": "soft", "gain_db": -6.0},
        "notes": ["a subtle melody over the last four bars"],
    });
    let script = vec![
        // Out of scope (the kick is another layer): refused, and the model recovers.
        message(
            vec![tool_use(
                "s1",
                "set_params",
                json!({"layer": "kick", "params": [{"name": "tone", "value": 0.3}]}),
            )],
            "tool_use",
        ),
        message(
            vec![tool_use(
                "s2",
                "compose_edit",
                json!({"edit_plan_json": edit_plan.to_string()}),
            )],
            "tool_use",
        ),
        message(
            vec![text(
                "Added a soft upright piano melody in bars 13-16; bars 1-12 are untouched.",
            )],
            "end_turn",
        ),
    ];
    let model = ScriptedModel::new(script.clone());
    let mut req = ask(&sound, "add a subtle melody here");
    req.scope = Some(TakeScope {
        layer: "piano".into(),
        bars: BarRange { start: 13, end: 16 },
    });
    let two = ops::ask_with(&req, &options(model.clone(), &events())).expect("take 2");
    let reqs = model.requests();
    reqs.iter().for_each(assert_api_rules);
    assert_history_appended(&reqs, &script);
    let refused = &last_results(&reqs[1])[0];
    assert_eq!(refused["is_error"], true);
    assert!(
        refused["content"]
            .as_str()
            .unwrap()
            .contains("outside the request's scope"),
        "{refused}"
    );
    assert!(
        reqs[0].body["messages"].as_array().unwrap().last().unwrap()["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Scope of this request: bars 13–16")
    );
    let steps = two["agent"]["steps"].as_array().unwrap();
    assert_eq!(steps[0]["ok"], false);
    assert_eq!(steps[1]["ok"], true);
    let t2 = &two["take"];
    let changed = t2["changed"].as_object().unwrap();
    assert_eq!(changed.len(), 1, "{changed:?}");
    assert!(changed.values().all(|r| r == &json!([13, 16])));
    assert_eq!(t2["scope"]["bars"], json!({"start": 13, "end": 16}));
    assert!(
        t2["ask"]
            .as_str()
            .unwrap()
            .starts_with("add a subtle melody here (bars 13–16")
    );
    // Bars 4-12 bit-identical (bars 1-3 hold the new melody's tail wrapped round the loop).
    let runs = t2["identical_bars"].as_array().unwrap();
    println!("identical bars: {runs:?}; {}", t2["untouched"]);
    assert!(
        runs.iter()
            .any(|r| r[0].as_u64().unwrap() <= 4 && r[1].as_u64().unwrap() >= 12),
        "{runs:?}"
    );
    let a = praxis_audio_io::decode::decode_file(Path::new(&mix1)).unwrap();
    let b = praxis_audio_io::decode::decode_file(Path::new(t2["render"]["path"].as_str().unwrap()))
        .unwrap();
    assert_eq!(a.audio.len(), b.audio.len());
}

#[test]
fn a_locked_layer_is_refused_out_loud() {
    if !have_piano("a_locked_layer_is_refused_out_loud") {
        return;
    }
    let sound = scratch("locked").join("loop");
    new_sound(&sound);
    // The person locks the kick by hand (a hand edit, no model).
    let mut lock = ask(&sound, "Lock the kick");
    lock.provider = Some(ops::ProviderChoice::Offline);
    lock.patch = Some(
        serde_json::from_value(json!({
            "description": "lock the kick",
            "origin": "user",
            "ops": [{"op": "set_lock", "id": "kick", "locked": true}],
        }))
        .unwrap(),
    );
    let revision = read_json(&sound.join("takes.json"))["revision"]
        .as_u64()
        .unwrap();
    let locked = ops::ask(&lock).expect("the lock is committed");
    // A lock changes how the live score plays: committed and undoable, no render, no take.
    assert_eq!(locked["take"], Value::Null);
    assert_eq!(locked["no_take"], "live");
    assert_eq!(locked["revision"].as_u64().unwrap(), revision + 1);
    assert_eq!(locked["can_undo"], true);
    assert!(!sound.join("takes").join("2").exists(), "nothing rendered");

    let reply = "The kick is locked, so I left it alone; unlock it if you want me to change it.";
    let script = vec![
        message(
            vec![tool_use(
                "l1",
                "apply_patch",
                json!({"patch_json": json!({
                    "description": "quieter kick",
                    "ops": [{"op": "update_layer", "id": "kick", "update": {"gain_db": -6.0}}],
                }).to_string()}),
            )],
            "tool_use",
        ),
        message(vec![text(reply)], "end_turn"),
    ];
    let model = ScriptedModel::new(script.clone());
    // A refusal is a reply, not an error.
    let out = ops::ask_with(
        &ask(&sound, "make the kick quieter"),
        &options(model.clone(), &events()),
    )
    .expect("a reply without a take");
    assert_eq!(out["take"], Value::Null);
    assert_eq!(out["no_take"], "refused");
    assert_eq!(out["reply"], reply);
    let agent = &out["agent"];
    assert_eq!(agent["outcome"], "no_change");
    assert_eq!(agent["reply"], reply);
    assert_eq!(agent["steps"][0]["ok"], false);
    let reqs = model.requests();
    let r = &last_results(&reqs[1])[0];
    assert_eq!(r["is_error"], true);
    assert!(r["content"].as_str().unwrap().contains("locked"), "{r}");
    // The state the model read said so too.
    let state = reqs[0].body["messages"].as_array().unwrap().last().unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(state.contains("LOCKED"), "{state}");
    let takes = ops::takes(&ops::TakesRequest {
        sound: sound.display().to_string(),
        include_scores: false,
    })
    .unwrap();
    assert_eq!(takes["takes"].as_array().unwrap().len(), 1, "no take 2");
    let conv = read_json(&sound.join("conversation.json"));
    assert_eq!(conv["turns"][1]["outcome"], "no_change");
    assert_eq!(conv["turns"][1]["reply"], reply);

    // The lock is one undo step.
    let undone = ops::undo_redo(&ops::UndoRedoRequest {
        sound: sound.display().to_string(),
        action: ops::HistoryAction::Undo,
    })
    .unwrap();
    assert_eq!(undone["moved"], true);
    let live: Score = serde_json::from_value(
        ops::takes(&ops::TakesRequest {
            sound: sound.display().to_string(),
            include_scores: false,
        })
        .unwrap()["score"]
            .clone(),
    )
    .unwrap();
    assert!(!live.layer("kick").unwrap().locked, "undo unlocks the kick");
}

#[test]
fn a_refusal_the_model_names_and_a_mute_by_patch_make_no_take() {
    if !have_piano("a_refusal_the_model_names_and_a_mute_by_patch_make_no_take") {
        return;
    }
    let sound = scratch("refused-by-name").join("loop");
    new_sound(&sound);
    // A mute through `ask --patch` makes no take either.
    let mut mute = ask(&sound, "Mute the kick");
    mute.patch = Some(
        serde_json::from_value(json!({
            "description": "Kick muted",
            "origin": "user",
            "ops": [{"op": "update_layer", "id": "kick", "update": {"muted": true}}],
        }))
        .unwrap(),
    );
    let muted = ops::ask(&mute).unwrap();
    assert_eq!(muted["no_take"], "live");
    assert_eq!(muted["reply"], "Kick muted; no take was made.");
    // A gain change by patch is a sound change: it still makes a take.
    let mut louder = ask(&sound, "Louder kick");
    louder.patch = Some(
        serde_json::from_value(json!({
            "description": "kick +1 dB",
            "origin": "user",
            "ops": [{"op": "update_layer", "id": "kick", "update": {"gain_db": -4.0}}],
        }))
        .unwrap(),
    );
    assert_eq!(ops::ask(&louder).unwrap()["take"]["n"], 2);

    // The model refuses without trying and says why through finish_turn.
    let reply = "I would rather not add a riser: you asked for none.";
    let model = ScriptedModel::new(vec![message(
        vec![
            text(reply),
            tool_use(
                "f1",
                "finish_turn",
                json!({"title": "", "no_take": "refused"}),
            ),
        ],
        "tool_use",
    )]);
    let out = ops::ask_with(&ask(&sound, "add a riser"), &options(model, &events())).unwrap();
    assert_eq!(out["take"], Value::Null);
    assert_eq!(out["no_take"], "refused");
    assert_eq!(out["reply"], reply);
    assert_eq!(out["current"], 2);
}

#[test]
fn undo_and_a_view_are_replies_without_a_take() {
    if !have_piano("undo_and_a_view_are_replies_without_a_take") {
        return;
    }
    let sound = scratch("undo-view").join("loop");
    new_sound(&sound);
    let mut louder = ask(&sound, "Louder kick");
    louder.patch = Some(
        serde_json::from_value(json!({
            "description": "kick +1 dB",
            "origin": "user",
            "ops": [{"op": "update_layer", "id": "kick", "update": {"gain_db": -4.0}}],
        }))
        .unwrap(),
    );
    ops::ask(&louder).unwrap();

    let reply = "Take 1 is current again; say redo to bring the louder kick back.";
    let model = ScriptedModel::new(vec![
        message(
            vec![tool_use("u1", "undo_redo", json!({"action": "undo"}))],
            "tool_use",
        ),
        message(vec![text(reply)], "end_turn"),
    ]);
    let ev = events();
    let out = ops::ask_with(&ask(&sound, "undo what you just did"), &options(model, &ev))
        .expect("an undo is a reply");
    assert_eq!(out["take"], Value::Null);
    assert_eq!(out["no_take"], "undo");
    assert_eq!(out["reply"], reply);
    assert_eq!(out["current"], 1);
    assert_eq!(out["can_redo"], true);
    assert_eq!(step_words(&ev), ["Undoing the last step"]);

    let model = ScriptedModel::new(vec![
        message(
            vec![tool_use("v1", "open_view", json!({"view": "notes"}))],
            "tool_use",
        ),
        message(vec![text("The notes are open.")], "end_turn"),
    ]);
    let out = ops::ask_with(
        &ask(&sound, "show me the notes"),
        &options(model, &events()),
    )
    .expect("a view is a reply");
    assert_eq!(out["take"], Value::Null);
    assert_eq!(out["no_take"], "view");
    let views = read_json(&sound.join("takes.json"))["views"].clone();
    assert_eq!(views.as_array().unwrap().last().unwrap()["view"], "notes");
    let takes = read_json(&sound.join("takes.json"))["takes"].clone();
    assert_eq!(takes.as_array().unwrap().len(), 2, "no take for either");
}

#[test]
fn the_step_budget_is_reached_and_explained() {
    let sound = scratch("budget").join("loop");
    let read = || json!({"part": "summary", "layer": "", "take": 0});
    let reply = "I read the sound but ran out of steps before changing it.";
    let script = vec![
        message(vec![tool_use("r1", "read_sound", read())], "tool_use"),
        message(
            vec![
                tool_use("r2", "read_sound", read()),
                tool_use("r3", "catalogue", json!({"preset": "felt_piano"})),
            ],
            "tool_use",
        ),
        message(vec![text(reply)], "end_turn"),
    ];
    let model = ScriptedModel::new(script.clone());
    let mut opts = options(model.clone(), &events());
    opts.agent.max_steps = 2;
    let out = ops::ask_with(&ask(&sound, "something calm"), &opts).unwrap();
    assert_eq!(out["take"], Value::Null);
    assert_eq!(out["no_take"], "nothing_to_change");
    assert_eq!(out["reply"], reply);
    assert_eq!(out["agent"]["outcome"], "out_of_steps");
    let reqs = model.requests();
    assert_eq!(reqs.len(), 3);
    reqs.iter().for_each(assert_api_rules);
    assert_history_appended(&reqs, &script);
    // Both parallel results in one user message, the second not run, then the wrap-up note.
    let results = last_results(&reqs[2]);
    assert_eq!(results.len(), 3);
    assert_eq!(results[0]["tool_use_id"], "r2");
    assert!(results[0].get("is_error").is_none());
    assert_eq!(results[1]["tool_use_id"], "r3");
    assert_eq!(results[1]["is_error"], true);
    assert!(
        results[1]["content"]
            .as_str()
            .unwrap()
            .contains("step budget")
    );
    assert_eq!(results[2]["type"], "text");
    assert!(
        results[2]["text"]
            .as_str()
            .unwrap()
            .contains("Do not call any more tools")
    );
}

#[test]
fn ask_user_ends_the_turn_and_the_answer_continues_it() {
    let sound = scratch("question").join("loop");
    let q = "Should it be a seamless loop or a one-shot sting?";
    let model = ScriptedModel::new(vec![message(
        vec![
            thinking("sig-q"),
            tool_use("q1", "ask_user", json!({"question": q})),
            tool_use(
                "q2",
                "read_sound",
                json!({"part": "summary", "layer": "", "take": 0}),
            ),
        ],
        "tool_use",
    )]);
    let out = ops::ask_with(
        &ask(&sound, "make me something for a logo"),
        &options(model.clone(), &events()),
    )
    .expect("a question is a reply");
    assert_eq!(out["take"], Value::Null);
    assert_eq!(out["no_take"], "question");
    assert_eq!(out["reply"], q);
    assert_eq!(out["current"], 0);
    assert_eq!(out["agent"]["outcome"], "question");
    assert_eq!(out["agent"]["question"], q);
    assert_eq!(
        model.requests().len(),
        1,
        "the turn ends without another request"
    );
    let conv = read_json(&sound.join("conversation.json"));
    let msgs = conv["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[2]["role"], "user");
    assert_eq!(
        msgs[2]["content"][1]["is_error"], true,
        "calls after ask_user are not run"
    );
    assert!(!sound.join("takes.json").exists(), "no take");

    // The answer joins the unanswered tool results: no two user messages in a row.
    let model = ScriptedModel::new(vec![message(
        vec![text(
            "A seamless loop it is; tell me the mood and I will compose it.",
        )],
        "end_turn",
    )]);
    let out = ops::ask_with(&ask(&sound, "a loop"), &options(model.clone(), &events())).unwrap();
    assert_eq!(out["agent"]["outcome"], "no_change");
    assert_eq!(out["no_take"], "nothing_to_change");
    let sent = model.requests()[0].body["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(sent.len(), 3);
    assert_eq!(&sent[..2], &msgs[..2]);
    let last = sent[2]["content"].as_array().unwrap();
    assert_eq!(last.len(), 3);
    assert_eq!(last[0]["type"], "tool_result");
    assert!(last[2]["text"].as_str().unwrap().ends_with("a loop"));
}

#[test]
fn refusals_and_api_failures() {
    let sound = scratch("refusal").join("loop");
    let mut refusal = message(vec![], "refusal");
    refusal["stop_details"] = json!({"type": "refusal", "category": "cyber"});
    let model = ScriptedModel::new(vec![refusal]);
    // A declined request is a reply (nothing changed), not a failure.
    let out = ops::ask_with(&ask(&sound, "something"), &options(model, &events())).unwrap();
    assert_eq!(out["no_take"], "refused");
    let said = out["reply"].as_str().unwrap();
    assert!(
        said.contains("declined") && said.contains("cyber"),
        "{said}"
    );
    assert_eq!(out["agent"]["outcome"], "declined");

    let model = Arc::new(ScriptedModel::default());
    model.push(Reply::Error(ModelError::Http {
        status: 500,
        message: "api_error: boom".into(),
    }));
    let err = ops::ask_with(&ask(&sound, "again"), &options(model, &events())).unwrap_err();
    assert_eq!(err.code, ErrorCode::ProviderFailed);
    assert!(err.message.contains("HTTP 500"));
    let conv = read_json(&sound.join("conversation.json"));
    assert_eq!(conv["turns"][1]["outcome"], "failed");
    // Two requests that got no answer are one user message.
    assert_eq!(conv["messages"].as_array().unwrap().len(), 1);

    // provider: claude without a client or key is no_api_key; offline never reaches a model.
    if ops::key::resolve_key().is_err() {
        let mut req = ask(&sound, "x");
        req.provider = Some(ops::ProviderChoice::Claude);
        assert_eq!(ops::ask(&req).unwrap_err().code, ErrorCode::NoApiKey);
    }
}

// --------------------------------------------------------------------------- replay over HTTP

/// The headers of each request sent.
type Headers = Arc<Mutex<Vec<Vec<(String, String)>>>>;

/// Serves recorded SSE streams in order and keeps the headers it was sent.
struct Replay {
    streams: Mutex<Vec<String>>,
    headers: Headers,
}

impl Poster for Replay {
    fn post(
        &self,
        _url: &str,
        headers: &[(String, String)],
        _body: &Value,
    ) -> Result<PostResponse, String> {
        self.headers.lock().unwrap().push(headers.to_vec());
        let body = self.streams.lock().unwrap().remove(0);
        Ok(PostResponse {
            status: 200,
            retry_after_s: None,
            body: Box::new(std::io::Cursor::new(body.into_bytes())),
        })
    }
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/agent")
            .join(name),
    )
    .unwrap()
}

#[test]
fn replayed_streams_through_the_http_client() {
    let sound = scratch("replay").join("loop");
    let headers = Arc::new(Mutex::new(Vec::new()));
    let client = HttpClient::with_parts(
        "http://replay.invalid/v1/messages".into(),
        Box::new(Replay {
            streams: Mutex::new(vec![fixture("read_summary.sse"), fixture("ask_user.sse")]),
            headers: headers.clone(),
        }),
        Box::new(|| Ok(FAKE_KEY.to_string())),
    );
    let ev = events();
    let out = ops::ask_with(
        &ask(&sound, "make a sound for my app's logo"),
        &options(Arc::new(client), &ev),
    )
    .unwrap();
    assert_eq!(out["agent"]["outcome"], "question");
    assert_eq!(out["no_take"], "question");
    assert_eq!(
        out["reply"],
        "Should the logo sound be a short rising sting or a soft tonal chime?"
    );
    let agent = &out["agent"];
    assert_eq!(agent["usage"]["requests"], 2);
    assert!(agent["usage"]["cache_read_input_tokens"].as_u64().unwrap() > 0);

    // The key went only into the x-api-key header of each request.
    for h in headers.lock().unwrap().iter() {
        let with_key: Vec<_> = h.iter().filter(|(_, v)| v.contains(FAKE_KEY)).collect();
        assert_eq!(with_key.len(), 1);
        assert_eq!(with_key[0].0, "x-api-key");
        let beta = &h.iter().find(|(k, _)| k == "anthropic-beta").unwrap().1;
        assert!(
            beta.contains("compact-2026-01-12") && beta.contains("server-side-fallback-2026-07-01")
        );
    }
    // ... and nowhere else.
    for entry in std::fs::read_dir(&sound).unwrap().flatten() {
        if entry.path().is_file() {
            let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
            assert!(!text.contains(FAKE_KEY), "{:?}", entry.path());
        }
    }
    assert!(!out.to_string().contains(FAKE_KEY));
    // The streamed thinking block (with its signature) is kept exactly.
    let conv = read_json(&sound.join("conversation.json"));
    let assistant = &conv["messages"][1]["content"];
    assert_eq!(assistant[0]["type"], "thinking");
    assert_eq!(
        assistant[0]["signature"],
        "EqQBCkYIBxgCKkDreplayfixturesignature"
    );
    assert_eq!(
        assistant[1]["input"],
        json!({"part": "summary", "layer": "", "take": 0})
    );
}

/// Sizes of a typical request, for the cost estimate in the report: the cached prefix (tools and
/// system prompt) and what each turn adds.
#[test]
fn request_sizes_for_the_cost_estimate() {
    let system = praxis_cli::agent::prompt::system_prompt();
    let tools = serde_json::to_string(&praxis_cli::agent::tools::definitions()).unwrap();
    println!(
        "cached prefix: system prompt {} bytes, tools {} bytes, total {} bytes",
        system.len(),
        tools.len(),
        system.len() + tools.len()
    );
    assert!(
        system.len() + tools.len() < 120_000,
        "the prefix stays small"
    );
}

/// "Make these bars muffled" with bars 5-8 of the piano pointed at: a tone change held to the
/// bars by automation passes the request's scope, and the render keeps every other bar
/// identical within the patch contract (bit-identical, or below -120 dBFS where the piano's
/// recirculating reverb carries the change).
#[test]
fn a_bar_limited_tone_edit_passes_the_scope_and_keeps_the_other_bars() {
    if !have_piano("a_bar_limited_tone_edit_passes_the_scope_and_keeps_the_other_bars") {
        return;
    }
    let sound = scratch("tone-bars").join("loop");
    let one = new_sound(&sound);
    let score = take_scores(&sound).remove(0);
    let piano = score.layer("piano").unwrap();
    let tone = piano.instrument.params.get("tone").copied().unwrap_or(0.5);
    let mut automation = serde_json::to_value(&piano.automation).unwrap();
    automation.as_array_mut().unwrap().push(json!({
        "target": "instrument.tone",
        "curve": "step",
        "points": [{"bar": 1.0, "value": tone}, {"bar": 5.0, "value": 0.15}, {"bar": 9.0, "value": tone}],
    }));
    let patch = json!({
        "description": "muffle the piano in bars 5-8",
        "ops": [{"op": "update_layer", "id": "piano", "update": {"automation": automation}}],
    });
    let script = vec![
        message(
            vec![tool_use(
                "t1",
                "apply_patch",
                json!({"patch_json": patch.to_string()}),
            )],
            "tool_use",
        ),
        message(vec![tool_use("t2", "render_check", json!({}))], "tool_use"),
        message(
            vec![
                text("Muffled the piano in bars 5-8 only; every other bar is as it was."),
                tool_use(
                    "t3",
                    "finish_turn",
                    json!({"title": "Muffled piano, bars 5-8", "no_take": ""}),
                ),
            ],
            "tool_use",
        ),
    ];
    let model = ScriptedModel::new(script);
    let mut req = ask(&sound, "make these bars muffled");
    req.scope = Some(TakeScope {
        layer: "piano".into(),
        bars: BarRange { start: 5, end: 8 },
    });
    let two = ops::ask_with(&req, &options(model.clone(), &events())).expect("take 2");
    let steps = two["agent"]["steps"].as_array().unwrap();
    assert_eq!(steps[0]["ok"], true, "the scope accepts it: {}", steps[0]);
    let t2 = &two["take"];
    assert_eq!(t2["title"], "Muffled piano, bars 5-8");
    assert_eq!(t2["changed"], json!({"piano": [5, 8]}));

    let a = praxis_audio_io::decode::decode_file(Path::new(
        one["take"]["render"]["path"].as_str().unwrap(),
    ))
    .unwrap();
    let b = praxis_audio_io::decode::decode_file(Path::new(t2["render"]["path"].as_str().unwrap()))
        .unwrap();
    let after = take_scores(&sound).remove(1);
    let db =
        praxis_cli::ops::changes::bar_differences_db(&after, &a.audio, &b.audio, b.sample_rate);
    println!("largest difference per bar (dBFS): {db:?}");
    println!(
        "untouched: {}; identical: {}",
        t2["untouched"], t2["identical_bars"]
    );
    // The change is audible in its bars...
    for bar in 5..=8 {
        assert!(db[bar - 1] > -60.0, "bar {bar}: {}", db[bar - 1]);
    }
    // ... and nothing else moved beyond rounding, outside the notes' release and reverb tail
    // just after the bars (bars 9-10, allowed by contracts/patch.md).
    for bar in (1..=4).chain(11..=16) {
        assert!(
            db[bar - 1] < praxis_cli::ops::changes::ROUNDING_DBFS,
            "bar {bar}: {} dBFS",
            db[bar - 1]
        );
    }
}

/// The live run's model ends a take with `finish_turn` and no text: its `reply` is the take's
/// `say` and the turn's reply (not the change summary), and its `title` replaces a kept
/// variation's default words.
#[test]
fn the_reply_comes_through_finish_turn() {
    if !have_piano("the_reply_comes_through_finish_turn") {
        return;
    }
    let sound = scratch("finish-reply").join("loop");
    new_sound(&sound);

    let reply = "The piano is muffled: its brightness dropped by about a tenth and the highs by \
                 3 dB; every note is as it was.";
    let model = ScriptedModel::new(vec![
        message(
            vec![tool_use(
                "p1",
                "set_params",
                json!({"layer": "piano", "params": [{"name": "tone", "value": 0.25}]}),
            )],
            "tool_use",
        ),
        message(vec![tool_use("p2", "render_check", json!({}))], "tool_use"),
        // Only the tool call, no text: the shape the real model used in the live run.
        message(
            vec![
                thinking("sig-f"),
                tool_use(
                    "p3",
                    "finish_turn",
                    json!({"reply": reply, "title": "Muffled felt piano", "no_take": ""}),
                ),
            ],
            "tool_use",
        ),
    ]);
    let ev = events();
    let two = ops::ask_with(&ask(&sound, "make the piano muffled"), &options(model, &ev))
        .expect("take 2");
    assert_eq!(two["take"]["n"], 2);
    assert_eq!(two["take"]["say"], reply);
    assert_eq!(two["take"]["title"], "Muffled felt piano");
    assert_eq!(two["agent"]["reply"], reply);
    assert!(ev.lock().unwrap().iter().any(|e| matches!(
        e,
        AgentEvent::Finished { reply: r, .. } if r == reply
    )));

    // A kept variation takes the assistant's title and words instead of "Variation 1, ...".
    let kept = "Kept the second of two new endings for the piano in bars 13-16; bars 3-12 are \
                identical note for note.";
    let model = ScriptedModel::new(vec![
        message(
            vec![tool_use(
                "v1",
                "make_variations",
                json!({"layers": ["piano"], "start_bar": 13, "end_bar": 16, "count": 2}),
            )],
            "tool_use",
        ),
        message(
            vec![tool_use("v2", "keep_variation", json!({"index": 2}))],
            "tool_use",
        ),
        message(
            vec![tool_use(
                "v3",
                "finish_turn",
                json!({"reply": kept, "title": "New piano ending", "no_take": ""}),
            )],
            "tool_use",
        ),
    ]);
    let three = ops::ask_with(
        &ask(
            &sound,
            "two alternatives for the last four bars; keep the better one",
        ),
        &options(model, &events()),
    )
    .expect("take 3");
    assert_eq!(three["take"]["n"], 3);
    assert_eq!(three["take"]["title"], "New piano ending");
    assert_eq!(three["take"]["say"], kept);
    assert_eq!(three["agent"]["reply"], kept);
    let stored = read_json(&sound.join("takes.json"))["takes"][2].clone();
    assert_eq!(stored["title"], "New piano ending");
    assert_eq!(stored["say"], kept);
    assert_eq!(stored["provider"], "variation");
}
