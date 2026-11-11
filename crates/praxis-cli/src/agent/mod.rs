//! The sound assistant: Claude with every Praxis tool and the conversation's history (WP-AG).
//!
//! When a person asks for something in the app's conversation (or `praxis ask`, or MCP
//! `praxis_ask`) and an API key is available, [`crate::ops::ask`] runs one assistant turn instead
//! of the one-shot planner:
//!
//! 1. The sound's `conversation.json` ([`conversation`]) gives the history: every earlier
//!    request, reply, tool call and result, with the assistant's content exactly as the API
//!    returned it. The new request is appended with the sound's current state.
//! 2. A Claude Messages API tool loop ([`client`], [`request`]) runs with the tools in [`tools`]:
//!    read the sound, the catalogue, compose a sound or an edit, apply a ScorePatch, set
//!    instrument parameters, render and measure, variations, restore, undo/redo, open a view,
//!    analyze a clip, ask the person. Changes collect in a draft of the score.
//! 3. When the model stops, the draft becomes one new take (rendered, checked, recorded with
//!    `who` = "Agent · Assistant"), and the model's last text is the take's `say`.
//!
//! API rules kept (tested in `tests/agent.rs`): adaptive thinking left at the model default (no
//! `thinking` field, never `budget_tokens`), `output_config.effort` set explicitly, `tool_choice`
//! `auto`, `strict: true` on every tool (at most 20 tools, no optional or union parameters),
//! server-side refusal fallbacks, `stop_reason` checked before any tool runs, parallel tool
//! results in one user message with `is_error` for failures, streaming, prompt caching on the
//! stable prefix (tools and system prompt, then the growing history), server-side compaction,
//! and a step budget and wall-clock cap per request.

pub mod catalogue;
pub mod client;
pub mod conversation;
mod draft;
pub mod measure;
pub mod prompt;
pub mod request;
pub mod sse;
pub mod testing;
pub mod tools;
mod turn;
pub mod words;

pub use client::{ApiRequest, HttpClient, ModelClient, ModelError};
pub use conversation::{Conversation, Outcome};
pub use turn::run_ask;

use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

/// The model the assistant uses unless told otherwise.
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
/// `output_config.effort` unless told otherwise.
pub const DEFAULT_EFFORT: &str = "medium";
/// Tool calls per request.
pub const DEFAULT_MAX_STEPS: u32 = 16;
/// Wall-clock time per request.
pub const DEFAULT_TIME_LIMIT: Duration = Duration::from_secs(600);
/// The assistant's name in the take history (`"Agent · Assistant"`).
pub const AGENT_NAME: &str = "Assistant";

/// How the assistant runs.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentConfig {
    pub model: String,
    /// `output_config.effort`: low, medium, high, xhigh or max.
    pub effort: String,
    /// Tool calls one request may make before the assistant has to wrap up.
    pub max_steps: u32,
    /// Wall-clock time one request may take before the assistant has to wrap up.
    pub time_limit: Duration,
    /// `max_tokens` of each response (the request streams, so this can be large).
    pub max_tokens: u32,
    /// Server-side compaction starts when a request's input reaches this many tokens.
    pub compact_at_tokens: u64,
    /// Server-side refusal fallbacks (`fallbacks: "default"`) on models that support them.
    pub fallbacks: bool,
}

impl Default for AgentConfig {
    fn default() -> Self {
        AgentConfig {
            model: DEFAULT_MODEL.to_string(),
            effort: DEFAULT_EFFORT.to_string(),
            max_steps: DEFAULT_MAX_STEPS,
            time_limit: DEFAULT_TIME_LIMIT,
            max_tokens: 32_000,
            compact_at_tokens: 150_000,
            fallbacks: true,
        }
    }
}

/// What the assistant is doing, for a progress display. `words` are plain sentences for people
/// ("Reading the sound", "Changing the Felt piano's tone and felt", "Rendering", "Checking: 8 of
/// 8 passed").
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AgentEvent {
    /// A request arrived.
    Started { request: String },
    /// The model is being asked (the `n`th request of this turn).
    Thinking { n: u32 },
    /// Text the model wrote (between tool calls, or its answer).
    Said { text: String },
    /// A tool call starts.
    Step { n: u32, tool: String, words: String },
    /// A tool call finished.
    StepDone {
        n: u32,
        tool: String,
        ok: bool,
        words: String,
    },
    /// The draft became a take.
    TakeMade { n: u32 },
    /// The turn is over.
    Finished { outcome: String, reply: String },
}

/// The progress callback.
pub type Progress = Arc<dyn Fn(&AgentEvent) + Send + Sync>;
