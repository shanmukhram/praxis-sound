//! The Messages API client: raw HTTP over `ureq` (there is no official Rust SDK), streaming,
//! retries, and the API key kept out of everything but one header.
//!
//! - The key comes from `ANTHROPIC_API_KEY` or the OS keychain ([`crate::ops::key::resolve_key`]),
//!   read when a request is sent. It is only ever placed in the `x-api-key` header; errors are
//!   passed through [`redact`] and `Debug` never shows it.
//! - Requests stream (`"stream": true`); [`super::sse::assemble`] turns the events back into the
//!   whole message. A body without `stream` is read as one JSON message (replays use both).
//! - 429, 5xx, 529 and a mid-stream `overloaded_error` are retried with back-off (the server's
//!   `retry-after` when it sends one).
//! - The HTTP layer is a [`Poster`], so tests run the real client against recorded responses.

use super::sse::{self, StreamError};
use serde_json::Value;
use std::io::{BufReader, Read};
use std::time::Duration;

pub use praxis_planner::claude::{API_URL, API_VERSION};

/// One Messages API request: the JSON body and the beta features it uses (sent as one
/// comma-separated `anthropic-beta` header).
#[derive(Clone, Debug, PartialEq)]
pub struct ApiRequest {
    pub body: Value,
    pub betas: Vec<String>,
}

/// Why a request failed. Messages never contain the key.
#[derive(Clone, Debug, PartialEq)]
pub enum ModelError {
    /// No key in the environment or the keychain.
    NoApiKey,
    /// The API answered with an error status (after retries).
    Http { status: u16, message: String },
    /// No usable answer: the connection failed or the stream broke.
    Transport(String),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::NoApiKey => f.write_str(
                "no Anthropic API key: set ANTHROPIC_API_KEY or store one with `praxis key set`",
            ),
            ModelError::Http { status, message } => write!(f, "HTTP {status}: {message}"),
            ModelError::Transport(m) => write!(f, "the request failed: {m}"),
        }
    }
}

impl std::error::Error for ModelError {}

/// Sends one request and returns the whole assistant message (`content`, `stop_reason`,
/// `usage`, ...). Implemented by [`HttpClient`] and by the scripted model in
/// [`super::testing`].
pub trait ModelClient: Send + Sync {
    fn send(&self, req: &ApiRequest) -> Result<Value, ModelError>;
}

/// One HTTP response, body unread.
pub struct PostResponse {
    pub status: u16,
    /// `retry-after` in seconds, when the server sent it.
    pub retry_after_s: Option<u64>,
    pub body: Box<dyn Read + Send>,
}

/// Sends one JSON POST. `Err` is a transport failure (no response).
pub trait Poster: Send + Sync {
    fn post(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<PostResponse, String>;
}

/// The real HTTP layer.
pub struct UreqPoster {
    agent: ureq::Agent,
}

impl Default for UreqPoster {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            // A long agent turn streams for minutes; the stream itself keeps the connection busy.
            .timeout_global(Some(Duration::from_secs(1200)))
            .http_status_as_error(false)
            .build();
        UreqPoster {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl Poster for UreqPoster {
    fn post(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<PostResponse, String> {
        let mut req = self.agent.post(url);
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send_json(body).map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let retry_after_s = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse().ok());
        Ok(PostResponse {
            status,
            retry_after_s,
            body: Box::new(resp.into_body().into_reader()),
        })
    }
}

/// Where the key comes from (called once per request).
pub type KeySource = Box<dyn Fn() -> Result<String, ModelError> + Send + Sync>;

/// The key from the environment or the keychain, as the rest of Praxis finds it.
pub fn default_key() -> Result<String, ModelError> {
    crate::ops::key::resolve_key().map_err(|_| ModelError::NoApiKey)
}

/// The Messages API over HTTP.
pub struct HttpClient {
    url: String,
    poster: Box<dyn Poster>,
    key: KeySource,
    /// Retries for 429, 5xx, 529 and mid-stream overloads.
    pub max_retries: u32,
    /// Back-off before the first retry, doubled after each.
    pub retry_base_ms: u64,
}

impl std::fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpClient")
            .field("url", &self.url)
            .field("key", &"<redacted>")
            .finish()
    }
}

impl HttpClient {
    /// The real API (or `PRAXIS_CLAUDE_URL`), key from the environment or keychain.
    pub fn new() -> HttpClient {
        let url = std::env::var(crate::ops::generate::CLAUDE_URL_ENV)
            .ok()
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| API_URL.to_string());
        HttpClient::with_parts(url, Box::new(UreqPoster::default()), Box::new(default_key))
    }

    pub fn with_parts(url: String, poster: Box<dyn Poster>, key: KeySource) -> HttpClient {
        HttpClient {
            url,
            poster,
            key,
            max_retries: 3,
            retry_base_ms: 1000,
        }
    }

    /// The request headers. The key is only ever placed here.
    pub fn headers(key: &str, betas: &[String]) -> Vec<(String, String)> {
        let mut h = vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("x-api-key".to_string(), key.to_string()),
            ("anthropic-version".to_string(), API_VERSION.to_string()),
        ];
        if !betas.is_empty() {
            h.push(("anthropic-beta".to_string(), betas.join(",")));
        }
        h
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        HttpClient::new()
    }
}

