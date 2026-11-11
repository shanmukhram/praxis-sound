//! The self-made labelled clip set (WP-D2): scores rendered by the real engine, because the
//! owner buys nothing and has no labelled clips.
//!
//! [`specs`] lists every clip. [`score_for`] turns a spec into a Score; the scores are written
//! once to `crates/praxis-analyze/tests/clips/` with `labels.json` by the ignored test
//! `write_clip_scores` (`cargo test -p praxis-cli --release --test labelled_set -- --ignored
//! write_clip_scores`). The accuracy test reads those files, renders each score with the engine,
//! applies the listed post-processing (level change, added noise) and measures the profile.
//! No audio is ever written.

#![allow(dead_code)]

use praxis_dsp::StereoBuffer;
use praxis_dsp::rng::Pcg32;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

pub const NAMES: [&str; 12] = [
    "C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
];
const MAJOR: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
const MINOR: [i32; 7] = [0, 2, 3, 5, 7, 8, 10];

pub fn clips_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../praxis-analyze/tests/clips")
}

/// Changes applied to the render before analysis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Post {
    /// Level change, dB.
    #[serde(default)]
    pub gain_db: f64,
    /// White noise at this level relative to the render's RMS, dB (e.g. -24).
    #[serde(default)]
    pub noise_db: Option<f64>,
}

/// One clip of the set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub id: String,
    pub score: String,
    /// music, sfx or ambience.
    pub class: String,
    pub tempo_bpm: Option<f64>,
    /// `{"tonic", "mode"}` or null (drums, sfx, ambience).
    pub key: Option<Value>,
    pub sample_rate: u32,
    #[serde(default)]
    pub post: Post,
    /// A pack the score needs (skipped when missing).
    #[serde(default)]
    pub needs_pack: Option<String>,
    /// What the clip is, in words.
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Labels {
    pub version: u32,
    pub note: String,
    pub clips: Vec<Label>,
}

pub fn read_labels() -> Labels {
    let text = std::fs::read_to_string(clips_dir().join("labels.json")).expect("labels.json");
    serde_json::from_str(&text).expect("labels.json parses")
}

/// What to generate.
#[derive(Clone, Debug)]
pub enum Spec {
    Music(Music),
    Sfx(Sfx),
    Ambience(Ambience),
}

#[derive(Clone, Debug)]
pub struct Music {
    pub tonic: usize,
    pub minor: bool,
    /// None: drums only (no key).
    pub main: Option<(&'static str, &'static str)>,
    pub bpm: f64,
    pub bars: u32,
    pub bass: Option<&'static str>,
    pub drums: bool,
    pub melody: Option<&'static str>,
    pub reverb: bool,
    pub looped: bool,
    pub swell: bool,
    pub progression: usize,
}

#[derive(Clone, Debug)]
pub struct Sfx {
    pub preset: &'static str,
    pub tonic: usize,
    pub length: f64,
    pub brightness: f64,
    pub weight: f64,
    pub pitch: f64,
    pub reverb: bool,
}

