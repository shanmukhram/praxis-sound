//! The ScorePlan: the compact, schema-constrained description a provider fills (ADR-0006), and
//! its validation. Issues are written so they can be sent back to a model verbatim.
//!
//! See the "Plan schema" section of docs/contracts/planner.md.

use crate::catalogue::{Catalogue, PresetSpec, class_name, kind_name};
use praxis_score::timeline::duration;
use praxis_score::validate::{unpitched_class, valid_id};
use praxis_score::{
    Accent, BarRange, GeneratorClass, Grid, InstrumentKind, Key, Mode, PitchClass, SectionRole,
    TimeSignature,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Name of the tool the provider must call.
pub const TOOL_NAME: &str = "plan_score";

/// Most layers a plan may have (restraint rule: few elements).
pub const MAX_LAYERS: usize = 5;

/// Practical tempo range the planner accepts (the Score allows 20..=400).
pub const MIN_BPM: f64 = 40.0;
pub const MAX_BPM: f64 = 220.0;

/// Most bars a plan may have.
pub const MAX_BARS: u32 = 512;

/// Trailer and filler classes that are switched off unless the brief explicitly asks for them.
pub const TRAILER_CLASSES: [GeneratorClass; 7] = [
    GeneratorClass::Riser,
    GeneratorClass::Sweep,
    GeneratorClass::Impact,
    GeneratorClass::Boom,
    GeneratorClass::Pad,
    GeneratorClass::Whoosh,
    GeneratorClass::Fill,
];

/// A compact plan of a Score. The arranger expands it deterministically.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScorePlan {
    /// Short title for the piece.
    #[serde(default)]
    pub title: String,
    /// Exactly one lower-case emotion word, e.g. "calm", "melancholy", "mysterious". Mood words
    /// describe feeling only; they never add instruments or effects.
    pub emotion: String,
    /// Key and scale. Every pitched note will be in this key.
    pub key: Key,
    /// Time signature; 4/4 unless the brief says otherwise.
    #[serde(default = "four_four")]
    pub time_signature: TimeSignature,
    /// Tempo in quarter-note BPM, when the brief gives one.
    #[serde(default)]
    pub tempo_bpm: Option<f64>,
    /// Target length in seconds, when the brief gives one. With `bars` it fixes the tempo; with
    /// `tempo_bpm` it fixes the bar count.
    #[serde(default)]
    pub duration_s: Option<f64>,
    /// Total bar count, when the brief gives one.
    #[serde(default)]
    pub bars: Option<u32>,
    /// Seamless loop or one-shot.
    #[serde(default)]
    pub playback: PlanPlayback,
    /// Generator classes the brief bans ("no risers, no pads"). Risers, sweeps, impacts, booms,
    /// pads, whooshes and fills are banned anyway unless the brief explicitly asks for them.
    #[serde(default)]
    pub avoid: Vec<GeneratorClass>,
    /// Sections in bar order, covering every bar once. Each lists the layer ids that play in it;
    /// a later section adds or removes whole layers and never rewrites an existing part.
    #[serde(default)]
    pub sections: Vec<PlanSection>,
    /// The layers (few: one emotion, few elements). Each has one motif that repeats.
    pub layers: Vec<PlanLayer>,
    /// Chord progression as symbols over bar ranges, e.g. "Am" bars 1-1, "F" bars 2-2. Chord roots
    /// must be in the key. Bars without a chord use the tonic chord.
    #[serde(default)]
    pub chords: Vec<PlanChord>,
    /// Remarks for the user: requested instruments that are not in the catalogue, assumptions.
    #[serde(default)]
    pub notes: Vec<String>,
    /// Trailer classes (riser, sweep, impact, boom, pad, whoosh, fill) the user explicitly asked
    /// for. Leave empty unless the request literally names one: "cinematic", "epic" or
    /// "mysterious" are moods, not requests. When planning from a brief the planner derives this
    /// from the brief's words and ignores the field.
    #[serde(default)]
    pub requested: Vec<GeneratorClass>,
}

