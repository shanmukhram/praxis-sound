//! A user patch that turns one score into another (used by `restore_version`).

use crate::patch::{Origin, PatchOp, Scope, ScorePatch};
use crate::types::{Layer, Score};
use std::collections::BTreeSet;

fn common_prefix<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// Builds an unscoped `origin: user` patch that makes `from` equal to `to` when applied.
///
/// Lists keep their longest common prefix; the rest of `from` is removed last-first and the rest
/// of `to` is appended in order. Layers that are removed and locked are unlocked first; re-added
/// layers carry their own `locked` flag. A kept layer only plays kept patterns, so pattern
/// removals never hit a pattern in use.
///
/// Fails only when the schema versions differ (no op changes `schema_version`).
pub fn patch_between(from: &Score, to: &Score, description: String) -> Result<ScorePatch, String> {
    if from.schema_version != to.schema_version {
        return Err(format!(
            "schema version {} cannot become {}",
            from.schema_version, to.schema_version
        ));
    }
    let mut ops = Vec::new();
    if from.seed != to.seed {
        ops.push(PatchOp::SetSeed { seed: to.seed });
    }
    if from.title != to.title {
        ops.push(PatchOp::SetTitle {
            title: to.title.clone(),
        });
    }
    if from.meta != to.meta {
        ops.push(PatchOp::SetMeta {
            meta: to.meta.clone(),
        });
    }
    if from.tempo != to.tempo {
        ops.push(PatchOp::SetTempo { bpm: to.tempo.bpm });
    }
    if from.time_signature != to.time_signature {
        ops.push(PatchOp::SetTimeSignature {
            time_signature: to.time_signature,
        });
    }
    if from.key != to.key {
        ops.push(PatchOp::SetKey {
            key: to.key,
            transpose_notes: false,
        });
    }
    if from.bars != to.bars {
        ops.push(PatchOp::SetBars { bars: to.bars });
    }
    if from.playback != to.playback {
        ops.push(PatchOp::SetPlayback {
            playback: to.playback.clone(),
        });
    }
    if from.constraints != to.constraints {
        ops.push(PatchOp::SetConstraints {
            constraints: to.constraints.clone(),
        });
    }
    if from.master != to.master {
        ops.push(PatchOp::SetMaster {
            master: to.master.clone(),
        });
    }
    if from.chords != to.chords {
        ops.push(PatchOp::SetChords {
            chords: to.chords.clone(),
        });
    }

    // Patterns and layers.
    let kp = common_prefix(&from.patterns, &to.patterns);
    let kept_patterns: BTreeSet<&str> = from.patterns[..kp].iter().map(|p| p.id.as_str()).collect();
    let plays_only_kept = |l: &Layer| {
        l.clips
            .iter()
            .all(|c| kept_patterns.contains(c.pattern.as_str()))
    };
    let mut kl = common_prefix(&from.layers, &to.layers);
    while kl > 0 && !from.layers[..kl].iter().all(plays_only_kept) {
        kl -= 1;
    }
    for l in from.layers[kl..].iter().rev() {
        if l.locked {
            ops.push(PatchOp::SetLock {
                id: l.id.clone(),
                locked: false,
            });
        }
        ops.push(PatchOp::RemoveLayer { id: l.id.clone() });
    }
    for p in from.patterns[kp..].iter().rev() {
        ops.push(PatchOp::RemovePattern { id: p.id.clone() });
    }
    for p in &to.patterns[kp..] {
        ops.push(PatchOp::AddPattern {
            pattern: p.clone(),
            index: None,
        });
    }

    // Sections and markers.
    let ks = common_prefix(&from.sections, &to.sections);
    for x in from.sections[ks..].iter().rev() {
        ops.push(PatchOp::RemoveSection { id: x.id.clone() });
    }
    for x in &to.sections[ks..] {
        ops.push(PatchOp::AddSection {
            section: x.clone(),
            index: None,
        });
    }
    let km = common_prefix(&from.markers, &to.markers);
    for x in from.markers[km..].iter().rev() {
        ops.push(PatchOp::RemoveMarker { id: x.id.clone() });
    }
    for x in &to.markers[km..] {
        ops.push(PatchOp::AddMarker {
            marker: x.clone(),
            index: None,
        });
    }

    for l in &to.layers[kl..] {
        ops.push(PatchOp::AddLayer {
            layer: l.clone(),
            index: None,
        });
    }
    Ok(ScorePatch {
        description,
        origin: Origin::User,
        scope: Scope::default(),
        ops,
    })
}
