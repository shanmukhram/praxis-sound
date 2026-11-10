//! The JSON envelope and exit codes (docs/contracts/cli.md).

use crate::OpError;
use serde_json::{Value, json};

/// `{"ok": true, "command": ..., ...payload}` or
/// `{"ok": false, "command": ..., "error": {"code", "message"[, "issues"]}}`.
pub fn envelope(command: &str, result: &Result<Value, OpError>) -> Value {
    match result {
        Ok(payload) => {
            let mut v = match payload {
                Value::Object(_) => payload.clone(),
                other => json!({ "result": other }),
            };
            v["ok"] = json!(true);
            v["command"] = json!(command);
            v
        }
        Err(e) => json!({ "ok": false, "command": command, "error": e.to_json() }),
    }
}

/// 0 on success, 1 on error. `check` also exits 1 when the render fails any check (its payload
/// still says `"ok": true`, because the command itself ran; `passed` says the verdict).
pub fn exit_code(command: &str, result: &Result<Value, OpError>) -> i32 {
    match result {
        Err(_) => 1,
        Ok(v) if command == "check" && v.get("passed") == Some(&Value::Bool(false)) => 1,
        Ok(_) => 0,
    }
}

/// Prints the result (one JSON object with `as_json`, else readable text) and returns the exit
/// code.
pub fn emit(command: &str, as_json: bool, result: Result<Value, OpError>) -> i32 {
    let code = exit_code(command, &result);
    if as_json {
        println!("{}", envelope(command, &result));
        return code;
    }
    match &result {
        Ok(v) => println!("{}", serde_json::to_string_pretty(v).unwrap_or_default()),
        Err(e) => {
            eprintln!("error [{}]: {}", e.code.as_str(), e.message);
            for i in &e.issues {
                eprintln!("  {i}");
            }
        }
    }
    code
}
