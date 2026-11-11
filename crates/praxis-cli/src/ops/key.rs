//! `key set|clear|status`: the Anthropic API key in the OS keychain (service `praxis-sound`,
//! user `anthropic`, through the planner's helper).
//!
//! The key is never printed, logged, written to a file or accepted on the command line: the CLI
//! reads it from stdin. Payloads only say whether a key is available and where from.
//! Set `PRAXIS_KEYCHAIN=off` to make Praxis ignore the keychain (only `ANTHROPIC_API_KEY`
//! counts); CI and the tests use it so they never read or change a real keychain.

use crate::error::{ErrorCode, OpError};
use praxis_planner::ProviderError;
use serde_json::{Value, json};

/// Environment variable that holds the key (takes precedence over the keychain).
pub const KEY_ENV: &str = "ANTHROPIC_API_KEY";
/// `PRAXIS_KEYCHAIN=off` disables keychain reads and writes.
pub const KEYCHAIN_ENV: &str = "PRAXIS_KEYCHAIN";

pub enum KeyAction {
    /// Store this key (read from stdin by the CLI).
    Set(String),
    /// Remove the stored key.
    Clear,
    /// Report whether a key is available, never the key.
    Status,
}

impl std::fmt::Debug for KeyAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyAction::Set(_) => f.write_str("Set(<redacted>)"),
            KeyAction::Clear => f.write_str("Clear"),
            KeyAction::Status => f.write_str("Status"),
        }
    }
}

/// Not `Serialize`/`Deserialize` on purpose: keys do not travel through JSON tool calls.
#[derive(Debug)]
pub struct KeyRequest {
    pub action: KeyAction,
}

pub fn keychain_enabled() -> bool {
    !matches!(
        std::env::var(KEYCHAIN_ENV).ok().as_deref().map(str::trim),
        Some("off" | "0" | "false" | "no")
    )
}

fn env_key() -> Option<String> {
    std::env::var(KEY_ENV)
        .ok()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}

/// The key for the Claude provider: `ANTHROPIC_API_KEY`, else the keychain (unless disabled).
pub fn resolve_key() -> Result<String, ProviderError> {
    if let Some(k) = env_key() {
        return Ok(k);
    }
    if !keychain_enabled() {
        return Err(ProviderError::NoApiKey);
    }
    praxis_planner::claude::default_key()
}

/// Where the key would come from: `"env"`, `"keychain"`, or `None`.
pub fn key_source() -> Option<&'static str> {
    if env_key().is_some() {
        Some("env")
    } else if keychain_enabled() && praxis_planner::claude::default_key().is_ok() {
        Some("keychain")
    } else {
        None
    }
}

pub fn key(req: &KeyRequest) -> Result<Value, OpError> {
    match &req.action {
        KeyAction::Status => {
            let source = key_source();
            Ok(json!({
                "available": source.is_some(),
                "source": source,
                "keychain": keychain_enabled(),
            }))
        }
        KeyAction::Set(raw) => {
            let k = raw.trim();
            if k.is_empty() {
                return Err(OpError::new(
                    ErrorCode::NoApiKey,
                    "no key on stdin: pipe it in, e.g. `praxis key set < key.txt`",
                ));
            }
            if k.chars().any(char::is_whitespace) {
                return Err(OpError::new(
                    ErrorCode::Error,
                    "the key on stdin contains whitespace; give exactly one key",
                ));
            }
            require_keychain()?;
            praxis_planner::claude::store_key_in_keychain(k)
                .map_err(|e| OpError::new(ErrorCode::Error, e.to_string()))?;
            Ok(json!({
                "stored": true,
                "store": "keychain",
                "env_overrides": env_key().is_some(),
            }))
        }
        KeyAction::Clear => {
            require_keychain()?;
            let existed = delete_keychain_entry()?;
            Ok(json!({
                "cleared": true,
                "existed": existed,
                "env_overrides": env_key().is_some(),
            }))
        }
    }
}

/// Deletes the keychain entry (service `praxis-sound`, user `anthropic`). `Ok(false)` when there
/// was none.
fn delete_keychain_entry() -> Result<bool, OpError> {
    use praxis_planner::claude::{KEYRING_SERVICE, KEYRING_USER};
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| OpError::new(ErrorCode::Error, format!("keychain: {e}")))?;
    match entry.delete_credential() {
        Ok(()) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(OpError::new(
            ErrorCode::Error,
            format!("could not delete the key from the keychain: {e}"),
        )),
    }
}

fn require_keychain() -> Result<(), OpError> {
    if keychain_enabled() {
        Ok(())
    } else {
        Err(OpError::new(
            ErrorCode::Error,
            format!("the keychain is disabled ({KEYCHAIN_ENV}=off)"),
        ))
    }
}
