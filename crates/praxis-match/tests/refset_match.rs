//! Phase 6 exit, measured on the self-made reference set (`praxis_match::refset`): every result
//! is closer to its own reference than to any other clip of the set, passes the originality
//! check, and its loop shows the similarity rising. Also mix and match, and determinism.
//!
//! Run with `--nocapture` to see the similarity matrix and each loop's trace.

use praxis_analyze::SoundProfile;
use praxis_dsp::StereoBuffer;
use praxis_engine::{RenderOptions, render_offline};
use praxis_instruments::Registry;
use praxis_match::analysis::{profile_of, similarity};
use praxis_match::refset::{self, Reference};
use praxis_match::{MatchOutcome, MatchRequest, match_reference};
use praxis_score::Score;
use std::sync::OnceLock;

const SR: u32 = 48_000;

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(Registry::with_default_packs)
}

fn render(score: &Score) -> StereoBuffer {
    let opts = RenderOptions {
        sample_rate: SR,
        ..RenderOptions::default()
    };
    render_offline(score, registry(), &opts)
        .unwrap_or_else(|e| panic!("render `{}`: {e}", score.title))
        .master
}

/// True when the Upright Piano KW pack is installed (the set's felt-piano lullaby needs it);
/// otherwise prints why `test` is skipped (CI does not install packs), like the other
/// pack-dependent tests.
fn have_piano(test: &str) -> bool {
    let ok = praxis_instruments::Packs::discover().has_pack("upright-piano-kw");
    if !ok {
        println!("SKIP {test}: the upright-piano-kw pack is not installed (set PRAXIS_PACKS)");
    }
    ok
}

/// The set, rendered and measured once.
fn set() -> &'static Vec<(Reference, SoundProfile)> {
    static S: OnceLock<Vec<(Reference, SoundProfile)>> = OnceLock::new();
    S.get_or_init(|| {
        refset::references()
            .into_iter()
            .map(|r| {
                let audio = render(&r.score);
                let p = profile_of(&audio, SR, &format!("{}.wav", r.name)).expect("profile");
                (r, p)
            })
            .collect()
    })
}

fn matched(i: usize) -> MatchOutcome {
    let (r, p) = &set()[i];
    let mut req = MatchRequest::new(p.clone());
    req.seed = 7;
    match_reference(&req, registry()).unwrap_or_else(|e| panic!("match `{}`: {e}", r.name))
}

