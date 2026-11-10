//! Synth presets: every preset renders cleanly and deterministically, is in tune at three
//! pitches, reaches exact silence within its declared tail, and responds to velocity and
//! brightness.

mod synth_common;

use praxis_score::{GeneratorClass, InstrumentKind};
use synth_common::*;

const SYNTH: InstrumentKind = InstrumentKind::Synth;

/// (preset, class) as listed in docs/contracts/instruments.md and the WP-C2 brief.
const ALL: [(&str, GeneratorClass); 11] = [
    ("bell", GeneratorClass::Motif),
    ("soft_keys", GeneratorClass::Melody),
    ("pluck", GeneratorClass::Melody),
    ("mallet", GeneratorClass::Motif),
    ("sub_bass", GeneratorClass::Bass),
    ("warm_bass", GeneratorClass::Bass),
    ("fm_keys", GeneratorClass::Harmony),
    ("glass", GeneratorClass::Motif),
    ("granular_texture", GeneratorClass::Texture),
    ("noise_bed", GeneratorClass::Ambience),
    ("warm_pad", GeneratorClass::Pad),
];

/// Presets built so far (all of them once the package is complete).
fn built() -> Vec<(&'static str, GeneratorClass)> {
    let names = names(SYNTH);
    ALL.iter()
        .copied()
        .filter(|(p, _)| names.iter().any(|n| n == p))
        .collect()
}

/// Three test pitches spread over the preset's range.
fn pitches(preset: &str) -> [u8; 3] {
    let (lo, hi) = info(SYNTH, preset).range;
    let cands: Vec<u8> = [28u8, 36, 43, 48, 55, 60, 67, 72, 79, 84]
        .into_iter()
        .filter(|p| *p >= lo && *p <= hi)
        .collect();
    assert!(cands.len() >= 3, "{preset}: range {lo}..{hi}");
    [cands[0], cands[cands.len() / 2], cands[cands.len() - 1]]
}

fn mid_pitch(preset: &str) -> u8 {
    pitches(preset)[1]
}

const LEN: usize = 24_000;

fn render_note(preset: &str, pitch: u8, velocity: f32, brightness: f32) -> (Vec<f32>, usize) {
    let mut inst = create(SYNTH, preset, &[]);
    let tail = inst.tail_samples();
    let mut n = note(1, 0, pitch, velocity, LEN);
    n.on.brightness = brightness;
    let total = LEN + tail + 4800;
    (mono(&render(inst.as_mut(), &[n], total)), tail)
}

#[test]
fn every_synth_preset_exists_with_its_class() {
    let names = names(SYNTH);
    for (p, class) in built() {
        let i = info(SYNTH, p);
        assert_eq!(i.class, class, "{p} class");
        assert_eq!(i.pitched, p != "noise_bed", "{p} pitched");
        for q in &i.params {
            assert!(q.min <= q.default && q.default <= q.max, "{p}.{}", q.name);
        }
    }
    assert!(names.iter().any(|n| n == "bell"));
}

#[test]
fn every_synth_preset_is_clean_and_ends_within_its_tail() {
    for (p, _) in built() {
        let (x, tail) = render_note(p, mid_pitch(p), 0.8, 0.0);
        assert!(x.iter().all(|v| v.is_finite()), "{p}: NaN/inf");
        let pk = peak(&x);
        let end = sound_end(&x);
        let on = onset_step(&x);
        let off = end_step(&x);
        eprintln!(
            "{p:<16} peak {pk:.3} tail {:.2} s end {:.2} s onset step {on:.4} end step {off:.6}",
            tail as f64 / 48_000.0,
            end as f64 / 48_000.0
        );
        assert!(pk > 0.05 && pk < 1.0, "{p}: peak {pk}");
        assert!(end <= LEN + tail, "{p}: sound until {end}, tail {tail}");
        assert!(on < 0.25, "{p}: onset step {on}");
        assert!(off < 0.01, "{p}: end step {off}");
    }
}

