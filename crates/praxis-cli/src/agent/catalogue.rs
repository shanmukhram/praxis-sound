//! The catalogue the agent works from, generated from code so it never goes stale: the
//! planner's presets (`praxis_planner::Catalogue`), each preset's parameters with their ranges
//! and defaults from the instrument registry, what each parameter does, the effect parameters of
//! `praxis_score::params`, and the layer parameters automation can reach.

use crate::ops::render::instrument_unavailable;
use praxis_engine::InstrumentFactory;
use praxis_instruments::Registry;
use praxis_planner::Catalogue;
use praxis_planner::catalogue::{class_name, kind_name};
use praxis_score::EffectKind;
use praxis_score::params::{LAYER_PARAMS, ParamSpec, effect_params};
use serde_json::{Value, json};

/// Every effect kind, in the order of `EffectKind`.
pub const EFFECT_KINDS: [(EffectKind, &str, &str); 11] = [
    (
        EffectKind::Eq,
        "eq",
        "four-band EQ with high-pass and low-pass cuts: tame lows, cut harshness, darken with lp_hz",
    ),
    (
        EffectKind::Filter,
        "filter",
        "state-variable filter (mode 0 low-pass, 1 high-pass, 2 band-pass, 3 notch)",
    ),
    (
        EffectKind::Compressor,
        "compressor",
        "evens out dynamics; with `sidechain` set to a layer id it ducks under that layer",
    ),
    (
        EffectKind::Saturation,
        "saturation",
        "soft drive and a tilt tone control: warmth and grit",
    ),
    (
        EffectKind::Delay,
        "delay",
        "echo, tempo-synced when time_beats > 0",
    ),
    (
        EffectKind::Reverb,
        "reverb",
        "algorithmic room: space and distance (decay_s, size, mix)",
    ),
    (
        EffectKind::ConvolutionReverb,
        "convolution_reverb",
        "a recorded space (`impulse` names it)",
    ),
    (
        EffectKind::Chorus,
        "chorus",
        "slow pitch modulation: width and shimmer",
    ),
    (
        EffectKind::Width,
        "width",
        "mid/side width; bass_mono_hz keeps the low end centred",
    ),
    (
        EffectKind::Limiter,
        "limiter",
        "true-peak limiter (the master already has one)",
    ),
    (EffectKind::Gain, "gain", "a plain gain stage"),
];

/// What a preset parameter does, in words a model can act on.
pub fn param_doc(preset: &str, name: &str) -> &'static str {
    match (preset, name) {
        ("felt_piano", "tone") => {
            "brightness of the felt filter: lower is darker and more muffled, higher is brighter \
             (the cutoff moves three octaves over 0..1)"
        }
        ("felt_piano", "felt") => {
            "amount of felt: higher is softer, rounder and more muffled (softer sample layers, a \
             darker filter, less hammer noise, gentler dynamics)"
        }
        ("upright_piano", "tone") => {
            "below 0.5 adds a low-pass (darker), above 0.5 lifts the highs (brighter)"
        }
        ("mallet", "material") => {
            "bar material: 0 marimba-like (short ring), 1 vibraphone-like (long ring)"
        }
        (_, "tone") => "brightness: lower is darker and softer, higher is brighter",
        (_, "release_s") => "how long a note rings after it ends, seconds",
        (_, "decay_s") => "how long a struck note takes to die away, seconds",
        (_, "attack_s") => "fade-in time of each note, seconds",
        (_, "sustain") => "level a held note settles to (0..1)",
        (_, "resonance") => "filter resonance: higher is more nasal and pronounced",
        (_, "density") => "how often grains start (x the normal rate)",
        (_, "drift") => "how far the colour wanders over time",
        (_, "tune") => "pitch of the drum, semitones",
        (_, "decay") => "length of the drum hit (x the normal length)",
        _ => "",
    }
}

fn spec_json(s: &ParamSpec) -> Value {
    json!({
        "name": s.name,
        "min": s.min,
        "max": s.max,
        "default": s.default,
        "unit": s.unit,
        "does": s.doc,
    })
}