#[test]
fn every_result_is_closest_to_its_own_reference_and_original() {
    if !have_piano("every_result_is_closest_to_its_own_reference_and_original") {
        return;
    }
    let set = set();
    println!("classification:");
    for (r, p) in set {
        println!(
            "  {:<14} labelled {:?}, profiled {:?} ({:.2})",
            r.name, r.kind, p.class.kind, p.class.confidence
        );
    }
    let mut failures = Vec::new();
    // `PRAXIS_MATCH_ONLY=name,name` limits the run to some references (for investigating).
    let only: Vec<String> = std::env::var("PRAXIS_MATCH_ONLY")
        .map(|v| v.split(',').map(|x| x.trim().to_string()).collect())
        .unwrap_or_default();
    for (i, (r, p)) in set.iter().enumerate() {
        if !only.is_empty() && !only.iter().any(|o| o == r.name) {
            continue;
        }
        println!(
            "  {} class scores music/sfx/ambience {:.2}/{:.2}/{:.2}, tempo {:?}, groove {}",
            r.name,
            p.class.scores[0],
            p.class.scores[1],
            p.class.scores[2],
            p.tempo
                .as_ref()
                .map(|t| (t.bpm.round(), (t.confidence * 100.0).round() / 100.0)),
            p.groove.is_some()
        );
        let mut req = MatchRequest::new(p.clone());
        req.seed = 7;
        let out = match match_reference(&req, registry()) {
            Ok(o) => o,
            Err(e) => {
                println!("{:<14} FAILED: {e}", r.name);
                failures.push(format!("{}: {e}", r.name));
                continue;
            }
        };
        let auditions: Vec<String> = out
            .iterations
            .iter()
            .filter(|it| it.stage == "candidate")
            .map(|it| format!("{} {:.3}", it.tried, it.similarity))
            .collect();
        println!("    candidates: {}", auditions.join(" | "));
        let first = out
            .iterations
            .first()
            .map(|it| it.similarity)
            .unwrap_or(0.0);
        let loop_best = out
            .iterations
            .iter()
            .filter(|it| it.stage == "candidate" || it.stage == "refine")
            .map(|it| it.best)
            .fold(0.0f32, f32::max);
        let audio = render(&out.score);
        let res = profile_of(&audio, SR, "result.wav").expect("profile");
        let sims: Vec<f32> = set.iter().map(|(_, p)| similarity(p, &res)).collect();
        let own = sims[i];
        let (other_i, other) = sims
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(j, s)| (j, *s))
            .unwrap_or((i, 0.0));
        println!(
            "{:<14} made with {:<34} {:>2} renders, first {:.3} -> loop best {:.3}, final {:.3}; \
             own {:.3} vs nearest other {:.3} ({}); originality {} (melodic {:.2}, rhythmic {:.2}, \
             attempt {})",
            r.name,
            out.made_with,
            out.iterations.len(),
            first,
            loop_best,
            out.similarity,
            own,
            other,
            set[other_i].0.name,
            if out.originality.passed {
                "passed"
            } else {
                "FAILED"
            },
            out.originality.melodic_similarity,
            out.originality.rhythmic_similarity,
            out.originality_attempts,
        );
        println!(
            "    row: {}",
            sims.iter()
                .map(|s| format!("{s:.2}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        if own <= other {
            failures.push(format!(
                "{}: own {own:.3} <= {} {other:.3}",
                r.name, set[other_i].0.name
            ));
        }
        if !out.originality.passed {
            failures.push(format!(
                "{}: not original ({})",
                r.name, out.originality.reason
            ));
        }
        if loop_best < first {
            failures.push(format!("{}: similarity fell", r.name));
        }
        if !praxis_score::validate::validate(&out.score).is_valid() {
            failures.push(format!("{}: invalid score", r.name));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn the_loop_shows_similarity_rising() {
    if !have_piano("the_loop_shows_similarity_rising") {
        return;
    }
    // The heavy impact: a short render, so the whole loop is quick.
    let i = set()
        .iter()
        .position(|(r, _)| r.name == "heavy_impact")
        .unwrap();
    let out = matched(i);
    let best: Vec<f32> = out.iterations.iter().map(|it| it.best).collect();
    for w in best.windows(2) {
        assert!(w[1] >= w[0], "best similarity fell: {best:?}");
    }
    let first = out.iterations[0].similarity;
    let loop_best = out
        .iterations
        .iter()
        .filter(|it| it.stage != "final" && it.stage != "originality")
        .map(|it| it.best)
        .fold(0.0f32, f32::max);
    assert!(
        loop_best > first,
        "the loop found nothing better than its first render ({first})"
    );
    assert!(out.iterations.iter().any(|it| it.stage == "refine"));
    assert_eq!(out.iterations.last().unwrap().stage, "final");
    // Same request, same result.
    let again = matched(i);
    assert_eq!(out.score, again.score);
    assert_eq!(out.iterations, again.iterations);
}

#[test]
fn mix_and_match_takes_groove_timbre_and_mood() {
    if !have_piano("mix_and_match_takes_groove_timbre_and_mood") {
        return;
    }
    let set = set();
    let idx = |name: &str| set.iter().position(|(r, _)| r.name == name).unwrap();
    let (groove_i, timbre_i) = (idx("night_groove"), idx("bell_arp"));
    let mut req = MatchRequest::new(set[groove_i].1.clone());
    req.timbre = Some(set[timbre_i].1.clone());
    req.mood = Some("melancholy".into());
    req.bars = Some(8);
    req.seed = 3;
    let out = match_reference(&req, registry()).expect("mix and match");
    assert_eq!(out.similarities.len(), 2);
    assert_eq!(out.similarities[0].role, "groove");
    assert_eq!(out.similarities[1].role, "timbre");
    assert_eq!(out.score.bars, 8);
    assert_eq!(out.score.meta.emotion.as_deref(), Some("melancholy"));
    assert!(out.originality.passed, "{:?}", out.originality);
    assert_eq!(out.score.meta.reference_profiles.len(), 2);
    // Closer to both of its sources than, on average, to the rest of the set.
    let res = profile_of(&render(&out.score), SR, "mix.wav").unwrap();
    let rest: Vec<f32> = set
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != groove_i && *j != timbre_i)
        .map(|(_, (_, p))| similarity(p, &res))
        .collect();
    let mean_rest = rest.iter().sum::<f32>() / rest.len() as f32;
    let g = similarity(&set[groove_i].1, &res);
    let t = similarity(&set[timbre_i].1, &res);
    println!(
        "mix and match: {} -> groove {g:.3}, timbre {t:.3}, rest mean {mean_rest:.3}",
        out.made_with
    );
    assert!(g > mean_rest && t > mean_rest);
}

#[test]
fn requested_key_tempo_and_length_are_kept() {
    if !have_piano("requested_key_tempo_and_length_are_kept") {
        return;
    }
    let set = set();
    let i = set
        .iter()
        .position(|(r, _)| r.name == "lullaby_felt")
        .unwrap();
    let mut req = MatchRequest::new(set[i].1.clone());
    req.key = praxis_match::parse_key("G major");
    req.bpm = Some(80.0);
    req.bars = Some(12);
    req.options.max_evals = 14;
    req.options.lead_candidates = 3;
    let out = match_reference(&req, registry()).expect("match");
    assert_eq!(out.score.key, praxis_match::parse_key("G major").unwrap());
    assert_eq!(out.score.tempo.bpm, 80.0);
    assert_eq!(out.score.bars, 12);
    // 3 leads + 3 bass/drum variants + 8 refinements, then at most 4 final renders.
    assert!(out.iterations.len() <= 14 + 4, "{}", out.iterations.len());
    // Only presets of the instrument registry: nothing of the reference's audio is in the Score.
    for l in &out.score.layers {
        assert!(
            registry()
                .entry(l.instrument.kind, &l.instrument.preset)
                .is_some(),
            "{}",
            l.instrument.preset
        );
        assert!(l.effects.iter().all(|e| e.impulse.is_none()));
    }
}

/// Investigation aid: `PRAXIS_MATCH_ONLY=<reference>` prints D2's group distances between that
/// reference and a bell, a pluck and a granular-texture candidate at the matcher's first point.
#[test]
#[ignore = "investigation aid"]
fn group_distances_of_first_candidates() {
    if !have_piano("group_distances_of_first_candidates") {
        return;
    }
    use praxis_analyze::similarity::group_distances;
    use praxis_match::character::Character;
    use praxis_match::music::{self, Palette, Setup};
    use praxis_planner::Catalogue;
    use praxis_score::InstrumentKind;
    let name = std::env::var("PRAXIS_MATCH_ONLY").unwrap_or_else(|_| "bell_arp".into());
    let (_, p) = set().iter().find(|(r, _)| r.name == name).unwrap();
    let c = Character::of(p);
    println!("{name}: {c:#?}");
    let ts = c.time_signature();
    let setup = Setup {
        key: c.key_or_default(),
        bpm: c.bpm.unwrap_or(100.0),
        ts,
        groove: c.groove.clone(),
        emotion: "calm".into(),
        octave: 5,
        intro: false,
        target_lufs: c.lufs,
        brief: None,
        references: vec![],
        seed: 1,
    };
    let x = music::first_params(&c);
    for lead in [
        ("bell", InstrumentKind::Synth),
        ("pluck", InstrumentKind::Synth),
    ] {
        let pal = Palette {
            lead: Some(lead),
            bass: None,
            drums: false,
        };
        let s = music::score(&setup, &pal, &x, 4, 0, &Catalogue::standard()).unwrap();
        let r = profile_of(&render(&s), SR, "c.wav").unwrap();
        println!(
            "{}: sim {:.3} kind {:?} {:?}",
            lead.0,
            similarity(p, &r),
            r.class.scores,
            group_distances(p, &r)
        );
    }
    let setup = praxis_match::sfx::SfxSetup::from_character(&c, None, None, vec![], None, 1);
    let s =
        praxis_match::sfx::bed_score(1, &setup, &praxis_match::sfx::first_bed_params(&c)).unwrap();
    let r = profile_of(&render(&s), SR, "c.wav").unwrap();
    println!(
        "granular: sim {:.3} kind {:?} {:?}",
        similarity(p, &r),
        r.class.scores,
        group_distances(p, &r)
    );
}
