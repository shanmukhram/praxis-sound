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
        "match_reference" => {
            let p = s("path");
            let name = std::path::Path::new(&p)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or(p);
            format!("Making something like {name}")
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
        "match_reference" => format!(
            "Made it: {} (similarity {:.2})",
            result["made_with"].as_str().unwrap_or("a new sound"),
            result["similarity"].as_f64().unwrap_or(0.0)
        ),
        "make_variations" => format!(
            "Made {} variations",
            result["variations"].as_array().map_or(0, |v| v.len())
        ),
        _ => "Done".into(),
    }
}

/// Longest take title kept as given, in words.
const TITLE_WORDS: usize = 8;

/// The take title the assistant gave (`finish_turn`), tidied: quotes and a final full stop
/// dropped, the first letter upper case. `None` when it is empty, starts with "I", or is too
/// long to keep whole (a title is never clipped).
pub fn clean_title(title: &str) -> Option<String> {
    let t = title
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '\u{201c}' | '\u{201d}' | '`'))
        .trim()
        .trim_end_matches('.')
        .trim();
    let words: Vec<&str> = t.split_whitespace().collect();
    if words.is_empty()
        || words.len() > TITLE_WORDS
        || t.chars().count() > 60
        || t.contains('\n')
        || words[0] == "I"
        || words[0].starts_with("I'")
    {
        return None;
    }
    let joined = words.join(" ");
    let mut c = joined.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect())
}

/// "Felt piano" -> "felt piano" (a name after another word).
fn lower_first(name: &str) -> String {
    let mut c = name.chars();
    match c.next() {
        Some(f) if !c.as_str().chars().next().is_some_and(char::is_uppercase) => {
            f.to_lowercase().chain(c).collect()
        }
        _ => name.to_string(),
    }
}

fn upper_first(name: &str) -> String {
    let mut c = name.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// A short title that sums up what changed from `base` to `after` (used when the assistant gave
/// none): "Felt piano sound", "New upright piano, bars 13–16", "Kick level", "Piano and kick
/// changed". `base` is `None` for a new sound.
pub fn summary_title(base: Option<&Score>, after: &Score) -> String {
    use crate::ops::changes::{bars_label, changed_layers};
    let Some(base) = base else {
        let t = after.title.trim();
        return if t.is_empty() || t.split_whitespace().count() > TITLE_WORDS {
            "New sound".to_string()
        } else {
            upper_first(t)
        };
    };
    let changed = changed_layers(base, after);
    let name = |id: &str| {
        let s = if after.layer(id).is_some() {
            after
        } else {
            base
        };
        layer_name(Some(s), id)
    };
    match changed.len() {
        0 => {
            if base.tempo != after.tempo {
                "New tempo".into()
            } else if base.key != after.key {
                "New key".into()
            } else {
                "Assistant edit".into()
            }
        }
        1 => {
            let (id, r) = changed.iter().next().expect("one");
            let n = name(id);
            let (Some(x), Some(y)) = (base.layer(id), after.layer(id)) else {
                return if base.layer(id).is_none() {
                    let whole = [1, after.bars];
                    if *r == whole {
                        format!("New {}", lower_first(&n))
                    } else {
                        format!("New {}, {}", lower_first(&n), bars_label(*r))
                    }
                } else {
                    format!("{} removed", upper_first(&n))
                };
            };
            let what = if x.instrument != y.instrument {
                if x.instrument.preset != y.instrument.preset {
                    "instrument"
                } else {
                    "sound"
                }
            } else if x.gain_db != y.gain_db {
                "level"
            } else if x.pan != y.pan {
                "pan"
            } else if x.effects != y.effects || x.automation != y.automation {
                "effects"
            } else if x.muted != y.muted || x.solo != y.solo {
                "mute"
            } else {
                "notes"
            };
            let extent = [
                y.clips.iter().map(|c| c.bars.start).min().unwrap_or(1),
                y.clips
                    .iter()
                    .map(|c| c.bars.end)
                    .max()
                    .unwrap_or(after.bars),
            ];
            if *r == extent || *r == [1, after.bars] {
                format!("{} {what}", upper_first(&n))
            } else {
                format!("{} {what}, {}", upper_first(&n), bars_label(*r))
            }
        }
        2 => {
            let ids: Vec<&String> = changed.keys().collect();
            format!(
                "{} and {} changed",
                upper_first(&name(ids[0])),
                lower_first(&name(ids[1]))
            )
        }
        n => format!("{n} layers changed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn titles_are_kept_whole_or_summarised() {
        assert_eq!(
            clean_title(" \"muffled felt piano.\" ").as_deref(),
            Some("Muffled felt piano")
        );
        assert_eq!(clean_title("I made a 16-bar loop"), None);
        assert_eq!(clean_title(""), None);
        assert_eq!(clean_title(&"word ".repeat(12)), None, "never clipped");
        let plan: Value = serde_json::from_str(include_str!(
            "../../../praxis-planner/tests/prompts/acceptance1.plan.json"
        ))
        .unwrap();
        let base = praxis_planner::compose_plan(&plan, &praxis_planner::Catalogue::standard(), 1)
            .unwrap()
            .score;
        assert_eq!(summary_title(None, &base), upper_first(base.title.trim()));
        let mut after = base.clone();
        after.layers[0].instrument.params.insert("tone".into(), 0.2);
        let t = summary_title(Some(&base), &after);
        assert!(t.ends_with(" sound"), "{t}");
        assert!(t.split_whitespace().count() <= 6, "{t}");
        let mut louder = base.clone();
        louder.layers[0].gain_db += 2.0;
        assert!(summary_title(Some(&base), &louder).ends_with(" level"));
    }

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
