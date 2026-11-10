//! Drum presets: every drum renders cleanly, deterministically and within its declared tail;
//! `soft_kick` is soft, low and short; hats are high; the kit follows the GM map and the closed
//! hat chokes the open one.

mod synth_common;

use praxis_engine::Instrument;
use praxis_score::{GeneratorClass, InstrumentKind};
use synth_common::*;

const DRUM: InstrumentKind = InstrumentKind::Drum;

const ALL: [&str; 12] = [
    "soft_kick",
    "kick",
    "snare",
    "rim",
    "clap",
    "hat_closed",
    "hat_open",
    "shaker",
    "tom_low",
    "tom_mid",
    "tom_high",
    "kit",
];

/// The note each preset is tested with (kit: its kick).
fn hit(preset: &str, id: u64, velocity: f32) -> Note {
    let pitch = if preset == "kit" { 36 } else { 60 };
    note(id, 0, pitch, velocity, 2400)
}

fn render_hit(preset: &str, velocity: f32, brightness: f32) -> (Vec<f32>, usize) {
    let mut inst = create(DRUM, preset, &[]);
    let tail = inst.tail_samples();
    let mut n = hit(preset, 1, velocity);
    n.on.brightness = brightness;
    let total = 2400 + tail + 4800;
    (mono(&render(inst.as_mut(), &[n], total)), tail)
}

#[test]
fn every_drum_preset_exists_with_its_class() {
    let names = names(DRUM);
    for p in ALL {
        assert!(names.iter().any(|n| n == p), "missing drum preset {p}");
        let i = info(DRUM, p);
        assert!(!i.pitched, "{p} is unpitched");
        assert!(!i.sustaining, "{p} is a one-shot");
        let expect = match p {
            "rim" | "clap" | "shaker" => GeneratorClass::Percussion,
            _ => GeneratorClass::Drums,
        };
        assert_eq!(i.class, expect, "{p} class");
        for q in &i.params {
            assert!(q.min <= q.default && q.default <= q.max, "{p}.{}", q.name);
        }
    }
}

#[test]
fn every_drum_is_clean_bounded_and_ends_within_its_tail() {
    for p in ALL {
        let (x, tail) = render_hit(p, 0.8, 0.0);
        assert!(x.iter().all(|v| v.is_finite()), "{p}: NaN/inf");
        let pk = peak(&x);
        let end = sound_end(&x);
        let on = onset_step(&x);
        let off = end_step(&x);
        eprintln!(
            "{p:<11} peak {pk:.3} tail {tail} end {end} onset step {on:.4} end step {off:.5}"
        );
        assert!(pk > 0.05 && pk < 1.0, "{p}: peak {pk}");
        assert!(end <= 2400 + tail, "{p}: sound until {end}, tail {tail}");
        assert!(on < 0.25, "{p}: onset step {on}");
        assert!(off < 0.01, "{p}: end step {off}");
    }
}

#[test]
fn drums_are_deterministic_and_block_size_independent() {
    for p in ALL {
        let notes = [hit(p, 1, 0.7), note(2, 3001, 60, 0.5, 100)];
        let notes = if p == "kit" {
            [notes[0], note(2, 3001, 42, 0.5, 100)]
        } else {
            notes
        };
        let mut a = create(DRUM, p, &[]);
        let mut b = create(DRUM, p, &[]);
        let ra = render_with(a.as_mut(), &notes, 40_000, 128);
        let rb = render_with(b.as_mut(), &notes, 40_000, 37);
        let mut c = create(DRUM, p, &[]);
        let rc = render_with(c.as_mut(), &notes, 40_000, 128);
        assert_eq!(ra, rc, "{p}: rerun differs");
        assert_eq!(ra, rb, "{p}: block size changes output");
    }
}

#[test]
fn velocity_raises_level_and_brightness_raises_centroid() {
    for p in ALL {
        let levels: Vec<f32> = [0.25, 0.5, 0.75, 1.0]
            .iter()
            .map(|v| rms(&render_hit(p, *v, 0.0).0))
            .collect();
        assert!(
            levels.windows(2).all(|w| w[1] > w[0]),
            "{p}: level not monotonic in velocity {levels:?}"
        );
        let dark = centroid(&render_hit(p, 0.7, 0.0).0);
        let bright = centroid(&render_hit(p, 0.7, 1.0).0);
        eprintln!("{p:<11} centroid dark {dark:.0} Hz bright {bright:.0} Hz");
        assert!(bright > dark * 1.02, "{p}: centroid {dark} -> {bright}");
    }
}

