//! Chroma and key estimation on synthetic tonal material in known keys.

mod features_synth;

use features_synth::*;
use praxis_analyze::features::chroma::chromagram;
use praxis_analyze::features::key::{estimate_key, mode_family};
use praxis_score::{Key, Mode, PitchClass};

/// Arpeggiated cadence (I-IV-V-I or i-iv-V-i with a raised leading tone) over a sustained bass,
/// then an ascending scale, twice. Eighth notes at 100 BPM, additive tones with decays.
fn cadence_and_scale(key: Key, sr: u32, detune_semitones: f64) -> Vec<f32> {
    let minor = key.mode == Mode::Minor;
    let t = 57 + (key.tonic.semitone() as i32 - 9).rem_euclid(12);
    let third = if minor { 3 } else { 4 };
    let sixth = if minor { 8 } else { 9 };
    // (root offset, third, fifth) per chord; V is major in both modes.
    let chords: [(i32, i32, i32); 4] = [
        (0, third, 7),
        (5, 5 + sixth - 5, 12),
        (7, 11, 14),
        (0, third, 7),
    ];
    let scale: Vec<i32> = if minor {
        vec![0, 2, 3, 5, 7, 8, 10, 12]
    } else {
        vec![0, 2, 4, 5, 7, 9, 11, 12]
    };
    let eighth = 60.0 / 100.0 / 2.0;
    let bar = 8.0 * eighth;
    let bars_per_pass = chords.len() + 1;
    let total = 2.0 * bars_per_pass as f64 * bar + 1.0;
    let mut buf = vec![0.0f32; (total * sr as f64) as usize];
    let hz = |p: i32| midi_hz(p as f64 + detune_semitones);
    for pass in 0..2 {
        let t0 = pass as f64 * bars_per_pass as f64 * bar;
        for (ci, (r, th, fi)) in chords.iter().enumerate() {
            let b0 = t0 + ci as f64 * bar;
            add_tone(&mut buf, sr, b0, hz(t + r - 12), 0.25, 0.8, bar, false);
            let arp = [*r, *th, *fi, r + 12, *fi, *th, *r, *th];
            for (i, p) in arp.iter().enumerate() {
                let v = if i == 0 { 0.3 } else { 0.22 };
                add_tone(
                    &mut buf,
                    sr,
                    b0 + i as f64 * eighth,
                    hz(t + p),
                    v,
                    0.3,
                    eighth * 1.5,
                    false,
                );
            }
        }
        let b0 = t0 + chords.len() as f64 * bar;
        for (i, d) in scale.iter().enumerate() {
            let len = if i == scale.len() - 1 {
                bar
            } else {
                eighth * 1.2
            };
            add_tone(
                &mut buf,
                sr,
                b0 + i as f64 * eighth,
                hz(t + d),
                0.25,
                0.3,
                len,
                false,
            );
        }
    }
    buf
}

const KEYS: [(PitchClass, Mode); 10] = [
    (PitchClass::A, Mode::Minor),
    (PitchClass::C, Mode::Major),
    (PitchClass::Fs, Mode::Minor),
    (PitchClass::Eb, Mode::Major),
    (PitchClass::D, Mode::Major),
    (PitchClass::G, Mode::Minor),
    (PitchClass::Bb, Mode::Major),
    (PitchClass::E, Mode::Minor),
    (PitchClass::B, Mode::Major),
    (PitchClass::Cs, Mode::Minor),
];

fn detect(sig: &[f32], sr: u32) -> (Key, f32, f32, f32) {
    let ch = chromagram(sig, sr);
    let r = estimate_key(&ch.mean()).expect("key");
    let mut sorted = r.scores;
    sorted.sort_by(|a, b| b.total_cmp(a));
    (
        r.key,
        r.confidence,
        sorted[0] - sorted[1],
        ch.tuning_semitones,
    )
}

#[test]
fn cadences_in_known_keys() {
    for (tonic, mode) in KEYS {
        let k = key(tonic, mode);
        let sig = cadence_and_scale(k, SR, 0.0);
        let (got, conf, margin, tuning) = detect(&sig, SR);
        println!(
            "{:<10} -> {:<10} confidence {conf:.2}, margin {margin:.3}, tuning {:+.1} cents",
            k.display(),
            got.display(),
            tuning * 100.0
        );
        assert_eq!(got, k);
    }
}

#[test]
fn detuned_material_keeps_its_key() {
    for (tonic, mode, cents) in [
        (PitchClass::A, Mode::Minor, 35.0),
        (PitchClass::Eb, Mode::Major, -40.0),
        (PitchClass::Fs, Mode::Minor, 45.0),
    ] {
        let k = key(tonic, mode);
        let sig = cadence_and_scale(k, 44_100, cents / 100.0);
        let (got, conf, _, tuning) = detect(&sig, 44_100);
        println!(
            "{:<10} detuned {cents:+.0} cents -> {} (confidence {conf:.2}, tuning estimate {:+.1} cents)",
            k.display(),
            got.display(),
            tuning * 100.0
        );
        assert_eq!(got, k);
        assert!((tuning as f64 * 100.0 - cents).abs() < 5.0);
    }
}

#[test]
fn relative_major_is_not_minor() {
    // C major and A minor share pitch classes; the cadence decides.
    let c = detect(
        &cadence_and_scale(key(PitchClass::C, Mode::Major), SR, 0.0),
        SR,
    )
    .0;
    let a = detect(
        &cadence_and_scale(key(PitchClass::A, Mode::Minor), SR, 0.0),
        SR,
    )
    .0;
    assert_eq!(c, key(PitchClass::C, Mode::Major));
    assert_eq!(a, key(PitchClass::A, Mode::Minor));
}

#[test]
fn silence_has_no_key() {
    let ch = chromagram(&vec![0.0; 48_000], SR);
    assert!(estimate_key(&ch.mean()).is_none());
}

#[test]
fn mode_families() {
    assert_eq!(mode_family(Mode::Dorian), Some(Mode::Minor));
    assert_eq!(mode_family(Mode::Mixolydian), Some(Mode::Major));
    assert_eq!(mode_family(Mode::Chromatic), None);
}
