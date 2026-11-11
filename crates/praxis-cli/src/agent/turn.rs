//! One assistant turn: the request in, a take (or a question, or a plain answer) out.
//!
//! 1. Hold the sound's `.agent-lock` for the turn (two turns on one sound run one after the
//!    other), read `conversation.json` and start a draft from the current take.
//! 2. Append the request, with the sound's state, to the history.
//! 3. Loop: send the request; append the assistant content exactly as returned; check
//!    `stop_reason` before running anything (`refusal` ends the turn, a reply cut off at
//!    `max_tokens` runs no tool); run every tool call of the response and return all results in
//!    one user message (`is_error` for failures); stop when the model answers without calling a
//!    tool, asks the person a question, or runs out of steps or time (then it is told to wrap up
//!    and gets one more response to say what it did).
//! 4. Commit the draft as one take (who "Agent · Assistant", `say` = the reply), record the turn
//!    and save the conversation.

use super::client::{HttpClient, ModelClient, ModelError};
use super::conversation::{Conversation, Outcome, Step, Turn, Usage};
use super::draft::Draft;
use super::draft::scope_label;
use super::tools::{self, ToolCtx};
use super::{AGENT_NAME, AgentEvent, prompt, request, words};
use crate::error::{ErrorCode, OpError};
use crate::ops::sound::{Sound, SoundLock, TakeRecord, normalize_who, resolve_sound};
use crate::ops::takes::{AskOptions, AskRequest, DEFAULT_WHO, Words, take_payload};
use crate::ops::{self, OpenViewRequest};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Instant;

/// Held for a whole turn.
pub const AGENT_LOCK_FILE: &str = ".agent-lock";

/// Sent with the tool results when the step budget or the time is used up.
pub const WRAP_UP: &str = "[Praxis] The step budget for this request is used up. Do not call \
any more tools. Reply to the person now in one or two sentences: what you changed, and what is \
left to do.";

fn text_of(content: &[Value]) -> String {
    content
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn tool_result(id: &str, content: String, is_error: bool) -> Value {
    let mut v = json!({"type": "tool_result", "tool_use_id": id, "content": content});
    if is_error {
        v["is_error"] = json!(true);
    }
    v
}

/// Why a turn made no take (`no_take` in the payload).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoTake {
    /// The assistant asked a clarifying question (`ask_user`).
    Question,
    /// It could not or would not do it (a locked layer, a declined request).
    Refused,
    /// It stepped the history back or forward, or went back to a take.
    Undo,
    /// It only opened a view.
    View,
    /// Nothing needed changing (an answer, advice, out of steps before any change).
    NothingToChange,
}

impl NoTake {
    pub fn as_str(self) -> &'static str {
        match self {
            NoTake::Question => "question",
            NoTake::Refused => "refused",
            NoTake::Undo => "undo",
            NoTake::View => "view",
            NoTake::NothingToChange => "nothing_to_change",
        }
    }
}

/// What the assistant said with `finish_turn`.
#[derive(Clone, Debug, Default)]
struct Finish {
    title: String,
    no_take: String,
}

