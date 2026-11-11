//! A scripted model for tests: no key, no network. It answers each request with the next
//! scripted message and records every request, so a test can drive the real loop, tools and
//! take history and then check what was sent.
//!
//! ```
//! use praxis_cli::agent::testing::{ScriptedModel, message, text, tool_use};
//! use serde_json::json;
//! let model = ScriptedModel::new(vec![
//!     message(vec![tool_use("t1", "read_sound", json!({"part": "summary", "layer": "", "take": 0}))], "tool_use"),
//!     message(vec![text("Nothing to change.")], "end_turn"),
//! ]);
//! # let _ = model;
//! ```

use super::client::{ApiRequest, ModelClient, ModelError};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// One scripted answer.
pub enum Reply {
    Message(Value),
    Error(ModelError),
    /// Built from the request (for answers that depend on what the loop sent).
    With(Box<dyn Fn(&ApiRequest) -> Value + Send + Sync>),
}

/// Answers requests from a script and records them.
#[derive(Default)]
pub struct ScriptedModel {
    script: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<ApiRequest>>,
}

impl std::fmt::Debug for ScriptedModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ScriptedModel")
    }
}

impl ScriptedModel {
    pub fn new(messages: Vec<Value>) -> Arc<ScriptedModel> {
        let m = ScriptedModel::default();
        m.extend(messages);
        Arc::new(m)
    }

    /// Adds messages to the end of the script.
    pub fn extend(&self, messages: Vec<Value>) {
        let mut s = self.script.lock().unwrap_or_else(|e| e.into_inner());
        s.extend(messages.into_iter().map(Reply::Message));
    }

    pub fn push(&self, reply: Reply) {
        self.script
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(reply);
    }

    /// Every request sent so far.
    pub fn requests(&self) -> Vec<ApiRequest> {
        self.requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Scripted answers not used yet.
    pub fn left(&self) -> usize {
        self.script.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

impl ModelClient for ScriptedModel {
    fn send(&self, req: &ApiRequest) -> Result<Value, ModelError> {
        self.requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(req.clone());
        let next = self
            .script
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front();
        match next {
            Some(Reply::Message(m)) => Ok(m),
            Some(Reply::Error(e)) => Err(e),
            Some(Reply::With(f)) => Ok(f(req)),
            None => Err(ModelError::Transport(
                "the script has no answer left".into(),
            )),
        }
    }
}

/// A text block.
pub fn text(t: &str) -> Value {
    json!({"type": "text", "text": t})
}

/// A `tool_use` block.
pub fn tool_use(id: &str, name: &str, input: Value) -> Value {
    json!({"type": "tool_use", "id": id, "name": name, "input": input})
}

/// A thinking block as Claude Opus 5.5 returns it by default (text omitted, signed).
pub fn thinking(signature: &str) -> Value {
    json!({"type": "thinking", "thinking": "", "signature": signature})
}

/// An assistant message with `content` and `stop_reason`, and plausible usage numbers.
pub fn message(content: Vec<Value>, stop_reason: &str) -> Value {
    json!({
        "id": "msg_scripted",
        "type": "message",
        "role": "assistant",
        "model": "claude-opus-5-5",
        "content": content,
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": {
            "input_tokens": 200,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 9000,
            "output_tokens": 150,
        },
    })
}
