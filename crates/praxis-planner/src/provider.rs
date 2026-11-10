//! Language-model provider interface (FR-34). FROZEN CONTRACT.
//!
//! A provider performs exactly one schema-constrained tool call and returns the tool input.
//! Implementations: `offline::RuleProvider` (deterministic, no network, used in CI) and
//! `claude::ClaudeProvider` (Claude API; key from ANTHROPIC_API_KEY or the OS keychain).

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

/// One forced tool call: the model must answer by calling `tool_name` with input matching
/// `input_schema` (JSON Schema).
#[derive(Clone, Debug, PartialEq)]
pub struct ToolRequest {
    pub system: String,
    pub messages: Vec<Message>,
    pub tool_name: String,
    pub tool_description: String,
    pub input_schema: serde_json::Value,
    pub max_tokens: u32,
    /// Seed for providers that support it; the offline provider is a pure function of it.
    pub seed: u64,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProviderError {
    #[error("no API key: set ANTHROPIC_API_KEY or store one with `praxis key set`")]
    NoApiKey,
    #[error("provider request failed: {0}")]
    Request(String),
    #[error("provider returned no tool call")]
    NoToolCall,
    #[error("provider cannot interpret this brief: {0}")]
    Unsupported(String),
}

pub trait Provider: Send + Sync {
    /// Short id recorded in score provenance, e.g. "offline-rules" or "claude:claude-opus-5-5".
    fn id(&self) -> String;

    /// Performs one tool call and returns the tool input JSON (unvalidated).
    fn call_tool(&self, req: &ToolRequest) -> Result<serde_json::Value, ProviderError>;
}
