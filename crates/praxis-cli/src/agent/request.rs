//! The Messages API request body of one assistant step.
//!
//! Render order on the API side is `tools`, `system`, `messages`. The tools and the system prompt
//! never change between requests (same text, same order), and the system block carries a
//! one-hour cache breakpoint, so they are written to the cache once and read from it afterwards.
//! The top-level automatic `cache_control` puts a second breakpoint on the last message, so the
//! growing history is read from the cache on every following step and turn.
//!
//! Kept off on purpose: `thinking` (Claude Opus 5.5 thinks adaptively by default; `disabled` and
//! `budget_tokens` are rejected), `temperature`/`top_p`/`top_k` (rejected on this API version),
//! forced `tool_choice` (rejected on this model), assistant prefill (rejected).

use super::AgentConfig;
use super::client::ApiRequest;
use serde_json::{Value, json};

/// Beta: server-side compaction of long conversations.
pub const COMPACTION_BETA: &str = "compact-2026-01-12";
/// Beta: server-side refusal fallbacks (`fallbacks: "default"`).
pub const FALLBACK_BETA: &str = praxis_planner::claude::FALLBACK_BETA;

/// The request for `messages` with the fixed `system` prompt and `tools`.
pub fn build(
    config: &AgentConfig,
    system: &str,
    tools: &[Value],
    messages: &[Value],
) -> ApiRequest {
    let mut body = json!({
        "model": config.model,
        "max_tokens": config.max_tokens,
        "stream": true,
        "system": [{
            "type": "text",
            "text": system,
            "cache_control": {"type": "ephemeral", "ttl": "1h"},
        }],
        "tools": tools,
        "tool_choice": {"type": "auto"},
        "messages": messages,
        "output_config": {"effort": config.effort},
        "cache_control": {"type": "ephemeral"},
        "context_management": {"edits": [{
            "type": "compact_20260112",
            "trigger": {"type": "input_tokens", "value": config.compact_at_tokens},
        }]},
    });
    let mut betas = vec![COMPACTION_BETA.to_string()];
    if config.fallbacks && praxis_planner::claude::supports_default_fallbacks(&config.model) {
        body["fallbacks"] = json!("default");
        betas.push(FALLBACK_BETA.to_string());
    }
    ApiRequest { body, betas }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_follows_the_api_rules() {
        let cfg = AgentConfig::default();
        let tools = vec![json!({"name": "t", "strict": true})];
        let msgs = vec![json!({"role": "user", "content": "hi"})];
        let r = build(&cfg, "sys", &tools, &msgs);
        let b = &r.body;
        assert_eq!(b["model"], "claude-opus-5-5");
        assert_eq!(b["tool_choice"], json!({"type": "auto"}));
        assert_eq!(b["output_config"]["effort"], "medium");
        assert!(b.get("thinking").is_none());
        assert!(!b.to_string().contains("budget_tokens"));
        assert!(b.get("temperature").is_none());
        assert_eq!(b["stream"], true);
        assert_eq!(b["fallbacks"], "default");
        assert!(r.betas.contains(&"compact-2026-01-12".to_string()));
        assert!(
            r.betas
                .contains(&"server-side-fallback-2026-07-01".to_string())
        );
        assert_eq!(b["system"][0]["cache_control"]["type"], "ephemeral");
        let haiku = AgentConfig {
            model: "claude-haiku-5-5".into(),
            ..AgentConfig::default()
        };
        let r = build(&haiku, "sys", &tools, &msgs);
        assert!(r.body.get("fallbacks").is_none());
        assert_eq!(r.betas, vec!["compact-2026-01-12".to_string()]);
    }
}