impl ModelClient for HttpClient {
    fn send(&self, req: &ApiRequest) -> Result<Value, ModelError> {
        let key = (self.key)()?;
        let headers = HttpClient::headers(&key, &req.betas);
        let streaming = req.body["stream"] == Value::Bool(true);
        let mut attempt = 0u32;
        loop {
            let resp = self
                .poster
                .post(&self.url, &headers, &req.body)
                .map_err(|e| ModelError::Transport(redact(&e, &key)))?;
            let retryable = matches!(resp.status, 429 | 500 | 502 | 503 | 504 | 529);
            let wait = |attempt: u32, after: Option<u64>| {
                after
                    .map(|s| s.min(60) * 1000)
                    .unwrap_or(self.retry_base_ms << attempt.min(6))
            };
            if resp.status == 200 {
                let outcome = if streaming {
                    sse::assemble(BufReader::new(resp.body))
                } else {
                    read_json(resp.body)
                };
                match outcome {
                    Ok(message) => return Ok(message),
                    Err(StreamError::Api { kind, .. })
                        if kind == "overloaded_error" && attempt < self.max_retries =>
                    {
                        sleep_ms(wait(attempt, None));
                        attempt += 1;
                        continue;
                    }
                    Err(e) => return Err(ModelError::Transport(redact(&e.to_string(), &key))),
                }
            }
            let mut text = String::new();
            let _ = resp.body.take(1 << 20).read_to_string(&mut text);
            if retryable && attempt < self.max_retries {
                sleep_ms(wait(attempt, resp.retry_after_s));
                attempt += 1;
                continue;
            }
            return Err(ModelError::Http {
                status: resp.status,
                message: redact(&api_error(resp.status, &text), &key),
            });
        }
    }
}

fn sleep_ms(ms: u64) {
    if ms > 0 {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

fn read_json(mut body: Box<dyn Read + Send>) -> Result<Value, StreamError> {
    let mut text = String::new();
    body.read_to_string(&mut text)
        .map_err(|e| StreamError::Read(e.to_string()))?;
    serde_json::from_str(&text).map_err(|e| StreamError::Read(format!("unreadable response: {e}")))
}

/// Replaces every occurrence of the key.
pub fn redact(text: &str, key: &str) -> String {
    if key.is_empty() {
        text.to_string()
    } else {
        text.replace(key, "<redacted>")
    }
}

/// `"<error type>: <message> (hint)"` from an API error body.
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
    format!("{kind}: {msg}{hint}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    const KEY: &str = "test-placeholder-not-a-key";

    struct Canned {
        responses: Mutex<Vec<(u16, String)>>,
        seen: Arc<Mutex<Vec<Vec<(String, String)>>>>,
    }

    impl Poster for Canned {
        fn post(
            &self,
            _url: &str,
            headers: &[(String, String)],
            _body: &Value,
        ) -> Result<PostResponse, String> {
            self.seen.lock().unwrap().push(headers.to_vec());
            let (status, body) = self.responses.lock().unwrap().remove(0);
            Ok(PostResponse {
                status,
                retry_after_s: Some(0),
                body: Box::new(std::io::Cursor::new(body.into_bytes())),
            })
        }
    }

    fn client(
        responses: Vec<(u16, String)>,
    ) -> (HttpClient, Arc<Mutex<Vec<Vec<(String, String)>>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut c = HttpClient::with_parts(
            "http://test.invalid/v1/messages".into(),
            Box::new(Canned {
                responses: Mutex::new(responses),
                seen: seen.clone(),
            }),
            Box::new(|| Ok(KEY.to_string())),
        );
        c.retry_base_ms = 0;
        (c, seen)
    }

    #[test]
    fn key_only_in_its_header_and_never_in_errors() {
        let body = json!({"type": "error", "error": {"type": "authentication_error",
            "message": format!("invalid x-api-key {KEY}")}})
        .to_string();
        let (c, seen) = client(vec![(401, body)]);
        let err = c
            .send(&ApiRequest {
                body: json!({"stream": true}),
                betas: vec!["a-1".into(), "b-2".into()],
            })
            .unwrap_err();
        let text = err.to_string();
        assert!(!text.contains(KEY), "{text}");
        assert!(text.contains("authentication_error") && text.contains("check the API key"));
        assert!(!format!("{c:?}").contains(KEY));
        let headers = &seen.lock().unwrap()[0];
        let with_key: Vec<_> = headers.iter().filter(|(_, v)| v.contains(KEY)).collect();
        assert_eq!(with_key.len(), 1);
        assert_eq!(with_key[0].0, "x-api-key");
        assert!(headers.contains(&("anthropic-beta".into(), "a-1,b-2".into())));
    }

    #[test]
    fn retries_overloads_then_reads_the_stream() {
        let msg = json!({"id": "m", "role": "assistant", "content": [{"type": "text", "text": "hi"}],
            "stop_reason": "end_turn", "usage": {"input_tokens": 1, "output_tokens": 1}});
        let overloaded_stream = "data: {\"type\":\"message_start\",\"message\":{}}\n\
data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n"
            .to_string();
        let (c, seen) = client(vec![
            (529, "{}".into()),
            (200, overloaded_stream),
            (200, super::super::sse::to_sse(&msg)),
        ]);
        let got = c
            .send(&ApiRequest {
                body: json!({"stream": true}),
                betas: vec![],
            })
            .unwrap();
        assert_eq!(got, msg);
        assert_eq!(seen.lock().unwrap().len(), 3);
    }
}
