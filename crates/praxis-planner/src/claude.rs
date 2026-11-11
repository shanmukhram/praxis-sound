//! `ClaudeProvider`: the Claude Messages API (`POST /v1/messages`) with one tool whose input
//! schema is the ScorePlan schema (feature `claude`).
//!
//! - Model id is configurable; default [`DEFAULT_MODEL`] (`claude-opus-5-5`).
//! - The tool is declared without `strict`: the ScorePlan and EditPlan schemas are too large for
//!   the API's strict grammar (a 400, "The compiled grammar is too large"; found in the first
//!   live run, 2026-11-11). The schema is still sent (rewritten by [`strict_schema`]: closed
//!   objects, `anyOf` for `oneOf`), and every answer goes through `validate_plan` and the repair
//!   loop, which is what keeps plans correct. Models that
//!   accept forced tool use get `tool_choice: {"type": "tool", "name": "plan_score"}`. Claude
//!   Opus 5.5, Sonnet 5.5 and the Fable/Mythos 5.1+ models reject forced tool use with a 400, so
//!   for them the request uses `tool_choice: {"type": "auto"}`, the system prompt names the tool,
//!   and the provider checks that a call was made and asks once more if not.
//! - Thinking is left at the model default (adaptive); `output_config.effort` is set explicitly.
//! - The API key comes from `ANTHROPIC_API_KEY` or the OS keychain (service `praxis-sound`,
//!   user `anthropic`). It is only ever placed in the `x-api-key` header; it is never logged,
//!   never included in errors and never shown by `Debug`.
//! - The HTTP layer is a [`Transport`] so tests replay recorded responses offline.
//! - Prompt caching: explicit `cache_control` breakpoints on the tool definition (the schema is
//!   the same for every request) and on the system prompt (the same for every brief; repair
//!   rounds resend it). Cache reads cost a twentieth of fresh input on Claude Opus 5.5.
//! - Every response's `usage` is added up ([`ClaudeProvider::usage`], [`Usage::cost_usd`]), so
//!   live runs report what they cost.

use crate::provider::{Message, Provider, ProviderError, Role, ToolRequest};
use serde_json::{Value, json};
use std::sync::Mutex;
use std::time::Duration;

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
pub const API_URL: &str = "https://api.anthropic.com/v1/messages";
pub const API_VERSION: &str = "2026-10-21";
pub const KEYRING_SERVICE: &str = "praxis-sound";
pub const KEYRING_USER: &str = "anthropic";
/// Beta header for server-side refusal fallbacks.
pub const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// One HTTP response.
#[derive(Clone, Debug, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    /// `retry-after` header in seconds, when present.
    pub retry_after_s: Option<u64>,
}

/// Sends one JSON POST. Errors are transport failures (no response).
pub trait Transport: Send + Sync {
    fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<HttpResponse, String>;
}

/// The real transport (ureq).
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl std::fmt::Debug for UreqTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UreqTransport")
    }
}

impl Default for UreqTransport {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(600)))
            .http_status_as_error(false)
            .build();
        UreqTransport {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl Transport for UreqTransport {
    fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<HttpResponse, String> {
        let mut req = self.agent.post(url);
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let mut resp = req.send_json(body).map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let retry_after_s = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse().ok());
        let body = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| e.to_string())?;
        Ok(HttpResponse {
            status,
            body,
            retry_after_s,
        })
    }
}

/// Where the API key comes from.
pub type KeySource = Box<dyn Fn() -> Result<String, ProviderError> + Send + Sync>;

/// `ANTHROPIC_API_KEY`, else the OS keychain entry (`praxis-sound` / `anthropic`).
pub fn default_key() -> Result<String, ProviderError> {
    if let Ok(k) = std::env::var("ANTHROPIC_API_KEY") {
        let k = k.trim();
        if !k.is_empty() {
            return Ok(k.to_string());
        }
    }
    match keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).and_then(|e| e.get_password()) {
        Ok(k) if !k.trim().is_empty() => Ok(k.trim().to_string()),
        _ => Err(ProviderError::NoApiKey),
    }
}