/// The sound's state as the model reads it at the top of a request.
fn state_text(
    sound: &Sound,
    conv: &Conversation,
    draft: &Draft,
    who: &str,
    req: &AskRequest,
) -> String {
    let mut out = String::from("[Sound state]\n");
    let Some(score) = &draft.base else {
        out.push_str(
            "This sound is new: it has no takes yet. Compose it from the request (compose_sound), \
             render and check it, then answer.\n",
        );
        out.push_str(&format!("\n[Request from {who}]\n"));
        return out;
    };
    let current = sound.takes.take(sound.takes.current);
    out.push_str(&format!(
        "Current take: {} of {}{}. Revision {}.\n",
        sound.takes.current,
        sound.takes.takes.len(),
        current
            .map(|t| format!(" (\"{}\", by {})", t.title, t.who))
            .unwrap_or_default(),
        sound.takes.revision
    ));
    let known = conv
        .turns
        .iter()
        .map(|t| t.take.unwrap_or(t.from_take))
        .max();
    if let Some(known) = known {
        let newer: Vec<String> = sound
            .takes
            .takes
            .iter()
            .filter(|t| t.n > known)
            .map(|t| format!("take {} (\"{}\", by {})", t.n, t.ask, t.who))
            .collect();
        if !newer.is_empty() {
            out.push_str(&format!(
                "Made since your last turn: {}.\n",
                newer.join("; ")
            ));
        }
    }
    let summary = tools::detailed_summary(score);
    out.push_str(&format!(
        "{} bars, {} BPM, {}, {}.\nLayers:\n",
        score.bars,
        score.tempo.bpm,
        score.key.display(),
        summary["time_signature"].as_str().unwrap_or("")
    ));
    if let Some(layers) = summary["layers"].as_array() {
        for l in layers {
            let params = l["params"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| format!("{k} {v}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let effects = l["effects"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|e| e["kind"].as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            out.push_str(&format!(
                "- `{}` \"{}\": {} ({}), gain {} dB, bars {}{}{}{}\n",
                l["id"].as_str().unwrap_or(""),
                l["name"].as_str().unwrap_or(""),
                l["preset"].as_str().unwrap_or(""),
                params,
                l["gain_db"],
                l["bars"],
                if effects.is_empty() {
                    String::new()
                } else {
                    format!(", effects: {effects}")
                },
                if l["muted"] == json!(true) {
                    ", muted"
                } else {
                    ""
                },
                if l["locked"] == json!(true) {
                    ", LOCKED (never change it)"
                } else {
                    ""
                },
            ));
        }
    }
    if let Some(scope) = &req.scope {
        out.push_str(&format!(
            "Scope of this request: {}. Change nothing outside bars {}-{} of `{}` (a new layer \
             inside those bars is allowed).\n",
            scope_label(score, scope),
            scope.bars.start,
            scope.bars.end,
            scope.layer
        ));
    }
    out.push_str(&format!("\n[Request from {who}]\n"));
    out
}

