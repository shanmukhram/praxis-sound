//! Assembles a streamed Messages API response (server-sent events) into the message object a
//! non-streaming request would have returned, so the loop and the conversation log only ever see
//! whole messages.
//!
//! Events (`data:` lines carry the JSON; `event:` lines repeat its `type`):
//! `message_start` (the message without content), `content_block_start` (a block at `index`),
//! `content_block_delta` (text, thinking, signature, citation or partial tool-input JSON),
//! `content_block_stop`, `message_delta` (`stop_reason`, `stop_details`, cumulative `usage`),
//! `message_stop`, `ping` and `error`.
//!
//! Deltas are merged generically, so block kinds this code does not know (a `compaction` block,
//! a `fallback` block) still come back whole: a string field of the delta is appended to the
//! block's field of the same name, `partial_json` is collected and parsed into `input` when the
//! block stops, a `citation` is pushed onto `citations`, anything else replaces the field.

use serde_json::{Map, Value, json};
use std::io::BufRead;

/// Reads the whole stream. `Err` carries a message for people: a stream `error` event (with the
/// error's type, so overloads can be retried), a stream that ends before `message_stop`, or
/// unreadable JSON.
pub fn assemble(reader: impl BufRead) -> Result<Value, StreamError> {
    let mut asm = Assembler::default();
    for line in reader.lines() {
        let line = line.map_err(|e| StreamError::Read(e.to_string()))?;
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        let event: Value = serde_json::from_str(data)
            .map_err(|e| StreamError::Read(format!("unreadable event: {e}")))?;
        if asm.event(&event)? {
            return Ok(asm.finish());
        }
    }
    if asm.started {
        Err(StreamError::Read(
            "the stream ended before the message was complete".into(),
        ))
    } else {
        Err(StreamError::Read("the stream was empty".into()))
    }
}

/// Why a stream could not be assembled.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamError {
    /// An `error` event: the API's error type (`overloaded_error`, `api_error`, ...) and message.
    Api { kind: String, message: String },
    /// The bytes could not be read or parsed, or the stream stopped early.
    Read(String),
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamError::Api { kind, message } => write!(f, "stream error {kind}: {message}"),
            StreamError::Read(m) => write!(f, "stream: {m}"),
        }
    }
}

#[derive(Default)]
struct Assembler {
    started: bool,
    message: Map<String, Value>,
    blocks: Vec<Value>,
    json: Vec<String>,
}

impl Assembler {
    /// Handles one event; true at `message_stop`.
    fn event(&mut self, e: &Value) -> Result<bool, StreamError> {
        match e["type"].as_str().unwrap_or("") {
            "message_start" => {
                self.started = true;
                if let Some(m) = e["message"].as_object() {
                    self.message = m.clone();
                }
            }
            "content_block_start" => {
                let i = index(e);
                self.slot(i);
                self.blocks[i] = e["content_block"].clone();
            }
            "content_block_delta" => {
                let i = index(e);
                self.slot(i);
                let Some(delta) = e["delta"].as_object() else {
                    return Ok(false);
                };
                for (k, v) in delta {
                    if k == "type" {
                        continue;
                    }
                    if k == "partial_json" {
                        self.json[i].push_str(v.as_str().unwrap_or(""));
                        continue;
                    }
                    let block = &mut self.blocks[i];
                    if !block.is_object() {
                        *block = json!({});
                    }
                    if k == "citation" {
                        let list = &mut block["citations"];
                        if !list.is_array() {
                            *list = json!([]);
                        }
                        if let Some(a) = list.as_array_mut() {
                            a.push(v.clone());
                        }
                        continue;
                    }
                    let merged = match (block.get(k).and_then(Value::as_str), v.as_str()) {
                        (Some(old), Some(new)) => json!(format!("{old}{new}")),
                        _ => v.clone(),
                    };
                    block[k] = merged;
                }
            }
            "content_block_stop" => {
                let i = index(e);
                self.slot(i);
                let text = std::mem::take(&mut self.json[i]);
                if !text.trim().is_empty() {
                    // Strict tools give valid JSON; a stream cut off at max_tokens may not. The
                    // loop checks `stop_reason` before running any tool.
                    self.blocks[i]["input"] =
                        serde_json::from_str(&text).unwrap_or_else(|_| json!({}));
                }
            }
            "message_delta" => {
                if let Some(d) = e["delta"].as_object() {
                    for (k, v) in d {
                        self.message.insert(k.clone(), v.clone());
                    }
                }
                if let Some(u) = e["usage"].as_object() {
                    let usage = self.message.entry("usage").or_insert_with(|| json!({}));
                    if !usage.is_object() {
                        *usage = json!({});
                    }
                    for (k, v) in u {
                        usage[k] = v.clone();
                    }
                }
            }
            "message_stop" => return Ok(true),
            "error" => {
                return Err(StreamError::Api {
                    kind: e["error"]["type"].as_str().unwrap_or("error").to_string(),
                    message: e["error"]["message"].as_str().unwrap_or("").to_string(),
                });
            }
            _ => {}
        }
        Ok(false)
    }