/// Stores a key in the OS keychain (for `praxis key set`). Never logs the key.
pub fn store_key_in_keychain(key: &str) -> Result<(), ProviderError> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .and_then(|e| e.set_password(key.trim()))
        .map_err(|e| ProviderError::Request(format!("could not store the key: {e}")))
}

/// Request settings.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeConfig {
    pub model: String,
    /// `output_config.effort`: low, medium, high, xhigh or max. None leaves the model default.
    pub effort: Option<String>,
    /// Ask the API to re-run a refused request on a fallback model (`fallbacks: "default"`),
    /// on models that support it.
    pub fallbacks: bool,
    pub url: String,
    /// Retries for 429, 5xx and 529 responses.
    pub max_retries: u32,
    /// Base back-off between retries in milliseconds (doubled per retry unless the server sends
    /// `retry-after`).
    pub retry_base_ms: u64,
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        ClaudeConfig {
            model: DEFAULT_MODEL.to_string(),
            effort: Some("medium".to_string()),
            fallbacks: true,
            url: API_URL.to_string(),
            max_retries: 2,
            retry_base_ms: 1000,
        }
    }
}

/// Models that reject forced `tool_choice` (`any` / `tool`) with a 400.
pub fn supports_forced_tool_choice(model: &str) -> bool {
    const NO_FORCED: [&str; 6] = [
        "claude-opus-5-5",
        "claude-sonnet-5-5",
        "claude-fable-5-1",
        "claude-fable-5-2",
        "claude-mythos-5-1",
        "claude-mythos-5-2",
    ];
    !NO_FORCED.iter().any(|m| model.starts_with(m))
}

/// Models that accept `fallbacks: "default"`.
pub fn supports_default_fallbacks(model: &str) -> bool {
    const WITH: [&str; 5] = [
        "claude-fable-5-2",
        "claude-fable-5-1",
        "claude-opus-5-5",
        "claude-opus-5",
        "claude-sonnet-5-5",
    ];
    WITH.iter()
        .any(|m| model == *m || model.starts_with(&format!("{m}-")))
}

