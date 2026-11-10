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
