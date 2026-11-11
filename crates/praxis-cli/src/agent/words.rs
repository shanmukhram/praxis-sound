//! Plain words for what the assistant is doing, for the progress display and the turn record:
//! "Reading the sound", "Changing the Felt piano's tone and felt", "Rendering", "Checking: 8 of
//! 8 passed".

use praxis_score::Score;
use serde_json::Value;

/// A layer as people call it: its name, else its id.
pub fn layer_name(score: Option<&Score>, id: &str) -> String {
    score
        .and_then(|s| s.layer(id))
        .map(|l| {
            if l.name.trim().is_empty() {
                l.id.clone()
            } else {
                l.name.clone()
            }
        })
        .unwrap_or_else(|| id.to_string())
}

/// `a`, `a and b`, `a, b and c`.
pub fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// What a tool call is about to do.
pub fn step_words(tool: &str, input: &Value, score: Option<&Score>) -> String {
    let s = |k: &str| input[k].as_str().unwrap_or("").to_string();
    match tool {
        "read_sound" => match s("part").as_str() {
            "layer" => format!("Reading the {}", layer_name(score, &s("layer"))),
            "score" => "Reading the whole score".into(),
            "takes" => "Reading the takes".into(),
            "changes" => match input["take"].as_u64() {
                Some(n) if n > 0 => format!("Reading what changed in take {n}"),
                _ => "Reading what changed in the current take".into(),
            },
            _ => "Reading the sound".into(),
        },
        "catalogue" => match s("preset").as_str() {
            "" => "Looking up the presets".into(),
            p => format!("Looking up {p}"),
        },
        "get_schema" => match s("which").as_str() {
            "score_plan" => "Looking up the plan format".into(),
            "edit_plan" => "Looking up the edit format".into(),
            _ => "Looking up the patch format".into(),
        },
        "compose_sound" => "Writing a new sound".into(),
        "compose_edit" => {
            let plan: Value = serde_json::from_str(&s("edit_plan_json")).unwrap_or(Value::Null);
            let intent = plan["intent"].as_str().unwrap_or("edit").replace('_', " ");
            let layers: Vec<String> = plan["layers"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(|l| layer_name(score, l))
                        .collect()
                })
                .unwrap_or_default();
            if layers.is_empty() {
                format!("Planning the edit: {intent}")
            } else {
                format!("Planning the edit: {intent} on the {}", and_list(&layers))
            }
        }
        "apply_patch" => {
            let patch: Value = serde_json::from_str(&s("patch_json")).unwrap_or(Value::Null);
            match patch["description"].as_str().map(str::trim) {
                Some(d) if !d.is_empty() => format!("Changing the score: {d}"),
                _ => "Changing the score".into(),
            }
        }
        "set_params" => {
            let names: Vec<String> = input["params"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|p| p["name"].as_str())
                        .map(|n| n.replace('_', " "))
                        .collect()
                })
                .unwrap_or_default();
            format!(
                "Changing the {}'s {}",
                layer_name(score, &s("layer")),
                and_list(&names)
            )
        }
        "render_check" => "Rendering".into(),
        "make_variations" => format!("Making {} variations", input["count"].as_u64().unwrap_or(3)),
        "keep_variation" => format!("Keeping variation {}", input["index"].as_u64().unwrap_or(0)),
        "restore_take" => format!("Going back to take {}", input["n"].as_u64().unwrap_or(0)),
        "undo_redo" => match s("action").as_str() {
            "redo" => "Redoing the last undone step".into(),
            _ => "Undoing the last step".into(),
        },
        "open_view" => format!("Opening the {} view", s("view")),
        "analyze_clip" => {
            let p = s("path");
            let name = std::path::Path::new(&p)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or(p);
            format!("Measuring {name}")
        }
        "ask_user" => "Asking you a question".into(),
        "discard_changes" => "Starting again from the current take".into(),
        other => format!("Running {other}"),
    }
}

/// What a finished call did (`result` is the tool's JSON result).
pub fn done_words(tool: &str, ok: bool, result: &Value, error: &str) -> String {
    if !ok {
        let first = error.lines().next().unwrap_or("").trim();
        return format!("That did not work: {first}");
    }
    match tool {
        "render_check" => {
            // Only decided checks count; "can't tell" ones are neither passed nor failed.
            let checks = result["check"]["checks"].as_array();
            let unsure = |k: &&Value| k["inconclusive"] == Value::Bool(true);
            let total = checks.map_or(0, |c| c.iter().filter(|k| !unsure(k)).count());
            let passed = checks.map_or(0, |c| {
                c.iter()
                    .filter(|k| k["passed"] == Value::Bool(true))
                    .count()
            });
            let cant = checks.map_or(0, |c| c.iter().filter(unsure).count());
            if cant == 0 {
                format!("Checking: {passed} of {total} passed")
            } else {
                format!("Checking: {passed} of {total} passed, {cant} can't tell")
            }
        }
        "make_variations" => format!(
            "Made {} variations",
            result["variations"].as_array().map_or(0, |v| v.len())
        ),
        _ => "Done".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn words_for_people() {
        assert_eq!(
            and_list(&["a".into(), "b".into(), "c".into()]),
            "a, b and c"
        );
        assert_eq!(
            step_words(
                "set_params",
                &json!({"layer": "piano", "params": [{"name": "tone", "value": 0.2},
                    {"name": "felt", "value": 0.9}]}),
                None
            ),
            "Changing the piano's tone and felt"
        );
        assert_eq!(step_words("render_check", &json!({}), None), "Rendering");
        let r = json!({"check": {"checks": [{"passed": true}, {"passed": false}]}});
        assert_eq!(
            done_words("render_check", true, &r, ""),
            "Checking: 1 of 2 passed"
        );
        let r = json!({"check": {"checks": [{"passed": true},
            {"passed": false, "inconclusive": true}, {"passed": true}]}});
        assert_eq!(
            done_words("render_check", true, &r, ""),
            "Checking: 2 of 2 passed, 1 can't tell"
        );
        assert_eq!(
            done_words(
                "apply_patch",
                false,
                &Value::Null,
                "layer `kick` is locked\nmore"
            ),
            "That did not work: layer `kick` is locked"
        );
    }
}
