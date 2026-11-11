//! Reference clips through the app's commands (WP-U6): a dropped clip's bytes are kept under the
//! home folder and measured by the shared command layer (`ops::analyze`, the full Sound Profile).
//!
//! - a music clip rendered by the engine (the acceptance bell loop, 128 BPM), a sound effect
//!   (a short noise hit) and an ambience (a long, slowly moving noise bed) are told apart;
//! - a video that cannot be read says why with the decoder's own code, naming the clip, not
//!   the app's copy;
//! - the reference library keeps profiles only, named and tagged, and never audio.

use praxis_app::clips::{self, CLIPS_DIR};
use praxis_engine::testing::TestFactory;
use praxis_engine::{RenderOptions, render_offline};
use serde_json::Value;
use std::path::{Path, PathBuf};

const SR: u32 = 48_000;

fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("praxis-app-clips-{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::path::absolute(dir).unwrap()
}

/// A 32-bit float stereo WAV in memory (what a person would drop).
fn wav(l: &[f32], r: &[f32], sr: u32) -> Vec<u8> {
    let n = l.len().min(r.len());
    let data = (n * 8) as u32;
    let mut b = Vec::with_capacity(44 + n * 8);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&sr.to_le_bytes());
    b.extend_from_slice(&(sr * 8).to_le_bytes());
    b.extend_from_slice(&8u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for i in 0..n {
        b.extend_from_slice(&l[i].to_le_bytes());
        b.extend_from_slice(&r[i].to_le_bytes());
    }
    b
}

/// The acceptance bell loop (128 BPM, A minor) rendered by the engine.
fn music_clip() -> Vec<u8> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/acceptance1_bell.score.json");
    let score = praxis_score::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    let out = render_offline(
        &score,
        &TestFactory::new(),
        &RenderOptions {
            sample_rate: SR,
            ..RenderOptions::default()
        },
    )
    .expect("render");
    wav(&out.master.left, &out.master.right, SR)
}

/// A deterministic noise source (xorshift), -1..1.
struct Noise(u32);
impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// A short hit: a noise burst with a sharp attack and a fast decay, then silence (0.8 s).
fn sfx_clip() -> Vec<u8> {
    let mut n = Noise(0x1234_5678);
    let len = (SR as f32 * 0.8) as usize;
    let (mut l, mut r) = (vec![0.0; len], vec![0.0; len]);
    let mut lp = 0.0f32;
    for i in 0..len {
        let t = i as f32 / SR as f32;
        lp += 0.35 * (n.next() - lp);
        let x = lp * 0.9 * (-t * 14.0).exp();
        l[i] = x;
        r[i] = x;
    }
    wav(&l, &r, SR)
}

/// A rain-like bed: 12 s of soft, slowly moving, decorrelated noise with no beat.
fn ambience_clip() -> Vec<u8> {
    let (mut a, mut b) = (Noise(0x0bad_cafe), Noise(0x00c0_ffee));
    let len = SR as usize * 12;
    let (mut l, mut r) = (vec![0.0; len], vec![0.0; len]);
    let (mut la, mut lb) = (0.0f32, 0.0f32);
    for i in 0..len {
        let t = i as f32 / SR as f32;
        la += 0.2 * (a.next() - la);
        lb += 0.2 * (b.next() - lb);
        let g = 0.12 * (1.0 + 0.15 * (t * 0.31 * std::f32::consts::TAU).sin());
        l[i] = la * g;
        r[i] = lb * g;
    }
    wav(&l, &r, SR)
}

fn kind(p: &Value) -> &str {
    p["class"]["kind"].as_str().unwrap_or("")
}

#[test]
fn a_music_clip_an_effect_and_an_ambience_are_measured_and_told_apart() {
    let home = scratch("kinds");
    let music = clips::analyze_clip(&home, "bell loop.wav", &music_clip()).expect("music");
    let sfx = clips::analyze_clip(&home, "hit.wav", &sfx_clip()).expect("sfx");
    let amb = clips::analyze_clip(&home, "rain bed.wav", &ambience_clip()).expect("ambience");
    for c in [&music, &sfx, &amb] {
        println!(
            "{}: {} ({:.2}), tempo {}, key {}, {:.1} LUFS, centroid {:.0} Hz",
            c.name,
            kind(&c.profile),
            c.profile["class"]["confidence"].as_f64().unwrap_or(0.0),
            c.profile["tempo"]["bpm"],
            c.profile["key"]["key"],
            c.profile["loudness"]["integrated_lufs"].as_f64().unwrap_or(f64::NAN),
            c.profile["spectral"]["centroid_hz"].as_f64().unwrap_or(f64::NAN),
        );
        assert!(Path::new(&c.path).is_file(), "the clip is kept for the assistant");
        assert!(c.path.starts_with(&home.join(CLIPS_DIR).display().to_string()));
        assert!(c.profile["loudness"]["integrated_lufs"].is_number());
    }
    assert_eq!(music.name, "bell loop.wav");
    assert_eq!(kind(&music.profile), "music");
    let bpm = music.profile["tempo"]["bpm"].as_f64().expect("a tempo");
    assert!((bpm - 128.0).abs() <= 2.0, "music tempo {bpm}");
    assert_eq!(kind(&sfx.profile), "sfx");
    assert_eq!(kind(&amb.profile), "ambience");
    assert!((amb.profile["source"]["duration_s"].as_f64().unwrap() - 12.0).abs() < 0.05);
}

