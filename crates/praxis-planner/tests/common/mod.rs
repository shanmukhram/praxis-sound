//! Shared checks for planner integration tests.
#![allow(dead_code)]

use praxis_score::timeline::{TimeMap, expand_notes};
use praxis_score::validate::validate;
use praxis_score::{
    Accent, GeneratorClass, Grid, InstrumentKind, Key, Mode, PitchClass, Playback, Score,
};
use std::collections::BTreeSet;

pub const SR: u32 = 48_000;

/// Bars in which a layer has a clip.
pub fn active_bars(score: &Score, layer_id: &str) -> BTreeSet<u32> {
    score
        .layer(layer_id)
        .map(|l| {
            l.clips
                .iter()
                .flat_map(|c| c.bars.start..=c.bars.end)
                .collect()
        })
        .unwrap_or_default()
}

/// Trailer classes that must never sound unless requested.
pub const TRAILER: [GeneratorClass; 7] = [
    GeneratorClass::Riser,
    GeneratorClass::Sweep,
    GeneratorClass::Impact,
    GeneratorClass::Boom,
    GeneratorClass::Pad,
    GeneratorClass::Whoosh,
    GeneratorClass::Fill,
];

/// Every structural condition of acceptance test 1 (PRD section 2).
pub fn check_acceptance1(score: &Score) -> Result<(), String> {
    let report = validate(score);
    if !report.is_valid() {
        return Err(format!("validation errors: {:?}", report.errors));
    }
    if score.bars != 16 {
        return Err(format!("bars {} != 16", score.bars));
    }
    if (
        score.time_signature.numerator,
        score.time_signature.denominator,
    ) != (4, 4)
    {
        return Err("not 4/4".into());
    }
    if score.key
        != (Key {
            tonic: PitchClass::A,
            mode: Mode::Minor,
        })
    {
        return Err(format!("key {:?} is not A minor", score.key));
    }
    if (score.tempo.bpm - 128.0).abs() > 1e-9 {
        return Err(format!("tempo {} != 128", score.tempo.bpm));
    }
    let tm = TimeMap::new(score, SR);
    if (tm.bar_pos_to_s(17.0) - 30.0).abs() > 1e-9 {
        return Err("length is not 30 s".into());
    }
    if score.playback != Playback::Loop {
        return Err("not a loop".into());
    }
    for c in [
        GeneratorClass::Riser,
        GeneratorClass::Sweep,
        GeneratorClass::Impact,
        GeneratorClass::Pad,
    ] {
        if !score.constraints.avoid.contains(&c) {
            return Err(format!("avoid-list lacks {c:?}"));
        }
    }
    if score.layers.len() != 2 {
        return Err(format!(
            "expected exactly 2 layers, got {:?}",
            score.layers.iter().map(|l| &l.id).collect::<Vec<_>>()
        ));
    }
    let piano = score
        .layers
        .iter()
        .find(|l| l.instrument.preset == "felt_piano")
        .ok_or("no felt_piano layer")?;
    if piano.instrument.kind != InstrumentKind::Sampler {
        return Err("felt_piano is not a sampler".into());
    }
    let kick = score
        .layers
        .iter()
        .find(|l| l.instrument.preset == "soft_kick")
        .ok_or("no soft_kick layer")?;
    if kick.instrument.kind != InstrumentKind::Drum || kick.class != GeneratorClass::Drums {
        return Err("soft_kick is not a drum layer".into());
    }
    // Piano: bars 1-16, one pattern on an eighth grid, accents, overlapping sustain, repeating.
    let pa = active_bars(score, &piano.id);
    if pa != (1..=16).collect() {
        return Err(format!("piano plays bars {pa:?}, not 1-16"));
    }
    let patterns: BTreeSet<&str> = piano.clips.iter().map(|c| c.pattern.as_str()).collect();
    if patterns.len() != 1 {
        return Err("piano uses more than one pattern".into());
    }
    let pat = score
        .pattern(patterns.iter().next().copied().unwrap_or(""))
        .ok_or("piano pattern missing")?;
    if pat.grid != Grid::Eighth {
        return Err(format!("piano grid {:?} is not eighth", pat.grid));
    }
    if pat.length_steps >= 16 * 8 {
        return Err("piano motif does not repeat within 16 bars".into());
    }
    let accents: BTreeSet<String> = pat
        .notes
        .iter()
        .map(|n| format!("{:?}", n.accent))
        .collect();
    if accents.len() < 2 || !pat.notes.iter().any(|n| n.accent == Accent::Strong) {
        return Err(format!("piano accents {accents:?} are not deliberate"));
    }
    if pat.notes.iter().any(|n| n.length <= 1.0) {
        return Err("piano notes do not overlap (length <= 1.0)".into());
    }
    if pat.notes.iter().any(|n| n.offset != 0.0) {
        return Err("piano notes are off the grid".into());
    }
    // Kick: only bars 9-16.
    let ka = active_bars(score, &kick.id);
    if ka != (9..=16).collect() {
        return Err(format!("kick plays bars {ka:?}, not 9-16"));
    }
    // The piano in bars 9-16 is identical to bars 1-8.
    let li = score
        .layers
        .iter()
        .position(|l| l.id == piano.id)
        .unwrap_or(0);
    let events: Vec<_> = expand_notes(score, SR)
        .into_iter()
        .filter(|e| e.layer == li)
        .collect();
    let shift = tm.bar_pos_to_s(9.0);
    let first: Vec<_> = events
        .iter()
        .filter(|e| e.bar <= 8)
        .map(|e| {
            (
                ((e.start_s + shift) * 1e6).round() as i64,
                e.pitch,
                (e.velocity * 1e4).round() as i32,
                e.accent,
                (e.length_s * 1e6).round() as i64,
            )
        })
        .collect();
    let second: Vec<_> = events
        .iter()
        .filter(|e| e.bar >= 9)
        .map(|e| {
            (
                (e.start_s * 1e6).round() as i64,
                e.pitch,
                (e.velocity * 1e4).round() as i32,
                e.accent,
                (e.length_s * 1e6).round() as i64,
            )
        })
        .collect();
    if first.is_empty() || first != second {
        return Err("piano bars 9-16 differ from bars 1-8".into());
    }
    // Nothing banned sounds.
    for l in &score.layers {
        if TRAILER.contains(&l.class) {
            return Err(format!("banned class {:?} present", l.class));
        }
    }
    Ok(())
}

