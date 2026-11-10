//! `presets`: the planner's preset catalogue (what a ScorePlan may name) and whether each one
//! can render on this machine (preset built, pack installed).

use crate::OpError;
use crate::ops::render::instrument_unavailable;
use praxis_engine::InstrumentFactory;
use praxis_instruments::Registry;
use praxis_planner::Catalogue;
use serde_json::{Value, json};

/// `{presets[], engine_only[]}`. Each catalogue preset carries `available` and, when false, a
/// `reason`. `engine_only` lists presets the engine has that the planner catalogue does not.
pub fn presets() -> Result<Value, OpError> {
    let catalogue = Catalogue::standard();
    let registry = Registry::with_default_packs();
    let list: Vec<Value> = catalogue
        .presets
        .iter()
        .map(|p| {
            let reason = instrument_unavailable(&registry, p.kind, &p.name);
            json!({
                "name": p.name,
                "kind": p.kind,
                "class": p.class,
                "pitched": p.pitched,
                "sustaining": p.sustaining,
                "range": [p.range.0, p.range.1],
                "description": p.description,
                "available": reason.is_none(),
                "reason": reason,
            })
        })
        .collect();
    let engine_only: Vec<Value> = registry
        .presets()
        .iter()
        .filter(|i| catalogue.get_kind(i.kind, &i.name).is_none())
        .map(|i| json!({ "name": i.name, "kind": i.kind, "class": i.class, "source": i.source }))
        .collect();
    Ok(json!({ "presets": list, "engine_only": engine_only }))
}