/// Rewrites a JSON Schema for strict tool use: every object gets `additionalProperties: false`,
/// and keywords strict mode does not support (numeric and length constraints, `format`,
/// `default`, `$schema`) are removed.
pub fn strict_schema(schema: &Value) -> Value {
    fn walk(v: &mut Value) {
        match v {
            Value::Object(map) => {
                // Strict tool schemas accept `anyOf` but not `oneOf` (the API answers 400
                // "Schema type 'oneOf' is not supported"). The variants schemars writes for enums
                // and tagged unions are disjoint, so `anyOf` accepts exactly the same inputs, and
                // every plan is validated again by `validate_plan` anyway.
                if let Some(one) = map.remove("oneOf") {
                    map.insert("anyOf".into(), one);
                }
                for k in [
                    "$schema",
                    "minimum",
                    "maximum",
                    "exclusiveMinimum",
                    "exclusiveMaximum",
                    "multipleOf",
                    "minLength",
                    "maxLength",
                    "minItems",
                    "maxItems",
                    "format",
                    "default",
                ] {
                    map.remove(k);
                }
                let is_object = map.get("type").is_some_and(|t| {
                    t == "object"
                        || t.as_array()
                            .is_some_and(|a| a.iter().any(|x| x == "object"))
                }) || map.contains_key("properties");
                if is_object {
                    map.insert("additionalProperties".into(), Value::Bool(false));
                }
                for (k, child) in map.iter_mut() {
                    // Property names are data, not schema keywords.
                    if k == "properties" || k == "$defs" || k == "definitions" {
                        if let Value::Object(props) = child {
                            for p in props.values_mut() {
                                walk(p);
                            }
                        }
                    } else {
                        walk(child);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(walk),
            _ => {}
        }
    }
    let mut s = schema.clone();
    walk(&mut s);
    s
}

/// Token usage added up over the responses a provider received (`usage` in each response).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Successful API responses counted.
    pub requests: u32,
    /// Fresh (uncached) input tokens.
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Input tokens written to the prompt cache.
    pub cache_creation_input_tokens: u64,
    /// Input tokens read from the prompt cache.
    pub cache_read_input_tokens: u64,
}

/// US$ per million tokens: (input, output, cache read, 5-minute cache write).
pub fn price_per_mtok(model: &str) -> Option<(f64, f64, f64, f64)> {
    // Anthropic first-party rates (2026-11); a cache write costs 1.25x input.
    let p = |i: f64, o: f64, r: f64| Some((i, o, r, i * 1.25));
    match model {
        m if m.starts_with("claude-opus-5-5") => p(4.0, 20.0, 0.20),
        m if m.starts_with("claude-sonnet-5-5") => p(2.0, 10.0, 0.10),
        m if m.starts_with("claude-fable-5-2") => p(8.0, 40.0, 0.20),
        m if m.starts_with("claude-haiku-5-5") => p(0.10, 0.50, 0.01),
        _ => None,
    }
}

impl Usage {
    /// Adds one response's `usage` object.
    pub fn add_response(&mut self, resp: &Value) {
        let u = &resp["usage"];
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        self.requests += 1;
        self.input_tokens += n("input_tokens");
        self.output_tokens += n("output_tokens");
        self.cache_creation_input_tokens += n("cache_creation_input_tokens");
        self.cache_read_input_tokens += n("cache_read_input_tokens");
    }

    /// The difference `self - earlier` (usage of the calls made since `earlier`).
    pub fn since(&self, earlier: &Usage) -> Usage {
        Usage {
            requests: self.requests.saturating_sub(earlier.requests),
            input_tokens: self.input_tokens.saturating_sub(earlier.input_tokens),
            output_tokens: self.output_tokens.saturating_sub(earlier.output_tokens),
            cache_creation_input_tokens: self
                .cache_creation_input_tokens
                .saturating_sub(earlier.cache_creation_input_tokens),
            cache_read_input_tokens: self
                .cache_read_input_tokens
                .saturating_sub(earlier.cache_read_input_tokens),
        }
    }

    /// Cost in US$ at the model's list price, if the model's price is known.
    pub fn cost_usd(&self, model: &str) -> Option<f64> {
        let (i, o, r, w) = price_per_mtok(model)?;
        Some(
            (self.input_tokens as f64 * i
                + self.output_tokens as f64 * o
                + self.cache_read_input_tokens as f64 * r
                + self.cache_creation_input_tokens as f64 * w)
                / 1e6,
        )
    }
}

impl std::fmt::Display for Usage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} requests, {} input + {} cache write + {} cache read, {} output tokens",
            self.requests,
            self.input_tokens,
            self.cache_creation_input_tokens,
            self.cache_read_input_tokens,
            self.output_tokens
        )
    }
}

/// Claude Messages API provider.
pub struct ClaudeProvider {
    config: ClaudeConfig,
    transport: Box<dyn Transport>,
    key: KeySource,
    usage: Mutex<Usage>,
}

impl std::fmt::Debug for ClaudeProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeProvider")
            .field("config", &self.config)
            .field("key", &"<redacted>")
            .finish()
    }
}

impl Default for ClaudeProvider {
    fn default() -> Self {
        ClaudeProvider::new()
    }
}

impl ClaudeProvider {
    /// Default model, real HTTP, key from the environment or keychain (read at call time).
    pub fn new() -> ClaudeProvider {
        ClaudeProvider::with_parts(
            ClaudeConfig::default(),
            Box::new(UreqTransport::default()),
            Box::new(default_key),
        )
    }

    pub fn with_model(model: &str) -> ClaudeProvider {
        let mut p = ClaudeProvider::new();
        p.config.model = model.to_string();
        p
    }

    pub fn with_parts(
        config: ClaudeConfig,
        transport: Box<dyn Transport>,
        key: KeySource,
    ) -> ClaudeProvider {
        ClaudeProvider {
            config,
            transport,
            key,
            usage: Mutex::new(Usage::default()),
        }
    }