/// Emotion words a case's expected emotion accepts. A model names the mood in its own word
/// ("ominous" for a dark brief, "suspenseful" for a tense one); each set holds only close
/// synonyms, so a plan in another mood still fails. The expected word itself always matches.
pub const EMOTION_SYNONYMS: &[(&str, &[&str])] = &[
    (
        "dark",
        &[
            "ominous",
            "brooding",
            "menacing",
            "grim",
            "foreboding",
            "somber",
            "sombre",
        ],
    ),
    (
        "tense",
        &[
            "suspenseful",
            "suspense",
            "anxious",
            "uneasy",
            "nervous",
            "apprehensive",
        ],
    ),
    ("mysterious", &["enigmatic", "mystical", "mystery"]),
    ("dreamy", &["dreamlike", "ethereal", "hazy"]),
    (
        "melancholy",
        &["melancholic", "sad", "sorrowful", "wistful", "mournful"],
    ),
    ("hopeful", &["optimistic", "uplifting"]),
    ("playful", &["whimsical", "cheerful"]),
    ("tender", &["gentle", "intimate"]),
    ("calm", &["serene", "peaceful", "tranquil", "relaxed"]),
];

/// True when `got` is the expected emotion or one of its documented synonyms (case-insensitive).
pub fn emotion_matches(expected: &str, got: &str) -> bool {
    let got = got.trim().to_lowercase();
    let expected = expected.to_lowercase();
    got == expected
        || EMOTION_SYNONYMS
            .iter()
            .any(|(e, syn)| *e == expected && syn.contains(&got.as_str()))
}

