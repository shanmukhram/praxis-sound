//! Phase 7 exit, measured on the render: a 2-minute score planned from a brief with three hit
//! points lands each one on its marker. The Score is planned offline, rendered by the real engine
//! (effects, mix bus, limiter) with the engine's test instruments standing in for the sampled
//! presets, and an onset detector on the master finds the attack near each marker: it must be
//! within 20 ms of the marker.

mod common;

use common::{SR, check_scoring};
use praxis_engine::testing::TestFactory;
use praxis_engine::{RenderOptions, render_offline};
use praxis_planner::{Catalogue, RuleProvider, plan_brief};
use praxis_score::lint::lint;
use praxis_score::validate::validate;
use praxis_score::{InstrumentKind, MarkerKind, Score};

const BRIEF: &str = "A 2-minute tense cue in D minor: intro, build, peak and release, with hits at \
0:42, 1:05 and 1:38. A felt piano motif, a warm bass and a soft kick. No risers.";

/// The engine's test instruments: pitched layers play the test `bell`, drums the test kick.
fn on_test_instruments(score: &mut Score) {
    for l in &mut score.layers {
        if l.instrument.kind == InstrumentKind::Drum {
            l.instrument.preset = "soft_kick".into();
        } else {
            l.instrument.kind = InstrumentKind::Synth;
            l.instrument.preset = "bell".into();
        }
        l.instrument.params.clear();
    }
}

/// An onset: the energy rises at least this much (dB) in 5 ms.
const ATTACK_DB: f64 = 10.0;

/// Onset strength every millisecond: how many dB the energy of the next 5 ms rises above the
/// energy of the 5 ms that ended 5 ms earlier.
fn onset_strength(mono: &[f32], sr: u32) -> Vec<f64> {
    let hop = (sr / 1000) as usize;
    let win = 5 * hop;
    let frames = mono.len() / hop;
    let energy: Vec<f64> = (0..frames)
        .map(|f| {
            let a = f * hop;
            let b = (a + win).min(mono.len());
            mono[a..b]
                .iter()
                .map(|x| (*x as f64) * (*x as f64))
                .sum::<f64>()
                / (b - a).max(1) as f64
        })
        .collect();
    let db = |e: f64| 10.0 * (e + 1e-12).log10();
    let mut out = vec![0.0; frames];
    for f in 10..frames {
        out[f] = (db(energy[f]) - db(energy[f - 10])).max(0.0);
    }
    out
}

#[test]
fn a_two_minute_cue_lands_three_hits_on_their_markers() {
    let catalogue = Catalogue::standard();
    let out = plan_brief(BRIEF, &RuleProvider::new(), &catalogue, 0).expect("plan");
    let score = &out.score;
    assert!(validate(score).is_valid(), "{:?}", validate(score).errors);
    let lint_ctx = praxis_planner::brief::lint_context(
        score,
        &praxis_planner::plan::PlanContext {
            catalogue: &catalogue,
            requested: &praxis_planner::brief::requested_classes(BRIEF),
        },
    );
    assert_eq!(lint(score, &lint_ctx), vec![]);
    check_scoring(
        score,
        &serde_json::json!({"hits": 3, "hits_within_ms": 1.0, "duration_within": [120.0, 2.0],
                            "section_roles": ["intro", "build", "peak", "release"]}),
    )
    .expect("scoring checks");
    println!("{}", out.summary());

    let mut s = score.clone();
    on_test_instruments(&mut s);
    let opts = RenderOptions {
        sample_rate: SR,
        ..RenderOptions::default()
    };
    let r = render_offline(&s, &TestFactory::new(), &opts).expect("render");
    let mono: Vec<f32> = r
        .master
        .left
        .iter()
        .zip(&r.master.right)
        .map(|(l, r)| 0.5 * (l + r))
        .collect();
    assert!(mono.len() as f64 / SR as f64 >= 119.0);
    let strength = onset_strength(&mono, SR);
    let markers: Vec<f64> = score
        .markers
        .iter()
        .filter(|m| m.kind == MarkerKind::Hit)
        .map(|m| m.time_s)
        .collect();
    assert_eq!(markers, vec![42.0, 65.0, 98.0]);
    for t in markers {
        // The onset near the marker: the first frame within 100 ms either side where the energy
        // rises by at least ATTACK_DB in 5 ms, a clear attack. (Later in a held chord the test
        // bell can swell again; that is not an onset.)
        let centre = (t * 1000.0).round() as usize;
        let best = (centre.saturating_sub(100)..(centre + 100).min(strength.len()))
            .find(|f| strength[*f] >= ATTACK_DB)
            .unwrap_or_else(|| panic!("no attack within 100 ms of the hit at {t} s"));
        let at_s = best as f64 / 1000.0;
        let err_ms = (at_s - t).abs() * 1000.0;
        println!("hit {t:.3} s: onset at {at_s:.3} s ({err_ms:.1} ms)");
        assert!(
            err_ms <= 20.0,
            "hit at {t} s: onset at {at_s:.3} s, {err_ms:.1} ms away"
        );
    }
}