#[test]
fn soft_kick_is_low_short_and_muted() {
    let (x, tail) = render_hit("soft_kick", 0.8, 0.0);
    let pk = peak(&x);
    let low = band_fraction(&x, 0.0, 150.0);
    let high = band_fraction(&x, 4000.0, 24_000.0);
    // Time from the peak to the point after which the 10 ms RMS stays below -40 dB of the peak
    // window RMS.
    let win = 480;
    let rmss: Vec<f32> = x.chunks(win).map(rms).collect();
    let top = rmss.iter().cloned().fold(0.0f32, f32::max);
    let last_loud = rmss.iter().rposition(|r| *r > top * 0.01).unwrap_or(0);
    let decay_ms = (last_loud + 1) as f64 * 10.0;
    let f = peak_near(&x[4800..24_000], 55.0, 100.0);
    eprintln!(
        "soft_kick: peak {pk:.3}, energy <150 Hz {:.1} %, >4 kHz {:.4} %, -40 dB after {decay_ms} ms, \
         settles at {f:.2} Hz, tail {} ms",
        low * 100.0,
        high * 100.0,
        tail as f64 / 48.0
    );
    assert!(low > 0.85, "energy below 150 Hz {low}");
    assert!(high < 0.001, "energy above 4 kHz {high}");
    assert!(decay_ms <= 400.0, "-40 dB after {decay_ms} ms");
    assert!((f - 55.0).abs() < 2.0, "settling pitch {f}");
    assert!(pk < 0.8, "restrained peak {pk}");
}

#[test]
fn hats_are_high() {
    for p in ["hat_closed", "hat_open"] {
        let (x, _) = render_hit(p, 0.8, 0.0);
        let hi = band_fraction(&x, 5000.0, 24_000.0);
        eprintln!("{p}: energy above 5 kHz {:.1} %", hi * 100.0);
        assert!(hi > 0.8, "{p}: energy above 5 kHz {hi}");
    }
}

#[test]
fn kit_follows_the_gm_drum_map() {
    let map = [
        (36u8, "kick"),
        (35, "kick"),
        (37, "rim"),
        (38, "snare"),
        (40, "snare"),
        (39, "clap"),
        (42, "hat_closed"),
        (44, "hat_closed"),
        (46, "hat_open"),
        (41, "tom_low"),
        (45, "tom_mid"),
        (48, "tom_high"),
        (70, "shaker"),
    ];
    for (gm, preset) in map {
        let n = note(1, 0, gm, 0.8, 2400);
        let mut kit = create(DRUM, "kit", &[]);
        let mut single = create(DRUM, preset, &[]);
        let a = render(kit.as_mut(), &[n], 30_000);
        let b = render(single.as_mut(), &[n], 30_000);
        assert!(peak(&a.0) > 0.01, "kit note {gm} silent");
        assert_eq!(a, b, "kit note {gm} should play {preset}");
    }
    let mut kit = create(DRUM, "kit", &[]);
    let x = render(kit.as_mut(), &[note(1, 0, 60, 0.8, 2400)], 10_000);
    assert_eq!(peak(&x.0), 0.0, "unmapped note is silent");
}

#[test]
fn closed_hat_chokes_open_hat_in_kit() {
    let open = note(1, 0, 46, 0.8, 2400);
    let closed = note(2, 4800, 42, 0.8, 2400);
    let mut kit = create(DRUM, "kit", &[]);
    let alone = mono(&render(kit.as_mut(), &[open], 48_000));
    let mut kit = create(DRUM, "kit", &[]);
    let both = mono(&render(kit.as_mut(), &[open, closed], 48_000));
    // After the closed hat (and the open hat's 8 ms choke fade) only silence is left; alone, the
    // open hat still rings there.
    let after = 4800 + 2880;
    let ringing = rms(&alone[after..after + 2400]);
    let choked = peak(&both[after..]);
    eprintln!("open hat alone {ringing:.5} rms; after choke peak {choked}");
    assert!(ringing > 1e-3);
    assert_eq!(choked, 0.0);
    // The choke fades, never steps: the open hat's part of the mix (mix minus the closed hat
    // alone) is silent 8 ms after the closed hat, and jumps no more than it did before.
    let mut kit = create(DRUM, "kit", &[]);
    let closed_alone = mono(&render(kit.as_mut(), &[closed], 48_000));
    let open_part: Vec<f32> = both
        .iter()
        .zip(closed_alone.iter())
        .map(|(a, b)| a - b)
        .collect();
    let jumps = |x: &[f32]| {
        x.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max)
    };
    let before = jumps(&alone[4800 - 480..4800]);
    let during = jumps(&open_part[4799..4800 + 480]);
    eprintln!("open hat largest step before choke {before:.5}, during fade {during:.5}");
    assert!(during <= before * 1.1, "choke step {during} vs {before}");
    assert!(
        peak(&open_part[4800 + 400..]) < 1e-6,
        "open hat gone after the fade"
    );
}

#[test]
fn tail_samples_follow_decay_param() {
    let short = create(DRUM, "tom_low", &[("decay", 0.5)]).tail_samples();
    let long = create(DRUM, "tom_low", &[("decay", 2.0)]).tail_samples();
    assert!(long > 3 * short, "{short} -> {long}");
    let mut inst: Box<dyn Instrument> = create(DRUM, "tom_low", &[("decay", 2.0)]);
    let tail = inst.tail_samples();
    let x = mono(&render(
        inst.as_mut(),
        &[note(1, 0, 60, 1.0, 10)],
        tail + 9600,
    ));
    let end = sound_end(&x);
    assert!(
        end <= 10 + tail && end + 4800 > tail,
        "end {end} tail {tail}"
    );
}