/// A spoken rhythm reduced to what a listener hears in one repetition: whitespace-separated
/// tokens, a sequence that is an exact repetition of a shorter one reduced to that one, then
/// trailing rests (`-`) removed (the arranger restarts a rhythm every bar, so rests at its end are
/// silence either way). Hits, accents, ties and rests between hits are kept exactly.
pub fn spoken_core(s: &str) -> Vec<String> {
    let toks: Vec<String> = s.split_whitespace().map(str::to_string).collect();
    let n = toks.len();
    let period = (1..=n)
        .find(|p| n.is_multiple_of(*p) && (0..n).all(|i| toks[i] == toks[i % p]))
        .unwrap_or(n);
    let mut core: Vec<String> = toks[..period].to_vec();
    while core.last().is_some_and(|t| t == "-") {
        core.pop();
    }
    core
}

/// Spoken rhythms compared after [`spoken_core`].
pub fn same_spoken(expected: &str, got: &str) -> bool {
    spoken_core(expected) == spoken_core(got)
}

/// Seconds of a score's bars (without a one-shot's tail).
pub fn score_seconds(score: &Score) -> f64 {
    TimeMap::new(score, SR).bar_s() * score.bars as f64
}

/// Onsets (seconds) of strong-accent notes: the hit accents among them.
pub fn strong_onsets(score: &Score) -> Vec<f64> {
    expand_notes(score, SR)
        .iter()
        .filter(|e| e.accent == Accent::Strong)
        .map(|e| e.start_s)
        .collect()
}

/// Notes (layer, start) starting in bar `bar` (1-based). A note nudged up to 20 ms early onto
/// the next bar line (a hit accent) belongs to the next bar.
fn bar_onsets(score: &Score, bar: u32) -> Vec<(usize, f64)> {
    let tm = TimeMap::new(score, SR);
    let (a, b) = (
        tm.bar_pos_to_s(bar as f64),
        tm.bar_pos_to_s(bar as f64 + 1.0),
    );
    expand_notes(score, SR)
        .iter()
        .filter(|e| e.start_s >= a - 0.02 && e.start_s < b - 0.02)
        .map(|e| (e.layer, e.start_s))
        .collect()
}

