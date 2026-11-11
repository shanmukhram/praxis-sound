//! The match loop (FR-15), mix and match (FR-17) and the originality guard (FR-18).
//!
//! 1. **Candidates.** The character suggests a starting point; the discrete choices (the lead
//!    instrument and whether a bass and drums play, the effect family, the ambience bed) are made
//!    by rendering each candidate and measuring it with the same analyzer as the reference.
//! 2. **Refine.** Coordinate descent over the winner's continuous parameters (tone, air, room,
//!    sustain, density, balance, width; or the effect's macros and room), each step a render and
//!    an analysis, until a full sweep at the smallest step finds no better similarity or the
//!    budget is spent.
//! 3. **Final.** The winner is written at the length the user asked for, rendered, measured and
//!    checked for originality against every reference. A close melodic copy is rejected and
//!    re-generated with a different motif.
//!
//! Every evaluation is an [`Iteration`] with its similarity and the best so far, so the user sees
//! the similarity rise. The reference only enters as a [`SoundProfile`] (numbers); its audio never
//! reaches the engine, a Score or an export.

use crate::analysis::{self, Originality};
use crate::character::Character;
use crate::error::MatchError;
use crate::music::{self, BASSES, CONTOURS, Palette, Setup};
use crate::optimize::{self, Settings};
use crate::sfx::{self, BEDS, FAMILIES, SfxSetup};
use praxis_analyze::SoundProfile;
use praxis_analyze::profile::ClipKind;
use praxis_dsp::StereoBuffer;
use praxis_engine::{InstrumentFactory, RenderOptions, render_offline};
use praxis_planner::Catalogue;
use praxis_score::timeline::TimeMap;
use praxis_score::{Key, Mode, PitchClass, Score};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How hard to try.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MatchOptions {
    /// Most renders in the candidate and refine stages together.
    pub max_evals: usize,
    /// Render sample rate.
    pub sample_rate: u32,
    /// Longest render inside the loop, seconds (the final render has the full length).
    pub max_loop_s: f64,
    /// Most motifs tried when the originality guard rejects a result.
    pub originality_attempts: u32,
    /// How many lead instruments are auditioned (closest nominal brightness first).
    pub lead_candidates: usize,
}

impl Default for MatchOptions {
    fn default() -> Self {
        MatchOptions {
            max_evals: 40,
            sample_rate: 48_000,
            max_loop_s: 10.0,
            originality_attempts: 4,
            lead_candidates: 8,
        }
    }
}

/// What to match.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchRequest {
    /// The reference (clip A): its whole character, or its groove when `timbre` is given.
    pub reference: SoundProfile,
    /// Clip B for mix and match: the timbre (instruments, brightness, room).
    pub timbre: Option<SoundProfile>,
    /// A text brief for the mood (FR-17), e.g. "melancholy, a little darker".
    pub mood: Option<String>,
    pub bars: Option<u32>,
    pub key: Option<Key>,
    pub bpm: Option<f64>,
    pub seed: u64,
    pub options: MatchOptions,
}

impl MatchRequest {
    pub fn new(reference: SoundProfile) -> MatchRequest {
        MatchRequest {
            reference,
            timbre: None,
            mood: None,
            bars: None,
            key: None,
            bpm: None,
            seed: 0,
            options: MatchOptions::default(),
        }
    }
}

/// One render of the loop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Iteration {
    /// 1-based.
    pub n: u32,
    /// `candidate`, `refine`, `final` or `originality`.
    pub stage: String,
    /// What was tried, e.g. "felt piano + sub bass" or "tone 0.62".
    pub tried: String,
    /// Similarity of this render to the reference(s), 0..1 (0 when it could not render).
    pub similarity: f32,
    /// Best similarity so far.
    pub best: f32,
    pub accepted: bool,
}

/// Similarity of the result to one reference.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReferenceSimilarity {
    pub reference: String,
    /// `character`, `groove` or `timbre`.
    pub role: String,
    pub similarity: f32,
}