/// The `catalogue` tool's payload. `preset` names one preset ("" = all). Each preset carries
/// `available` (renders on this machine) and, when false, `reason`.
pub fn catalogue_json(preset: &str) -> Value {
    let catalogue = Catalogue::standard();
    let registry = Registry::with_default_packs();
    let wanted = preset.trim();
    let presets: Vec<Value> = catalogue
        .presets
        .iter()
        .filter(|p| wanted.is_empty() || p.name == wanted)
        .map(|p| {
            let params: Vec<Value> = registry
                .describe(p.kind, &p.name)
                .map(|info| {
                    info.params
                        .iter()
                        .map(|q| {
                            json!({
                                "name": q.name,
                                "min": q.min,
                                "max": q.max,
                                "default": q.default,
                                "unit": q.unit,
                                "does": param_doc(&p.name, &q.name),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let reason = instrument_unavailable(&registry, p.kind, &p.name);
            json!({
                "name": p.name,
                "kind": kind_name(p.kind),
                "class": class_name(p.class),
                "pitched": p.pitched,
                "sustaining": p.sustaining,
                "range": [p.range.0, p.range.1],
                "description": p.description,
                "params": params,
                "available": reason.is_none(),
                "reason": reason,
            })
        })
        .collect();
    let effects: Vec<Value> = EFFECT_KINDS
        .iter()
        .map(|(kind, name, does)| {
            json!({
                "kind": name,
                "does": does,
                "params": effect_params(*kind).iter().map(spec_json).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "presets": presets,
        "effects": if wanted.is_empty() { json!(effects) } else { Value::Null },
        "layer_params": LAYER_PARAMS.iter().map(spec_json).collect::<Vec<_>>(),
        "automation_targets": "\"gain_db\", \"pan\", \"instrument.<param>\" or \"effects.<effect id>.<param>\"",
    })
}

fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        let s = format!("{v:.3}");
        s.trim_end_matches('0').to_string()
    }
}

fn range_text(min: f64, max: f64, default: f64, unit: &str) -> String {
    let u = if unit.is_empty() || unit == "enum" || unit == "bool" {
        String::new()
    } else {
        format!(" {unit}")
    };
    format!(
        "{}..{}{u}, default {}",
        fmt_num(min),
        fmt_num(max),
        fmt_num(default)
    )
}

/// The catalogue as Markdown for the system prompt: stable (no availability, which depends on
/// the machine; the `catalogue` tool reports that).
pub fn prompt_tables() -> String {
    let catalogue = Catalogue::standard();
    let registry = Registry::with_default_packs();
    let mut out = String::from(
        "## Presets and their parameters (generated from the code)\n\n\
         Set a preset parameter with `set_params` (it lands in the layer's `instrument.params`). \
         Unset parameters use the default.\n\n\
         | Preset | Kind | Class | Notes | Parameters |\n| --- | --- | --- | --- | --- |\n",
    );
    for p in &catalogue.presets {
        let params = registry
            .describe(p.kind, &p.name)
            .map(|info| {
                info.params
                    .iter()
                    .map(|q| {
                        let doc = param_doc(&p.name, &q.name);
                        format!(
                            "`{}` {}{}",
                            q.name,
                            range_text(q.min, q.max, q.default, &q.unit),
                            if doc.is_empty() {
                                String::new()
                            } else {
                                format!(": {doc}")
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} |\n",
            p.name,
            kind_name(p.kind),
            class_name(p.class),
            p.description.replace('|', "/"),
            if params.is_empty() {
                "-"
            } else {
                params.as_str()
            }
        ));
    }
    out.push_str(
        "\n## Effects (praxis_score::params, generated)\n\n\
         An effect is `{id, kind, params{}, bypass}` in a layer's `effects` list (set the whole \
         list with an `update_layer` patch). Unset parameters use the default.\n\n\
         | Kind | Does | Parameters |\n| --- | --- | --- |\n",
    );
    for (kind, name, does) in EFFECT_KINDS {
        let params = effect_params(kind)
            .iter()
            .map(|s| {
                format!(
                    "`{}` {} ({})",
                    s.name,
                    range_text(s.min, s.max, s.default, s.unit),
                    s.doc
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        out.push_str(&format!("| `{name}` | {does} | {params} |\n"));
    }
    out.push_str("\nLayer parameters (also automation targets): ");
    out.push_str(
        &LAYER_PARAMS
            .iter()
            .map(|s| {
                format!(
                    "`{}` {} ({})",
                    s.name,
                    range_text(s.min, s.max, s.default, s.unit),
                    s.doc
                )
            })
            .collect::<Vec<_>>()
            .join(", "),
    );
    out.push_str(
        ". Automation targets: `gain_db`, `pan`, `instrument.<param>`, \
         `effects.<effect id>.<param>`; points are `{bar, value}` with bar 1.0 the start of bar 1.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn felt_piano_parameters_are_listed_with_meaning() {
        let c = catalogue_json("felt_piano");
        let p = &c["presets"][0];
        assert_eq!(p["name"], "felt_piano");
        let names: Vec<&str> = p["params"]
            .as_array()
            .unwrap()
            .iter()
            .map(|q| q["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["tone", "felt", "release_s"]);
        assert!(p["params"][0]["does"].as_str().unwrap().contains("muffled"));
        let t = prompt_tables();
        assert!(t.contains("`felt_piano`") && t.contains("`tone` 0..1, default 0.5"));
        assert!(t.contains("`reverb`") && t.contains("`decay_s` 0.1..20 s, default 2.2"));
        // Every effect kind of the frozen params table is listed.
        for (_, name, _) in EFFECT_KINDS {
            assert!(t.contains(&format!("| `{name}` |")), "{name}");
        }
    }
}