    pub fn config(&self) -> &ClaudeConfig {
        &self.config
    }

    /// Token usage of every successful response so far.
    pub fn usage(&self) -> Usage {
        self.usage.lock().map(|u| *u).unwrap_or_default()
    }

    /// The Messages API request body for a tool request.
    pub fn request_body(&self, req: &ToolRequest) -> Value {
        let forced = supports_forced_tool_choice(&self.config.model);
        let system_text = if forced {
            req.system.clone()
        } else {
            format!(
                "{}\n\nAnswer only by calling the `{}` tool with the complete plan.",
                req.system, req.tool_name
            )
        };
        let messages: Vec<Value> = req
            .messages
            .iter()
            .map(|m| {
                json!({
                    "role": match m.role { Role::User => "user", Role::Assistant => "assistant" },
                    "content": m.content,
                })
            })
            .collect();
        let tool_choice = if forced {
            json!({"type": "tool", "name": req.tool_name})
        } else {
            json!({"type": "auto"})
        };
        let mut body = json!({
            "model": self.config.model,
            "max_tokens": req.max_tokens,
            "system": [{
                "type": "text",
                "text": system_text,
                "cache_control": {"type": "ephemeral"},
            }],
            "messages": messages,
            "tools": [{
                "name": req.tool_name,
                "description": req.tool_description,
                "input_schema": strict_schema(&req.input_schema),
                "cache_control": {"type": "ephemeral"},
            }],
            "tool_choice": tool_choice,
        });
        if let Some(effort) = &self.config.effort {
            body["output_config"] = json!({ "effort": effort });
        }
        if self.uses_fallbacks() {
            body["fallbacks"] = json!("default");
        }
        body
    }

    fn uses_fallbacks(&self) -> bool {
        self.config.fallbacks && supports_default_fallbacks(&self.config.model)
    }

    /// Request headers. The key is only ever placed here.
    fn headers(&self, key: &str) -> Vec<(String, String)> {
        let mut h = vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("x-api-key".to_string(), key.to_string()),
            ("anthropic-version".to_string(), API_VERSION.to_string()),
        ];
        if self.uses_fallbacks() {
            h.push(("anthropic-beta".to_string(), FALLBACK_BETA.to_string()));
        }
        h
    }

    fn send(&self, body: &Value, key: &str) -> Result<Value, ProviderError> {
        let headers = self.headers(key);
        let mut attempt = 0u32;
        loop {
            let resp = self
                .transport
                .post_json(&self.config.url, &headers, body)
                .map_err(|e| ProviderError::Request(redact(&e, key)))?;
            let retryable = matches!(resp.status, 429 | 500 | 502 | 503 | 504 | 529);
            if resp.status == 200 {
                let v: Value = serde_json::from_str(&resp.body).map_err(|e| {
                    ProviderError::Request(format!("unreadable response from the API: {e}"))
                })?;
                if let Ok(mut u) = self.usage.lock() {
                    u.add_response(&v);
                }
                return Ok(v);
            }
            if retryable && attempt < self.config.max_retries {
                let wait_ms = resp
                    .retry_after_s
                    .map(|s| s.min(60) * 1000)
                    .unwrap_or(self.config.retry_base_ms << attempt.min(6));
                if wait_ms > 0 {
                    std::thread::sleep(Duration::from_millis(wait_ms));
                }
                attempt += 1;
                continue;
            }
            return Err(ProviderError::Request(redact(
                &api_error(resp.status, &resp.body),
                key,
            )));
        }
    }
}

fn redact(text: &str, key: &str) -> String {
    if key.is_empty() {
        text.to_string()
    } else {
        text.replace(key, "<redacted>")
    }
}

