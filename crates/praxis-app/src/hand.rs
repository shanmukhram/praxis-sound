//! Building a sound by hand (WP-U5): a blank sound without a request, and one take from the live
//! score after a batch of hand edits.
//!
//! The shared take op (`praxis_cli::ops::ask`) starts a sound from a brief and makes one take per
//! patch. Writing notes by hand needs two things it does not offer, built here on the public
//! `Sound`, `SoundLock` and `Project` API (as `commands::undo` is):
//!
//! - [`new_sound`] writes a sound folder whose project holds a blank score (key, tempo or bars +
//!   length through the duration solver, metre, bars, loop or one-shot) and whose `takes.json`
//!   has no take yet. Every hand edit after it is an ordinary patch (`commands::apply_patch`), so
//!   undo, locks and the real-time engine work before the first take exists.
//! - [`make_take`] renders, checks and records the live score as one take: the first take of a
//!   blank sound, or every hand edit since the current take. The take's patch is the exact diff
//!   from the current take's score (`patch_between`), so the history reads like any other take
//!   and agents see it through the same `takes.json`.

use crate::commands::{CmdError, CmdResult, live_of, made_sound, no_take_yet};
use crate::sounds::new_sound_path;
use praxis_audio_io::decode::decode_file;
use praxis_cli::ErrorCode;
use praxis_cli::ops::changes::{bars_label, changed_layers, identical_bars, untouched_text};
use praxis_cli::ops::render::{RenderSettings, render_score};
use praxis_cli::ops::sound::{
    Sound, SoundLock, TakeRecord, TakeRender, TakeScope, TakesFile, normalize_who,
};
use praxis_cli::ops::{self, score_summary};
use praxis_score::patch::{Origin, PatchOp, Scope, ScorePatch};
use praxis_score::project::{Project, patch_between};
use praxis_score::timeline::duration::solve_tempo;
use praxis_score::{Key, Playback, Score, TimeSignature};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

/// What a blank sound starts as. Give the tempo (`bpm`), or a target length (`seconds`) and the
/// duration solver picks the tempo that makes `bars` bars last exactly that long.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewSoundArgs {
    /// The sound's name (also its folder name); "Untitled sound" when empty.
    #[serde(default)]
    pub title: Option<String>,
    pub key: Key,
    #[serde(default)]
    pub bpm: Option<f64>,
    #[serde(default)]
    pub seconds: Option<f64>,
    pub time_signature: TimeSignature,
    pub bars: u32,
    pub playback: Playback,
}

/// The new sound's folder and its live (blank) score.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NewSound {
    pub sound: String,
    pub revision: u64,
    pub live: crate::commands::Live,
}

fn bad(message: impl Into<String>) -> CmdError {
    CmdError::new(ErrorCode::Error, message)
}

/// The blank score `args` describe, validated.
pub fn blank_score(args: &NewSoundArgs) -> CmdResult<Score> {
    let bpm = match (args.bpm, args.seconds) {
        (Some(bpm), _) => bpm,
        (None, Some(s)) if s > 0.0 && s.is_finite() => {
            solve_tempo(args.bars, args.time_signature, s)
        }
        _ => {
            return Err(bad(
                "give the tempo, or a length in seconds to fit the bars into",
            ));
        }
    };
    // Two decimals, as a person would type it (16 bars of 4/4 in 30 s is exactly 128).
    let bpm = (bpm * 100.0).round() / 100.0;
    let title = args
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("Untitled sound");
    let value = json!({
        "schema_version": praxis_score::SCHEMA_VERSION,
        "title": title,
        "seed": 1,
        "meta": { "provenance": { "source": "built by hand" } },
        "tempo": { "bpm": bpm },
        "time_signature": args.time_signature,
        "key": args.key,
        "bars": args.bars,
        "playback": args.playback,
    });
    let score: Score = serde_json::from_value(value)
        .map_err(|e| CmdError::new(ErrorCode::BadScore, e.to_string()))?;
    let report = praxis_score::validate::validate(&score);
    if !report.is_valid() {
        let words: Vec<String> = report.errors.iter().map(|e| e.message.clone()).collect();
        return Err(CmdError::new(
            ErrorCode::InvalidScore,
            format!("this sound cannot start like that: {}", words.join("; ")),
        ));
    }
    Ok(score)
}

/// Writes a new sound folder under `root` holding a blank score and an empty take history.
pub fn new_sound(root: &Path, args: &NewSoundArgs) -> CmdResult<NewSound> {
    let score = blank_score(args)?;
    let dir = new_sound_path(root, &score.title);
    let _lock = SoundLock::acquire(&dir)?;
    let mut sound = Sound {
        dir: dir.clone(),
        project: Some(Project::new(score)),
        takes: TakesFile::default(),
    };
    sound.save()?;
    let project = sound.project.as_ref().expect("set above");
    Ok(NewSound {
        sound: dir.display().to_string(),
        revision: sound.takes.revision,
        live: live_of(project, 0),
    })
}