#[test]
fn the_library_keeps_named_tagged_profiles_and_never_audio() {
    let home = scratch("library");
    let rain = clips::analyze_clip(&home, "rain bed.wav", &ambience_clip()).expect("ambience");
    assert!(clips::profile_cache(Path::new(&rain.path)).is_file(), "the profile is cached");

    // Saved from the kept clip: its cached profile, named and tagged.
    let e = clips::library_add(&home, " Night rain ", &["rain".into(), "night".into()], Some(&rain.path), None)
        .expect("add");
    assert_eq!((e.id.as_str(), e.name.as_str()), ("night-rain", "Night rain"));
    assert_eq!(e.tags, ["rain", "night"]);
    assert_eq!(e.kind, "ambience");
    assert!(!e.already);
    assert!(e.summary.starts_with("ambience"), "{}", e.summary);
    let p = e.profile.as_ref().expect("the saved profile");
    assert_eq!(p["source"]["pcm_hash"], rain.profile["source"]["pcm_hash"]);
    assert_eq!(p["style_name"], "Night rain");

    // The same clip again is the same entry (by its decoded PCM hash), with the new tag.
    let again = clips::library_add(&home, "Night rain", &["bed".into()], Some(&rain.path), None).unwrap();
    assert!(again.already);
    assert_eq!(again.id, "night-rain");
    assert_eq!(again.tags, ["rain", "night", "bed"]);

    // A profile the window holds (its kept clip gone) is saved as it is.
    let hit = clips::analyze_clip(&home, "hit.wav", &sfx_clip()).unwrap();
    let e2 = clips::library_add(&home, "Hit", &[], Some("C:\\elsewhere\\hit.wav"), Some(&hit.profile)).unwrap();
    assert_eq!((e2.id.as_str(), e2.kind.as_str()), ("hit", "sfx"));

    let list = clips::library_list(&home).unwrap();
    assert_eq!(list.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["hit", "night-rain"]);
    assert!(list.iter().all(|e| e.profile.is_none()), "a listing carries no profiles");

    let t = clips::library_tag(&home, "night-rain", &["loop".into(), "rain".into()]).unwrap();
    assert_eq!(t.tags, ["rain", "loop"]);
    assert_eq!(clips::library_show(&home, "night-rain").unwrap().tags, ["rain", "loop"]);

    // Profiles only: every file in the library is a profile JSON, far smaller than the audio.
    for f in std::fs::read_dir(clips::library_dir(&home)).unwrap().flatten() {
        let name = f.file_name().to_string_lossy().into_owned();
        assert!(name.ends_with(".profile.json"), "{name} in the library");
        let text = std::fs::read_to_string(f.path()).unwrap();
        assert!(!text.starts_with("RIFF"));
    }
    assert!(clips::library_remove(&home, "hit").unwrap());
    assert_eq!(clips::library_list(&home).unwrap().len(), 1);
    let gone = clips::library_show(&home, "hit").unwrap_err();
    assert_eq!(gone.code, "io");
    assert_eq!(
        clips::library_add(&home, "  ", &[], Some(&rain.path), None).unwrap_err().code,
        "bad_json"
    );
}

#[test]
fn a_video_that_cannot_be_read_says_why_and_names_the_clip() {
    let home = scratch("video");
    let e = clips::analyze_clip(&home, "scene.mp4", b"not really a video").unwrap_err();
    assert!(
        ["ffmpeg_missing", "ffmpeg_failed", "decode_failed"].contains(&e.code.as_str()),
        "{e}"
    );
    assert!(e.message.contains("scene.mp4"), "{e}");
    assert!(!e.message.contains(CLIPS_DIR), "the app's copy leaked into: {e}");
}

#[test]
fn files_that_are_not_clips_are_refused_before_measuring() {
    let home = scratch("refuse");
    let e = clips::analyze_clip(&home, "notes.txt", b"hello").unwrap_err();
    assert_eq!(e.code, "bad_format");
    assert!(!home.join(CLIPS_DIR).exists() || std::fs::read_dir(home.join(CLIPS_DIR)).unwrap().count() == 0);
}
