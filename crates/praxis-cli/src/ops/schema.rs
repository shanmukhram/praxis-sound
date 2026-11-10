//! `schema`: the published JSON Schemas. `plan` is the ScorePlan schema an agent fills for
//! `generate --plan` (printed only; `--write` keeps writing just the two files in `schema/`).

use crate::error::{OpError, io_error};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SchemaRequest {
    /// Write schema/score.schema.json and schema/patch.schema.json in the source tree.
    pub write: bool,
}

pub fn schema(req: &SchemaRequest) -> Result<Value, OpError> {
    let score = praxis_score::score_schema();
    let patch = praxis_score::patch_schema();
    if req.write {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema");
        std::fs::create_dir_all(&dir).map_err(|e| io_error(&dir, e))?;
        for (name, value) in [("score.schema.json", &score), ("patch.schema.json", &patch)] {
            let path = dir.join(name);
            let text = serde_json::to_string_pretty(value).unwrap_or_default() + "\n";
            std::fs::write(&path, text).map_err(|e| io_error(&path, e))?;
        }
        return Ok(json!({ "written": ["schema/score.schema.json", "schema/patch.schema.json"] }));
    }
    Ok(json!({ "score": score, "patch": patch, "plan": praxis_planner::plan_schema() }))
}
