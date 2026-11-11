//! The complete Sound Profile on synthetic signals: every field present and finite, the hash
//! agrees with the `blake3` crate, results are deterministic, and the character measures move
//! the right way (reverb, width, tonality, similarity).

mod features_synth;

use features_synth::{SR, add_click, add_noise_tail, add_tone, midi_hz};
use praxis_analyze::profile::{ClipKind, FEATURE_NAMES};
use praxis_analyze::similarity::similarity;
use praxis_analyze::{ProfileOptions, SoundProfile, sound_profile};
use praxis_dsp::StereoBuffer;

fn opts() -> ProfileOptions {
    ProfileOptions {
        file_name: "clip.wav".into(),
        ..ProfileOptions::default()
    }
}

/// Eight bars of an A-minor arpeggio on eighths at `bpm`, a click on every eighth (louder on
/// the beats), transposed `shift` semitones.
fn music(bpm: f64, shift: i32) -> Vec<f32> {
    let beat = 60.0 / bpm;
    let bars = 8;
    let mut buf = vec![0.0f32; ((bars as f64 * 4.0 * beat + 1.5) * SR as f64) as usize];
    let chords: [[i32; 3]; 4] = [[57, 60, 64], [62, 65, 69], [64, 67, 71], [57, 60, 64]];
    for bar in 0..bars {
        let c = chords[bar % 4];
        for e in 0..8 {
            let t = (bar * 4) as f64 * beat + e as f64 * beat / 2.0;
            let p = c[[0, 2, 1, 2, 0, 2, 1, 2][e]] + if e == 4 { 12 } else { 0 } + shift;
            let amp = if e % 2 == 0 { 0.35 } else { 0.28 };
            add_tone(
                &mut buf,
                SR,
                t,
                midi_hz(p as f64),
                amp,
                0.3,
                beat * 0.45,
                false,
            );
            let click = if e % 2 == 0 { 0.3 } else { 0.15 };
            add_click(&mut buf, SR, t, click, (bar * 8 + e) as u64);
        }
    }
    buf
}

fn mono_profile(buf: &[f32]) -> SoundProfile {
    sound_profile(buf, buf, SR, &opts()).expect("profile")
}

fn all_finite(p: &SoundProfile) {
    let v = serde_json::to_value(p).unwrap();
    fn walk(v: &serde_json::Value) {
        match v {
            serde_json::Value::Number(n) => assert!(n.as_f64().unwrap().is_finite()),
            serde_json::Value::Array(a) => a.iter().for_each(walk),
            serde_json::Value::Object(o) => o.values().for_each(walk),
            _ => {}
        }
    }
    walk(&v);
}

#[test]
fn music_profile_is_complete_and_plausible() {
    let buf = music(110.0, 0);
    let p = mono_profile(&buf);
    all_finite(&p);
    assert_eq!(p.features.len(), FEATURE_NAMES.len());
    assert_eq!(p.source.file_name, "clip.wav");
    assert_eq!(p.source.sample_rate, SR);
    assert_eq!(p.source.pcm_hash.len(), 64);
    let t = p.tempo.as_ref().expect("tempo");
    assert!((t.bpm - 110.0).abs() < 2.0, "{}", t.bpm);
    let k = p.key.as_ref().expect("key");
    assert_eq!(k.key.display(), "a minor");
    assert_eq!(p.class.kind, ClipKind::Music, "{:?}", p.class);
    let g = p.groove.as_ref().expect("groove");
    assert!(g.steps_per_bar >= 8, "{g:?}");
    assert!(g.swing < 0.2);
    assert!(p.tonality > 0.5, "{}", p.tonality);
    assert_eq!(p.stereo.width, 0.0);
    assert!((p.stereo.correlation - 1.0).abs() < 1e-6);
    assert_eq!(p.spectral.bands_db.len(), 10);
    assert!(p.envelope.frames_db.len() <= 2000 && !p.envelope.frames_db.is_empty());
    assert_eq!(
        p.envelope.frames_db.len(),
        p.spectral.centroid_track_hz.len()
    );
    assert!(p.pitch.midi.iter().flatten().count() > 100);
    assert!(!p.sections.is_empty());
    assert_eq!(p.sections[0].start_s, 0.0);
    assert!((p.sections.last().unwrap().end_s - p.source.duration_s).abs() < 1e-9);
}

#[test]
fn profile_is_deterministic_and_serialises() {
    let buf = music(96.0, 2);
    let a = mono_profile(&buf);
    let b = mono_profile(&buf);
    assert_eq!(a, b);
    let text = serde_json::to_string(&a).unwrap();
    let back: SoundProfile = serde_json::from_str(&text).unwrap();
    // f64 fields may differ in the last bit through JSON; everything else is exact.
    assert!((back.loudness.crest_db - a.loudness.crest_db).abs() < 1e-9);
    let mut back = back;
    back.loudness = a.loudness.clone();
    back.source.duration_s = a.source.duration_s;
    for (x, y) in back.onsets_s.iter_mut().zip(a.onsets_s.iter()) {
        assert!((*x - *y).abs() < 1e-9);
        *x = *y;
    }
    if let (Some(bt), Some(at)) = (back.tempo.as_mut(), a.tempo.as_ref()) {
        bt.bpm = at.bpm;
        bt.downbeat_s = at.downbeat_s;
        bt.beats_s = at.beats_s.clone();
    }
    for (x, y) in back.sections.iter_mut().zip(a.sections.iter()) {
        x.start_s = y.start_s;
        x.end_s = y.end_s;
    }
    assert_eq!(back, a);
}