    fn slot(&mut self, i: usize) {
        if self.blocks.len() <= i {
            self.blocks.resize(i + 1, Value::Null);
            self.json.resize(i + 1, String::new());
        }
    }

    fn finish(mut self) -> Value {
        let content: Vec<Value> = self.blocks.into_iter().filter(|b| !b.is_null()).collect();
        self.message.insert("content".into(), Value::Array(content));
        Value::Object(self.message)
    }
}

fn index(e: &Value) -> usize {
    e["index"].as_u64().unwrap_or(0) as usize
}

/// The SSE text of a whole message, for tests and replay fixtures: the events a streaming
/// response would carry for `message` (text in one delta, tool input as one JSON delta).
pub fn to_sse(message: &Value) -> String {
    let mut out = String::new();
    let mut push = |v: Value| {
        out.push_str(&format!(
            "event: {}\ndata: {}\n\n",
            v["type"].as_str().unwrap_or(""),
            v
        ));
    };
    let mut start = message.clone();
    start["content"] = json!([]);
    if let Some(m) = start.as_object_mut() {
        m.remove("stop_reason");
        m.remove("stop_sequence");
        m.remove("stop_details");
    }
    push(json!({"type": "message_start", "message": start}));
    let content = message["content"].as_array().cloned().unwrap_or_default();
    for (i, block) in content.iter().enumerate() {
        let kind = block["type"].as_str().unwrap_or("");
        let mut head = block.clone();
        match kind {
            "text" => head["text"] = json!(""),
            "tool_use" => head["input"] = json!({}),
            "thinking" => {
                head["thinking"] = json!("");
                head["signature"] = json!("");
            }
            _ => {}
        }
        push(json!({"type": "content_block_start", "index": i, "content_block": head}));
        match kind {
            "text" => push(json!({"type": "content_block_delta", "index": i,
                "delta": {"type": "text_delta", "text": block["text"]}})),
            "tool_use" => push(json!({"type": "content_block_delta", "index": i,
                "delta": {"type": "input_json_delta", "partial_json": block["input"].to_string()}})),
            "thinking" => {
                push(json!({"type": "content_block_delta", "index": i,
                    "delta": {"type": "thinking_delta", "thinking": block["thinking"]}}));
                push(json!({"type": "content_block_delta", "index": i,
                    "delta": {"type": "signature_delta", "signature": block["signature"]}}));
            }
            _ => {}
        }
        push(json!({"type": "content_block_stop", "index": i}));
    }
    let mut delta = json!({"stop_reason": message["stop_reason"]});
    for k in ["stop_sequence", "stop_details"] {
        if let Some(v) = message.get(k) {
            delta[k] = v.clone();
        }
    }
    push(json!({"type": "message_delta", "delta": delta, "usage": message["usage"]}));
    push(json!({"type": "message_stop"}));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_text_thinking_and_tool_input() {
        let msg = json!({
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
            "content": [
                {"type": "thinking", "thinking": "", "signature": "sig-abc"},
                {"type": "text", "text": "Reading the sound."},
                {"type": "tool_use", "id": "toolu_1", "name": "read_sound",
                 "input": {"part": "summary", "layer": "", "take": 0}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });
        let back = assemble(to_sse(&msg).as_bytes()).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn split_deltas_and_unknown_blocks() {
        let text = "event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"role\":\"assistant\",\"content\":[],\"usage\":{\"input_tokens\":3}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t\",\"name\":\"x\",\"input\":{}}}\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"a\\\":\"}}\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"1}\"}}\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"compaction\",\"content\":\"\"}}\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"compaction_delta\",\"content\":\"sum\"}}\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"compaction_delta\",\"content\":\"mary\"}}\n\
data: {\"type\":\"content_block_stop\",\"index\":1}\n\
event: ping\ndata: {\"type\":\"ping\"}\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":9}}\n\
data: {\"type\":\"message_stop\"}\n";
        let m = assemble(text.as_bytes()).unwrap();
        assert_eq!(m["content"][0]["input"], json!({"a": 1}));
        assert_eq!(
            m["content"][1],
            json!({"type": "compaction", "content": "summary"})
        );
        assert_eq!(m["stop_reason"], "tool_use");
        assert_eq!(m["usage"], json!({"input_tokens": 3, "output_tokens": 9}));
    }

    #[test]
    fn errors_and_truncation() {
        let err = "data: {\"type\":\"message_start\",\"message\":{}}\n\
data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n";
        assert_eq!(
            assemble(err.as_bytes()).unwrap_err(),
            StreamError::Api {
                kind: "overloaded_error".into(),
                message: "Overloaded".into()
            }
        );
        let cut = "data: {\"type\":\"message_start\",\"message\":{}}\n";
        assert!(matches!(
            assemble(cut.as_bytes()),
            Err(StreamError::Read(_))
        ));
        assert!(matches!(assemble(&b""[..]), Err(StreamError::Read(_))));
    }
}