fn four_four() -> TimeSignature {
    TimeSignature {
        numerator: 4,
        denominator: 4,
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanPlayback {
    /// Seamless loop: the last bar leads into the first, tails wrap.
    Loop,
    /// Plays once and rings out.
    #[default]
    OneShot,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanSection {
    /// Section name, e.g. "Piano alone".
    pub name: String,
    /// Inclusive 1-based bar range.
    pub bars: BarRange,
    pub role: SectionRole,
    /// 0..1.
    #[serde(default = "half")]
    pub intensity: f32,
    /// Ids of the layers that play in this section.
    pub layers: Vec<String>,
}

fn half() -> f32 {
    0.5
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanLayer {
    /// Stable id: lower-case letters, digits, `_` or `-`.
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Musical role. Must agree with the preset's class for drums, effects and pads.
    pub class: GeneratorClass,
    /// A preset from the catalogue.
    pub preset: PresetChoice,
    /// Register: low, mid or high. Used when `motif.octave` is not given.
    #[serde(default)]
    pub register: Register,
    /// Overall dynamics of the layer.
    #[serde(default)]
    pub dynamics: Dynamics,
    /// Layer gain in dB (-60..=24). Omit for the arranger's default balance.
    #[serde(default)]
    pub gain_db: Option<f32>,
    pub motif: Motif,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PresetChoice {
    pub kind: InstrumentKind,
    /// Preset name from the catalogue, e.g. "felt_piano", "soft_kick".
    pub name: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Register {
    Low,
    #[default]
    Mid,
    High,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Dynamics {
    Soft,
    #[default]
    Normal,
    Strong,
}

/// A rhythm-first motif: a spoken rhythm on a step grid, with scale degrees cycled over its hits.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Motif {
    /// Step grid: quarter, eighth, sixteenth, eighth_triplet or sixteenth_triplet.
    pub grid: Grid,
    /// Motif length in whole bars (1..=8). The motif repeats for as long as the layer plays.
    #[serde(default = "one_bar")]
    pub length_bars: u32,
    /// Spoken rhythm, one token per grid step: DON strong, din soft, don normal, TIN bright,
    /// "-" rest, "~" tie. Shorter than the motif: it repeats from the start of every bar.
    pub spoken: String,
    /// Scale degrees (0 = tonic, 7 = tonic an octave up, negative below) cycled over the hits.
    /// With `follow_chords` they are relative to each bar's chord root.
    #[serde(default)]
    pub degrees: Vec<i32>,
    /// Octave of degree 0 (MIDI numbering, C4 = 60 is octave 4). Omit to use the register.
    #[serde(default)]
    pub octave: Option<i32>,
    /// Length of each hit in steps. Above 1.0 the note rings into the next (sustaining
    /// instruments must overlap). Omit for the instrument's default.
    #[serde(default)]
    pub sustain_steps: Option<f32>,
    /// Degrees follow the chord progression (bass, chords, arps).
    #[serde(default)]
    pub follow_chords: bool,
    /// Optional explicit notes. When present they replace the notes built from `spoken` and
    /// `degrees`. Steps are whole grid steps within the motif.
    #[serde(default)]
    pub notes: Vec<PlanNote>,
}

fn one_bar() -> u32 {
    1
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanNote {
    /// 0-based whole step within the motif.
    pub step: f64,
    /// MIDI pitch (60 = C4). Must be in the key for pitched layers.
    pub pitch: i32,
    #[serde(default)]
    pub accent: Accent,
    /// 0 < velocity <= 1; omit for the accent's default.
    #[serde(default)]
    pub velocity: Option<f32>,
    /// Length in steps; omit for the motif's sustain.
    #[serde(default)]
    pub length: Option<f32>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanChord {
    pub bars: BarRange,
    /// Chord symbol: root, optional quality and bass, e.g. "Am", "Fmaj7", "C/E".
    pub symbol: String,
}

/// JSON Schema of [`ScorePlan`] (the tool's input schema).
pub fn plan_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(ScorePlan)).unwrap_or(serde_json::Value::Null)
}

// ---------------------------------------------------------------------------------------------
// Issues

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PlanIssueCode {
    /// The tool input does not match the plan schema.
    Schema,
    Timing,
    TimeSignature,
    Bars,
    Emotion,
    BadId,
    DuplicateId,
    NoLayers,
    TooManyLayers,
    UnknownPreset,
    PresetKind,
    ClassMismatch,
    AvoidedClass,
    UnrequestedClass,
    Grid,
    MotifLength,
    Spoken,
    Degrees,
    Octave,
    Sustain,
    OffScale,
    OffGrid,
    NoteRange,
    Velocity,
    Gain,
    Sections,
    UnknownLayer,
    InactiveLayer,
    Intensity,
    Chord,
    // Quality issues (model providers only, `brief::quality_issues`): the plan is valid, but the
    // brief or the arranged Score's lints say it breaks the owner's taste. They go back to the
    // model in a repair round; nothing rejects a plan for them.
    /// A layer plays an instrument the brief does not name (restraint).
    UnrequestedLayer,
    /// A layer enters late or leaves early although the brief never says when.
    LayerBars,
    /// Lint `isolated_notes` on the arranged Score.
    IsolatedNotes,
    /// Lint `no_motif_repetition`.
    NoMotifRepetition,
    /// Lint `missing_sustain_overlap`.
    MissingSustainOverlap,
    /// Lint `too_many_elements`.
    TooManyElements,
    /// Lint `unrequested_effect_class`.
    UnrequestedEffectClass,
}

impl PlanIssueCode {
    /// Quality issues are advice for a model: a plan with only these is valid.
    pub fn is_quality(self) -> bool {
        use PlanIssueCode::*;
        matches!(
            self,
            UnrequestedLayer
                | LayerBars
                | IsolatedNotes
                | NoMotifRepetition
                | MissingSustainOverlap
                | TooManyElements
                | UnrequestedEffectClass
        )
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PlanIssue {
    pub code: PlanIssueCode,
    /// JSON pointer into the plan, e.g. "/layers/0/motif/spoken".
    pub path: String,
    pub message: String,
}

impl PlanIssue {
    pub fn new(code: PlanIssueCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        PlanIssue {
            code,
            path: path.into(),
            message: message.into(),
        }
    }
}

/// Renders issues as a bullet list for a repair request.
pub fn issues_text(issues: &[PlanIssue]) -> String {
    issues
        .iter()
        .map(|i| format!("- {} ({}): {}", i.path, code_name(i.code), i.message))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn code_name(code: PlanIssueCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// What validation needs besides the plan.
#[derive(Clone, Copy, Debug)]
pub struct PlanContext<'a> {
    pub catalogue: &'a Catalogue,
    /// Trailer classes the brief explicitly asked for (see `brief::requested_classes`).
    pub requested: &'a BTreeSet<GeneratorClass>,
}

/// The avoid-list the Score gets: the plan's bans plus every trailer class not requested.
pub fn effective_avoid(
    plan: &ScorePlan,
    requested: &BTreeSet<GeneratorClass>,
) -> BTreeSet<GeneratorClass> {
    let mut avoid: BTreeSet<GeneratorClass> = plan.avoid.iter().copied().collect();
    for c in TRAILER_CLASSES {
        if !requested.contains(&c) {
            avoid.insert(c);
        }
    }
    avoid
}

/// Classes a preset imposes on its layer: a layer cannot relabel a drum, effect, pad or texture
/// preset as something else (a pad cannot hide in a "melody" layer).
pub fn class_is_intrinsic(class: GeneratorClass) -> bool {
    use GeneratorClass as C;
    !matches!(
        class,
        C::Melody | C::Motif | C::Harmony | C::Chords | C::Arp | C::Bass
    )
}

/// The class the Score layer will carry.
pub fn effective_class(layer: &PlanLayer, preset: Option<&PresetSpec>) -> GeneratorClass {
    match preset {
        Some(p) if class_is_intrinsic(p.class) || unpitched_class(layer.class) => p.class,
        _ => layer.class,
    }
}

/// Resolved tempo and length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub bpm: f64,
    pub bars: u32,
}

/// Tempo and bars from the plan, or a message saying what is missing or inconsistent.
pub fn resolve_timing(plan: &ScorePlan) -> Result<Timing, String> {
    let ts = plan.time_signature;
    if ts.numerator == 0 || ![2u8, 4, 8, 16].contains(&ts.denominator) {
        return Err("time signature is invalid".into());
    }
    let pos = |v: Option<f64>| v.filter(|x| x.is_finite() && *x > 0.0);
    let (tempo, secs, bars) = (pos(plan.tempo_bpm), pos(plan.duration_s), plan.bars);
    if plan.tempo_bpm.is_some() && tempo.is_none() {
        return Err("tempo_bpm must be a positive number".into());
    }
    if plan.duration_s.is_some() && secs.is_none() {
        return Err("duration_s must be a positive number".into());
    }
    if bars == Some(0) {
        return Err("bars must be at least 1".into());
    }
    let t = match (tempo, secs, bars) {
        (Some(bpm), Some(s), Some(b)) => {
            let need = duration::solve_tempo(b, ts, s);
            if (need - bpm).abs() > 0.5 {
                return Err(format!(
                    "tempo_bpm {bpm} does not give {b} bars in {s} s ({b} bars in {s} s needs \
                     {need:.2} BPM); make tempo_bpm, duration_s and bars agree or drop one"
                ));
            }
            Timing { bpm, bars: b }
        }
        (_, Some(s), Some(b)) => Timing {
            bpm: duration::solve_tempo(b, ts, s),
            bars: b,
        },
        (Some(bpm), Some(s), None) => Timing {
            bpm,
            bars: duration::solve_bars(bpm, ts, s).0,
        },
        (Some(bpm), None, Some(b)) => Timing { bpm, bars: b },
        _ => {
            return Err(
                "give bars plus tempo_bpm or duration_s, or tempo_bpm plus duration_s".into(),
            );
        }
    };
    if !(MIN_BPM..=MAX_BPM).contains(&t.bpm) {
        return Err(format!(
            "the tempo works out at {:.1} BPM, outside {MIN_BPM}-{MAX_BPM}; change bars or \
             duration_s",
            t.bpm
        ));
    }
    if t.bars > MAX_BARS {
        return Err(format!("{} bars is more than {MAX_BARS}", t.bars));
    }
    Ok(t)
}

/// A parsed chord symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChordSymbol {
    pub root: PitchClass,
    pub bass: Option<PitchClass>,
}

/// Parses "Am", "F#m7", "Bbmaj7", "C/E", "Dsus4".
pub fn parse_chord(symbol: &str) -> Option<ChordSymbol> {
    let s = symbol.trim();
    let (main, bass) = match s.split_once('/') {
        Some((m, b)) => (m, Some(b)),
        None => (s, None),
    };
    let mut chars = main.chars();
    let letter = chars.next()?;
    if !letter.is_ascii_uppercase() {
        return None;
    }
    let rest: String = chars.collect();
    let (acc, quality) = match rest.chars().next() {
        Some(c @ ('#' | 'b' | '♯' | '♭')) => (c.to_string(), &rest[c.len_utf8()..]),
        _ => (String::new(), rest.as_str()),
    };
    let root = PitchClass::parse(&format!("{letter}{acc}"))?;
    const QUALITIES: [&str; 18] = [
        "", "m", "min", "maj", "maj7", "m7", "min7", "7", "dim", "dim7", "aug", "sus2", "sus4",
        "6", "m6", "9", "add9", "m9",
    ];
    if !QUALITIES.contains(&quality) {
        return None;
    }
    let bass = match bass {
        Some(b) => Some(PitchClass::parse(b)?),
        None => None,
    };
    Some(ChordSymbol { root, bass })
}

/// Index of `pc` in the key's scale.
pub fn scale_degree(key: &Key, pc: PitchClass) -> Option<i32> {
    key.scale().iter().position(|p| *p == pc).map(|i| i as i32)
}

/// Diatonic triad symbol on `degree` of the key, e.g. A minor degree 5 -> "F".
pub fn diatonic_symbol(key: &Key, degree: i32) -> String {
    let scale = key.scale();
    let n = scale.len() as i32;
    let root = scale[degree.rem_euclid(n) as usize];
    if key.mode == Mode::Chromatic || n < 7 {
        return root.name().to_string();
    }
    let third = scale[(degree + 2).rem_euclid(n) as usize];
    let fifth = scale[(degree + 4).rem_euclid(n) as usize];
    let iv = |p: PitchClass| (p.semitone() as i32 - root.semitone() as i32).rem_euclid(12);
    let q = match (iv(third), iv(fifth)) {
        (3, 6) => "dim",
        (4, 8) => "aug",
        (3, _) => "m",
        _ => "",
    };
    format!("{}{q}", root.name())
}

// ---------------------------------------------------------------------------------------------
// Validation

/// Comma-separated preset names of the catalogue.
pub fn preset_names(catalogue: &Catalogue) -> String {
    catalogue
        .presets
        .iter()
        .map(|p| p.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Pitch-class names of the key's scale, e.g. "A, B, C, D, E, F, G".
pub fn scale_names(key: &Key) -> String {
    key.scale()
        .iter()
        .map(|p| p.name())
        .collect::<Vec<_>>()
        .join(", ")
}

/// snake_case grid name, as in the plan JSON.
pub fn grid_name(grid: Grid) -> String {
    serde_json::to_value(grid)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Grids that divide a bar of `ts`, comma-separated.
pub fn allowed_grids(ts: TimeSignature) -> String {
    ALL_GRIDS
        .iter()
        .filter(|g| ts.steps_per_bar(**g).is_some())
        .map(|g| grid_name(*g))
        .collect::<Vec<_>>()
        .join(", ")
}

pub const ALL_GRIDS: [Grid; 5] = [
    Grid::Quarter,
    Grid::Eighth,
    Grid::Sixteenth,
    Grid::EighthTriplet,
    Grid::SixteenthTriplet,
];

fn add(out: &mut Vec<PlanIssue>, code: PlanIssueCode, path: String, msg: String) {
    out.push(PlanIssue::new(code, path, msg));
}

/// Validates a plan. An empty list means the arranger can expand it.
pub fn validate_plan(plan: &ScorePlan, ctx: &PlanContext) -> Vec<PlanIssue> {
    use PlanIssueCode as K;
    let mut out = Vec::new();

    let ts = plan.time_signature;
    if ts.numerator == 0 || ts.numerator > 32 || ![2u8, 4, 8, 16].contains(&ts.denominator) {
        add(
            &mut out,
            K::TimeSignature,
            "/time_signature".into(),
            "numerator 1..=32 and denominator 2, 4, 8 or 16".into(),
        );
    }
    let timing = resolve_timing(plan);
    let bars = match &timing {
        Ok(t) => t.bars,
        Err(m) => {
            add(&mut out, K::Timing, "/tempo_bpm".into(), m.clone());
            plan.bars.unwrap_or(1).clamp(1, MAX_BARS)
        }
    };
    let emotion = plan.emotion.trim();
    if emotion.is_empty()
        || emotion.split_whitespace().count() != 1
        || emotion.chars().any(|c| !c.is_alphabetic() && c != '-')
    {
        add(
            &mut out,
            K::Emotion,
            "/emotion".into(),
            format!(
                "emotion must be exactly one word (restraint: one emotion per piece), got `{}`",
                plan.emotion
            ),
        );
    }

    let avoid = effective_avoid(plan, ctx.requested);
    if plan.layers.is_empty() {
        add(
            &mut out,
            K::NoLayers,
            "/layers".into(),
            "a plan needs at least one layer".into(),
        );
    }
    if plan.layers.len() > MAX_LAYERS {
        add(
            &mut out,
            K::TooManyLayers,
            "/layers".into(),
            format!(
                "{} layers is too many; keep at most {MAX_LAYERS} (one emotion, few elements)",
                plan.layers.len()
            ),
        );
    }

    let mut ids = BTreeSet::new();
    for (i, layer) in plan.layers.iter().enumerate() {
        let lp = format!("/layers/{i}");
        if !valid_id(&layer.id) {
            add(
                &mut out,
                K::BadId,
                format!("{lp}/id"),
                format!(
                    "bad layer id `{}`: use 1-64 of [a-z0-9_-], starting with a letter or digit",
                    layer.id
                ),
            );
        }
        if !ids.insert(layer.id.as_str()) {
            add(
                &mut out,
                K::DuplicateId,
                format!("{lp}/id"),
                format!("duplicate layer id `{}`", layer.id),
            );
        }
        let preset = ctx.catalogue.get(&layer.preset.name);
        match preset {
            None => add(
                &mut out,
                K::UnknownPreset,
                format!("{lp}/preset/name"),
                format!(
                    "unknown preset `{}`; allowed presets: {}",
                    layer.preset.name,
                    preset_names(ctx.catalogue)
                ),
            ),
            Some(p) if p.kind != layer.preset.kind => add(
                &mut out,
                K::PresetKind,
                format!("{lp}/preset/kind"),
                format!(
                    "preset `{}` is a {} preset, not {}",
                    p.name,
                    kind_name(p.kind),
                    kind_name(layer.preset.kind)
                ),
            ),
            Some(_) => {}
        }
        if let Some(p) = preset
            && layer.class != p.class
            && (class_is_intrinsic(p.class) || unpitched_class(layer.class))
        {
            add(
                &mut out,
                K::ClassMismatch,
                format!("{lp}/class"),
                format!(
                    "preset `{}` is intrinsically {}; a layer using it cannot be {} (set class \
                     to `{}`)",
                    p.name,
                    class_name(p.class),
                    class_name(layer.class),
                    class_name(p.class)
                ),
            );
        }
        let class = effective_class(layer, preset);
        if avoid.contains(&class) || preset.is_some_and(|p| avoid.contains(&p.class)) {
            let requested_trailer =
                TRAILER_CLASSES.contains(&class) && !plan.avoid.contains(&class);
            add(
                &mut out,
                if requested_trailer {
                    K::UnrequestedClass
                } else {
                    K::AvoidedClass
                },
                format!("{lp}/class"),
                if requested_trailer {
                    format!(
                        "layer `{}` is a {} but the brief did not ask for one; mood words such \
                         as \"cinematic\" are not a request for trailer effects. Remove the layer",
                        layer.id,
                        class_name(class)
                    )
                } else {
                    format!(
                        "layer `{}` is a {}, which the brief bans; remove the layer",
                        layer.id,
                        class_name(class)
                    )
                },
            );
        }
        if let Some(g) = layer.gain_db
            && !(g.is_finite() && (-60.0..=24.0).contains(&g))
        {
            add(
                &mut out,
                K::Gain,
                format!("{lp}/gain_db"),
                "gain_db -60..=24".into(),
            );
        }
        validate_motif(
            &mut out,
            plan,
            layer,
            preset,
            &format!("{lp}/motif"),
            bars,
            ts,
        );
    }

    validate_sections(&mut out, plan, bars);
    validate_chords(&mut out, plan, bars);
    out
}

fn validate_motif(
    out: &mut Vec<PlanIssue>,
    plan: &ScorePlan,
    layer: &PlanLayer,
    preset: Option<&PresetSpec>,
    mp: &str,
    bars: u32,
    ts: TimeSignature,
) {
    use PlanIssueCode as K;
    let mut push = |code, path: String, msg: String| out.push(PlanIssue::new(code, path, msg));
    let m = &layer.motif;
    let spb = ts.steps_per_bar(m.grid);
    if spb.is_none() {
        push(
            K::Grid,
            format!("{mp}/grid"),
            format!(
                "a {} grid does not divide a {}/{} bar; allowed grids here: {}",
                grid_name(m.grid),
                ts.numerator,
                ts.denominator,
                allowed_grids(ts)
            ),
        );
    }
    if m.length_bars == 0 || m.length_bars > 8 || m.length_bars > bars.max(1) {
        push(
            K::MotifLength,
            format!("{mp}/length_bars"),
            format!(
                "length_bars must be 1..=8 and at most the piece's {bars} bars, got {}",
                m.length_bars
            ),
        );
    }
    let len_steps = spb.map(|s| s * m.length_bars.max(1));
    let pitched = preset.map(|p| p.pitched).unwrap_or(true) && !unpitched_class(layer.class);
    if m.notes.is_empty() {
        match praxis_score::rhythm::parse_spoken(&m.spoken) {
            Err(e) => push(
                K::Spoken,
                format!("{mp}/spoken"),
                format!("{e}; use DON, din, don, TIN, - (rest) and ~ (tie) separated by spaces"),
            ),
            Ok(steps) => {
                let hits = steps
                    .iter()
                    .filter(|s| matches!(s, praxis_score::rhythm::SpokenStep::Hit { .. }))
                    .count();
                if hits == 0 {
                    push(
                        K::Spoken,
                        format!("{mp}/spoken"),
                        "the spoken rhythm has no hits; rhythm first: give a repeating motif"
                            .into(),
                    );
                }
                if let Some(len) = len_steps
                    && steps.len() as u32 > len
                {
                    push(
                        K::Spoken,
                        format!("{mp}/spoken"),
                        format!(
                            "the spoken rhythm has {} steps but the motif has only {len} \
                             ({} bar(s) of {:?}); lengthen length_bars or shorten the rhythm",
                            steps.len(),
                            m.length_bars,
                            m.grid
                        ),
                    );
                }
            }
        }
        if pitched && m.degrees.is_empty() {
            push(
                K::Degrees,
                format!("{mp}/degrees"),
                "a pitched motif needs at least one scale degree".into(),
            );
        }
    }
    for (j, d) in m.degrees.iter().enumerate() {
        if !(-21..=28).contains(d) {
            push(
                K::Degrees,
                format!("{mp}/degrees/{j}"),
                format!("degree {d} is outside -21..=28"),
            );
        }
    }
    if let Some(o) = m.octave
        && !(0..=8).contains(&o)
    {
        push(
            K::Octave,
            format!("{mp}/octave"),
            format!("octave {o} outside 0..=8"),
        );
    }
    if let Some(s) = m.sustain_steps {
        if !(s.is_finite() && s > 0.0 && s <= 64.0) {
            push(
                K::Sustain,
                format!("{mp}/sustain_steps"),
                "sustain_steps must be in (0, 64]".into(),
            );
        } else if preset.is_some_and(|p| p.sustaining) && s <= 1.0 {
            push(
                K::Sustain,
                format!("{mp}/sustain_steps"),
                format!(
                    "`{}` rings: notes must overlap the next step (sustain_steps above 1.0, \
                     e.g. 1.6), got {s}",
                    layer.preset.name
                ),
            );
        }
    }
    let key = plan.key;
    for (j, n) in m.notes.iter().enumerate() {
        let np = format!("{mp}/notes/{j}");
        if !n.step.is_finite() || n.step < 0.0 || n.step.fract() != 0.0 {
            push(
                K::OffGrid,
                format!("{np}/step"),
                format!(
                    "step {} is off the grid; use a whole step number (0-based)",
                    n.step
                ),
            );
        } else if let Some(len) = len_steps
            && n.step as u32 >= len
        {
            push(
                K::OffGrid,
                format!("{np}/step"),
                format!("step {} is beyond the motif's {len} steps", n.step),
            );
        }
        if !(0..=127).contains(&n.pitch) {
            push(
                K::NoteRange,
                format!("{np}/pitch"),
                format!("pitch {} outside 0..=127", n.pitch),
            );
        } else {
            if pitched && key.mode != Mode::Chromatic && !key.contains(n.pitch as u8) {
                push(
                    K::OffScale,
                    format!("{np}/pitch"),
                    format!(
                        "{} is not in {}; allowed pitch classes: {}",
                        praxis_score::theory::midi_name(n.pitch as u8),
                        key.display(),
                        scale_names(&key)
                    ),
                );
            }
            if let Some(p) = preset
                && pitched
                && !(p.range.0 as i32..=p.range.1 as i32).contains(&n.pitch)
            {
                push(
                    K::NoteRange,
                    format!("{np}/pitch"),
                    format!(
                        "{} is outside the range of `{}` ({}-{})",
                        n.pitch, p.name, p.range.0, p.range.1
                    ),
                );
            }
        }
        if let Some(v) = n.velocity
            && !(v > 0.0 && v <= 1.0)
        {
            push(
                K::Velocity,
                format!("{np}/velocity"),
                "velocity in (0, 1]".into(),
            );
        }
        if let Some(l) = n.length
            && !(l.is_finite() && l > 0.0 && l <= 64.0)
        {
            push(
                K::Sustain,
                format!("{np}/length"),
                "length in (0, 64] steps".into(),
            );
        }
    }
}

fn validate_sections(out: &mut Vec<PlanIssue>, plan: &ScorePlan, bars: u32) {
    use PlanIssueCode as K;
    let mut push = |code, path: String, msg: String| out.push(PlanIssue::new(code, path, msg));
    if plan.sections.is_empty() {
        push(
            K::Sections,
            "/sections".into(),
            format!("no sections: add at least one section covering bars 1-{bars}"),
        );
        return;
    }
    let ids: BTreeSet<&str> = plan.layers.iter().map(|l| l.id.as_str()).collect();
    let mut covered = vec![0u32; bars as usize + 2];
    let mut active = BTreeSet::new();
    for (i, s) in plan.sections.iter().enumerate() {
        let sp = format!("/sections/{i}");
        if s.bars.start == 0 || s.bars.end < s.bars.start || s.bars.end > bars {
            push(
                K::Sections,
                format!("{sp}/bars"),
                format!(
                    "section bars {}-{} are outside 1-{bars}",
                    s.bars.start, s.bars.end
                ),
            );
        } else {
            for b in s.bars.start..=s.bars.end {
                covered[b as usize] += 1;
            }
        }
        if !(s.intensity.is_finite() && (0.0..=1.0).contains(&s.intensity)) {
            push(
                K::Intensity,
                format!("{sp}/intensity"),
                "intensity 0..=1".into(),
            );
        }
        for (j, l) in s.layers.iter().enumerate() {
            if ids.contains(l.as_str()) {
                active.insert(l.as_str());
            } else {
                push(
                    K::UnknownLayer,
                    format!("{sp}/layers/{j}"),
                    format!("section `{}` names unknown layer `{l}`", s.name),
                );
            }
        }
    }
    let overlapping: Vec<u32> = (1..=bars).filter(|b| covered[*b as usize] > 1).collect();
    if let Some(b) = overlapping.first() {
        push(
            K::Sections,
            "/sections".into(),
            format!("sections overlap at bar {b}; each bar belongs to one section"),
        );
    }
    let gaps: Vec<u32> = (1..=bars).filter(|b| covered[*b as usize] == 0).collect();
    if let (Some(first), Some(last)) = (gaps.first(), gaps.last()) {
        push(
            K::Sections,
            "/sections".into(),
            format!("bars {first}-{last} are not in any section; sections must cover 1-{bars}"),
        );
    }
    for (i, l) in plan.layers.iter().enumerate() {
        if !active.contains(l.id.as_str()) {
            push(
                K::InactiveLayer,
                format!("/layers/{i}"),
                format!("layer `{}` is not active in any section", l.id),
            );
        }
    }
}

fn validate_chords(out: &mut Vec<PlanIssue>, plan: &ScorePlan, bars: u32) {
    use PlanIssueCode as K;
    for (i, c) in plan.chords.iter().enumerate() {
        let cp = format!("/chords/{i}");
        if c.bars.start == 0 || c.bars.end < c.bars.start || c.bars.end > bars {
            out.push(PlanIssue::new(
                K::Chord,
                format!("{cp}/bars"),
                format!(
                    "chord bars {}-{} are outside 1-{bars}",
                    c.bars.start, c.bars.end
                ),
            ));
        }
        match parse_chord(&c.symbol) {
            None => out.push(PlanIssue::new(
                K::Chord,
                format!("{cp}/symbol"),
                format!("cannot read chord symbol `{}`", c.symbol),
            )),
            Some(sym) if plan.key.mode != Mode::Chromatic => {
                for pc in [Some(sym.root), sym.bass].into_iter().flatten() {
                    if scale_degree(&plan.key, pc).is_none() {
                        out.push(PlanIssue::new(
                            K::Chord,
                            format!("{cp}/symbol"),
                            format!(
                                "chord `{}` uses {} which is not in {}; chord roots \
                                 allowed: {}",
                                c.symbol,
                                pc.name(),
                                plan.key.display(),
                                scale_names(&plan.key)
                            ),
                        ));
                    }
                }
            }
            Some(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_minor() -> Key {
        Key {
            tonic: PitchClass::A,
            mode: Mode::Minor,
        }
    }

    #[test]
    fn chords_parse_and_diatonic() {
        assert_eq!(parse_chord("Am").map(|c| c.root), Some(PitchClass::A));
        assert_eq!(parse_chord("F#m7").map(|c| c.root), Some(PitchClass::Fs));
        assert_eq!(parse_chord("Bbmaj7").map(|c| c.root), Some(PitchClass::Bb));
        assert_eq!(parse_chord("C/E").and_then(|c| c.bass), Some(PitchClass::E));
        assert!(parse_chord("am").is_none());
        assert!(parse_chord("Hm").is_none());
        assert!(parse_chord("Cxyz").is_none());
        let k = a_minor();
        assert_eq!(diatonic_symbol(&k, 0), "Am");
        assert_eq!(diatonic_symbol(&k, 5), "F");
        assert_eq!(diatonic_symbol(&k, -2), "F");
        assert_eq!(diatonic_symbol(&k, 1), "Bdim");
        assert_eq!(scale_degree(&k, PitchClass::F), Some(5));
    }

    #[test]
    fn schema_is_an_object_with_layers() {
        let s = plan_schema();
        assert_eq!(s["type"], "object");
        assert!(s["properties"]["layers"].is_object());
        assert!(
            s["required"]
                .as_array()
                .is_some_and(|r| r.iter().any(|v| v == "layers"))
        );
    }

    #[test]
    fn timing_rules() {
        let mut p = ScorePlan {
            title: String::new(),
            emotion: "calm".into(),
            key: a_minor(),
            time_signature: four_four(),
            tempo_bpm: None,
            duration_s: Some(30.0),
            bars: Some(16),
            playback: PlanPlayback::Loop,
            avoid: vec![],
            sections: vec![],
            layers: vec![],
            chords: vec![],
            notes: vec![],
            requested: vec![],
        };
        assert_eq!(
            resolve_timing(&p),
            Ok(Timing {
                bpm: 128.0,
                bars: 16
            })
        );
        p.tempo_bpm = Some(120.0);
        assert!(resolve_timing(&p).unwrap_err().contains("128.00"));
        p.tempo_bpm = Some(90.0);
        p.bars = None;
        p.duration_s = Some(60.0);
        assert_eq!(resolve_timing(&p).map(|t| t.bars), Ok(23));
        p.duration_s = None;
        assert!(resolve_timing(&p).is_err());
        p.bars = Some(4);
        p.tempo_bpm = Some(300.0);
        assert!(resolve_timing(&p).unwrap_err().contains("outside"));
    }
}
