//! Sound Profile measures on engine renders that the labelled set does not cover: a section
//! boundary where the arrangement changes, and reverb that lengthens the measured decay.

use praxis_analyze::{ProfileOptions, SoundProfile, sound_profile};
use praxis_engine::{RenderOptions, render_offline};
use praxis_instruments::Registry;
use serde_json::{Value, json};

fn note(step: u32, pitch: i32, velocity: f64, length: f64) -> Value {
    json!({"step": step, "pitch": pitch, "velocity": velocity, "accent": "normal", "length": length})
}

fn layer(id: &str, class: &str, kind: &str, preset: &str, pattern: &str, bars: [u32; 2]) -> Value {
    json!({
        "id": id, "name": id, "class": class,
        "instrument": {"kind": kind, "preset": preset, "params": {}},
        "clips": [{"bars": {"start": bars[0], "end": bars[1]}, "pattern": pattern, "transpose": 0, "velocity_scale": 1.0}],
        "effects": [], "automation": [],
    })
}

fn score(
    bars: u32,
    patterns: Vec<Value>,
    layers: Vec<Value>,
    playback: Value,
) -> praxis_score::Score {
    let v = json!({
        "schema_version": 1, "title": "profile test", "seed": 3,
        "tempo": {"bpm": 120.0}, "time_signature": {"numerator": 4, "denominator": 4},
        "key": {"tonic": "C", "mode": "major"}, "bars": bars, "playback": playback,
        "patterns": patterns, "layers": layers,
    });
    serde_json::from_value(v).expect("score")
}

fn profile(s: &praxis_score::Score) -> SoundProfile {
    let out = render_offline(
        s,
        &Registry::with_default_packs(),
        &RenderOptions::default(),
    )
    .expect("render");
    let a = out.master;
    sound_profile(&a.left, &a.right, 48_000, &ProfileOptions::default()).expect("profile")
}

#[test]
fn a_new_arrangement_at_bar_9_is_a_section_boundary() {
    // Bars 1-8: a high pluck arpeggio. Bars 9-16: bass, drums and low keys instead.
    let arp: Vec<Value> = (0..8u32)
        .map(|i| note(i, [72, 76, 79, 84, 79, 76, 72, 67][i as usize], 0.7, 1.4))
        .collect();
    let bass = vec![note(0, 36, 0.85, 3.5), note(4, 43, 0.75, 3.5)];
    let keys = vec![
        note(0, 48, 0.7, 7.5),
        note(0, 52, 0.6, 7.5),
        note(0, 55, 0.6, 7.5),
    ];
    let mut drums = Vec::new();
    for st in 0..8u32 {
        drums.push(note(st, 42, 0.45, 1.0));
    }
    drums.extend([
        note(0, 36, 0.9, 1.0),
        note(4, 36, 0.8, 1.0),
        note(2, 38, 0.8, 1.0),
        note(6, 38, 0.8, 1.0),
    ]);
    let s = score(
        16,
        vec![
            json!({"id": "arp", "grid": "eighth", "length_steps": 8, "notes": arp}),
            json!({"id": "bass", "grid": "eighth", "length_steps": 8, "notes": bass}),
            json!({"id": "keys", "grid": "eighth", "length_steps": 8, "notes": keys}),
            json!({"id": "drums", "grid": "eighth", "length_steps": 8, "notes": drums}),
        ],
        vec![
            layer("arp", "arp", "synth", "pluck", "arp", [1, 8]),
            layer("bass", "bass", "synth", "warm_bass", "bass", [9, 16]),
            layer("keys", "chords", "synth", "soft_keys", "keys", [9, 16]),
            layer("drums", "drums", "drum", "kit", "drums", [9, 16]),
        ],
        json!({"mode": "loop"}),
    );
    let p = profile(&s);
    eprintln!("sections: {:?}", p.sections);
    assert!(p.sections.len() >= 2, "{:?}", p.sections);
    // Bar 9 starts at 16 s; within a bar.
    assert!(
        p.sections.iter().any(|b| (b.start_s - 16.0).abs() <= 2.0),
        "{:?}",
        p.sections
    );
    assert_eq!(p.sections[0].label, "A");
    assert_ne!(p.sections[1].label, "A");
}

#[test]
fn reverb_on_a_one_shot_lengthens_its_decay() {
    let notes = vec![
        note(0, 60, 0.8, 1.0),
        note(4, 64, 0.8, 1.0),
        note(8, 67, 0.8, 1.0),
    ];
    let pattern = json!({"id": "p", "grid": "quarter", "length_steps": 12, "notes": notes});
    let mut rts = Vec::new();
    for wet in [false, true] {
        let mut l = layer("pluck", "motif", "synth", "pluck", "p", [1, 3]);
        if wet {
            l["effects"] =
                json!([{"id": "room", "kind": "reverb", "params": {"decay_s": 4.0, "mix": 0.5}}]);
        }
        let s = score(
            3,
            vec![pattern.clone()],
            vec![l],
            json!({"mode": "one_shot", "tail_seconds": 6.0}),
        );
        let p = profile(&s);
        let r = p.reverb.expect("a one-shot decays");
        eprintln!(
            "reverb {}: rt60 {:.2} s (confidence {:.2})",
            if wet { "on" } else { "off" },
            r.rt60_s,
            r.confidence
        );
        rts.push(r.rt60_s);
    }
    assert!(rts[1] > rts[0] * 1.5, "{rts:?}");
}