/// Runs one assistant turn for `ask` (see the module docs). The payload is `ask`'s, plus
/// `agent`. A turn without a take (a question, a refusal, an undo, a view, nothing to change) is
/// a reply, not an error: `take: null`, `no_take` (why) and `reply`. Only a real failure (the
/// API unreachable, no key, a render that failed) is an error, with the reply as the message and
/// the `agent` record as the only issue.
pub fn run_ask(req: &AskRequest, opts: &AskOptions) -> Result<Value, OpError> {
    let dir = resolve_sound(&req.sound)?;
    let text = req.text.trim().to_string();
    if text.is_empty() {
        return Err(OpError::new(ErrorCode::Error, "say what to make or change"));
    }
    if opts.client.is_none() {
        // A test address that is not this machine is refused before anything is written.
        crate::ops::generate::claude_url_override()?;
    }
    let _turn = SoundLock::acquire_file(&dir, AGENT_LOCK_FILE)?;
    let sound = Sound::open(&dir)?;
    if sound.takes.takes.is_empty() {
        if sound.project.is_some() {
            return Err(OpError::new(
                ErrorCode::Error,
                format!(
                    "{} holds a project.praxis without takes.json; use a new folder for a new sound",
                    dir.display()
                ),
            ));
        }
        if req.scope.is_some() {
            return Err(OpError::new(
                ErrorCode::Error,
                "this sound has no takes yet: describe the sound first (no patch or scope)",
            ));
        }
    }
    if let (Some(scope), Some(project)) = (&req.scope, &sound.project) {
        let score = &project.score;
        if score.layer(&scope.layer).is_none() {
            return Err(OpError::new(
                ErrorCode::PatchRejected,
                format!(
                    "the scope names layer `{}`, which is not in the sound",
                    scope.layer
                ),
            ));
        }
        let b = scope.bars;
        if b.start < 1 || b.end < b.start || b.end > score.bars {
            return Err(OpError::new(
                ErrorCode::PatchRejected,
                format!(
                    "the scope's bars {}-{} are outside the sound's bars 1-{}",
                    b.start, b.end, score.bars
                ),
            ));
        }
    }

    let mut config = opts.agent.clone();
    if let Some(m) = req
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        config.model = m.to_string();
    }
    let client: Arc<dyn ModelClient> = match &opts.client {
        Some(c) => c.clone(),
        None => Arc::new(HttpClient::new()),
    };
    let emit = |e: AgentEvent| {
        if let Some(p) = &opts.progress {
            p(&e);
        }
    };
    let seed = req.seed.unwrap_or(sound.takes.next_n() as u64);
    let mut draft = Draft::open(dir.clone(), req.scope.clone(), seed, req.skip_missing)?;
    let mut conv = Conversation::load(&dir)?;
    let who = normalize_who(req.who.as_deref(), DEFAULT_WHO);
    let from_take = draft.base_take;
    emit(AgentEvent::Started {
        request: text.clone(),
    });
    let first = format!("{}{text}", state_text(&sound, &conv, &draft, &who, req));
    conv.push_user(vec![json!({"type": "text", "text": first})]);
    drop(sound);

    let system = prompt::system_prompt();
    let tool_defs = tools::definitions();
    let mut steps: Vec<Step> = Vec::new();
    let mut usage = Usage::default();
    let started = Instant::now();
    let mut wrap_up_sent = false;
    let mut cutoffs = 0u32;
    let mut fail_code = ErrorCode::ProviderFailed;
    let mut finish: Option<Finish> = None;
    // What the turn did without a take, for `no_take`.
    let (mut undone, mut viewed, mut locked) = (false, false, false);

    let (mut outcome, mut reply) = loop {
        emit(AgentEvent::Thinking {
            n: usage.requests + 1,
        });
        let api = request::build(&config, &system, &tool_defs, &conv.messages);
        let msg = match client.send(&api) {
            Ok(m) => m,
            Err(e) => {
                if e == ModelError::NoApiKey {
                    fail_code = ErrorCode::NoApiKey;
                }
                break (
                    Outcome::Failed,
                    format!("I could not reach the model, so nothing was changed: {e}"),
                );
            }
        };
        usage.add(&msg["usage"]);
        let content: Vec<Value> = msg["content"].as_array().cloned().unwrap_or_default();
        let stop = msg["stop_reason"]
            .as_str()
            .unwrap_or("end_turn")
            .to_string();
        if !content.is_empty() {
            conv.push_assistant(Value::Array(content.clone()));
        }
        let said = text_of(&content);
        if !said.is_empty() {
            emit(AgentEvent::Said { text: said.clone() });
        }
        if stop == "refusal" {
            // Every tool_use in the history needs its result, even one that never runs.
            let unanswered: Vec<Value> = content
                .iter()
                .filter(|b| b["type"] == "tool_use")
                .map(|b| {
                    tool_result(
                        b["id"].as_str().unwrap_or(""),
                        "Not run: the request was declined.".into(),
                        true,
                    )
                })
                .collect();
            if !unanswered.is_empty() {
                conv.push_user(unanswered);
            }
            let category = msg["stop_details"]["category"]
                .as_str()
                .map(|c| format!(" ({c})"))
                .unwrap_or_default();
            break (
                Outcome::Declined,
                format!("The model declined this request{category}, so nothing was changed."),
            );
        }
        if stop == "pause_turn" {
            continue;
        }
        let calls: Vec<Value> = content
            .iter()
            .filter(|b| b["type"] == "tool_use")
            .cloned()
            .collect();
        if calls.is_empty() {
            let outcome = if wrap_up_sent {
                Outcome::OutOfSteps
            } else if draft.dirty() || draft.take_made.is_some() {
                Outcome::Take
            } else {
                Outcome::NoChange
            };
            break (outcome, said);
        }
        let truncated = matches!(
            stop.as_str(),
            "max_tokens" | "model_context_window_exceeded"
        );
        let over_time = started.elapsed() > config.time_limit;
        let mut results = Vec::new();
        let mut ended = false;
        let mut finished = false;
        for call in &calls {
            let id = call["id"].as_str().unwrap_or("");
            let name = call["name"].as_str().unwrap_or("");
            let input = &call["input"];
            if name == tools::FINISH_TURN && !truncated && !ended {
                if calls.len() > 1 {
                    results.push(tool_result(
                        id,
                        "Not run: call finish_turn as the only tool in its message, once you \
                         have seen the results of your other calls."
                            .into(),
                        true,
                    ));
                } else {
                    finish = Some(Finish {
                        title: input["title"].as_str().unwrap_or("").to_string(),
                        no_take: input["no_take"].as_str().unwrap_or("").to_string(),
                    });
                    results.push(tool_result(id, "Turn finished.".into(), false));
                    finished = true;
                }
                continue;
            }
            let refuse = if truncated {
                Some(
                    "Not run: your message was cut off (max_tokens) before this call was \
                     complete. Make one smaller call at a time.",
                )
            } else if ended {
                Some("Not run: the turn ended with ask_user.")
            } else if wrap_up_sent || steps.len() as u32 >= config.max_steps || over_time {
                Some("Not run: the step budget for this request is used up.")
            } else {
                None
            };
            if let Some(why) = refuse {
                results.push(tool_result(id, why.to_string(), true));
                continue;
            }
            let n = steps.len() as u32 + 1;
            let step_words = words::step_words(name, input, draft.score.as_ref());
            emit(AgentEvent::Step {
                n,
                tool: name.to_string(),
                words: step_words.clone(),
            });
            let r = tools::run(
                name,
                input,
                &mut ToolCtx {
                    draft: &mut draft,
                    request: &text,
                    config: &config,
                },
            );
            emit(AgentEvent::StepDone {
                n,
                tool: name.to_string(),
                ok: r.ok,
                words: words::done_words(name, r.ok, &r.value, &r.error),
            });
            steps.push(Step {
                tool: name.to_string(),
                input: input.clone(),
                ok: r.ok,
                words: step_words,
                result: r.short(),
            });
            match name {
                "undo_redo" => undone |= r.ok && r.value["moved"] == json!(true),
                "restore_take" => undone |= r.ok,
                "open_view" => viewed |= r.ok && !r.value["opened"].is_null(),
                _ => {}
            }
            locked |= !r.ok && r.error.contains("Only the person can unlock");
            results.push(tool_result(id, r.content(), !r.ok));
            ended |= r.ends_turn;
        }
        if truncated {
            cutoffs += 1;
        }
        if finished {
            // Not sent: the next request is added to this message.
            conv.push_user(results);
            let outcome = if draft.dirty() || draft.take_made.is_some() {
                Outcome::Take
            } else if wrap_up_sent {
                Outcome::OutOfSteps
            } else {
                Outcome::NoChange
            };
            break (outcome, said);
        }
        if ended {
            // Not sent: the person's answer is added to this message next time.
            conv.push_user(results);
            break (
                Outcome::Question,
                draft.question.clone().unwrap_or_default(),
            );
        }
        if wrap_up_sent || cutoffs >= 2 {
            conv.push_user(results);
            break (Outcome::OutOfSteps, said);
        }
        if steps.len() as u32 >= config.max_steps || started.elapsed() > config.time_limit {
            results.push(json!({"type": "text", "text": WRAP_UP}));
            wrap_up_sent = true;
        }
        conv.push_user(results);
    };

    // The take.
    let mut made: Option<(TakeRecord, Value)> = None;
    if matches!(
        outcome,
        Outcome::Take | Outcome::NoChange | Outcome::OutOfSteps
    ) && draft.dirty()
    {
        let say = if reply.trim().is_empty() {
            match &draft.new_sound {
                Some(n) => n.summary.clone(),
                None => {
                    let d = draft.merged_patch().description;
                    if d.trim().is_empty() {
                        "Changed by the assistant.".to_string()
                    } else {
                        format!("Changed: {d}.")
                    }
                }
            }
        } else {
            reply.clone()
        };
        let ask_text = match (&req.scope, &draft.base) {
            (Some(s), Some(base)) => format!("{text} ({})", scope_label(base, s)),
            _ => text.clone(),
        };
        // The assistant's own title, else a summary of what changed (never a clipped reply).
        let title = req
            .title
            .clone()
            .or_else(|| finish.as_ref().and_then(|f| words::clean_title(&f.title)))
            .unwrap_or_else(|| match &draft.score {
                Some(score) => words::summary_title(draft.base.as_ref(), score),
                None => "Assistant edit".to_string(),
            });
        let take_words = Words {
            who: normalize_who(Some(AGENT_NAME), DEFAULT_WHO),
            ask: ask_text,
            title,
            say: say.clone(),
            scope: req.scope.clone(),
            provider: format!("agent:{}", config.model),
            seed,
        };
        match draft.commit(take_words) {
            Ok(Some(t)) => {
                emit(AgentEvent::TakeMade { n: t.0.n });
                if let Some(view) = draft.opens {
                    let _ = ops::open_view(&OpenViewRequest {
                        sound: dir.display().to_string(),
                        view,
                        who: Some(AGENT_NAME.to_string()),
                    });
                }
                if outcome == Outcome::NoChange {
                    outcome = Outcome::Take;
                }
                if reply.trim().is_empty() {
                    reply = say;
                }
                made = Some(t);
            }
            Ok(None) => {}
            Err(e) => {
                fail_code = e.code;
                outcome = Outcome::Failed;
                reply = format!("{} The take could not be made: {}", reply.trim(), e.message)
                    .trim()
                    .to_string();
            }
        }
    }
    let take_n = made.as_ref().map(|(t, _)| t.n).or(draft.take_made);
    if take_n.is_some() && outcome == Outcome::NoChange {
        outcome = Outcome::Take;
    }
    if reply.trim().is_empty() {
        reply = match (outcome, take_n) {
            (_, Some(n)) => format!("Made take {n}."),
            (Outcome::OutOfSteps, None) => {
                "I ran out of steps for this request before changing anything.".to_string()
            }
            _ => "I did not change anything.".to_string(),
        };
    }

    // The record.
    conv.turns.push(Turn {
        n: conv.turns.len() as u32 + 1,
        request: text.clone(),
        who,
        scope: req
            .scope
            .as_ref()
            .map(|s| serde_json::to_value(s).unwrap_or(Value::Null)),
        model: config.model.clone(),
        from_take,
        outcome,
        take: take_n,
        reply: reply.clone(),
        steps: steps.clone(),
        usage: usage.clone(),
    });
    conv.save(&dir)?;
    emit(AgentEvent::Finished {
        outcome: outcome.as_str().to_string(),
        reply: reply.clone(),
    });

    let agent = json!({
        "outcome": outcome.as_str(),
        "reply": reply,
        "question": if outcome == Outcome::Question { json!(reply) } else { Value::Null },
        "model": config.model,
        "turn": conv.turns.len(),
        "steps": steps.iter().map(|s| json!({
            "tool": s.tool, "words": s.words, "ok": s.ok, "result": s.result,
        })).collect::<Vec<_>>(),
        "usage": usage,
    });
    match take_n {
        Some(n) => {
            let sound = Sound::open(&dir)?;
            let record = sound.takes.take(n).cloned().ok_or_else(|| {
                OpError::new(ErrorCode::Error, format!("take {n} is not in takes.json"))
            })?;
            let mut v = take_payload(&sound, &record, req.include_score);
            if let Some((_, p)) = &made
                && !p["repairs"].is_null()
            {
                v["repairs"] = p["repairs"].clone();
            }
            v["provider_note"] = Value::Null;
            v["agent"] = agent;
            Ok(v)
        }
        None if outcome == Outcome::Failed => Err(OpError::new(fail_code, reply.clone())
            .with_issues(vec![json!({
                "code": "agent",
                "path": "",
                "message": reply,
                "agent": agent,
            })])),
        None => {
            // A reply without a take: a question, a refusal, an undo, a view, or nothing to do.
            let no_take = match outcome {
                Outcome::Question => NoTake::Question,
                Outcome::Declined => NoTake::Refused,
                _ if undone => NoTake::Undo,
                _ if locked || finish.as_ref().is_some_and(|f| f.no_take == "refused") => {
                    NoTake::Refused
                }
                _ if viewed => NoTake::View,
                _ => NoTake::NothingToChange,
            };
            let sound = Sound::open(&dir)?;
            let project = sound.project.as_ref();
            Ok(json!({
                "sound": dir.display().to_string(),
                "project": sound.project_path().display().to_string(),
                "revision": sound.takes.revision,
                "current": sound.takes.current,
                "take": null,
                "no_take": no_take.as_str(),
                "reply": reply,
                "score_summary": project.map(|p| ops::score_summary(&p.score)),
                "can_undo": project.is_some_and(|p| p.can_undo()),
                "can_redo": project.is_some_and(|p| p.can_redo()),
                "provider_note": null,
                "agent": agent,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_take_reasons_have_their_contract_names() {
        let all = [
            NoTake::Question,
            NoTake::Refused,
            NoTake::Undo,
            NoTake::View,
            NoTake::NothingToChange,
        ];
        let names: Vec<&str> = all.iter().map(|n| n.as_str()).collect();
        assert_eq!(
            names,
            ["question", "refused", "undo", "view", "nothing_to_change"]
        );
    }
}
