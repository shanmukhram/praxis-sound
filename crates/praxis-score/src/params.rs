//! Parameter specifications. FROZEN CONTRACT: the DSP and engine implement exactly these
//! parameters with these units; the validator rejects names and values outside this table.

use crate::types::EffectKind;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamSpec {
    pub name: &'static str,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub unit: &'static str,
    pub doc: &'static str,
}

const fn p(
    name: &'static str,
    min: f64,
    max: f64,
    default: f64,
    unit: &'static str,
    doc: &'static str,
) -> ParamSpec {
    ParamSpec {
        name,
        min,
        max,
        default,
        unit,
        doc,
    }
}

/// Parameters of each effect kind.
pub fn effect_params(kind: EffectKind) -> &'static [ParamSpec] {
    use EffectKind::*;
    match kind {
        // Four-band EQ: low shelf, two peaks, high shelf, plus high-pass and low-pass cuts.
        Eq => {
            const P: &[ParamSpec] = &[
                p(
                    "hp_hz",
                    10.0,
                    2000.0,
                    20.0,
                    "Hz",
                    "12 dB/oct high-pass; 10 = off",
                ),
                p(
                    "lp_hz",
                    1000.0,
                    22000.0,
                    22000.0,
                    "Hz",
                    "12 dB/oct low-pass; 22000 = off",
                ),
                p("low_hz", 30.0, 1000.0, 120.0, "Hz", "low shelf corner"),
                p("low_db", -24.0, 24.0, 0.0, "dB", "low shelf gain"),
                p("mid1_hz", 80.0, 8000.0, 500.0, "Hz", "peak 1 centre"),
                p("mid1_db", -24.0, 24.0, 0.0, "dB", "peak 1 gain"),
                p("mid1_q", 0.1, 10.0, 1.0, "", "peak 1 Q"),
                p("mid2_hz", 200.0, 16000.0, 3000.0, "Hz", "peak 2 centre"),
                p("mid2_db", -24.0, 24.0, 0.0, "dB", "peak 2 gain"),
                p("mid2_q", 0.1, 10.0, 1.0, "", "peak 2 Q"),
                p(
                    "high_hz",
                    1000.0,
                    20000.0,
                    8000.0,
                    "Hz",
                    "high shelf corner",
                ),
                p("high_db", -24.0, 24.0, 0.0, "dB", "high shelf gain"),
            ];
            P
        }
        // State-variable filter. mode: 0 low-pass, 1 high-pass, 2 band-pass, 3 notch.
        Filter => {
            const P: &[ParamSpec] = &[
                p("mode", 0.0, 3.0, 0.0, "enum", "0 lp, 1 hp, 2 bp, 3 notch"),
                p("cutoff_hz", 20.0, 20000.0, 8000.0, "Hz", "cutoff"),
                p(
                    "resonance",
                    0.0,
                    1.0,
                    0.1,
                    "",
                    "0 = Q 0.5, 1 = self-oscillation edge",
                ),
                p("mix", 0.0, 1.0, 1.0, "", "wet fraction"),
            ];
            P
        }
        Compressor => {
            const P: &[ParamSpec] = &[
                p("threshold_db", -60.0, 0.0, -18.0, "dB", "threshold"),
                p("ratio", 1.0, 20.0, 3.0, ":1", "ratio"),
                p("attack_ms", 0.1, 200.0, 10.0, "ms", "attack"),
                p("release_ms", 5.0, 2000.0, 150.0, "ms", "release"),
                p("knee_db", 0.0, 24.0, 6.0, "dB", "soft knee width"),
                p("makeup_db", -12.0, 24.0, 0.0, "dB", "make-up gain"),
                p("mix", 0.0, 1.0, 1.0, "", "parallel mix"),
            ];
            P
        }
        Saturation => {
            const P: &[ParamSpec] = &[
                p("drive_db", 0.0, 36.0, 6.0, "dB", "input drive"),
                p("tone", -1.0, 1.0, 0.0, "", "post tilt: -1 dark, +1 bright"),
                p("mix", 0.0, 1.0, 1.0, "", "wet fraction"),
                p("output_db", -24.0, 12.0, 0.0, "dB", "output trim"),
            ];
            P
        }
        // Tempo-synced when time_beats > 0, otherwise time_ms.
        Delay => {
            const P: &[ParamSpec] = &[
                p(
                    "time_beats",
                    0.0,
                    4.0,
                    0.75,
                    "beats",
                    "delay in quarter beats; 0 = use time_ms",
                ),
                p("time_ms", 1.0, 2000.0, 350.0, "ms", "delay when not synced"),
                p("feedback", 0.0, 0.95, 0.35, "", "feedback"),
                p("ping_pong", 0.0, 1.0, 0.0, "bool", "alternate L/R"),
                p("damp_hz", 500.0, 20000.0, 6000.0, "Hz", "feedback low-pass"),
                p("mix", 0.0, 1.0, 0.2, "", "wet fraction"),
            ];
            P
        }
        // Algorithmic reverb (FDN).
        Reverb => {
            const P: &[ParamSpec] = &[
                p("decay_s", 0.1, 20.0, 2.2, "s", "RT60"),
                p("predelay_ms", 0.0, 200.0, 15.0, "ms", "pre-delay"),
                p("size", 0.0, 1.0, 0.6, "", "room size"),
                p("damp_hz", 500.0, 20000.0, 7000.0, "Hz", "high damping"),
                p("width", 0.0, 1.0, 1.0, "", "stereo width of the wet signal"),
                p("mix", 0.0, 1.0, 0.18, "", "wet fraction"),
            ];
            P
        }
        ConvolutionReverb => {
            const P: &[ParamSpec] = &[
                p("predelay_ms", 0.0, 200.0, 0.0, "ms", "pre-delay"),
                p(
                    "decay_scale",
                    0.1,
                    2.0,
                    1.0,
                    "",
                    "IR time-stretch of the envelope",
                ),
                p("hp_hz", 20.0, 1000.0, 120.0, "Hz", "wet high-pass"),
                p("mix", 0.0, 1.0, 0.18, "", "wet fraction"),
            ];
            P
        }
        Chorus => {
            const P: &[ParamSpec] = &[
                p("rate_hz", 0.01, 8.0, 0.4, "Hz", "LFO rate"),
                p("depth_ms", 0.0, 20.0, 3.0, "ms", "modulation depth"),
                p("delay_ms", 1.0, 40.0, 12.0, "ms", "base delay"),
                p("mix", 0.0, 1.0, 0.3, "", "wet fraction"),
            ];
            P
        }
        // Mid/side width. 0 = mono, 1 = unchanged, 2 = double side.
        Width => {
            const P: &[ParamSpec] = &[
                p("width", 0.0, 2.0, 1.0, "", "side gain"),
                p(
                    "bass_mono_hz",
                    0.0,
                    500.0,
                    0.0,
                    "Hz",
                    "mono below this frequency; 0 = off",
                ),
            ];
            P
        }
        // True-peak limiter with lookahead.
        Limiter => {
            const P: &[ParamSpec] = &[
                p(
                    "ceiling_dbtp",
                    -12.0,
                    0.0,
                    -1.0,
                    "dBTP",
                    "true-peak ceiling",
                ),
                p("release_ms", 1.0, 1000.0, 80.0, "ms", "release"),
                p(
                    "lookahead_ms",
                    0.5,
                    10.0,
                    3.0,
                    "ms",
                    "lookahead (adds latency)",
                ),
            ];
            P
        }
        Gain => {
            const P: &[ParamSpec] = &[p("gain_db", -60.0, 24.0, 0.0, "dB", "gain")];
            P
        }
    }
}

/// Parameters every layer has, addressable by automation as bare names.
pub const LAYER_PARAMS: &[ParamSpec] = &[
    p("gain_db", -60.0, 24.0, 0.0, "dB", "layer gain"),
    p("pan", -1.0, 1.0, 0.0, "", "pan"),
];

pub fn effect_param(kind: EffectKind, name: &str) -> Option<&'static ParamSpec> {
    effect_params(kind).iter().find(|s| s.name == name)
}

pub fn effect_param_value(
    kind: EffectKind,
    params: &std::collections::BTreeMap<String, f64>,
    name: &str,
) -> f64 {
    params
        .get(name)
        .copied()
        .or_else(|| effect_param(kind, name).map(|s| s.default))
        .unwrap_or(0.0)
}
