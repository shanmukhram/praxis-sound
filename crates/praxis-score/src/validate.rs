//! Structural and musical validation. Issue codes are a FROZEN CONTRACT (the planner's repair
//! loop and the CLI report them); new codes may be added, existing ones never change meaning.

use crate::params;
use crate::types::*;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum IssueCode {
    SchemaVersion,
    BadId,
    DuplicateId,
    TempoRange,
    TimeSignature,
    BarsRange,
    BarRange,
    UnknownPattern,
    UnknownLayer,
    GridMismatch,
    StepOutOfRange,
    VelocityRange,
    LengthRange,
    OffsetRange,
    OffScale,
    PitchRange,
    AvoidedClass,
    OverlappingClips,
    UnknownParam,
    ParamRange,
    PanRange,
    AutomationTarget,
    AutomationPoints,
    LoudnessRange,
    MarkerRange,
    SectionOverlap,
    SidechainSource,
    /// Quality lints (warnings): rhythm, sustain, restraint. Implemented in `lint`.
    IsolatedNotes,
    NoMotifRepetition,
    MissingSustainOverlap,
    TooManyElements,
    UnrequestedEffectClass,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Issue {
    pub code: IssueCode,
    /// JSON pointer into the score, e.g. "/patterns/0/notes/3/pitch".
    pub path: String,
    pub message: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Report {
    pub errors: Vec<Issue>,
    pub warnings: Vec<Issue>,
}

impl Report {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
    fn err(&mut self, code: IssueCode, path: impl Into<String>, message: impl Into<String>) {
        self.errors.push(Issue {
            code,
            path: path.into(),
            message: message.into(),
        });
    }
    pub fn warn(&mut self, code: IssueCode, path: impl Into<String>, message: impl Into<String>) {
        self.warnings.push(Issue {
            code,
            path: path.into(),
            message: message.into(),
        });
    }
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        && id.as_bytes()[0].is_ascii_alphanumeric()
}

/// Layer classes that never carry pitch, so the in-key check does not apply.
pub fn unpitched_class(class: GeneratorClass) -> bool {
    matches!(
        class,
        GeneratorClass::Drums | GeneratorClass::Percussion | GeneratorClass::Tick
    )
}

/// Validates a score. Preset-level checks (does the preset exist, its intrinsic class, pitch
/// range, parameter names) need the instrument registry and live in `praxis-instruments`.
pub fn validate(score: &Score) -> Report {
    let mut r = Report::default();
    if score.schema_version != SCHEMA_VERSION {
        r.err(
            IssueCode::SchemaVersion,
            "/schema_version",
            format!("expected {SCHEMA_VERSION}, got {}", score.schema_version),
        );
    }
    if !(20.0..=400.0).contains(&score.tempo.bpm) || !score.tempo.bpm.is_finite() {
        r.err(
            IssueCode::TempoRange,
            "/tempo/bpm",
            "tempo must be 20..=400 BPM",
        );
    }
    let ts = score.time_signature;
    if ts.numerator == 0 || ts.numerator > 32 || ![2u8, 4, 8, 16].contains(&ts.denominator) {
        r.err(
            IssueCode::TimeSignature,
            "/time_signature",
            "numerator 1..=32, denominator 2, 4, 8 or 16",
        );
    }
    if score.bars == 0 || score.bars > 2048 {
        r.err(IssueCode::BarsRange, "/bars", "bars must be 1..=2048");
    }
    let check_range = |r: &mut Report, br: &BarRange, path: String| {
        if br.start == 0 || br.end < br.start || br.end > score.bars {
            r.err(
                IssueCode::BarRange,
                path,
                format!("bar range {}-{} outside 1-{}", br.start, br.end, score.bars),
            );
        }
    };

    // Sections
    let mut ids = BTreeSet::new();
    for (i, s) in score.sections.iter().enumerate() {
        let path = format!("/sections/{i}");
        id_check(&mut r, &mut ids, &s.id, &format!("{path}/id"));
        check_range(&mut r, &s.bars, format!("{path}/bars"));
    }
    let mut secs: Vec<&Section> = score.sections.iter().collect();
    secs.sort_by_key(|s| s.bars.start);
    for w in secs.windows(2) {
        if w[1].bars.start <= w[0].bars.end {
            r.err(
                IssueCode::SectionOverlap,
                "/sections",
                format!("sections `{}` and `{}` overlap", w[0].id, w[1].id),
            );
        }
    }

    // Patterns
    let mut pids = BTreeSet::new();
    for (i, p) in score.patterns.iter().enumerate() {
        let path = format!("/patterns/{i}");
        id_check(&mut r, &mut pids, &p.id, &format!("{path}/id"));
        match ts.steps_per_bar(p.grid) {
            None => r.err(
                IssueCode::GridMismatch,
                format!("{path}/grid"),
                "grid does not divide the bar",
            ),
            Some(spb) => {
                if p.length_steps == 0 || p.length_steps % spb != 0 {
                    r.err(
                        IssueCode::GridMismatch,
                        format!("{path}/length_steps"),
                        format!("length_steps must be a positive multiple of {spb} (one bar)"),
                    );
                }
            }
        }
        for (j, n) in p.notes.iter().enumerate() {
            let np = format!("{path}/notes/{j}");
            if n.step >= p.length_steps {
                r.err(
                    IssueCode::StepOutOfRange,
                    format!("{np}/step"),
                    "step beyond pattern",
                );
            }
            if n.pitch > 127 {
                r.err(
                    IssueCode::PitchRange,
                    format!("{np}/pitch"),
                    "pitch 0..=127",
                );
            }
            if !(n.velocity > 0.0 && n.velocity <= 1.0) {
                r.err(
                    IssueCode::VelocityRange,
                    format!("{np}/velocity"),
                    "velocity in (0, 1]",
                );
            }
            if !(n.length > 0.0 && n.length <= 64.0) {
                r.err(
                    IssueCode::LengthRange,
                    format!("{np}/length"),
                    "length in (0, 64] steps",
                );
            }
            if !(-0.5..=0.5).contains(&n.offset) {
                r.err(
                    IssueCode::OffsetRange,
                    format!("{np}/offset"),
                    "offset -0.5..=0.5",
                );
            }
        }
    }

    // Layers
    let mut lids = BTreeSet::new();
    for l in &score.layers {
        lids.insert(l.id.as_str());
    }
    let mut seen = BTreeSet::new();
    for (i, l) in score.layers.iter().enumerate() {
        let path = format!("/layers/{i}");
        id_check(&mut r, &mut seen, &l.id, &format!("{path}/id"));
        if score.constraints.avoid.contains(&l.class) {
            r.err(
                IssueCode::AvoidedClass,
                format!("{path}/class"),
                format!("class {:?} is on the avoid-list", l.class),
            );
        }
        if !(-1.0..=1.0).contains(&l.pan) {
            r.err(IssueCode::PanRange, format!("{path}/pan"), "pan -1..=1");
        }
        if !(-60.0..=24.0).contains(&l.gain_db) {
            r.err(
                IssueCode::ParamRange,
                format!("{path}/gain_db"),
                "gain_db -60..=24",
            );
        }
        let pitched = !unpitched_class(l.class) && score.key.mode != Mode::Chromatic;
        let mut spans: Vec<(u32, u32, usize)> = Vec::new();
        for (c, clip) in l.clips.iter().enumerate() {
            let cp = format!("{path}/clips/{c}");
            check_range(&mut r, &clip.bars, format!("{cp}/bars"));
            spans.push((clip.bars.start, clip.bars.end, c));
            match score.patterns.iter().find(|p| p.id == clip.pattern) {
                None => r.err(
                    IssueCode::UnknownPattern,
                    format!("{cp}/pattern"),
                    format!("unknown pattern `{}`", clip.pattern),
                ),
                Some(p) if pitched => {
                    for (j, n) in p.notes.iter().enumerate() {
                        let pitch = n.pitch as i32 + clip.transpose as i32;
                        if !(0..=127).contains(&pitch) {
                            r.err(
                                IssueCode::PitchRange,
                                format!("{cp}/transpose"),
                                "transposed pitch out of range",
                            );
                        } else if !score.key.contains(pitch as u8) {
                            r.err(
                                IssueCode::OffScale,
                                format!("/patterns/{}/notes/{j}/pitch", pidx(score, &p.id)),
                                format!(
                                    "{} is not in {} (layer `{}`)",
                                    crate::theory::midi_name(pitch as u8),
                                    score.key.display(),
                                    l.id
                                ),
                            );
                        }
                    }
                }
                Some(_) => {}
            }
            if let Some(p) = score.patterns.iter().find(|p| p.id == clip.pattern)
                && clip.pattern_offset_steps >= p.length_steps.max(1)
            {
                r.err(
                    IssueCode::StepOutOfRange,
                    format!("{cp}/pattern_offset_steps"),
                    "pattern_offset_steps must be below the pattern length",
                );
            }
            if !(clip.velocity_scale > 0.0 && clip.velocity_scale <= 2.0) {
                r.err(
                    IssueCode::VelocityRange,
                    format!("{cp}/velocity_scale"),
                    "0 < scale <= 2",
                );
            }
        }
        spans.sort();
        for w in spans.windows(2) {
            if w[1].0 <= w[0].1 {
                r.err(
                    IssueCode::OverlappingClips,
                    format!("{path}/clips/{}", w[1].2),
                    "clips on one layer overlap",
                );
            }
        }
        let mut eids = BTreeSet::new();
        for (e, fx) in l.effects.iter().enumerate() {
            effect_check(&mut r, &mut eids, fx, &format!("{path}/effects/{e}"), &lids);
        }
        for (a, au) in l.automation.iter().enumerate() {
            automation_check(&mut r, score, l, au, &format!("{path}/automation/{a}"));
        }
    }

    // Master
    let mut eids = BTreeSet::new();
    for (e, fx) in score.master.effects.iter().enumerate() {
        effect_check(
            &mut r,
            &mut eids,
            fx,
            &format!("/master/effects/{e}"),
            &lids,
        );
    }
    let ld = score.master.loudness;
    if !(-40.0..=-5.0).contains(&ld.target_lufs) {
        r.err(
            IssueCode::LoudnessRange,
            "/master/loudness/target_lufs",
            "target -40..=-5 LUFS",
        );
    }
    if !(-12.0..=0.0).contains(&ld.true_peak_dbtp) {
        r.err(
            IssueCode::LoudnessRange,
            "/master/loudness/true_peak_dbtp",
            "ceiling -12..=0 dBTP",
        );
    }

    // Markers and chords
    let total_s =
        crate::timeline::TimeMap::new(score, 48_000).bar_pos_to_s(score.bars as f64 + 1.0);
    let mut mids = BTreeSet::new();
    for (i, m) in score.markers.iter().enumerate() {
        id_check(&mut r, &mut mids, &m.id, &format!("/markers/{i}/id"));
        if !(m.time_s >= 0.0 && m.time_s <= total_s) {
            r.err(
                IssueCode::MarkerRange,
                format!("/markers/{i}/time_s"),
                "marker outside the score",
            );
        }
    }
    for (i, c) in score.chords.iter().enumerate() {
        check_range(&mut r, &c.bars, format!("/chords/{i}/bars"));
    }
    r
}

fn pidx(score: &Score, id: &str) -> usize {
    score.patterns.iter().position(|p| p.id == id).unwrap_or(0)
}

fn id_check<'a>(r: &mut Report, seen: &mut BTreeSet<&'a str>, id: &'a str, path: &str) {
    if !valid_id(id) {
        r.err(
            IssueCode::BadId,
            path,
            format!("bad id `{id}`: use [a-z0-9_-], max 64"),
        );
    }
    if !seen.insert(id) {
        r.err(IssueCode::DuplicateId, path, format!("duplicate id `{id}`"));
    }
}

fn effect_check<'a>(
    r: &mut Report,
    seen: &mut BTreeSet<&'a str>,
    fx: &'a Effect,
    path: &str,
    layers: &BTreeSet<&str>,
) {
    id_check(r, seen, &fx.id, &format!("{path}/id"));
    for (name, v) in &fx.params {
        match params::effect_param(fx.kind, name) {
            None => r.err(
                IssueCode::UnknownParam,
                format!("{path}/params/{name}"),
                format!("{:?} has no parameter `{name}`", fx.kind),
            ),
            Some(s) => {
                if !(v.is_finite() && *v >= s.min && *v <= s.max) {
                    r.err(
                        IssueCode::ParamRange,
                        format!("{path}/params/{name}"),
                        format!("{name} = {v} outside {}..={} {}", s.min, s.max, s.unit),
                    );
                }
            }
        }
    }
    if let Some(sc) = &fx.sidechain
        && (fx.kind != EffectKind::Compressor || !layers.contains(sc.as_str()))
    {
        r.err(
            IssueCode::SidechainSource,
            format!("{path}/sidechain"),
            "sidechain must name an existing layer and sit on a compressor",
        );
    }
}