fn api_error(status: u16, body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let kind = parsed
        .as_ref()
        .and_then(|v| v["error"]["type"].as_str().map(str::to_string))
        .unwrap_or_else(|| "error".into());
    let msg = parsed
        .as_ref()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_default();
    let hint = match status {
        401 | 403 => " (check the API key)",
        429 => " (rate limited; try again later)",
        529 => " (the API is overloaded; try again later)",
        _ => "",
    };
    format!("HTTP {status} {kind}: {msg}{hint}")
}

/// Wrapper keys a model sometimes puts around a whole tool input.
/// "args" was seen in the WP-P3 live run of 2026-11-12 (a scored cue).
const WRAPPER_KEYS: [&str; 4] = ["parameters", "input", "arguments", "args"];

/// Removes a wrapper a model put around the whole tool input. In the live runs of 2026-11-11 the
/// planner answered `{"parameters": {...the plan...}}` in about one brief in six, which cost a
/// schema repair round each time. The input is unwrapped only when its single key is one of
/// [`WRAPPER_KEYS`], that key is not a property of the tool's schema, and the wrapped value is
/// an object (or a JSON string holding one) that has at least one of the schema's properties.
/// Anything else is returned unchanged, so validation still sees exactly what the model sent.
pub fn unwrap_tool_input(input: Value, schema: &Value) -> Value {
    let props = &schema["properties"];
    let Some((key, inner)) = input
        .as_object()
        .filter(|o| o.len() == 1)
        .and_then(|o| o.iter().next())
    else {
        return input;
    };
    if !WRAPPER_KEYS.contains(&key.as_str()) || props.get(key).is_some() {
        return input;
    }
    let inner = match inner {
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(v) => v,
            Err(_) => return input,
        },
        v => v.clone(),
    };
    let looks_like_input = inner
        .as_object()
        .is_some_and(|o| o.keys().any(|k| props.get(k).is_some()));
    if looks_like_input { inner } else { input }
}

/// What a response contained.
enum Reply {
    Tool(Value),
    /// No tool call; the visible text, if any.
    Text(String),
}

fn read_reply(resp: &Value, tool_name: &str) -> Result<Reply, ProviderError> {
    let stop = resp["stop_reason"].as_str().unwrap_or("");
    if stop == "refusal" {
        let cat = resp["stop_details"]["category"]
            .as_str()
            .map(|c| format!(" ({c})"))
            .unwrap_or_default();
        return Err(ProviderError::Request(format!(
            "the model declined the brief{cat}"
        )));
    }
    let content = resp["content"].as_array().cloned().unwrap_or_default();
    if let Some(block) = content
        .iter()
        .find(|b| b["type"] == "tool_use" && b["name"] == tool_name)
    {
        if stop == "max_tokens" {
            return Err(ProviderError::Request(
                "the model ran out of max_tokens before finishing the plan".into(),
            ));
        }
        return Ok(Reply::Tool(block["input"].clone()));
    }
    if stop == "max_tokens" {
        return Err(ProviderError::Request(
            "the model ran out of max_tokens before calling the tool".into(),
        ));
    }
    let text = content
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Reply::Text(text))
}

impl Provider for ClaudeProvider {
    fn id(&self) -> String {
        format!("claude:{}", self.config.model)
    }

