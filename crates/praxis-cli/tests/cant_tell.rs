//! "Can't tell" checks through the real render path (WP-D1c): the owner's hand-built loop, take 1
//! of "storyloop" (2026-11-11), rendered by the engine with the felt piano and soft kick and
//! checked as `praxis render` and every take do. Before WP-D1c it read "2 of 8 checks failed"
//! (tempo 85.34 BPM at confidence 0.42, key "a major" at 0.58) although the render was right.
//! Requires the Upright Piano KW pack (skips otherwise).

use praxis_cli::ops::RenderSettings;
use praxis_cli::ops::render::render_score;
use praxis_instruments::Registry;

/// The score exactly as the owner built it: A minor, 128 BPM, 16-bar loop; the piano repeats A3
/// on the first eight eighths with D5 on the fourth, then E4 and B4 (no third); a soft kick on
/// steps 0, 3, 4 and 5 from bar 9.
const OWNER_LOOP: &str = r#"{"schema_version": 1, "title": "storyloop", "seed": 1,
 "meta": {"tags": [], "reference_profiles": [], "provenance": {"source": "built by hand"}},
 "tempo": {"bpm": 128.0}, "time_signature": {"numerator": 4, "denominator": 4},
 "key": {"tonic": "A", "mode": "minor"}, "bars": 16, "playback": {"mode": "loop"},
 "constraints": {"avoid": []}, "sections": [],
 "patterns": [
  {"id": "felt_piano_p1", "grid": "eighth", "length_steps": 16, "notes": [
   {"step": 0, "pitch": 57, "velocity": 0.8, "accent": "strong", "length": 1.6},
   {"step": 1, "pitch": 57, "velocity": 0.55, "accent": "soft", "length": 1.6},
   {"step": 2, "pitch": 57, "velocity": 0.65, "accent": "normal", "length": 1.6},
   {"step": 3, "pitch": 57, "velocity": 0.55, "accent": "soft", "length": 1.6},
   {"step": 3, "pitch": 74, "velocity": 0.6, "accent": "normal", "length": 1.6},
   {"step": 4, "pitch": 57, "velocity": 0.7, "accent": "bright", "length": 1.6},
   {"step": 5, "pitch": 57, "velocity": 0.55, "accent": "soft", "length": 1.6},
   {"step": 6, "pitch": 57, "velocity": 0.65, "accent": "normal", "length": 1.6},
   {"step": 7, "pitch": 57, "velocity": 0.55, "accent": "soft", "length": 1.6},
   {"step": 12, "pitch": 64, "velocity": 0.6, "accent": "normal", "length": 1.6},
   {"step": 13, "pitch": 71, "velocity": 0.6, "accent": "normal", "length": 1.6}]},
  {"id": "soft_kick_p1", "grid": "eighth", "length_steps": 8, "notes": [
   {"step": 0, "pitch": 36, "velocity": 0.62, "accent": "normal", "length": 1.0},
   {"step": 3, "pitch": 36, "velocity": 0.62, "accent": "strong", "length": 1.0},
   {"step": 4, "pitch": 36, "velocity": 0.62, "accent": "strong", "length": 1.0},
   {"step": 5, "pitch": 36, "velocity": 0.62, "accent": "bright", "length": 1.0}]}],
 "layers": [
  {"id": "felt_piano", "name": "Felt piano", "class": "motif",
   "instrument": {"kind": "sampler", "preset": "felt_piano", "params": {}},
   "clips": [{"bars": {"start": 1, "end": 16}, "pattern": "felt_piano_p1", "transpose": 0,
              "velocity_scale": 1.0}],
   "locked": false, "muted": false, "solo": false, "gain_db": 0.0, "pan": 0.0, "effects": [],
   "automation": [], "seed_offset": 0},
  {"id": "soft_kick", "name": "Soft kick", "class": "drums",
   "instrument": {"kind": "drum", "preset": "soft_kick", "params": {}},
   "clips": [{"bars": {"start": 9, "end": 16}, "pattern": "soft_kick_p1", "transpose": 0,
              "velocity_scale": 1.0}],
   "locked": false, "muted": false, "solo": false, "gain_db": 0.0, "pan": 0.0, "effects": [],
   "automation": [], "seed_offset": 0}],
 "master": {"effects": [], "loudness": {"target_lufs": -16.0, "true_peak_dbtp": -1.0,
            "normalize": true}, "frozen_gain_db": -0.6518525601350067},
 "markers": [], "chords": []}"#;

#[test]
fn owners_hand_built_loop_says_cant_tell_instead_of_failing() {
    if !Registry::with_default_packs()
        .packs()
        .has_pack("upright-piano-kw")
    {
        eprintln!(
            "SKIP: Upright Piano KW pack not installed (praxis packs install upright-piano-kw)"
        );
        return;
    }
    let score = praxis_score::from_json(OWNER_LOOP).expect("the owner's score is valid");
    let dir = std::env::temp_dir().join(format!("praxis-cant-tell-{}", std::process::id()));
    let out = dir.join("mix.wav");
    let v = render_score(&score, &out, &RenderSettings::default()).expect("render");
    let _ = std::fs::remove_dir_all(&dir);
    let check = &v["check"];
    for c in check["checks"].as_array().unwrap() {
        eprintln!(
            "check {:<10} passed {:<5} inconclusive {:<5} measured {}",
            c["name"].as_str().unwrap(),
            c["passed"],
            c["inconclusive"],
            c["measured"].as_str().unwrap()
        );
    }
    eprintln!("summary: {}", check["summary"]);

    assert_eq!(check["passed"], true);
    assert_eq!(check["failed"], serde_json::json!([]));
    assert_eq!(check["inconclusive"], serde_json::json!(["tempo", "key"]));
    let checks = check["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 8);
    for c in checks {
        let unsure = c["name"] == "tempo" || c["name"] == "key";
        assert_eq!(c["inconclusive"], unsure, "{c}");
        assert_eq!(c["passed"], !unsure, "{c}");
    }
    assert_eq!(
        check["summary"],
        "6 of 6 checks passed \u{b7} 2 can't tell from these notes: tempo (the rhythm doesn't \
         clearly show 128 BPM), key (no third, so A minor vs A major can't be heard)"
    );
}
