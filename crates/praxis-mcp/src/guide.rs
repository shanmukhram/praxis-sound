//! Short texts an agent reads through the server: the server instructions and the plan how-to.
//! The full guide is docs/agents/praxis-skill/SKILL.md.

/// Sent in the `initialize` result.
pub const INSTRUCTIONS: &str = "Praxis Sound makes layered, editable music loops, scores and \
sound effects as files. You cannot listen, so every render returns measurements: read \
`render.check.passed` and `render.check.failed`. Plan-first workflow: (1) praxis_plan_schema for \
the ScorePlan schema, how-to and an example; (2) praxis_presets for preset names that render \
here; (3) write the plan and call praxis_compose with `project` and `render` paths; (4) read the \
checks, fix the plan, compose again; (5) praxis_export for WAV/FLAC/stems/MIDI. praxis_generate \
takes a plain brief instead. Edits: praxis_edit changes only the bars and layers in scope \
(locked layers never); praxis_variations makes alternates to audition. praxis_match makes \
something like a reference clip and returns its similarity per render and an originality check. \
Shared history with the \
person (a sound folder): praxis_ask makes a take (a brief on a new sound, then scoped edits; \
`scope` {layer, bars} holds it to exactly those bars), praxis_takes lists them (watch \
`revision`), praxis_restore goes back, praxis_keep_variation keeps an alternate, \
praxis_open_view shows the person a panel view. Each take says what changed and which bars \
stayed bit-identical. Reference clips: praxis_analyze measures a clip's Sound Profile (class, \
tempo, key, loudness, envelope, spectrum, groove, pitch, reverb, sections); praxis_library saves \
profiles as named, tagged styles (never audio). Paths in, paths and measurements out; audio is \
never returned inline. \
Errors come back with isError=true and `error.code` (io, bad_json, bad_score, invalid_score, \
bad_format, render_failed, no_api_key, provider_failed, pack_missing, pack_verify_failed, \
decode_failed, patch_rejected, error); `error.issues[]` carry a JSON pointer `path` into your \
input and the allowed values.";

/// Returned by `praxis_plan_schema` under `how_to`.
pub const PLAN_HOW_TO: &[&str] = &[
    "Write one ScorePlan JSON object and pass it as `plan` to praxis_compose. No API key is needed.",
    "Timing: give `bars` plus `duration_s` or `tempo_bpm` (any two fix the third; 16 bars in 30 s = 128 BPM). Tempo 40-220 BPM.",
    "One `emotion` word. `key` is {tonic, mode}, e.g. {\"tonic\": \"A\", \"mode\": \"minor\"}. `playback`: \"loop\" (seamless) or \"one_shot\".",
    "One to three layers (at most 5). Each layer: id, class, preset {kind, name} from praxis_presets, dynamics, and a motif.",
    "Rhythm first: motif.spoken is a spoken rhythm on motif.grid (a grid that divides the bar): DON strong, don normal, din soft, TIN bright, - rest, ~ tie.",
    "Notes: motif.degrees are scale degrees (0 = tonic, 7 = an octave up, negative goes down), cycled over the hits. Never MIDI pitches; the planner keeps them in key.",
    "Notes connect: sustain_steps above 1.0 (1.6 is a good default) on piano, keys and mallets.",
    "Layering: sections[] cover every bar once, in order; each lists the layer ids that play. Later sections add layers; never rewrite an earlier part.",
    "Restraint: no riser, sweep, impact, boom, pad, whoosh or fill unless the user asked; then name the class in `requested`.",
    "praxis_compose repairs what it can and lists each change in `repairs`. If it fails with invalid_score, each issue's `path` is a JSON pointer into your plan and its message lists the allowed values: fix those fields and call again.",
    "Pass `render` (a .wav path) to get measurements back. render.check.failed names failing checks (tempo, key, grid, loudness, true_peak, clipping, loop_seam, silence).",
];

/// A complete, valid ScorePlan (acceptance test 1: a 16-bar, 30 s A-minor felt-piano loop with
/// a soft kick entering at bar 9).
pub const EXAMPLE_PLAN: &str =
    include_str!("../../praxis-planner/tests/prompts/acceptance1.plan.json");
