//! Compressor sidechain keyed by another layer, and latency compensation with real processors
//! (docs/contracts/engine.md "Sidechain", "Latency compensation").

mod common;

use common::*;
use praxis_engine::{RenderError, RenderOptions};
use praxis_score::timeline::{self, TimeMap};
use praxis_score::*;

fn raw(stems: bool) -> RenderOptions {
    RenderOptions {
        master: false,
        ..opts(stems)
    }
}

fn effect(id: &str, kind: EffectKind, params: &[(&str, f64)]) -> Effect {
    Effect {
        id: id.into(),
        kind,
        params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        bypass: false,
        sidechain: None,
        impulse: None,
    }
}

/// A ducking compressor on the piano keyed by the kick.
fn ducker() -> Effect {
    Effect {
        sidechain: Some("kick".into()),
        ..effect(
            "duck",
            EffectKind::Compressor,
            &[
                ("threshold_db", -40.0),
                ("ratio", 8.0),
                ("attack_ms", 1.0),
                ("release_ms", 120.0),
                ("knee_db", 0.0),
            ],
        )
    }
}

fn ducked_score() -> Score {
    let mut s = bell_score();
    s.layers[0].effects.push(ducker());
    s
}

fn kick_onsets(score: &Score) -> Vec<usize> {
    let ki = score.layers.iter().position(|l| l.id == "kick").unwrap();
    timeline::expand_notes(score, SR)
        .into_iter()
        .filter(|n| n.layer == ki)
        .map(|n| n.start_sample as usize)
        .collect()
}

#[test]
fn piano_ducks_when_the_kick_plays() {
    let plain = bell_score();
    let score = ducked_score();
    let a = render(&plain, &raw(true));
    let b = render(&score, &raw(true));
    assert!(b.report.warnings.is_empty(), "{:?}", b.report.warnings);
    let (dry, duck) = (stem(&a, "piano"), stem(&b, "piano"));
    // The kick stem itself is untouched.
    assert!(buffers_equal(stem(&a, "kick"), stem(&b, "kick")));

    let tm = TimeMap::new(&score, SR);
    let bar = |n: u32| tm.bar_start_sample(n) as usize;
    let ratio = |r: std::ops::Range<usize>| rms(&duck.left[r.clone()]) / rms(&dry.left[r]);
    // Bars 2-7: no kick, so no gain reduction at all (the key is silent).
    let quiet = ratio(bar(2)..bar(8));
    // 80 ms after every kick onset.
    let w = SR as usize * 80 / 1000;
    let mut hit = Vec::new();
    for t in kick_onsets(&score) {
        hit.push(20.0 * ratio(t..t + w).log10());
    }
    let mean_db = hit.iter().sum::<f64>() / hit.len() as f64;
    println!(
        "sidechain: piano level bars 2-7 {:.3} dB; 80 ms after the {} kick hits: mean {mean_db:.2} dB, \
         least {:.2} dB",
        20.0 * quiet.log10(),
        hit.len(),
        hit.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
    );
    assert!(
        (quiet - 1.0).abs() < 1e-3,
        "no ducking without the kick: {quiet}"
    );
    assert!(
        mean_db < -3.0,
        "the piano ducks under the kick: {mean_db} dB"
    );
    assert!(hit.iter().all(|d| *d < -1.0), "{hit:?}");
}

#[test]
fn a_muted_key_layer_still_keys_but_is_not_mixed() {
    let score = ducked_score();
    let full = render(&score, &raw(true));
    let mut muted = score.clone();
    muted.layers[1].muted = true;
    let m = render(&muted, &raw(true));
    assert_eq!(m.stems.len(), 1);
    assert_eq!(m.stems[0].layer_id, "piano");
    assert!(buffers_equal(stem(&m, "piano"), stem(&full, "piano")));
    assert!(buffers_equal(&m.master, stem(&full, "piano")));
    assert!(
        m.report
            .warnings
            .iter()
            .any(|w| w.contains("runs as a key only")),
        "{:?}",
        m.report.warnings
    );
    // Rendering the piano alone gives the same stem as in the full mix.
    let o = RenderOptions {
        only_layers: vec!["piano".into()],
        ..raw(true)
    };
    assert!(buffers_equal(
        stem(&render(&score, &o), "piano"),
        stem(&full, "piano")
    ));
}