/// The result of a match.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MatchOutcome {
    pub score: Score,
    pub kind: ClipKind,
    /// What the result is made of, e.g. "felt piano + sub bass" or "whoosh".
    pub made_with: String,
    /// Similarity of the final render to the reference(s) (the weighted mean for mix and match).
    pub similarity: f32,
    pub similarities: Vec<ReferenceSimilarity>,
    pub iterations: Vec<Iteration>,
    /// The originality verdict of the final render (the worst over all references).
    pub originality: Originality,
    /// Motifs tried before one passed (1 = the first passed).
    pub originality_attempts: u32,
    /// The tuned parameters, by name.
    pub params: BTreeMap<String, f64>,
    /// Remarks for the user (assumptions, ignored options).
    pub notes: Vec<String>,
}

/// A discrete choice.
#[derive(Clone, Debug, PartialEq)]
enum Candidate {
    Music(Palette),
    Sfx(&'static str),
    Bed(usize),
}

impl Candidate {
    fn describe(&self) -> String {
        match self {
            Candidate::Music(p) => p.describe(),
            Candidate::Sfx(f) => (*f).to_string(),
            Candidate::Bed(b) => BEDS[*b].0.replace('_', " "),
        }
    }

    fn dim_names(&self) -> &'static [&'static str] {
        match self {
            Candidate::Music(_) => music::DIMS,
            Candidate::Sfx(_) => sfx::SFX_DIMS,
            Candidate::Bed(_) => sfx::BED_DIMS,
        }
    }

    /// Indices of the dimensions the optimiser moves.
    fn active_dims(&self) -> Vec<usize> {
        match self {
            Candidate::Music(p) => p.active_dims(),
            Candidate::Sfx(f) => {
                // The drone's and riser's pitch is the note; the other macros all count.
                let mut d = vec![0, 1, 2, 4, 5];
                if !matches!(*f, "drone" | "riser") {
                    d.push(3);
                }
                d.sort_unstable();
                d
            }
            Candidate::Bed(_) => (0..sfx::BED_DIMS.len()).collect(),
        }
    }
}

/// Everything the loop needs to build and judge a candidate.
struct Ctx<'a> {
    targets: Vec<(&'a SoundProfile, f32, &'static str)>,
    factory: &'a dyn InstrumentFactory,
    catalogue: Catalogue,
    sample_rate: u32,
    music: Option<Setup>,
    sfx: Option<SfxSetup>,
    loop_bars: u32,
    final_bars: u32,
    iterations: Vec<Iteration>,
    best: f32,
    last_error: Option<MatchError>,
}

impl Ctx<'_> {
    fn build(
        &self,
        c: &Candidate,
        x: &[f64],
        bars: u32,
        variant: u32,
    ) -> Result<Score, MatchError> {
        match c {
            Candidate::Music(p) => {
                let setup = self
                    .music
                    .as_ref()
                    .ok_or_else(|| MatchError::Invalid("no music setup".into()))?;
                music::score(setup, p, x, bars, variant as usize, &self.catalogue)
            }
            Candidate::Sfx(f) => {
                let mut setup = self
                    .sfx
                    .clone()
                    .ok_or_else(|| MatchError::Invalid("no effect setup".into()))?;
                setup.seed = setup.seed.wrapping_add(variant as u64 * 7919);
                sfx::score(f, &setup, x)
            }
            Candidate::Bed(b) => {
                let mut setup = self
                    .sfx
                    .clone()
                    .ok_or_else(|| MatchError::Invalid("no bed setup".into()))?;
                setup.seed = setup.seed.wrapping_add(variant as u64 * 7919);
                sfx::bed_score(*b, &setup, x)
            }
        }
    }

    fn render(&self, score: &Score) -> Result<StereoBuffer, MatchError> {
        let opts = RenderOptions {
            sample_rate: self.sample_rate,
            ..RenderOptions::default()
        };
        Ok(render_offline(score, self.factory, &opts)?.master)
    }

    /// Renders and measures a score: its profile and the weighted similarity.
    fn measure(&self, score: &Score) -> Result<(SoundProfile, f32), MatchError> {
        let audio = self.render(score)?;
        let profile = analysis::profile_of(&audio, self.sample_rate, "match render")?;
        let total: f32 = self.targets.iter().map(|t| t.1).sum();
        let sim = self
            .targets
            .iter()
            .map(|(p, w, _)| w * analysis::similarity(p, &profile))
            .sum::<f32>()
            / total.max(1e-6);
        Ok((profile, sim))
    }

    /// One evaluation inside the loop; records the iteration.
    fn evaluate(&mut self, c: &Candidate, x: &[f64], stage: &str, tried: String) -> Option<f32> {
        let result = self
            .build(c, x, self.loop_bars, 0)
            .and_then(|s| self.measure(&s));
        let sim = match result {
            Ok((_, s)) => Some(s),
            Err(e) => {
                self.last_error = Some(e);
                None
            }
        };
        let accepted = sim.is_some_and(|s| s > self.best + 1e-4);
        if accepted {
            self.best = sim.unwrap_or(self.best);
        }
        self.iterations.push(Iteration {
            n: self.iterations.len() as u32 + 1,
            stage: stage.into(),
            tried,
            similarity: sim.unwrap_or(0.0),
            best: self.best.max(0.0),
            accepted,
        });
        sim
    }
}

