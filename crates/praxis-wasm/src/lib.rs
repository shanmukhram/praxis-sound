//! praxis-wasm: browser build of the engine and analyzer (Phase 8, WP-W).

#![forbid(unsafe_code)]

use wasm_bindgen::prelude::*;

/// Validates a score JSON document and returns the validation report as JSON.
#[wasm_bindgen]
pub fn validate_score(json: &str) -> String {
    match serde_json::from_str::<praxis_score::Score>(json) {
        Ok(score) => serde_json::to_string(&praxis_score::validate::validate(&score))
            .unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}")),
        Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
    }
}