#[test]
fn pcm_hash_agrees_with_the_blake3_crate() {
    // Long enough for a multi-level BLAKE3 tree (well over 1024-byte chunks).
    let buf: Vec<f32> = (0..300_000).map(|i| ((i as f32) * 0.001).sin()).collect();
    let right: Vec<f32> = buf.iter().map(|v| v * 0.5).collect();
    let ours = praxis_analyze::hash::pcm_hash(&buf, &right);
    let theirs = praxis_testkit::audio_hash(&StereoBuffer {
        left: buf.clone(),
        right: right.clone(),
    });
    assert_eq!(ours, theirs);
    for n in [0usize, 1, 255, 256, 257, 4096, 70_000] {
        let l = &buf[..n];
        let r = &right[..n];
        assert_eq!(
            praxis_analyze::hash::pcm_hash(l, r),
            praxis_testkit::audio_hash(&StereoBuffer {
                left: l.to_vec(),
                right: r.to_vec(),
            }),
            "{n} frames"
        );
    }
}

#[test]
fn silence_and_empty_clips_do_not_break() {
    for len in [0usize, 10, SR as usize * 3] {
        let buf = vec![0.0f32; len];
        let p = mono_profile(&buf);
        all_finite(&p);
        assert!(p.tempo.is_none() && p.key.is_none() && p.reverb.is_none());
        assert_eq!(p.features.len(), FEATURE_NAMES.len());
    }
}

#[test]
fn longer_decay_reads_longer_reverb() {
    let mut rts = Vec::new();
    for decay in [0.08, 0.3] {
        let mut buf = vec![0.0f32; SR as usize * 6];
        for k in 0..3 {
            add_noise_tail(
                &mut buf,
                SR,
                0.2 + 1.9 * k as f64,
                0.5,
                decay,
                1.8,
                false,
                k,
            );
        }
        let p = mono_profile(&buf);
        let r = p.reverb.expect("decays");
        // A time constant tau decays 60 dB in 6.9 tau (energy: 13.8 tau / 2).
        eprintln!(
            "tau {decay}: rt60 {:.2} (expected {:.2}) confidence {:.2}",
            r.rt60_s,
            6.91 * decay,
            r.confidence
        );
        assert!((r.rt60_s as f64 - 6.91 * decay).abs() / (6.91 * decay) < 0.25);
        rts.push(r.rt60_s);
    }
    assert!(rts[1] > rts[0] * 2.0);
}

#[test]
fn width_and_correlation_follow_the_channels() {
    let l = music(120.0, 0);
    let mut r = l.clone();
    // Delay the right channel by 7 ms: wide, less correlated.
    let d = (0.007 * SR as f64) as usize;
    r.rotate_right(d);
    let p = sound_profile(&l, &r, SR, &opts()).unwrap();
    assert!(p.stereo.width > 0.1, "{}", p.stereo.width);
    assert!(p.stereo.correlation < 0.9, "{}", p.stereo.correlation);
}

#[test]
fn similarity_prefers_the_same_character() {
    let a = mono_profile(&music(110.0, 0));
    let same_char = mono_profile(&music(112.0, 3));
    let mut noise = vec![0.0f32; SR as usize * 12];
    add_noise_tail(&mut noise, SR, 0.0, 0.3, 50.0, 12.0, false, 9);
    let n = mono_profile(&noise);
    let s_self = similarity(&a, &a);
    let s_close = similarity(&a, &same_char);
    let s_far = similarity(&a, &n);
    eprintln!("self {s_self:.3} close {s_close:.3} far {s_far:.3}");
    assert!((s_self - 1.0).abs() < 1e-6);
    assert!(s_close > 0.8, "{s_close}");
    assert!(s_far < s_close - 0.2, "{s_far} vs {s_close}");
    assert!((similarity(&a, &n) - similarity(&n, &a)).abs() < 1e-6);
}

#[test]
fn a_held_tone_has_a_pitch_track_and_long_sustain() {
    let mut buf = vec![0.0f32; SR as usize * 4];
    add_tone(&mut buf, SR, 0.1, midi_hz(64.0), 0.4, 50.0, 3.5, false);
    let p = mono_profile(&buf);
    let voiced: Vec<f32> = p.pitch.midi.iter().flatten().copied().collect();
    assert!(voiced.len() > 300);
    let mut v = voiced.clone();
    v.sort_by(|a, b| a.total_cmp(b));
    assert!((v[v.len() / 2] - 64.0).abs() < 0.1);
    assert!(p.envelope.sustain > 0.8, "{:?}", p.envelope);
    assert!(p.tonality > 0.8);
}