/// Matches a reference: the Score of a new sound with its character (FR-14, FR-15, FR-17, FR-18).
pub fn match_reference(
    req: &MatchRequest,
    factory: &dyn InstrumentFactory,
) -> Result<MatchOutcome, MatchError> {
    let mut notes = Vec::new();
    let a = Character::of(&req.reference);
    let b = req.timbre.as_ref().map(Character::of);
    let catalogue = Catalogue::standard();
    let name = |p: &SoundProfile| {
        p.style_name
            .clone()
            .unwrap_or_else(|| p.source.file_name.clone())
    };
    let mut references = vec![name(&req.reference)];
    let targets: Vec<(&SoundProfile, f32, &'static str)> = match &req.timbre {
        Some(t) => {
            references.push(name(t));
            vec![(&req.reference, 0.5, "groove"), (t, 0.5, "timbre")]
        }
        None => vec![(&req.reference, 1.0, "character")],
    };
    // The timbre clip, when given, decides the sound; the reference decides the rest.
    let sound = b.as_ref().unwrap_or(&a);
    let kind = a.kind;
    if req.timbre.is_some() && kind != ClipKind::Music {
        notes.push(
            "mix and match takes the groove from a music clip; this reference is not music, so \
             both clips only steer the similarity"
                .into(),
        );
    }

    // Key, tempo and mood.
    let mut key = req.key.unwrap_or_else(|| a.key_or_default());
    if req.key.is_none() && a.key.is_none() {
        notes.push(format!("no clear key in the reference; used {}", key.display()));
    }
    let emotion = req
        .mood
        .as_deref()
        .map(|m| emotion_from_text(m, &catalogue, req.seed))
        .unwrap_or_else(|| emotion_from(&key, a.bpm.unwrap_or(90.0)));
    if req.key.is_none()
        && let Some(k) = mood_key(&key, &emotion)
    {
        notes.push(format!(
            "the mood `{emotion}` asks for {}; moved to the relative key {}",
            if matches!(k.mode, Mode::Minor) {
                "minor"
            } else {
                "major"
            },
            k.display()
        ));
        key = k;
    }
    let bpm = req
        .bpm
        .or(a.bpm)
        .unwrap_or_else(|| {
            notes.push("no clear tempo in the reference; used 90 BPM".into());
            90.0
        })
        .clamp(40.0, 220.0);

    let mut ctx = Ctx {
        targets,
        factory,
        catalogue,
        sample_rate: req.options.sample_rate.max(8000),
        music: None,
        sfx: None,
        loop_bars: 1,
        final_bars: 1,
        iterations: Vec::new(),
        best: f32::NEG_INFINITY,
        last_error: None,
    };
    let candidates: Vec<(Candidate, Vec<f64>)> = match kind {
        ClipKind::Music => {
            let ts = a.time_signature();
            let mut probe = Score::empty(1, bpm);
            probe.time_signature = ts;
            let bar_s = TimeMap::new(&probe, 48_000).bar_s().max(0.1);
            let final_bars = match req.bars {
                Some(b) => b.clamp(1, 512),
                None => ((a.duration_s / bar_s).round() as u32).clamp(2, 64),
            };
            let loop_bars = ((a.duration_s.min(req.options.max_loop_s) / bar_s).ceil() as u32)
                .clamp(2, final_bars.max(2))
                .min(final_bars);
            ctx.final_bars = final_bars;
            ctx.loop_bars = loop_bars.max(1);
            let octave = sound
                .median_midi
                .map(|m| ((m / 12.0).floor() as i32 - 1).clamp(3, 5))
                .unwrap_or(4);
            ctx.music = Some(Setup {
                key,
                bpm,
                ts,
                groove: a.groove.clone(),
                emotion: emotion.clone(),
                octave,
                intro: a.sections >= 2,
                target_lufs: a.lufs,
                brief: req.mood.clone(),
                references: references.clone(),
                seed: req.seed,
            });
            let mut x0 = music::first_params(sound);
            x0[music::dim::TONE] = (x0[music::dim::TONE] + mood_tone(&emotion)).clamp(0.0, 1.0);
            // Density follows the groove clip.
            x0[music::dim::DENSITY] = music::first_params(&a)[music::dim::DENSITY];
            let first = music::first_palette(sound);
            let mut list: Vec<(Candidate, Vec<f64>)> = Vec::new();
            if first.lead.is_none() {
                list.push((Candidate::Music(first.clone()), x0.clone()));
            }
            for lead in music::leads_by_distance(sound)
                .into_iter()
                .take(req.options.lead_candidates.max(1))
            {
                list.push((
                    Candidate::Music(Palette {
                        lead: Some(lead),
                        ..first.clone()
                    }),
                    x0.clone(),
                ));
            }
            list
        }
        ClipKind::Sfx | ClipKind::Ambience => {
            if req.bars.is_some() {
                notes.push("bars apply to music; the effect keeps the reference's length".into());
            }
            let setup = SfxSetup::from_character(
                sound,
                req.key,
                req.bpm,
                references.clone(),
                req.mood.clone(),
                req.seed,
            );
            let list = if kind == ClipKind::Sfx {
                FAMILIES
                    .iter()
                    .map(|&f| (Candidate::Sfx(f), sfx::first_params(f, sound, &setup)))
                    .collect()
            } else {
                (0..BEDS.len())
                    .map(|b| (Candidate::Bed(b), sfx::first_bed_params(sound)))
                    .collect()
            };
            ctx.sfx = Some(setup);
            list
        }
    };

    // Stage 1: audition the candidates.
    let mut best: Option<(Candidate, Vec<f64>, f32)> = None;
    for (c, x) in candidates {
        let tried = c.describe();
        if let Some(s) = ctx.evaluate(&c, &x, "candidate", tried)
            && best.as_ref().is_none_or(|b| s > b.2)
        {
            best = Some((c, x, s));
        }
    }
    // Music: try the bass and drums choices around the best lead.
    if let Some((Candidate::Music(p), x, _)) = best.clone() {
        let mut variants = Vec::new();
        for bass in std::iter::once(None).chain(BASSES.iter().map(|b| Some(*b))) {
            if bass != p.bass {
                variants.push(Palette {
                    bass,
                    ..p.clone()
                });
            }
        }
        if p.lead.is_some() {
            variants.push(Palette {
                drums: !p.drums,
                ..p.clone()
            });
        }
        for v in variants {
            let c = Candidate::Music(v);
            let tried = c.describe();
            if let Some(s) = ctx.evaluate(&c, &x, "candidate", tried)
                && best.as_ref().is_none_or(|b| s > b.2)
            {
                best = Some((c, x.clone(), s));
            }
        }
    }
    let Some((candidate, x_start, _)) = best else {
        return Err(ctx
            .last_error
            .take()
            .unwrap_or_else(|| MatchError::Invalid("no candidate could be rendered".into())));
    };

    // Stage 2: refine the winner's continuous parameters.
    let used = ctx.iterations.len();
    let budget = req.options.max_evals.saturating_sub(used).max(8);
    let dims = candidate.active_dims();
    let names = candidate.dim_names();
    let sub0: Vec<f64> = dims.iter().map(|d| x_start[*d]).collect();
    let full = |sub: &[f64]| {
        let mut x = x_start.clone();
        for (i, d) in dims.iter().enumerate() {
            x[*d] = sub[i];
        }
        x
    };
    let settings = Settings {
        max_evals: budget + 1,
        ..Settings::default()
    };
    let mut first = true;
    let mut current = sub0.clone();
    let out = optimize::coordinate_descent(&sub0, &settings, |sub| {
        if first {
            // The starting point was measured in stage 1 (same score, same render).
            first = false;
            return Some(ctx.best as f64);
        }
        let x = full(sub);
        let moved = (0..sub.len())
            .find(|i| (sub[*i] - current[*i]).abs() > 1e-12)
            .map(|i| format!("{} {:.2}", names[dims[i]], sub[i]))
            .unwrap_or_else(|| "start".into());
        let before = ctx.best;
        let sim = ctx.evaluate(&candidate, &x, "refine", moved);
        if sim.is_some_and(|s| s > before + 1e-4) {
            // The optimiser accepts the same improvements (min_gain 1e-4).
            current = sub.to_vec();
        }
        sim.map(|s| s as f64)
    });
    let x_best = full(&out.best_x);

    // Stage 3: the final length, the originality guard.
    let attempts = req.options.originality_attempts.max(1);
    let base = match &candidate {
        Candidate::Music(_) => (req.seed % CONTOURS.len() as u64) as u32,
        _ => 0,
    };
    let mut last: Option<(Score, f32, Vec<ReferenceSimilarity>, Originality)> = None;
    for k in 0..attempts {
        let score = ctx.build(&candidate, &x_best, ctx.final_bars, base + k)?;
        let (profile, sim) = ctx.measure(&score)?;
        let mut checks: Vec<Originality> = ctx
            .targets
            .iter()
            .map(|(p, _, _)| analysis::originality(p, &profile))
            .collect();
        checks.sort_by(|x, y| {
            x.passed
                .cmp(&y.passed)
                .then(y.melodic_similarity.total_cmp(&x.melodic_similarity))
        });
        let worst = checks.into_iter().next().unwrap_or_else(|| Originality {
            passed: true,
            melodic_similarity: 0.0,
            rhythmic_similarity: 0.0,
            reason: String::new(),
            reference: String::new(),
        });
        let sims: Vec<ReferenceSimilarity> = ctx
            .targets
            .iter()
            .map(|(p, _, role)| ReferenceSimilarity {
                reference: name(p),
                role: (*role).to_string(),
                similarity: analysis::similarity(p, &profile),
            })
            .collect();
        let passed = worst.passed;
        ctx.iterations.push(Iteration {
            n: ctx.iterations.len() as u32 + 1,
            stage: if k == 0 { "final" } else { "originality" }.into(),
            tried: if k == 0 {
                format!("{} bars", ctx.final_bars)
            } else {
                format!("another motif ({})", k + 1)
            },
            similarity: sim,
            best: ctx.best.max(sim),
            accepted: passed,
        });
        let done = passed;
        last = Some((score, sim, sims, worst));
        if done {
            let (score, sim, sims, worst) = last.take().ok_or_else(|| {
                MatchError::Invalid("internal: lost the final render".into())
            })?;
            let params = candidate
                .dim_names()
                .iter()
                .enumerate()
                .map(|(i, n)| (n.to_string(), round3(x_best[i])))
                .collect();
            return Ok(MatchOutcome {
                score,
                kind,
                made_with: candidate.describe(),
                similarity: sim,
                similarities: sims,
                iterations: ctx.iterations,
                originality: worst,
                originality_attempts: k + 1,
                params,
                notes,
            });
        }
    }
    let reason = last
        .map(|l| l.3.reason)
        .unwrap_or_else(|| "no attempt".into());
    Err(MatchError::NotOriginal(reason))
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// The planner's reading of a mood brief: its one emotion word.
pub fn emotion_from_text(text: &str, catalogue: &Catalogue, seed: u64) -> String {
    praxis_planner::offline::parse_brief(text, catalogue, seed)
        .emotion
        .trim()
        .to_lowercase()
}

/// An emotion when no mood is given: from the key's mode and the tempo.
pub fn emotion_from(key: &Key, bpm: f64) -> String {
    let minor = is_minor(key.mode);
    match (minor, bpm < 100.0) {
        (true, true) => "melancholy",
        (true, false) => "tense",
        (false, true) => "calm",
        (false, false) => "happy",
    }
    .into()
}

fn is_minor(mode: Mode) -> bool {
    matches!(
        mode,
        Mode::Minor
            | Mode::HarmonicMinor
            | Mode::MelodicMinor
            | Mode::Dorian
            | Mode::Phrygian
            | Mode::Locrian
            | Mode::MinorPentatonic
    )
}

fn minor_emotion(e: &str) -> bool {
    matches!(
        e,
        "melancholy"
            | "mysterious"
            | "tense"
            | "dark"
            | "sad"
            | "eerie"
            | "somber"
            | "ominous"
            | "wistful"
            | "anxious"
            | "lonely"
            | "bittersweet"
    )
}

fn major_emotion(e: &str) -> bool {
    matches!(
        e,
        "happy" | "joyful" | "uplifting" | "triumphant" | "playful" | "hopeful" | "warm"
    )
}

/// The relative key when the mood clearly asks for the other mode (same notes, so the
/// reference's harmony is kept).
pub fn mood_key(key: &Key, emotion: &str) -> Option<Key> {
    if key.mode == Mode::Chromatic {
        return None;
    }
    let t = key.tonic.semitone() as i32;
    if minor_emotion(emotion) && !is_minor(key.mode) {
        Some(Key {
            tonic: PitchClass::from_semitone(t - 3),
            mode: Mode::Minor,
        })
    } else if major_emotion(emotion) && is_minor(key.mode) {
        Some(Key {
            tonic: PitchClass::from_semitone(t + 3),
            mode: Mode::Major,
        })
    } else {
        None
    }
}

/// A small push of the starting tone for dark or bright moods.
fn mood_tone(emotion: &str) -> f64 {
    match emotion {
        "dark" | "somber" | "ominous" | "eerie" | "mysterious" => -0.15,
        "happy" | "joyful" | "uplifting" | "triumphant" | "playful" => 0.1,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use praxis_score::InstrumentKind;

    #[test]
    fn mood_moves_to_the_relative_key() {
        let c_major = Key {
            tonic: PitchClass::C,
            mode: Mode::Major,
        };
        let k = mood_key(&c_major, "melancholy").unwrap();
        assert_eq!(k.tonic, PitchClass::A);
        assert_eq!(k.mode, Mode::Minor);
        assert_eq!(mood_key(&c_major, "calm"), None);
        let a_minor = Key {
            tonic: PitchClass::A,
            mode: Mode::Minor,
        };
        assert_eq!(mood_key(&a_minor, "joyful").unwrap().tonic, PitchClass::C);
    }

    #[test]
    fn emotion_from_mood_text() {
        let cat = Catalogue::standard();
        assert_eq!(emotion_from_text("something sad and slow", &cat, 1), "sad");
        assert_eq!(emotion_from_text("melancholic", &cat, 1), "melancholy");
    }

    #[test]
    fn every_lead_is_in_the_catalogue() {
        let cat = Catalogue::standard();
        for (p, k) in music::LEADS {
            assert!(cat.get_kind(*k, p).is_some(), "{p}");
        }
        for b in BASSES {
            assert!(cat.get_kind(InstrumentKind::Synth, b).is_some(), "{b}");
        }
    }
}
