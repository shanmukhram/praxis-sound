//! `conversation.json`: what the assistant needs to continue a sound's conversation.
//!
//! ```text
//! {format: "praxis-conversation", version: 1,
//!  messages: [...],   the Messages API history, exactly as sent and returned
//!  turns: [...]}      one compact record per request
//! ```
//!
//! `messages` is append-only: every user message as sent and every assistant message exactly as
//! the API returned it (thinking blocks with their signatures, compaction blocks, fallback
//! blocks), so the next request replays a byte-identical prefix (prompt caching, preserved
//! thinking, server-side compaction all depend on that). The last message may be a user message
//! that was never answered (the tool results of an `ask_user` call, or a request that failed);
//! the next request adds its text to that message instead of starting a second one.
//!
//! `turns` is for people and for the app: the request, who asked, the scope, the reply, each
//! tool call (name, input, whether it worked, a short result), the take made, and token usage.
//! Writes are atomic (temporary file and rename), like `takes.json`.

use crate::error::{ErrorCode, OpError, io_error};
use crate::ops::sound::write_atomic;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const CONVERSATION_FORMAT: &str = "praxis-conversation";
pub const CONVERSATION_VERSION: u32 = 1;
pub const CONVERSATION_FILE: &str = "conversation.json";

/// One tool call of a turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub tool: String,
    pub input: Value,
    pub ok: bool,
    /// What it did, in plain words ("Changing the Felt piano's tone and felt").
    pub words: String,
    /// The result in short (or the error).
    pub result: String,
}

/// Tokens a turn used, summed over its requests.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub requests: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
}

impl Usage {
    /// Adds a response's `usage`.
    pub fn add(&mut self, usage: &Value) {
        let n = |k: &str| usage[k].as_u64().unwrap_or(0);
        self.requests += 1;
        self.input_tokens += n("input_tokens");
        self.output_tokens += n("output_tokens");
        self.cache_creation_input_tokens += n("cache_creation_input_tokens");
        self.cache_read_input_tokens += n("cache_read_input_tokens");
    }
}

/// How a turn ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// A take was made.
    Take,
    /// The assistant asked the person a question (no take).
    Question,
    /// The assistant answered without changing anything (a refusal, a locked layer, advice).
    NoChange,
    /// The step budget or the time limit ran out.
    OutOfSteps,
    /// The model declined the request (`stop_reason: refusal`).
    Declined,
    /// The API or a final render failed.
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Take => "take",
            Outcome::Question => "question",
            Outcome::NoChange => "no_change",
            Outcome::OutOfSteps => "out_of_steps",
            Outcome::Declined => "declined",
            Outcome::Failed => "failed",
        }
    }
}

/// One request and what came of it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    /// 1-based.
    pub n: u32,
    pub request: String,
    /// Who asked (`"You"` or `"Agent · <name>"`).
    pub who: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Value>,
    pub model: String,
    /// The take that was current when the request came in (0 for a new sound).
    pub from_take: u32,
    pub outcome: Outcome,
    /// The take this turn made (`null` when none).
    pub take: Option<u32>,
    /// The assistant's answer to the person (for a question, the question).
    pub reply: String,
    pub steps: Vec<Step>,
    pub usage: Usage,
}

/// `conversation.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub format: String,
    pub version: u32,
    pub messages: Vec<Value>,
    pub turns: Vec<Turn>,
}

impl Default for Conversation {
    fn default() -> Self {
        Conversation {
            format: CONVERSATION_FORMAT.into(),
            version: CONVERSATION_VERSION,
            messages: Vec::new(),
            turns: Vec::new(),
        }
    }
}

impl Conversation {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(CONVERSATION_FILE)
    }

    /// Reads the sound's conversation; a missing file is an empty conversation.
    pub fn load(dir: &Path) -> Result<Conversation, OpError> {
        let path = Conversation::path(dir);
        if !path.is_file() {
            return Ok(Conversation::default());
        }
        let text = std::fs::read_to_string(&path).map_err(|e| io_error(&path, e))?;
        let c: Conversation = serde_json::from_str(&text)
            .map_err(|e| OpError::new(ErrorCode::BadJson, format!("{}: {e}", path.display())))?;
        if c.format != CONVERSATION_FORMAT || c.version != CONVERSATION_VERSION {
            return Err(OpError::new(
                ErrorCode::BadJson,
                format!(
                    "{}: expected format `{CONVERSATION_FORMAT}` version {CONVERSATION_VERSION}",
                    path.display()
                ),
            ));
        }
        Ok(c)
    }

    pub fn save(&self, dir: &Path) -> Result<(), OpError> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| OpError::new(ErrorCode::Error, e.to_string()))?;
        write_atomic(&Conversation::path(dir), (text + "\n").as_bytes())
    }

    /// Adds user content: to the last message when it is an unanswered user message, else as a
    /// new user message.
    pub fn push_user(&mut self, mut blocks: Vec<Value>) {
        if let Some(last) = self.messages.last_mut()
            && last["role"] == "user"
        {
            let content = &mut last["content"];
            if let Some(s) = content.as_str() {
                *content = json!([{"type": "text", "text": s}]);
            }
            if let Some(a) = content.as_array_mut() {
                a.append(&mut blocks);
                return;
            }
        }
        self.messages
            .push(json!({"role": "user", "content": blocks}));
    }

    /// Adds an assistant message with `content` exactly as the API returned it.
    pub fn push_assistant(&mut self, content: Value) {
        self.messages
            .push(json!({"role": "assistant", "content": content}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unanswered_user_messages_are_extended_not_doubled() {
        let mut c = Conversation::default();
        c.push_user(vec![json!({"type": "text", "text": "one"})]);
        c.push_assistant(json!([{"type": "text", "text": "ok"}]));
        c.push_user(vec![
            json!({"type": "tool_result", "tool_use_id": "t", "content": "q"}),
        ]);
        c.push_user(vec![json!({"type": "text", "text": "two"})]);
        assert_eq!(c.messages.len(), 3);
        assert_eq!(c.messages[2]["content"].as_array().unwrap().len(), 2);
        assert_eq!(c.messages[2]["content"][0]["type"], "tool_result");
    }

    #[test]
    fn round_trips_through_the_file() {
        let dir = std::env::temp_dir().join(format!("praxis-conv-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Conversation::load(&dir).unwrap(), Conversation::default());
        let mut c = Conversation::default();
        c.push_user(vec![json!({"type": "text", "text": "hello"})]);
        c.turns.push(Turn {
            n: 1,
            request: "hello".into(),
            who: "You".into(),
            scope: None,
            model: "claude-opus-5-5".into(),
            from_take: 0,
            outcome: Outcome::Question,
            take: None,
            reply: "Which part?".into(),
            steps: vec![],
            usage: Usage::default(),
        });
        c.save(&dir).unwrap();
        assert_eq!(Conversation::load(&dir).unwrap(), c);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
