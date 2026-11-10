//! Acceptance test 1 (PRD section 2), end to end, through the real code path:
//! one brief -> offline planner -> Score -> real instruments -> offline render -> verifier.
//!
//! Structural conditions are asserted on the Score. Audio conditions are asserted with
//! `praxis_analyze::verify::verify_render`. Layers whose presets are not built yet, and checks
//! that need the mix bus (loudness), are reported and skipped with a printed reason, so this test
//! tightens automatically as packages land. Requires the Upright Piano KW pack (skips otherwise).

use praxis_analyze::verify::{Tolerances, verify_render};
use praxis_engine::{InstrumentFactory, RenderOptions, render_offline};
use praxis_instruments::Registry;
use praxis_planner::{Catalogue, RuleProvider, plan_brief};
use praxis_score::{GeneratorClass, InstrumentKind, Mode, PitchClass, Playback};

const BRIEF: &str = "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt \
piano playing a repeating rhythmic motif on an eighth-note grid, with accents and overlapping \
sustain. Bars 9-16: the identical piano part, plus a soft muted kick layered underneath. No new \
melody, no fills. No risers, sweeps, impacts or pads anywhere.";

#[test]
fn acceptance_test_1_from_one_prompt() {
    let out = plan_brief(BRIEF, &RuleProvider::new(), &Catalogue::standard(), 1)
        .expect("planner produces a score first try");
    let s = &out.score;
    assert_eq!(out.attempts, 1, "first try");

    // --- structure ---
    assert_eq!(s.bars, 16);
    assert_eq!(
        (s.time_signature.numerator, s.time_signature.denominator),
        (4, 4)
    );
    assert_eq!((s.key.tonic, s.key.mode), (PitchClass::A, Mode::Minor));
    assert!((s.tempo.bpm - 128.0).abs() < 1e-9, "tempo {}", s.tempo.bpm);
    assert_eq!(s.playback, Playback::Loop);
    for c in [
        GeneratorClass::Riser,
        GeneratorClass::Sweep,
        GeneratorClass::Impact,
        GeneratorClass::Pad,
    ] {
        assert!(
            s.constraints.avoid.contains(&c),
            "{c:?} must be on the avoid-list"
        );
        assert!(s.layers.iter().all(|l| l.class != c), "no {c:?} layer");
    }
    assert_eq!(
        s.layers.len(),
        2,
        "piano and kick only: {:?}",
        s.layers.iter().map(|l| &l.id).collect::<Vec<_>>()
    );
    let piano = s
        .layers
        .iter()
        .find(|l| l.instrument.preset == "felt_piano")
        .expect("felt piano layer");
    let kick = s
        .layers
        .iter()
        .find(|l| l.instrument.preset == "soft_kick")
        .expect("soft kick layer");
    assert_eq!(piano.instrument.kind, InstrumentKind::Sampler);
    let ev = praxis_score::timeline::expand_notes(s, 48_000);
    let pi = s.layers.iter().position(|l| l.id == piano.id).unwrap();
    let ki = s.layers.iter().position(|l| l.id == kick.id).unwrap();
    // Kick only in bars 9-16; piano in 1-16 with bars 9-16 identical to bars 1-8 (same notes,
    // shifted by 8 bars).
    assert!(ev.iter().filter(|e| e.layer == ki).all(|e| e.bar >= 9));
    assert!(ev.iter().any(|e| e.layer == ki));
    let bar_samples = 48_000 * 60 * 4 / 128; // 90_000 (exact at 128 BPM)
    let first: Vec<_> = ev
        .iter()
        .filter(|e| e.layer == pi && e.bar <= 8)
        .map(|e| {
            (
                e.start_sample,
                e.pitch,
                e.velocity.to_bits(),
                e.length_samples,
            )
        })
        .collect();
    let second: Vec<_> = ev
        .iter()
        .filter(|e| e.layer == pi && e.bar >= 9)
        .map(|e| {
            (
                e.start_sample - 8 * bar_samples,
                e.pitch,
                e.velocity.to_bits(),
                e.length_samples,
            )
        })
        .collect();
    assert!(!first.is_empty());
    assert_eq!(
        first, second,
        "bars 9-16 of the piano repeat bars 1-8 exactly"
    );
    // Rhythm first and notes connect: eighth grid, overlapping sustain.
    let pat = s.pattern(&piano.clips[0].pattern).unwrap();
    assert_eq!(pat.grid, praxis_score::Grid::Eighth);
    assert!(
        pat.notes.iter().all(|n| n.length > 1.0),
        "overlapping sustain"
    );
    assert!(
        pat.notes
            .iter()
            .any(|n| n.accent != praxis_score::Accent::Normal),
        "accents"
    );

    // --- audio ---
    let registry = Registry::with_default_packs();
    if !registry.packs().has_pack("upright-piano-kw") {
        eprintln!(
            "SKIP audio part: Upright Piano KW pack not installed (praxis packs install upright-piano-kw)"
        );
        return;
    }
    let mut only = Vec::new();
    for l in &s.layers {
        if registry
            .describe(l.instrument.kind, &l.instrument.preset)
            .is_some()
        {
            only.push(l.id.clone());
        } else {
            eprintln!(
                "NOTE: preset `{}` not built yet; layer `{}` not rendered",
                l.instrument.preset, l.id
            );
        }
    }
    let full = only.len() == s.layers.len();
    let opts = RenderOptions {
        stems: true,
        only_layers: only,
        ..RenderOptions::default()
    };
    let r1 = render_offline(s, &registry, &opts).expect("render");
    let r2 = render_offline(s, &registry, &opts).expect("render again");
    assert_eq!(r1.master, r2.master, "determinism: bit-identical rerun");
    assert_eq!(r1.master.len(), 30 * 48_000, "exactly 30 s loop");
    for w in &r1.report.warnings {
        eprintln!("render warning: {w}");
    }
    let rep = verify_render(s, &r1.master, 48_000, &Tolerances::default());
    let mixed = r1.report.loudness_lufs.is_finite();
    for c in &rep.checks {
        eprintln!(
            "check {:<10} {:<5} measured {} | expected {}",
            c.name,
            if c.passed { "PASS" } else { "FAIL" },
            c.measured,
            c.expected
        );
        let waived = (c.name == "loudness" && !mixed) || (c.name == "key" && !full);
        if waived && !c.passed {
            eprintln!("  waived until the mix bus / all presets land");
            continue;
        }
        assert!(
            c.passed,
            "check `{}` failed: measured {} expected {}",
            c.name, c.measured, c.expected
        );
    }
    if let Ok(dir) = std::env::var("PRAXIS_DEMO_DIR") {
        let p = std::path::Path::new(&dir);
        std::fs::create_dir_all(p).unwrap();
        praxis_audio_io::write_wav(
            &p.join("acceptance1_master.wav"),
            &r1.master,
            48_000,
            praxis_audio_io::WavFormat::Pcm24,
        )
        .unwrap();
        std::fs::write(p.join("acceptance1.score.json"), praxis_score::to_json(s)).unwrap();
    }
}