    fn call_tool(&self, req: &ToolRequest) -> Result<Value, ProviderError> {
        let key = (self.key)()?;
        let mut req = req.clone();
        // With `tool_choice: auto` a model may answer in text; ask once more.
        for round in 0..2 {
            let body = self.request_body(&req);
            let resp = self.send(&body, &key)?;
            match read_reply(&resp, &req.tool_name)? {
                Reply::Tool(input) => return Ok(unwrap_tool_input(input, &req.input_schema)),
                Reply::Text(text) if round == 0 => {
                    req.messages.push(Message {
                        role: Role::Assistant,
                        content: if text.trim().is_empty() {
                            "(no tool call)".to_string()
                        } else {
                            text
                        },
                    });
                    req.messages.push(Message {
                        role: Role::User,
                        content: format!(
                            "Call the `{}` tool now with the complete plan.",
                            req.tool_name
                        ),
                    });
                }
                Reply::Text(_) => break,
            }
        }
        Err(ProviderError::NoToolCall)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_schema_rules() {
        let s = json!({
            "$schema": "x",
            "type": "object",
            "properties": {
                "minimum": {"type": "integer", "format": "uint32", "minimum": 0},
                "inner": {"type": ["object", "null"], "properties": {"a": {"type": "string"}}}
            },
            "$defs": {"T": {"type": "object", "properties": {}}}
        });
        let t = strict_schema(&s);
        assert!(t.get("$schema").is_none());
        assert_eq!(t["additionalProperties"], false);
        // A property called "minimum" survives; its constraint keywords do not.
        assert!(t["properties"]["minimum"].is_object());
        assert!(t["properties"]["minimum"].get("minimum").is_none());
        assert!(t["properties"]["minimum"].get("format").is_none());
        assert_eq!(t["properties"]["inner"]["additionalProperties"], false);
        assert_eq!(t["$defs"]["T"]["additionalProperties"], false);
        // `oneOf` (rejected by the API in strict tools) becomes `anyOf`, everywhere.
        let u = strict_schema(
            &json!({"$defs": {"E": {"oneOf": [{"const": "a"}, {"const": "b"}]}},
            "properties": {"x": {"oneOf": [{"type": "string"}, {"type": "null"}]}}}),
        );
        assert!(!u.to_string().contains("oneOf"), "{u}");
        assert_eq!(u["$defs"]["E"]["anyOf"][1]["const"], "b");
        assert_eq!(u["properties"]["x"]["anyOf"][0]["type"], "string");
    }

    #[test]
    fn a_wrapped_tool_input_is_unwrapped_and_nothing_else_is() {
        let schema = crate::plan::plan_schema();
        let plan = json!({"emotion": "calm", "key": {"tonic": "A", "mode": "minor"}, "layers": []});
        // The live shape, and the same plan sent as a JSON string.
        assert_eq!(
            unwrap_tool_input(json!({"parameters": plan.clone()}), &schema),
            plan
        );
        assert_eq!(
            unwrap_tool_input(json!({"input": plan.to_string()}), &schema),
            plan
        );
        assert_eq!(
            unwrap_tool_input(json!({"args": plan.clone()}), &schema),
            plan
        );
        // Unchanged: a plain plan, an unknown wrapper, a wrapper beside other fields, a wrapped
        // value that is not a plan, and a key that is a real field of the schema.
        let same = |v: Value, s: &Value| assert_eq!(unwrap_tool_input(v.clone(), s), v);
        same(plan.clone(), &schema);
        same(json!({"plan": plan.clone()}), &schema);
        same(
            json!({"parameters": plan.clone(), "emotion": "calm"}),
            &schema,
        );
        same(json!({"parameters": {"foo": 1}}), &schema);
        same(json!({"parameters": "not json"}), &schema);
        let edit = crate::edit::edit_schema();
        same(json!({"params": {"tone": "darker"}}), &edit);
        let with_field = json!({"properties": {"parameters": {"type": "object"}}});
        same(json!({"parameters": {"parameters": 1}}), &with_field);
        // The edit tool gets the same unwrap.
        let e = json!({"intent": "change_tone", "params": {"tone": "brighter"}});
        assert_eq!(
            unwrap_tool_input(json!({"parameters": e.clone()}), &edit),
            e
        );
    }

    #[test]
    fn model_capabilities() {
        assert!(!supports_forced_tool_choice("claude-opus-5-5"));
        assert!(supports_forced_tool_choice("claude-opus-5"));
        assert!(supports_default_fallbacks("claude-opus-5-5"));
        assert!(!supports_default_fallbacks("claude-haiku-5-5"));
    }

    #[test]
    fn errors_never_contain_the_key() {
        assert_eq!(
            redact("bad key sk-test-123", "sk-test-123"),
            "bad key <redacted>"
        );
        let e = api_error(
            401,
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
        );
        assert!(e.contains("authentication_error"));
        assert!(e.contains("check the API key"));
    }
}