fn automation_check(r: &mut Report, score: &Score, l: &Layer, au: &Automation, path: &str) {
    if au.points.is_empty() || au.points.windows(2).any(|w| w[1].bar < w[0].bar) {
        r.err(
            IssueCode::AutomationPoints,
            format!("{path}/points"),
            "points must be non-empty and sorted",
        );
    }
    for (i, pt) in au.points.iter().enumerate() {
        if !(pt.bar >= 1.0 && pt.bar <= score.bars as f64 + 1.0) {
            r.err(
                IssueCode::AutomationPoints,
                format!("{path}/points/{i}/bar"),
                "point outside the score",
            );
        }
    }
    let t = au.target.as_str();
    let spec = if let Some(s) = params::LAYER_PARAMS.iter().find(|s| s.name == t) {
        Some(*s)
    } else if let Some(rest) = t.strip_prefix("effects.") {
        rest.split_once('.').and_then(|(eid, pname)| {
            l.effects
                .iter()
                .find(|e| e.id == eid)
                .and_then(|e| params::effect_param(e.kind, pname).copied())
        })
    } else if t.starts_with("instrument.") {
        // Checked against the preset's specs by praxis-instruments.
        return;
    } else {
        None
    };
    match spec {
        None => r.err(
            IssueCode::AutomationTarget,
            format!("{path}/target"),
            format!("unknown target `{t}`"),
        ),
        Some(s) => {
            for (i, pt) in au.points.iter().enumerate() {
                if !(pt.value >= s.min && pt.value <= s.max) {
                    r.err(
                        IssueCode::ParamRange,
                        format!("{path}/points/{i}/value"),
                        format!("{} outside {}..={}", pt.value, s.min, s.max),
                    );
                }
            }
        }
    }
}