fn role_name(r: praxis_score::SectionRole) -> String {
    serde_json::to_value(r)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Root pitch class (semitones above C) of a chord symbol.
fn chord_root(symbol: &str) -> Option<u8> {
    let two = symbol.chars().nth(1).is_some_and(|x| x == '#' || x == 'b');
    let n = if two { 2 } else { 1 };
    symbol
        .get(..n)
        .and_then(PitchClass::parse)
        .map(|p| p.semitone())
}

/// Scoring-depth expectations of a prompt case (each optional):
/// `section_roles` [roles], `section_names` [names], `section_letters` (an A/B form: the same
/// name has the same role, different names different roles), `hits` (marker count), `hits_within_ms`
/// (a strong accent within this of every hit marker), `duration_within` [seconds, tolerance],
/// `bpm_range` [lo, hi], `layers_enter_in_order`, `voice_leading_max` (semitones any voice of a
/// chord layer moves between consecutive bars), `progression_start` [symbols of the first bars],
/// `bass_passing_tones`, `drop_before` (role: the bar before that part has one layer sounding),
/// `hold_before` (role: every note in the bar before that part starts on its downbeat).
pub fn check_scoring(score: &Score, expect: &serde_json::Value) -> Result<(), String> {
    use serde_json::Value;
    if let Some(roles) = expect.get("section_roles").and_then(Value::as_array) {
        let got: Vec<String> = score.sections.iter().map(|s| role_name(s.role)).collect();
        let want: Vec<String> = roles
            .iter()
            .filter_map(|r| r.as_str().map(str::to_string))
            .collect();
        if got != want {
            return Err(format!("section roles {got:?} != {want:?}"));
        }
    }
    if let Some(names) = expect.get("section_names").and_then(Value::as_array) {
        let got: Vec<&str> = score.sections.iter().map(|s| s.name.as_str()).collect();
        let want: Vec<&str> = names.iter().filter_map(Value::as_str).collect();
        if got != want {
            return Err(format!("section names {got:?} != {want:?}"));
        }
    }
    // An A/B form: parts with the same name share a role, parts with different names differ.
    if expect.get("section_letters").and_then(Value::as_bool) == Some(true) {
        for a in &score.sections {
            for b in &score.sections {
                if (a.name == b.name) != (a.role == b.role) {
                    return Err(format!(
                        "sections `{}` ({}) and `{}` ({}): the same letter needs the same role, different letters different roles",
                        a.name,
                        role_name(a.role),
                        b.name,
                        role_name(b.role)
                    ));
                }
            }
        }
    }
    let hit_markers: Vec<f64> = score
        .markers
        .iter()
        .filter(|m| m.kind == praxis_score::MarkerKind::Hit)
        .map(|m| m.time_s)
        .collect();
    if let Some(n) = expect.get("hits").and_then(Value::as_u64)
        && hit_markers.len() as u64 != n
    {
        return Err(format!("{} hit markers, expected {n}", hit_markers.len()));
    }
    if let Some(ms) = expect.get("hits_within_ms").and_then(Value::as_f64) {
        let onsets = strong_onsets(score);
        for t in &hit_markers {
            let near = onsets
                .iter()
                .map(|o| (o - t).abs())
                .fold(f64::INFINITY, f64::min);
            if near * 1000.0 > ms {
                return Err(format!(
                    "hit at {t:.3} s: nearest strong accent is {:.1} ms away",
                    near * 1000.0
                ));
            }
        }
    }
    if let Some(d) = expect.get("duration_within").and_then(Value::as_array) {
        let want = d.first().and_then(Value::as_f64).unwrap_or(0.0);
        let tol = d.get(1).and_then(Value::as_f64).unwrap_or(0.0);
        let got = score_seconds(score);
        if (got - want).abs() > tol {
            return Err(format!("length {got:.2} s, expected {want} +- {tol}"));
        }
    }
    if let Some(r) = expect.get("bpm_range").and_then(Value::as_array) {
        let lo = r.first().and_then(Value::as_f64).unwrap_or(0.0);
        let hi = r.get(1).and_then(Value::as_f64).unwrap_or(0.0);
        if !(lo..=hi).contains(&score.tempo.bpm) {
            return Err(format!("tempo {} outside {lo}-{hi}", score.tempo.bpm));
        }
    }
    if expect.get("layers_enter_in_order").and_then(Value::as_bool) == Some(true) {
        let firsts: Vec<u32> = score
            .layers
            .iter()
            .map(|l| {
                l.clips
                    .iter()
                    .map(|c| c.bars.start)
                    .min()
                    .unwrap_or(u32::MAX)
            })
            .collect();
        if firsts.windows(2).any(|w| w[1] < w[0]) {
            return Err(format!("layers enter out of order: first bars {firsts:?}"));
        }
        if firsts.len() > 1 && firsts.iter().all(|f| *f == firsts[0]) {
            return Err("every layer enters at once; the form should layer them".into());
        }
    }
    if let Some(max) = expect.get("voice_leading_max").and_then(Value::as_i64) {
        let chord_layers: Vec<usize> = score
            .layers
            .iter()
            .enumerate()
            .filter(|(_, l)| matches!(l.class, GeneratorClass::Chords | GeneratorClass::Harmony))
            .map(|(i, _)| i)
            .collect();
        if chord_layers.is_empty() {
            return Err("no chord layer to check voice-leading on".into());
        }
        let tm = TimeMap::new(score, SR);
        let events = expand_notes(score, SR);
        let mut compared = 0;
        for li in chord_layers {
            let mut prev: Option<Vec<i64>> = None;
            for bar in 1..=score.bars {
                let t0 = tm.bar_pos_to_s(bar as f64);
                let mut v: Vec<i64> = events
                    .iter()
                    .filter(|e| e.layer == li && (e.start_s - t0).abs() < 1e-6)
                    .map(|e| e.pitch as i64)
                    .collect();
                v.sort();
                v.dedup();
                if v.len() != 3 {
                    prev = None;
                    continue;
                }
                if let Some(p) = &prev {
                    compared += 1;
                    let worst = v
                        .iter()
                        .zip(p)
                        .map(|(a, b)| (a - b).abs())
                        .max()
                        .unwrap_or(0);
                    if worst > max {
                        return Err(format!(
                            "bar {bar}: a chord voice moves {worst} semitones ({p:?} -> {v:?})"
                        ));
                    }
                }
                prev = Some(v);
            }
        }
        if compared == 0 {
            return Err("no consecutive three-note chords to check voice-leading on".into());
        }
    }
    if let Some(syms) = expect.get("progression_start").and_then(Value::as_array) {
        let mut per_bar: Vec<String> = Vec::new();
        for c in &score.chords {
            for _ in c.bars.start..=c.bars.end {
                per_bar.push(c.symbol.clone());
            }
        }
        let want: Vec<String> = syms
            .iter()
            .filter_map(|s| s.as_str().map(str::to_string))
            .collect();
        if per_bar.len() < want.len() || per_bar[..want.len()] != want[..] {
            return Err(format!(
                "chords start {:?}, expected {want:?}",
                &per_bar[..want.len().min(per_bar.len())]
            ));
        }
    }
    if expect.get("bass_passing_tones").and_then(Value::as_bool) == Some(true) {
        let Some(bi) = score
            .layers
            .iter()
            .position(|l| l.class == GeneratorClass::Bass)
        else {
            return Err("no bass layer".into());
        };
        let tm = TimeMap::new(score, SR);
        let mut roots: Vec<Option<u8>> = vec![None; score.bars as usize + 2];
        for c in &score.chords {
            let root = chord_root(&c.symbol);
            for b in c.bars.start..=c.bars.end.min(score.bars) {
                roots[b as usize] = root;
            }
        }
        let passing = expand_notes(score, SR).iter().any(|e| {
            let bar = tm.s_to_bar_pos(e.start_s + 1e-9).floor() as usize;
            e.layer == bi
                && e.accent != Accent::Strong
                && roots
                    .get(bar)
                    .copied()
                    .flatten()
                    .is_some_and(|r| e.pitch % 12 != r)
        });
        if !passing {
            return Err("the bass plays only chord roots; expected passing tones".into());
        }
    }
    for (key, kind) in [("drop_before", "drop"), ("hold_before", "hold")] {
        let Some(role) = expect.get(key).and_then(Value::as_str) else {
            continue;
        };
        let Some(s) = score.sections.iter().find(|s| role_name(s.role) == role) else {
            return Err(format!("no {role} section"));
        };
        let bar = s.bars.start - 1;
        let notes = bar_onsets(score, bar);
        if kind == "drop" {
            let layers: BTreeSet<usize> = notes.iter().map(|n| n.0).collect();
            if layers.len() != 1 {
                return Err(format!(
                    "bar {bar} before the {role}: {} layers sound, expected a drop to one",
                    layers.len()
                ));
            }
        } else {
            let t0 = TimeMap::new(score, SR).bar_pos_to_s(bar as f64);
            if notes.is_empty() || notes.iter().any(|n| (n.1 - t0).abs() > 1e-6) {
                return Err(format!(
                    "bar {bar} before the {role}: expected one held chord on the downbeat"
                ));
            }
        }
    }
    Ok(())
}