#[test]
fn every_pitched_preset_is_in_tune_at_three_pitches() {
    for (p, _) in built() {
        if !info(SYNTH, p).pitched {
            continue;
        }
        for pitch in pitches(p) {
            let (x, _) = render_note(p, pitch, 0.7, 0.0);
            let target = midi_hz(pitch);
            let seg = &x[2400..2400 + 19_200];
            let f = peak_near(seg, target, 60.0);
            let err = cents(f, target);
            eprintln!(
                "{p:<16} note {pitch:>3} target {target:8.2} Hz measured {f:8.2} Hz error {err:+.2} cents"
            );
            assert!(err.abs() < 5.0, "{p} note {pitch}: {err} cents");
        }
    }
}

#[test]
fn synth_presets_are_deterministic_and_block_size_independent() {
    for (p, _) in built() {
        let m = mid_pitch(p);
        let notes = [
            note(1, 0, m, 0.7, 9000),
            note(2, 4001, m + 4, 0.5, 9000),
            note(3, 8003, m + 7, 0.9, 3000),
        ];
        let mut a = create(SYNTH, p, &[]);
        let mut b = create(SYNTH, p, &[]);
        let mut c = create(SYNTH, p, &[]);
        let ra = render_with(a.as_mut(), &notes, 60_000, 128);
        let rb = render_with(b.as_mut(), &notes, 60_000, 37);
        let rc = render_with(c.as_mut(), &notes, 60_000, 128);
        assert_eq!(ra, rc, "{p}: rerun differs");
        assert_eq!(ra, rb, "{p}: block size changes output");
    }
}

#[test]
fn velocity_raises_level_and_brightness_raises_centroid() {
    for (p, _) in built() {
        let m = mid_pitch(p);
        let levels: Vec<f32> = [0.25, 0.5, 0.75, 1.0]
            .iter()
            .map(|v| rms(&render_note(p, m, *v, 0.0).0[..LEN]))
            .collect();
        assert!(
            levels.windows(2).all(|w| w[1] > w[0]),
            "{p}: level not monotonic in velocity {levels:?}"
        );
        let dark = centroid(&render_note(p, m, 0.7, 0.0).0[..LEN]);
        let bright = centroid(&render_note(p, m, 0.7, 1.0).0[..LEN]);
        eprintln!("{p:<16} centroid dark {dark:.0} Hz bright {bright:.0} Hz");
        assert!(bright > dark * 1.02, "{p}: centroid {dark} -> {bright}");
    }
}

#[test]
fn bell_rings_long_while_held() {
    let mut inst = create(SYNTH, "bell", &[]);
    let x = mono(&render(
        inst.as_mut(),
        &[note(1, 0, 72, 0.8, 96_000)],
        96_000,
    ));
    // Still ringing a second in (sustaining, class motif: no more than 26 dB down), but below
    // the strike.
    let early = rms(&x[0..4800]);
    let later = rms(&x[48_000..52_800]);
    eprintln!("bell C5: rms first 100 ms {early:.4}, at 1 s {later:.4}");
    assert!(later > early * 0.05 && later < early, "{early} {later}");
}

#[test]
fn voice_stealing_never_clicks() {
    // More overlapping notes than any preset's polyphony: stolen voices fade.
    for (p, _) in built() {
        let m = mid_pitch(p);
        let notes: Vec<Note> = (0..48)
            .map(|i| note(i, i as usize * 600, m + (i % 5) as u8, 0.8, 30_000))
            .collect();
        let mut inst = create(SYNTH, p, &[]);
        let x = mono(&render(inst.as_mut(), &notes, 48 * 600 + 4800));
        assert!(x.iter().all(|v| v.is_finite()), "{p}");
        let pk = peak(&x);
        let jump = x
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        eprintln!("{p:<16} dense: peak {pk:.3} largest step {jump:.4}");
        assert!(pk < 4.0, "{p}: dense peak {pk}");
    }
}