/// One ambience layer: instrument kind, preset, instrument params.
pub type AmbLayer = (&'static str, &'static str, Vec<(&'static str, f64)>);

#[derive(Clone, Debug)]
pub struct Ambience {
    pub tonic: usize,
    pub layers: Vec<AmbLayer>,
    pub bars: u32,
}

pub struct Entry {
    pub id: String,
    pub spec: Spec,
    pub sample_rate: u32,
    pub post: Post,
    pub description: String,
}

const MAINS: [(&str, &str); 8] = [
    ("sampler", "felt_piano"),
    ("synth", "pluck"),
    ("synth", "soft_keys"),
    ("sampler", "upright_piano"),
    ("synth", "mallet"),
    ("synth", "fm_keys"),
    ("synth", "bell"),
    ("synth", "glass"),
];

/// Every clip of the set, in order.
pub fn specs() -> Vec<Entry> {
    let mut out = Vec::new();
    // 48 music clips: every one of the 24 keys twice, tempos spread over 60..180 BPM.
    for i in 0..48usize {
        let tonic = (i * 7) % 12;
        let minor = (i / 12) % 2 == 1;
        let main = MAINS[(i + i / 8) % MAINS.len()];
        let bpm = 60.0 + ((i * 37) % 121) as f64;
        let bars = [8, 4, 8, 16][i % 4];
        let drums = i % 3 == 0;
        let bass = match i % 4 {
            0 => Some("warm_bass"),
            2 => Some("sub_bass"),
            _ => None,
        };
        let melody = match i % 6 {
            1 => Some("bell"),
            4 => Some("pluck"),
            _ => None,
        };
        let reverb = i % 5 == 2;
        let swell = i % 8 == 5;
        let post = Post {
            gain_db: if i % 6 == 3 { -20.0 } else { 0.0 },
            noise_db: if i % 7 == 3 { Some(-24.0) } else { None },
        };
        let m = Music {
            tonic,
            minor,
            main: Some(main),
            bpm,
            bars,
            bass,
            drums,
            melody,
            reverb,
            looped: i % 2 == 0,
            swell,
            progression: (i / 2) % 2,
        };
        let mode = if minor { "minor" } else { "major" };
        let mut d = format!(
            "{} {} at {} BPM, {} bars, {}",
            NAMES[tonic], mode, bpm, bars, main.1
        );
        if let Some(b) = bass {
            d += &format!(" + {b}");
        }
        if let Some(mel) = melody {
            d += &format!(" + {mel} melody");
        }
        if drums {
            d += " + drum kit";
        }
        if reverb {
            d += ", reverb";
        }
        if swell {
            d += ", level swell";
        }
        if post.gain_db != 0.0 {
            d += &format!(", {} dB", post.gain_db);
        }
        if let Some(n) = post.noise_db {
            d += &format!(", noise at {n} dB");
        }
        d += if m.looped { ", loop" } else { ", one-shot" };
        out.push(Entry {
            id: format!(
                "m{:02}_{}_{}_{}_{}",
                i + 1,
                NAMES[tonic].replace('#', "s").to_lowercase(),
                mode,
                bpm as u32,
                main.1
            ),
            spec: Spec::Music(m),
            sample_rate: if i % 5 == 4 { 44_100 } else { 48_000 },
            post,
            description: d,
        });
    }
    // 4 drum loops (tempo only).
    for (j, bpm) in [92.0, 124.0, 140.0, 76.0].iter().enumerate() {
        out.push(Entry {
            id: format!("d{:02}_drums_{}", j + 1, *bpm as u32),
            spec: Spec::Music(Music {
                tonic: 0,
                minor: false,
                main: None,
                bpm: *bpm,
                bars: 8,
                bass: None,
                drums: true,
                melody: None,
                reverb: j == 1,
                looped: true,
                swell: false,
                progression: 0,
            }),
            sample_rate: 48_000,
            post: Post {
                gain_db: if j == 2 { -12.0 } else { 0.0 },
                noise_db: if j == 3 { Some(-30.0) } else { None },
            },
            description: format!("drum kit loop at {bpm} BPM, 8 bars"),
        });
    }
    // 17 sound effects.
    let sfx: [(&str, f64, f64, f64, f64, bool); 17] = [
        ("whoosh", 0.5, 0.5, 0.3, 0.5, false),
        ("whoosh", 0.2, 0.8, 0.0, 0.7, false),
        ("whoosh", 0.8, 0.3, 0.6, 0.3, true),
        ("whoosh", 0.95, 0.6, 0.5, 0.5, false),
        ("impact", 0.5, 0.5, 0.7, 0.5, false),
        ("impact", 0.2, 0.8, 0.4, 0.75, false),
        ("impact", 0.8, 0.3, 1.0, 0.25, true),
        ("impact", 0.95, 0.5, 0.6, 0.5, false),
        ("riser", 0.5, 0.5, 0.5, 0.5, false),
        ("riser", 0.3, 0.8, 0.2, 0.6, false),
        ("riser", 0.7, 0.4, 0.8, 0.4, true),
        ("riser", 0.6, 0.6, 0.0, 0.5, false),
        ("glitch", 0.5, 0.6, 0.3, 0.5, false),
        ("glitch", 0.8, 0.9, 0.0, 0.6, false),
        ("glitch", 0.3, 0.4, 0.7, 0.4, false),
        ("tick", 0.5, 0.5, 0.0, 0.5, false),
        ("tick", 0.8, 0.7, 0.6, 0.6, true),
    ];
    for (j, (preset, length, brightness, weight, pitch, reverb)) in sfx.iter().enumerate() {
        let tonic = (j * 5) % 12;
        out.push(Entry {
            id: format!("s{:02}_{}", j + 1, preset),
            spec: Spec::Sfx(Sfx {
                preset,
                tonic,
                length: *length,
                brightness: *brightness,
                weight: *weight,
                pitch: *pitch,
                reverb: *reverb,
            }),
            sample_rate: if j % 4 == 3 { 44_100 } else { 48_000 },
            post: Post {
                gain_db: if j % 5 == 2 { -15.0 } else { 0.0 },
                noise_db: if j % 6 == 4 { Some(-40.0) } else { None },
            },
            description: format!(
                "{preset} one-shot (length {length}, brightness {brightness}, weight {weight}, pitch {pitch}){}",
                if *reverb { ", reverb" } else { "" }
            ),
        });
    }
    // 12 ambiences.
    let amb: Vec<(Vec<AmbLayer>, u32, &str)> = vec![
        (
            vec![("synth", "noise_bed", vec![("tone", 0.35)])],
            10,
            "noise bed",
        ),
        (
            vec![("synth", "noise_bed", vec![("tone", 0.1), ("drift", 0.8)])],
            12,
            "dark drifting noise bed",
        ),
        (
            vec![("synth", "noise_bed", vec![("tone", 0.8), ("drift", 0.2)])],
            8,
            "bright noise bed",
        ),
        (
            vec![("synth", "granular_texture", vec![("density", 0.6)])],
            10,
            "sparse granular texture",
        ),
        (
            vec![(
                "synth",
                "granular_texture",
                vec![("density", 1.8), ("tone", 0.7)],
            )],
            12,
            "dense granular texture",
        ),
        (
            vec![
                ("synth", "noise_bed", vec![("tone", 0.3)]),
                ("synth", "granular_texture", vec![("density", 1.0)]),
            ],
            12,
            "noise bed under a granular texture",
        ),
        (
            vec![
                ("synth", "noise_bed", vec![("tone", 0.5)]),
                ("sfx", "drone", vec![("brightness", 0.3)]),
            ],
            10,
            "drone over a noise bed",
        ),
        (
            vec![("sfx", "drone", vec![("brightness", 0.6), ("weight", 0.5)])],
            10,
            "drone bed",
        ),
        (
            vec![("synth", "noise_bed", vec![("tone", 0.55), ("drift", 1.0)])],
            16,
            "long moving noise bed",
        ),
        (
            vec![
                (
                    "synth",
                    "granular_texture",
                    vec![("density", 0.3), ("tone", 0.2)],
                ),
                ("synth", "noise_bed", vec![("tone", 0.15)]),
            ],
            10,
            "very sparse grains over a dark bed",
        ),
        (
            vec![("synth", "noise_bed", vec![("tone", 0.45)])],
            6,
            "short noise bed",
        ),
        (
            vec![
                ("sfx", "drone", vec![("brightness", 0.2), ("length", 0.8)]),
                ("synth", "granular_texture", vec![("density", 0.8)]),
            ],
            12,
            "drone with grains",
        ),
    ];
    for (j, (layers, bars, what)) in amb.into_iter().enumerate() {
        let tonic = (j * 3 + 2) % 12;
        out.push(Entry {
            id: format!("a{:02}_{}", j + 1, what.replace(' ', "_")),
            spec: Spec::Ambience(Ambience {
                tonic,
                layers,
                bars,
            }),
            sample_rate: if j % 4 == 1 { 44_100 } else { 48_000 },
            post: Post {
                gain_db: if j % 4 == 2 { -18.0 } else { 0.0 },
                noise_db: if j % 5 == 3 { Some(-20.0) } else { None },
            },
            description: format!("{what}, {bars} bars at 120 BPM"),
        });
    }
    out
}

fn scale(minor: bool) -> [i32; 7] {
    if minor { MINOR } else { MAJOR }
}

/// MIDI pitch of scale degree `d` (any integer) over `base` (the tonic's MIDI note).
fn degree(base: i32, minor: bool, d: i32) -> i32 {
    let s = scale(minor);
    let oct = d.div_euclid(7);
    base + 12 * oct + s[d.rem_euclid(7) as usize]
}

fn note(step: u32, pitch: i32, velocity: f64, length: f64, accent: &str) -> Value {
    json!({"step": step, "pitch": pitch, "velocity": velocity, "accent": accent, "length": length})
}

fn layer(id: &str, class: &str, kind: &str, preset: &str, pattern: &str, bars: u32) -> Value {
    json!({
        "id": id, "name": id, "class": class,
        "instrument": {"kind": kind, "preset": preset, "params": {}},
        "clips": [{"bars": {"start": 1, "end": bars}, "pattern": pattern, "transpose": 0, "velocity_scale": 1.0}],
        "effects": [], "automation": [],
    })
}

fn reverb(decay: f64, mix: f64) -> Value {
    json!({"id": "room", "kind": "reverb", "params": {"decay_s": decay, "mix": mix}})
}

fn base_score(
    title: &str,
    bpm: f64,
    tonic: usize,
    minor: bool,
    bars: u32,
    playback: Value,
) -> Value {
    json!({
        "schema_version": 1,
        "title": title,
        "seed": 7,
        "meta": {"tags": ["labelled-set"], "reference_profiles": [], "provenance": {"source": "WP-D2 labelled set"}},
        "tempo": {"bpm": bpm},
        "time_signature": {"numerator": 4, "denominator": 4},
        "key": {"tonic": NAMES[tonic], "mode": if minor { "minor" } else { "major" }},
        "bars": bars,
        "playback": playback,
        "constraints": {"avoid": []},
        "sections": [],
        "patterns": [],
        "layers": [],
        "master": {"effects": [], "loudness": {"target_lufs": -16.0, "true_peak_dbtp": -1.0, "normalize": true}},
        "markers": [],
        "chords": [],
    })
}

fn music_score(title: &str, m: &Music) -> Value {
    let playback = if m.looped {
        json!({"mode": "loop"})
    } else {
        json!({"mode": "one_shot", "tail_seconds": 2.0})
    };
    let mut s = base_score(title, m.bpm, m.tonic, m.minor, m.bars, playback);
    let mut patterns = Vec::new();
    let mut layers = Vec::new();
    let prog: [i32; 4] = match (m.minor, m.progression) {
        (false, 0) => [0, 3, 4, 0],
        (false, _) => [0, 4, 5, 3],
        (true, 0) => [0, 3, 4, 0],
        (true, _) => [0, 5, 3, 4],
    };
    let tonic = m.tonic as i32;
    if let Some((kind, preset)) = m.main {
        // Arpeggio on eighths, accents on the beats, notes ring into the next (length 1.6).
        let base = 48 + tonic + if tonic > 5 { 0 } else { 12 };
        let shape = [0, 4, 2, 4, 7, 4, 2, 4];
        let mut notes = Vec::new();
        for (bar, chord) in prog.iter().enumerate() {
            for (k, sh) in shape.iter().enumerate() {
                let d = chord + if *sh == 7 { 7 } else { *sh };
                let p = degree(base, m.minor, d);
                let (v, a) = match k {
                    0 => (0.85, "strong"),
                    4 => (0.72, "normal"),
                    2 | 6 => (0.62, "normal"),
                    _ => (0.5, "soft"),
                };
                notes.push(note((bar * 8 + k) as u32, p, v, 1.6, a));
            }
        }
        patterns
            .push(json!({"id": "main_p", "grid": "eighth", "length_steps": 32, "notes": notes}));
        let mut l = layer("main", "motif", kind, preset, "main_p", m.bars);
        if m.reverb {
            l["effects"] = json!([reverb(2.8, 0.35)]);
        }
        if m.swell {
            l["automation"] = json!([{"target": "gain_db", "curve": "smooth", "points": [
                {"bar": 1.0, "value": -18.0}, {"bar": (m.bars / 2 + 1) as f64, "value": 0.0}]}]);
        }
        layers.push(l);
    }
    if let Some(bass) = m.bass {
        let base = 36 + tonic - if tonic > 7 { 12 } else { 0 };
        let mut notes = Vec::new();
        for (bar, chord) in prog.iter().enumerate() {
            let root = degree(base, m.minor, *chord);
            let fifth = degree(base, m.minor, chord + 4);
            notes.push(note((bar * 4) as u32, root, 0.8, 1.8, "strong"));
            notes.push(note((bar * 4 + 2) as u32, fifth, 0.65, 1.8, "normal"));
        }
        patterns
            .push(json!({"id": "bass_p", "grid": "quarter", "length_steps": 16, "notes": notes}));
        layers.push(layer("bass", "bass", "synth", bass, "bass_p", m.bars));
    }
    if let Some(mel) = m.melody {
        let base = 72 + tonic - if tonic > 4 { 12 } else { 0 };
        let degs = [4, 4, 5, 4, 2, 1, 0, 2, 4, 7, 5, 4, 2, 4, 1, 0];
        let steps = [0, 2, 3, 4, 6, 8, 10, 12, 16, 18, 19, 20, 22, 24, 26, 28];
        let notes: Vec<Value> = degs
            .iter()
            .zip(steps.iter())
            .map(|(d, st)| note(*st, degree(base, m.minor, *d), 0.7, 1.5, "normal"))
            .collect();
        patterns.push(json!({"id": "mel_p", "grid": "eighth", "length_steps": 32, "notes": notes}));
        layers.push(layer("melody", "melody", "synth", mel, "mel_p", m.bars));
    }
    if m.drums {
        let mut notes = Vec::new();
        for st in 0..8u32 {
            let hv = if st % 2 == 0 { 0.5 } else { 0.35 };
            notes.push(note(st, 42, hv, 1.0, "soft"));
        }
        notes.push(note(0, 36, 0.9, 1.0, "strong"));
        notes.push(note(4, 36, 0.8, 1.0, "normal"));
        notes.push(note(2, 38, 0.75, 1.0, "normal"));
        notes.push(note(6, 38, 0.75, 1.0, "normal"));
        patterns.push(json!({"id": "drum_p", "grid": "eighth", "length_steps": 8, "notes": notes}));
        let mut l = layer("drums", "drums", "drum", "kit", "drum_p", m.bars);
        if m.reverb && m.main.is_none() {
            l["effects"] = json!([reverb(1.2, 0.2)]);
        }
        layers.push(l);
    }
    s["patterns"] = json!(patterns);
    s["layers"] = json!(layers);
    s
}

fn sfx_score(title: &str, x: &Sfx) -> Value {
    let mut s = base_score(
        title,
        120.0,
        x.tonic,
        false,
        2,
        json!({"mode": "one_shot", "tail_seconds": 1.5}),
    );
    let pitch = 48 + x.tonic as i32;
    // A riser builds into the end of its note; give it the whole first bar.
    let len = if x.preset == "riser" { 4.0 } else { 1.0 };
    s["patterns"] = json!([{"id": "fx_p", "grid": "quarter", "length_steps": 8,
        "notes": [note(0, pitch, 0.9, len, "normal")]}]);
    let mut l = layer("fx", x.preset, "sfx", x.preset, "fx_p", 1);
    l["instrument"]["params"] = json!({
        "length": x.length, "brightness": x.brightness, "weight": x.weight, "pitch": x.pitch,
    });
    if x.reverb {
        l["effects"] = json!([reverb(2.0, 0.3)]);
    }
    s["layers"] = json!([l]);
    s
}

fn ambience_score(title: &str, a: &Ambience) -> Value {
    let mut s = base_score(
        title,
        120.0,
        a.tonic,
        false,
        a.bars,
        json!({"mode": "one_shot", "tail_seconds": 2.0}),
    );
    let mut patterns = Vec::new();
    let mut layers = Vec::new();
    let steps = a.bars * 4;
    for (i, (kind, preset, params)) in a.layers.iter().enumerate() {
        let pid = format!("amb{i}_p");
        let root = 48 + a.tonic as i32;
        // One held note (two for textures: root and fifth) over the whole clip.
        let mut notes = vec![note(0, root, 0.7, steps as f64 - 0.5, "normal")];
        if *preset == "granular_texture" {
            notes.push(note(0, root + 7, 0.6, steps as f64 - 0.5, "normal"));
        }
        patterns.push(json!({"id": pid, "grid": "quarter", "length_steps": steps, "notes": notes}));
        let class = match *preset {
            "drone" => "drone",
            "granular_texture" => "texture",
            _ => "ambience",
        };
        let mut l = layer(&format!("amb{i}"), class, kind, preset, &pid, a.bars);
        let p: serde_json::Map<String, Value> = params
            .iter()
            .map(|(k, v)| (k.to_string(), json!(v)))
            .collect();
        l["instrument"]["params"] = Value::Object(p);
        if i > 0 {
            l["gain_db"] = json!(-6.0);
        }
        layers.push(l);
    }
    s["patterns"] = json!(patterns);
    s["layers"] = json!(layers);
    s
}

/// The Score JSON of an entry.
pub fn score_for(e: &Entry) -> Value {
    match &e.spec {
        Spec::Music(m) => music_score(&e.id, m),
        Spec::Sfx(x) => sfx_score(&e.id, x),
        Spec::Ambience(a) => ambience_score(&e.id, a),
    }
}

/// The label of an entry.
pub fn label_for(e: &Entry) -> Label {
    let (class, tempo, key, pack) = match &e.spec {
        Spec::Music(m) => (
            "music",
            Some(m.bpm),
            m.main.map(|_| {
                json!({"tonic": NAMES[m.tonic], "mode": if m.minor { "minor" } else { "major" }})
            }),
            m.main
                .filter(|(k, _)| *k == "sampler")
                .map(|_| "upright-piano-kw".to_string()),
        ),
        Spec::Sfx(_) => ("sfx", None, None, None),
        Spec::Ambience(_) => ("ambience", None, None, None),
    };
    Label {
        id: e.id.clone(),
        score: format!("{}.score.json", e.id),
        class: class.into(),
        tempo_bpm: tempo,
        key,
        sample_rate: e.sample_rate,
        post: e.post.clone(),
        needs_pack: pack,
        description: e.description.clone(),
    }
}

/// Applies the label's post-processing (deterministic noise seeded by the clip index).
pub fn apply_post(audio: &mut StereoBuffer, post: &Post, seed: u64) {
    let g = 10f32.powf(post.gain_db as f32 / 20.0);
    for ch in [&mut audio.left, &mut audio.right] {
        for v in ch.iter_mut() {
            *v *= g;
        }
    }
    if let Some(db) = post.noise_db {
        let n = audio.left.len().max(1);
        let e: f64 = audio
            .left
            .iter()
            .chain(audio.right.iter())
            .map(|v| (*v as f64).powi(2))
            .sum::<f64>()
            / (2 * n) as f64;
        let rms = e.sqrt();
        // Uniform noise in -a..a has RMS a / sqrt(3).
        let a = (rms * 10f64.powf(db / 20.0) * 3f64.sqrt()) as f32;
        let mut r = Pcg32::new(0x5eed_0000 + seed);
        for i in 0..audio.left.len() {
            audio.left[i] += a * r.next_bipolar();
            audio.right[i] += a * r.next_bipolar();
        }
    }
}