#[test]
fn sidechain_cycles_are_render_errors() {
    let mut s = ducked_score();
    s.layers[1].effects.push(Effect {
        sidechain: Some("piano".into()),
        ..effect("pump", EffectKind::Compressor, &[])
    });
    let err =
        praxis_engine::render_offline(&s, &praxis_engine::testing::TestFactory::new(), &raw(false))
            .unwrap_err();
    match err {
        RenderError::InvalidScore(m) => {
            assert!(m.contains("sidechain cycle"), "{m}");
            assert!(m.contains("piano") && m.contains("kick"), "{m}");
        }
        e => panic!("expected InvalidScore, got {e:?}"),
    }
}

/// Lag (in samples, within +-`max`) at which `x` correlates best with `y`.
fn best_lag(x: &[f32], y: &[f32], max: i64) -> i64 {
    let mut best = (0i64, f64::NEG_INFINITY);
    for lag in -max..=max {
        let mut c = 0.0f64;
        for (i, &v) in y.iter().enumerate() {
            let j = i as i64 + lag;
            if j >= 0 && (j as usize) < x.len() {
                c += x[j as usize] as f64 * v as f64;
            }
        }
        if c > best.1 {
            best = (lag, c);
        }
    }
    best.0
}

#[test]
fn latency_compensation_keeps_a_delayed_path_on_the_grid() {
    // The limiter has 144 samples of lookahead latency (3 ms at 48 kHz). On the kick it must not
    // move the kick off the grid, and the master limiter must not move the mix either.
    let mut score = bell_score();
    score.playback = Playback::OneShot { tail_seconds: 1.0 };
    let plain = render(&score, &raw(true));
    let mut lim = score.clone();
    lim.layers[1].effects.push(effect(
        "lim",
        EffectKind::Limiter,
        &[("ceiling_dbtp", 0.0), ("lookahead_ms", 3.0)],
    ));
    let out = render(&lim, &raw(true));
    let (k0, k1) = (stem(&plain, "kick"), stem(&out, "kick"));
    let first = |x: &[f32]| x.iter().position(|v| *v != 0.0);
    let onset = kick_onsets(&score)[0];
    println!(
        "latency: kick onset {onset}, first non-zero without limiter {:?}, with {:?}",
        first(&k0.left),
        first(&k1.left)
    );
    assert_eq!(first(&k0.left), Some(onset));
    assert_eq!(first(&k1.left), Some(onset), "aligned to the sample");
    let w = onset..onset + SR as usize / 4;
    assert_eq!(best_lag(&k1.left[w.clone()], &k0.left[w.clone()], 300), 0);
    // The quiet kick stays below the ceiling: the compensated limiter is transparent.
    let err = k0
        .left
        .iter()
        .zip(&k1.left)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(err < 1e-6, "max difference {err}");
    // The piano (no latency) is unchanged and still lines up with the kick in the mix.
    assert!(buffers_equal(stem(&plain, "piano"), stem(&out, "piano")));

    // Master limiter: the first note of the master starts on its sample.
    let master = render(&score, &opts(false));
    let piano_onset = timeline::expand_notes(&score, SR)
        .iter()
        .filter(|n| n.layer == 0)
        .map(|n| n.start_sample as usize)
        .min()
        .unwrap();
    assert_eq!(first(&master.master.left), Some(piano_onset));
    let w = piano_onset..piano_onset + SR as usize / 4;
    assert_eq!(
        best_lag(&master.master.left[w.clone()], &plain.master.left[w], 300),
        0
    );
}
