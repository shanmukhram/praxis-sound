//! Calibration set for the originality check (FR-18): close copies of seeded melodies must
//! fail, unrelated melodies must pass. Melodies are synthesised with the additive test tone (see
//! `features_synth`) and measured through the full profile, exactly as a reference clip and a
//! render would be.
//!
//! `cargo test -p praxis-analyze --release --test originality -- --nocapture` prints every pair.

mod features_synth;

use features_synth::{SR, add_tone, midi_hz};
use praxis_analyze::originality::{self, MELODIC_THRESHOLD, check};
use praxis_analyze::{ProfileOptions, SoundProfile, sound_profile};
use praxis_dsp::rng::Pcg32;

const MAJOR: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];

#[derive(Clone, Debug)]
struct Melody {
    /// (MIDI pitch, duration in eighths)
    notes: Vec<(i32, u32)>,
}

fn degree_pitch(tonic: i32, d: i32) -> i32 {
    tonic + 12 * d.div_euclid(7) + MAJOR[d.rem_euclid(7) as usize]
}

fn melody(seed: u64) -> Melody {
    let mut r = Pcg32::new(seed);
    let tonic = 55 + (r.next_u32() % 10) as i32;
    let steps = [-3, -2, -1, -1, 1, 1, 2, 2, 3, 4, -4, 0];
    let durs = [1u32, 1, 1, 2, 2, 3];
    let mut d = (r.next_u32() % 5) as i32;
    let mut notes = Vec::new();
    for _ in 0..16 {
        notes.push((degree_pitch(tonic, d), durs[(r.next_u32() % 6) as usize]));
        let mut s = steps[(r.next_u32() % steps.len() as u32) as usize];
        if s == 0 {
            s = 1;
        }
        d = (d + s).clamp(-3, 10);
    }
    Melody { notes }
}

/// Renders a melody: eighth = `eighth_s`, tone decay `decay_s`, transposed `shift` semitones.
fn render(m: &Melody, eighth_s: f64, decay_s: f64, shift: i32) -> Vec<f32> {
    let total: u32 = m.notes.iter().map(|n| n.1).sum();
    let len = ((total as f64 * eighth_s + 1.0) * SR as f64) as usize;
    let mut buf = vec![0.0f32; len];
    let mut t = 0.2;
    for (p, d) in &m.notes {
        let dur = *d as f64 * eighth_s;
        add_tone(
            &mut buf,
            SR,
            t,
            midi_hz((p + shift) as f64),
            0.4,
            decay_s,
            dur * 0.92,
            false,
        );
        t += dur;
    }
    buf
}

fn profile(buf: &[f32]) -> SoundProfile {
    sound_profile(buf, buf, SR, &ProfileOptions::default()).expect("profile")
}

#[test]
fn close_copies_fail_and_unrelated_melodies_pass() {
    let seeds: Vec<u64> = (1..=12).map(|i| 1000 + i * 17).collect();
    let mut copies: Vec<(String, f32)> = Vec::new();
    let mut unrelated: Vec<(String, f32)> = Vec::new();
    for (i, seed) in seeds.iter().enumerate() {
        let m = melody(*seed);
        let reference = profile(&render(&m, 0.3, 0.8, 0));
        let mut push = |name: String, other: &SoundProfile, copy: bool| {
            let r = check(&reference, other);
            let ia =
                originality::intervals(&originality::notes(&reference.pitch, &reference.onsets_s));
            let ib = originality::intervals(&originality::notes(&other.pitch, &other.onsets_s));
            let line = format!(
                "{name:<40} melodic {:.3} (n-gram {:.3}, dtw {:.3}) rhythmic {:.3}",
                r.melodic_similarity,
                originality::ngram_score(&ia, &ib),
                originality::dtw_similarity(&ia, &ib),
                r.rhythmic_similarity
            );
            eprintln!("{line}");
            if copy {
                copies.push((name, r.melodic_similarity));
            } else {
                unrelated.push((name, r.melodic_similarity));
            }
        };
        // Close copies.
        push(
            format!("{i:02} copy: transposed +5"),
            &profile(&render(&m, 0.3, 0.8, 5)),
            true,
        );
        push(
            format!("{i:02} copy: transposed -7, slower"),
            &profile(&render(&m, 0.38, 0.8, -7)),
            true,
        );
        push(
            format!("{i:02} copy: faster, staccato"),
            &profile(&render(&m, 0.24, 0.25, 0)),
            true,
        );
        let mut changed = m.clone();
        let mut r = Pcg32::new(seed + 1);
        for _ in 0..2 {
            let k = (r.next_u32() % 16) as usize;
            changed.notes[k].0 += if r.next_u32().is_multiple_of(2) { 2 } else { -2 };
        }
        push(
            format!("{i:02} copy: two notes changed"),
            &profile(&render(&changed, 0.3, 0.8, 0)),
            true,
        );
        let half = Melody {
            notes: m.notes[..9].to_vec(),
        };
        push(
            format!("{i:02} copy: first half"),
            &profile(&render(&half, 0.3, 0.8, 3)),
            true,
        );
        // Unrelated.
        let other = melody(seeds[(i + 1) % seeds.len()]);
        push(
            format!("{i:02} unrelated: another melody"),
            &profile(&render(&other, 0.3, 0.8, 0)),
            false,
        );
        let other2 = melody(seeds[(i + 5) % seeds.len()] + 3);
        push(
            format!("{i:02} unrelated: another, other tempo"),
            &profile(&render(&other2, 0.25, 0.5, 0)),
            false,
        );
        let mut same_rhythm = melody(seed + 99);
        for (k, n) in same_rhythm.notes.iter_mut().enumerate() {
            n.1 = m.notes[k].1;
        }
        push(
            format!("{i:02} unrelated: same rhythm, new pitches"),
            &profile(&render(&same_rhythm, 0.3, 0.8, 0)),
            false,
        );
    }
    let min_copy = copies.iter().map(|c| c.1).fold(f32::INFINITY, f32::min);
    let max_unrelated = unrelated
        .iter()
        .map(|c| c.1)
        .fold(f32::NEG_INFINITY, f32::max);
    let caught = copies.iter().filter(|c| c.1 >= MELODIC_THRESHOLD).count();
    let passed = unrelated.iter().filter(|c| c.1 < MELODIC_THRESHOLD).count();
    eprintln!("---");
    eprintln!(
        "least similar copy {min_copy:.3}, most similar unrelated {max_unrelated:.3}, threshold {MELODIC_THRESHOLD}"
    );
    eprintln!(
        "copies caught {caught} of {}; unrelated passed {passed} of {}",
        copies.len(),
        unrelated.len()
    );
    assert_eq!(caught, copies.len(), "a close copy passed: {copies:?}");
    assert_eq!(
        passed,
        unrelated.len(),
        "an unrelated melody failed: {unrelated:?}"
    );
}

#[test]
fn a_clip_without_melody_passes_with_a_reason() {
    let silent = vec![0.0f32; SR as usize * 2];
    let p = profile(&silent);
    let r = check(&p, &p);
    assert!(r.passed);
    assert!(r.reason.contains("not enough melody"), "{}", r.reason);
}