/// A request to record the live score as a take.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MakeTakeArgs {
    pub sound: String,
    /// The request as it reads in the thread: what was done by hand.
    #[serde(default)]
    pub ask: String,
    #[serde(default)]
    pub who: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub say: Option<String>,
    /// Bars of one layer, when every edit was there.
    #[serde(default)]
    pub scope: Option<TakeScope>,
    #[serde(default)]
    pub skip_missing: bool,
}

fn layer_names(score: &Score) -> String {
    let names: Vec<String> = score
        .layers
        .iter()
        .map(|l| {
            if l.name.is_empty() {
                l.id.clone()
            } else {
                l.name.to_lowercase()
            }
        })
        .collect();
    match names.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// "A minor", as the app writes keys (the score's display form is lower case).
fn key_words(score: &Score) -> String {
    let key = score.key.display();
    let mut c = key.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// Renders, checks and records the live score as the next take, and makes it current: the first
/// take of a sound started by hand, or every hand edit since the current take as one take.
/// Fails, making no take, when the sound has no layer or nothing changed since the current take.
pub fn make_take(args: &MakeTakeArgs) -> CmdResult<Value> {
    let dir = made_sound(&args.sound)?;
    let _lock = SoundLock::acquire(&dir)?;
    let mut sound = Sound::open(&dir)?;
    let prev = sound.takes.current;
    let first = sound.takes.takes.is_empty();
    let score = sound
        .project
        .as_ref()
        .map(|p| p.score.clone())
        .ok_or_else(|| no_take_yet(&dir))?;
    if score.layers.is_empty() {
        return Err(bad(
            "add a layer first: a sound with no layers has nothing to play",
        ));
    }
    let before: Option<Score> = if first {
        None
    } else {
        let version = sound
            .takes
            .take(prev)
            .map(|t| t.version.clone())
            .ok_or_else(|| bad(format!("there is no take {prev}")))?;
        let v = sound
            .project
            .as_ref()
            .and_then(|p| p.version(&version))
            .ok_or_else(|| {
                CmdError::new(
                    ErrorCode::BadScore,
                    format!("the project has no version `{version}`"),
                )
            })?;
        Some(v.score.clone())
    };
    if before.as_ref() == Some(&score) {
        return Err(CmdError::new(
            ErrorCode::PatchRejected,
            format!("nothing changed since take {prev}: edit the sound, then make a take"),
        ));
    }
    let ask = match args.ask.trim() {
        "" if first => "Built by hand".to_string(),
        "" => "Hand edits".to_string(),
        a => a.to_string(),
    };
    let patch = match &before {
        Some(b) => Some(
            patch_between(b, &score, ask.clone())
                .map_err(|e| CmdError::new(ErrorCode::PatchRejected, e))?,
        ),
        None => None,
    };

    let n = sound.takes.next_n();
    let path = sound.mix_path(n);
    let settings = RenderSettings {
        skip_missing: args.skip_missing,
        ..RenderSettings::default()
    };
    let render = render_score(&score, &path, &settings)?;

    let project = sound.project.as_mut().expect("checked above");
    if first && score.master.frozen_gain_db.is_none() {
        // As the shared take op does after a new sound's first render: later takes render at the
        // same gain, so a scoped edit leaves untouched bars of the master bit-identical.
        let gain = render["applied_gain_db"].as_f64().unwrap_or(0.0);
        let mut master = score.master.clone();
        master.frozen_gain_db = Some(gain);
        project
            .commit(ScorePatch {
                description: format!(
                    "freeze the master gain at {gain:.2} dB after the first render, so later \
                     edits leave untouched bars sample-identical"
                ),
                origin: Origin::System,
                scope: Scope::default(),
                ops: vec![PatchOp::SetMaster { master }],
            })
            .map_err(|e| CmdError::new(ErrorCode::PatchRejected, e.to_string()))?;
    }
    let after = project.score.clone();
    let version = format!("take-{n}");
    if project.version(&version).is_some() {
        project
            .delete_version(&version)
            .map_err(|e| bad(e.to_string()))?;
    }
    project
        .save_version(&version)
        .map_err(|e| bad(e.to_string()))?;

    let (changed, identical, untouched) = match (&before, sound.takes.take(prev)) {
        (Some(b), Some(prev_take)) => {
            let changed = changed_layers(b, &after);
            let decode = |p: &Path| {
                decode_file(p).map_err(|e| {
                    CmdError::new(ErrorCode::DecodeFailed, format!("{}: {e}", p.display()))
                })
            };
            let a = decode(&sound.absolute(&prev_take.render.path))?;
            let c = decode(&path)?;
            let identical = if a.sample_rate == c.sample_rate {
                identical_bars(&after, &a.audio, &c.audio, c.sample_rate)
            } else {
                Vec::new()
            };
            let text = untouched_text(b, &after, &changed, &identical, prev);
            (changed, identical, text)
        }
        _ => (Default::default(), Vec::new(), String::new()),
    };
    let title = args.title.clone().unwrap_or_else(|| {
        if first {
            "Built by hand".into()
        } else {
            let lo = changed.values().map(|r| r[0]).min();
            let hi = changed.values().map(|r| r[1]).max();
            match lo.zip(hi) {
                Some((lo, hi)) if lo > 1 || hi < after.bars => {
                    format!("Hand edit, {}", bars_label([lo, hi]))
                }
                _ => "Hand edit".into(),
            }
        }
    });
    let say = args.say.clone().unwrap_or_else(|| {
        if first {
            format!(
                "Built by hand: {}, {} bars at {} BPM in {}.",
                layer_names(&after),
                after.bars,
                after.tempo.bpm,
                key_words(&after)
            )
        } else {
            let what = ask.strip_prefix("By hand: ").unwrap_or(&ask);
            format!("Changed by hand: {}.", what.trim_end_matches('.'))
        }
    });
    let record = TakeRecord {
        n,
        who: normalize_who(args.who.as_deref(), "You"),
        ask,
        title,
        say,
        patch,
        changed,
        untouched,
        render: TakeRender {
            path: Sound::mix_rel(n),
            report: render_report(&render),
        },
        checks: render["check"].clone(),
        opens: None,
        scope: args.scope.clone(),
        version,
        from: if first { 0 } else { prev },
        identical_bars: identical,
        provider: "hand".into(),
        seed: after.seed,
    };
    sound.takes.takes.push(record.clone());
    sound.takes.current = n;
    sound.save()?;
    let project = sound.project.as_ref();
    Ok(json!({
        "sound": sound.dir.display().to_string(),
        "project": sound.project_path().display().to_string(),
        "revision": sound.takes.revision,
        "current": sound.takes.current,
        "take": sound.take_json(&record, true),
        "score_summary": project.map(|p| score_summary(&p.score)),
        "can_undo": project.is_some_and(|p| p.can_undo()),
        "can_redo": project.is_some_and(|p| p.can_redo()),
    }))
}

/// The TypeScript `RenderReport` from a render payload (the fields the shared take op stores).
fn render_report(r: &Value) -> Value {
    json!({
        "mock": false,
        "sample_rate": r["sample_rate"],
        "length_samples": r["length_samples"],
        "tail_samples": r["tail_samples"],
        "is_loop": r["is_loop"],
        "loudness_before_lufs": r["loudness_before_lufs"],
        "loudness_lufs": r["loudness_lufs"],
        "true_peak_dbtp": r["true_peak_dbtp"],
        "applied_gain_db": r["applied_gain_db"],
        "warmup_cycles": r["warmup_cycles"],
        "notes_rendered": r["notes_rendered"],
        "blocked_layers": r["blocked_layers"],
        "render_seconds": r["render_seconds"],
        "warnings": r["warnings"],
        "skipped_layers": r["skipped_layers"],
        "format": r["format"],
        "frames": r["frames"],
        "duration_s": r["duration_s"],
    })
}

/// The instrument presets a layer can use, each with whether it renders on this machine
/// (`ops::presets`: the planner catalogue plus engine-only presets).
pub fn presets() -> CmdResult<Value> {
    Ok(ops::presets()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use praxis_score::{Mode, PitchClass};

    fn args(bpm: Option<f64>, seconds: Option<f64>) -> NewSoundArgs {
        NewSoundArgs {
            title: Some("Hand loop".into()),
            key: Key {
                tonic: PitchClass::A,
                mode: Mode::Minor,
            },
            bpm,
            seconds,
            time_signature: TimeSignature {
                numerator: 4,
                denominator: 4,
            },
            bars: 16,
            playback: Playback::Loop,
        }
    }

    #[test]
    fn the_duration_solver_picks_the_tempo() {
        let s = blank_score(&args(None, Some(30.0))).unwrap();
        assert_eq!(s.tempo.bpm, 128.0);
        assert_eq!(s.bars, 16);
        assert!(s.layers.is_empty());
        assert_eq!(s.title, "Hand loop");
        let s = blank_score(&args(Some(96.5), Some(30.0))).unwrap();
        assert_eq!(s.tempo.bpm, 96.5, "a given tempo wins over the length");
        assert!(blank_score(&args(None, None)).is_err());
        let e = blank_score(&args(Some(900.0), None)).unwrap_err();
        assert_eq!(e.code, "invalid_score");
    }
}
