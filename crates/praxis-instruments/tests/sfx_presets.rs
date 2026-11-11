//! Sound-effect presets, measured on real renders through the `Instrument` path (registry ->
//! `create` -> `note_on` / `process`, blocks split at events like the engine): every preset is
//! clean (no NaN, no DC, no click at start or end), pitched presets are in tune and in key, and
//! each macro moves its property monotonically: `length` the duration (within 1%), `brightness`
//! the spectral centroid, `weight` the energy below 150 Hz, `pitch` the tuned frequency.

mod synth_common;

use praxis_score::{GeneratorClass, InstrumentKind};
use synth_common::*;

const SFX: InstrumentKind = InstrumentKind::Sfx;

const ALL: [(&str, GeneratorClass, bool, bool); 6] = [
    ("whoosh", GeneratorClass::Whoosh, true, false),
    ("impact", GeneratorClass::Impact, true, false),
    ("riser", GeneratorClass::Riser, true, false),
    ("drone", GeneratorClass::Drone, true, true),
    ("glitch", GeneratorClass::Glitch, true, false),
    ("tick", GeneratorClass::Tick, false, false),
];

const MACROS: [&str; 4] = ["length", "brightness", "weight", "pitch"];
const STEPS: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];
const SR_F: f64 = SR as f64;

/// Default test note: A3 (the tonic of the test key, A minor).
const A3: u8 = 57;
/// Note length for drones and risers.
const HELD: usize = 96_000;

fn ms(x: f64) -> usize {
    (x * SR_F / 1000.0).round() as usize
}

/// Renders one note of `preset` with macro overrides. Returns the mono render and the note
/// length used. One-shots get a short note (they ignore note-offs); risers and drones are held.
fn render_one(
    preset: &str,
    params: &[(&str, f64)],
    pitch: u8,
    seed: u64,
    len: usize,
) -> (Vec<f32>, Vec<f32>) {
    let mut inst = create(SFX, preset, params);
    let tail = inst.tail_samples();
    let mut n = note(1, 0, pitch, 0.8, len);
    n.on.seed = seed;
    let total = len + tail + 2400;
    render(inst.as_mut(), &[n], total)
}

fn note_len(preset: &str) -> usize {
    match preset {
        "riser" | "drone" => HELD,
        _ => 2400,
    }
}

fn mono_one(preset: &str, params: &[(&str, f64)], pitch: u8) -> Vec<f32> {
    mono(&render_one(preset, params, pitch, 0x5EED, note_len(preset)))
}

/// Energy (sum of squares of the spectrum) below `hz`.
fn energy_below(x: &[f32], hz: f64) -> f64 {
    let n = 8192;
    let p = power_spectrum(x, n);
    let bin = SR_F / n as f64;
    p.iter()
        .enumerate()
        .filter(|(k, _)| (*k as f64 * bin) < hz)
        .map(|(_, v)| v)
        .sum()
}

fn first_sound(x: &[f32]) -> usize {
    x.iter().position(|v| *v != 0.0).unwrap_or(x.len())
}

/// Click at the end: the last sample before silence relative to the peak, and the
/// high-frequency (first difference) energy of the last 2 ms relative to the 20 ms before.
fn end_click(x: &[f32]) -> (f32, f32) {
    let end = sound_end(x);
    let pk = peak(x).max(1e-12);
    let last = x[end - 1].abs() / pk;
    let d: Vec<f32> = x[..end + 1].windows(2).map(|w| w[1] - w[0]).collect();
    let n2 = ms(2.0).min(d.len() / 4);
    let n20 = ms(20.0).min(d.len() / 2);
    let tail = rms(&d[d.len() - n2..]);
    let before = rms(&d[d.len() - n2 - n20..d.len() - n2]);
    (last, tail / before.max(1e-12))
}

/// Where a sound peaks: the end (exclusive) of its loudest 25 ms, by RMS in a window sliding one
/// sample at a time. The window spans a full period of the lowest tone a riser plays (the
/// 41 Hz sub), so neither the waveform of low tones nor single noise spikes move the peak;
/// a riser "peaks at its end" when this window ends within 5 ms of the sound's end.
fn loudness_peak(x: &[f32]) -> usize {
    let w = ms(25.0);
    let mut acc: f64 = x[..w].iter().map(|v| (*v as f64) * (*v as f64)).sum();
    let (mut best, mut best_end) = (acc, w);
    for end in w + 1..=x.len() {
        let a = x[end - 1] as f64;
        let b = x[end - 1 - w] as f64;
        acc += a * a - b * b;
        if acc > best {
            best = acc;
            best_end = end;
        }
    }
    best_end
}

/// Expected scale-step transposition in A minor (natural minor), like the presets do.
fn a_minor_step(pitch: u8, steps: i32) -> u8 {
    let scale = [0, 2, 3, 5, 7, 8, 10];
    let in_key = |p: i32| scale.contains(&((p - 9).rem_euclid(12)));
    let mut p = pitch as i32;
    for _ in 0..steps.abs() {
        p += steps.signum();
        while !in_key(p) {
            p += steps.signum();
        }
    }
    p as u8
}

/// Length macro for tuning measurements: the longest take for `glitch`, the default otherwise.
fn glitch_len(preset: &str) -> f64 {
    if preset == "glitch" { 1.0 } else { 0.5 }
}

fn pitch_steps(p: f64) -> i32 {
    ((p - 0.5) * 14.0).round() as i32
}

#[test]
fn every_sfx_preset_exists_with_its_class_and_macros() {
    let names = names(SFX);
    assert_eq!(names.len(), ALL.len(), "{names:?}");
    for (p, class, pitched, sustaining) in ALL {
        assert!(names.iter().any(|n| n == p), "missing {p}");
        let i = info(SFX, p);
        assert_eq!(i.kind, SFX);
        assert_eq!(i.class, class, "{p} class");
        assert_eq!(i.pitched, pitched, "{p} pitched");
        assert_eq!(i.sustaining, sustaining, "{p} sustaining");
        let params: Vec<&str> = i.params.iter().map(|q| q.name.as_str()).collect();
        assert_eq!(params, MACROS, "{p} macros");
        for q in &i.params {
            assert_eq!((q.min, q.max, q.default), (0.0, 1.0, 0.5), "{p}.{}", q.name);
        }
    }
}

#[test]
fn every_sfx_preset_is_clean_without_dc_or_clicks() {
    for (p, ..) in ALL {
        let (l, r) = render_one(p, &[], A3, 0x5EED, note_len(p));
        for (ch, x) in [("left", &l), ("right", &r)] {
            assert!(x.iter().all(|v| v.is_finite()), "{p} {ch}: NaN/inf");
            let pk = peak(x);
            let mean = x.iter().map(|v| *v as f64).sum::<f64>() / sound_end(x) as f64;
            let dc = mean.abs() / pk as f64;
            let on = onset_step(x);
            let (last, hf) = end_click(x);
            eprintln!(
                "{p:<7} {ch:<5} peak {pk:.3} dc {dc:.5} onset step {on:.4} last {last:.5} end hf {hf:.3} length {:.3} s",
                sound_end(x) as f64 / SR_F
            );
            assert!(pk > 0.05 && pk < 0.9, "{p}: peak {pk}");
            assert!(dc < 0.002, "{p} {ch}: DC {dc} of peak");
            assert!(on < 0.25, "{p} {ch}: onset step {on}");
            assert!(last < 0.01, "{p} {ch}: last sample {last} of peak");
            assert!(hf < 1.5, "{p} {ch}: high-frequency burst at the end {hf}");
        }
    }
}

#[test]
fn sfx_presets_are_deterministic_per_seed_and_block_size_independent() {
    for (p, ..) in ALL {
        let len = note_len(p);
        let notes = |seed: u64| {
            let mut a = note(1, 0, A3, 0.8, len);
            let mut b = note(2, 7001, 64, 0.6, len);
            a.on.seed = seed;
            b.on.seed = seed ^ 0xABCD;
            [a, b]
        };
        let total = 7001 + len + create(SFX, p, &[]).tail_samples() + 100;
        let run = |seed: u64, block: usize| {
            let mut inst = create(SFX, p, &[]);
            render_with(inst.as_mut(), &notes(seed), total, block)
        };
        let a = run(11, 128);
        assert_eq!(a, run(11, 128), "{p}: rerun differs");
        assert_eq!(a, run(11, 37), "{p}: block size changes output");
        assert_ne!(a, run(12, 128), "{p}: a different seed gives the same take");
    }
}

#[test]
fn pitched_presets_land_on_the_requested_note() {
    // (preset, notes, analysis window in samples)
    let cases: [(&str, [u8; 3], (usize, usize)); 4] = [
        ("drone", [36, 48, 60], (ms(1500.0), HELD)),
        ("riser", [45, 57, 69], (0, HELD)),
        ("glitch", [45, 57, 69], (0, ms(2000.0))),
        // Impact bodies sit on the tonic: test with tonic notes (A1, A2, A3).
        ("impact", [33, 45, 57], (ms(100.0), ms(700.0))),
    ];
    for (p, notes, (from, to)) in cases {
        for pitch in notes {
            // The glitch is measured over its longest (2 s) take, for frequency resolution.
            let x = mono_one(p, &[("weight", 0.0), ("length", glitch_len(p))], pitch);
            let target = midi_hz(pitch);
            let f = peak_near(&x[from..to], target, 60.0);
            let err = cents(f, target);
            eprintln!(
                "{p:<7} note {pitch:>3} target {target:8.2} Hz measured {f:8.2} Hz error {err:+.2} cents"
            );
            assert!(err.abs() < 5.0, "{p} note {pitch}: {err} cents");
        }
    }
    // Whoosh: the tonal core passes through the note at the pass point (60% of the length);
    // measured on a slow (4 s) pass whose Doppler bend is gentle, bright so the noise sits far
    // above the core, no weight layer.
    for pitch in [45u8, 57, 69] {
        let x = mono_one(
            "whoosh",
            &[("length", 1.0), ("brightness", 1.0), ("weight", 0.0)],
            pitch,
        );
        let pass = (4.0 * SR_F * 0.6).round() as usize;
        let w = ms(100.0);
        let target = midi_hz(pitch);
        let f = peak_near(&x[pass - w..pass + w], target, 60.0);
        let err = cents(f, target);
        eprintln!(
            "whoosh  note {pitch:>3} target {target:8.2} Hz measured {f:8.2} Hz error {err:+.2} cents (at the pass)"
        );
        assert!(err.abs() < 5.0, "whoosh note {pitch}: {err} cents");
    }
}

#[test]
fn impact_body_is_on_the_tonic_whatever_the_note() {
    // C3, E3, G2 in A minor: bodies on the nearest A.
    for (pitch, tonic) in [(48u8, 45u8), (52, 57), (43, 45)] {
        let x = mono_one("impact", &[("weight", 1.0)], pitch);
        let target = midi_hz(tonic);
        let f = peak_near(&x[ms(100.0)..ms(700.0)], target, 60.0);
        let err = cents(f, target);
        eprintln!("impact note {pitch} -> body {tonic}: {f:.2} Hz ({err:+.2} cents)");
        assert!(
            err.abs() < 5.0,
            "impact note {pitch}: {err} cents from the tonic"
        );
    }
}

#[test]
fn length_sets_the_duration_within_one_percent() {
    let bpm = 120.0;
    for (p, ..) in ALL {
        let mut last = 0.0;
        for v in STEPS {
            let (x, expected) = match p {
                "riser" => {
                    // A note longer than the longest build: the riser waits, then builds.
                    let len = (17.0 * 60.0 / bpm * SR_F) as usize;
                    let x = mono(&render_one(p, &[("length", v)], A3, 1, len));
                    let beats = 16f64.powf(v);
                    (x, beats * 60.0 / bpm)
                }
                "drone" => {
                    let x = mono_one(p, &[("length", v)], A3);
                    (x, 0.25 * 32f64.powf(v))
                }
                _ => {
                    let x = mono_one(p, &[("length", v)], A3);
                    let (lo, hi): (f64, f64) = match p {
                        "whoosh" => (0.25, 4.0),
                        "impact" => (0.5, 8.0),
                        "glitch" => (0.08, 2.0),
                        _ => (0.010, 0.250),
                    };
                    (x, lo * (hi / lo).powf(v))
                }
            };
            let measured = match p {
                "riser" => (sound_end(&x) - first_sound(&x)) as f64 / SR_F,
                "drone" => (sound_end(&x) - HELD) as f64 / SR_F,
                _ => sound_end(&x) as f64 / SR_F,
            };
            let err = measured / expected - 1.0;
            eprintln!(
                "{p:<7} length {v:.2}: expected {expected:.4} s measured {measured:.4} s ({:+.3}%)",
                100.0 * err
            );
            assert!(
                err.abs() < 0.01,
                "{p} length {v}: {measured} s vs {expected} s"
            );
            assert!(
                measured > last,
                "{p}: duration not increasing at length {v}"
            );
            last = measured;
        }
    }
}

#[test]
fn brightness_raises_the_spectral_centroid() {
    for (p, ..) in ALL {
        let c: Vec<f64> = STEPS
            .iter()
            .map(|v| centroid(&mono_one(p, &[("brightness", *v)], A3)))
            .collect();
        eprintln!(
            "{p:<7} centroid by brightness: {:?}",
            c.iter().map(|v| v.round()).collect::<Vec<_>>()
        );
        assert!(
            c.windows(2).all(|w| w[1] > w[0] * 1.02),
            "{p}: centroid not rising {c:?}"
        );
    }
}

#[test]
fn weight_raises_the_energy_below_150_hz() {
    for (p, ..) in ALL {
        let e: Vec<f64> = STEPS
            .iter()
            .map(|v| energy_below(&mono_one(p, &[("weight", *v)], A3), 150.0))
            .collect();
        let db: Vec<f64> = e.iter().map(|v| 10.0 * v.max(1e-30).log10()).collect();
        eprintln!(
            "{p:<7} energy < 150 Hz by weight (dB): {:?}",
            db.iter()
                .map(|v| (v * 10.0).round() / 10.0)
                .collect::<Vec<_>>()
        );
        assert!(
            e.windows(2).all(|w| w[1] > w[0] * 1.1),
            "{p}: low energy not rising {db:?}"
        );
    }
}

#[test]
fn pitch_moves_tuned_parts_by_scale_steps_and_noise_continuously() {
    // Tuned presets: the measured fundamental follows the key's scale steps.
    for (p, (from, to)) in [
        ("drone", (ms(1500.0), HELD)),
        ("riser", (0, HELD)),
        ("glitch", (0, ms(2000.0))),
    ] {
        for v in STEPS {
            let expected = a_minor_step(A3, pitch_steps(v));
            let x = mono_one(
                p,
                &[("pitch", v), ("weight", 0.0), ("length", glitch_len(p))],
                A3,
            );
            let target = midi_hz(expected);
            let f = peak_near(&x[from..to], target, 60.0);
            let err = cents(f, target);
            eprintln!(
                "{p:<7} pitch {v:.2}: expected MIDI {expected} ({target:.2} Hz), error {err:+.2} cents"
            );
            assert!(
                err.abs() < 5.0,
                "{p} pitch {v}: {err} cents from MIDI {expected}"
            );
        }
    }
    // Impact: octave down, fifth below, tonic, fifth above, octave up.
    for (v, body) in STEPS.iter().zip([33u8, 40, 45, 52, 57]) {
        let x = mono_one("impact", &[("pitch", *v), ("weight", 1.0)], 45);
        let target = midi_hz(body);
        let f = peak_near(&x[ms(100.0)..ms(700.0)], target, 60.0);
        let err = cents(f, target);
        eprintln!("impact  pitch {v:.2}: body MIDI {body}, error {err:+.2} cents");
        assert!(err.abs() < 5.0, "impact pitch {v}: {err} cents");
    }
    // Every preset: the centroid rises with pitch.
    for (p, ..) in ALL {
        let c: Vec<f64> = STEPS
            .iter()
            .map(|v| centroid(&mono_one(p, &[("pitch", *v), ("weight", 0.0)], A3)))
            .collect();
        eprintln!(
            "{p:<7} centroid by pitch: {:?}",
            c.iter().map(|v| v.round()).collect::<Vec<_>>()
        );
        assert!(
            c.windows(2).all(|w| w[1] > w[0]),
            "{p}: centroid not rising with pitch {c:?}"
        );
    }
}

#[test]
fn riser_ends_on_the_note_end_and_peaks_within_5_ms_of_it() {
    // Peak = the loudest 25 ms (see `loudness_peak`).
    // Note lengths around and below the 4-beat (2 s) default build, at odd sample counts.
    for (i, len) in [96_001usize, 150_017, 60_013, 30_011]
        .into_iter()
        .enumerate()
    {
        let start = 1000 + 37 * i;
        let mut inst = create(SFX, "riser", &[]);
        let n = note(1, start, A3, 0.8, len);
        let x = mono(&render(inst.as_mut(), &[n], start + len + 4800));
        let end = start + len;
        let last = sound_end(&x);
        let before = (end - loudness_peak(&x)) as f64 * 1000.0 / SR_F;
        let build = (last - first_sound(&x)) as f64 / SR_F;
        eprintln!(
            "riser note {len} samples: sound ends at {last} (note end {end}), loudest 25 ms ends {before:.2} ms before the end, build {build:.3} s"
        );
        assert_eq!(last, end, "riser must end on the note's last sample");
        assert!(before <= 5.0, "riser peak {before} ms before its end");
    }
}

#[test]
fn riser_peak_is_at_its_end_for_any_seed_and_note() {
    // The noise is random per seed: check the worst case over many takes, notes and macros.
    let len = 48_000usize;
    let (mut worst, mut worst_sample): (f64, f64) = (0.0, 0.0);
    for seed in 0..40u64 {
        for (k, pitch) in [33u8, 45, 57, 69, 81].into_iter().enumerate() {
            let params = [
                ("length", 0.25),
                ("brightness", [0.0, 0.25, 0.5, 0.75, 1.0][k]),
                (
                    "weight",
                    [1.0, 0.0, 0.5, 0.25, 0.75][(k + seed as usize) % 5],
                ),
            ];
            let mut inst = create(SFX, "riser", &params);
            let mut n = note(1, 0, pitch, 0.8, len);
            n.on.seed = seed * 7919 + k as u64;
            let (l, r) = render(inst.as_mut(), &[n], len + 480);
            for x in [&l, &r] {
                let ms_before = (len - loudness_peak(x)) as f64 * 1000.0 / SR_F;
                if ms_before > 5.0 {
                    eprintln!("  seed {seed} pitch {pitch} {params:?}: {ms_before:.2} ms");
                }
                worst = worst.max(ms_before);
                let at = x
                    .iter()
                    .enumerate()
                    .fold(
                        (0, 0.0f32),
                        |m, (j, v)| if v.abs() > m.1 { (j, v.abs()) } else { m },
                    )
                    .0;
                worst_sample = worst_sample.max((len - at) as f64 * 1000.0 / SR_F);
            }
        }
    }
    eprintln!(
        "riser over 400 takes: loudest 25 ms ends at most {worst:.2} ms before the end (largest single sample at most {worst_sample:.2} ms before)"
    );
    assert!(worst <= 5.0, "riser peak {worst} ms before its end");
}

#[test]
fn drone_loops_exactly_with_its_note_length() {
    // A 2 s drone note repeated back to back (the engine's loop wrap does the same): once the
    // first note's release has passed, every cycle is identical and the joins are seamless.
    let len = 96_000usize;
    for v in [0.2, 0.5] {
        let mut inst = create(SFX, "drone", &[("length", v)]);
        let notes: Vec<Note> = (0..4)
            .map(|k| {
                let mut n = note(k, k as usize * len, 48, 0.8, len);
                n.on.seed = 42; // same note key every cycle, as in a loop
                n
            })
            .collect();
        let (l, r) = render(inst.as_mut(), &notes, 4 * len);
        let mut worst = 0.0f32;
        for x in [&l, &r] {
            for t in 0..len {
                worst = worst.max((x[2 * len + t] - x[len + t]).abs());
            }
        }
        let pk = peak(&l);
        // The level never dips at a join (attack + release sum to the steady drone).
        let level = |x: &[f32], at: usize| rms(&x[at - 2400..at + 2400]);
        let join = level(&l, 2 * len);
        let mid = level(&l, 2 * len + len / 2);
        eprintln!(
            "drone length {v}: cycle-to-cycle difference {worst:.2e} (peak {pk:.3}), level at join {join:.4} vs mid-cycle {mid:.4}"
        );
        assert!(worst < 1e-4 * pk, "drone cycles differ by {worst}");
        assert!(
            (join / mid - 1.0).abs() < 0.25,
            "level dips at the join: {join} vs {mid}"
        );
    }
}

#[test]
fn ticks_keep_time_exactly() {
    // Eighth notes at 120 BPM: every tick starts on its note's first sample, peaks within 2 ms,
    // and the seed changes only the level slightly.
    let step = (0.25 * SR_F) as usize;
    let notes: Vec<Note> = (0..8)
        .map(|k| note(k, 100 + k as usize * step, 60, 0.8, 1200))
        .collect();
    let mut inst = create(SFX, "tick", &[]);
    let x = mono(&render(inst.as_mut(), &notes, 100 + 8 * step + 4800));
    let mut peaks = Vec::new();
    for k in 0..8 {
        let at = 100 + k * step;
        let seg = &x[at - 50..at + step - 50];
        let first = first_sound(seg) + at - 50;
        assert_eq!(first, at, "tick {k} starts at {first}, note at {at}");
        let pk =
            seg.iter().enumerate().fold(
                (0, 0.0f32),
                |m, (j, v)| if v.abs() > m.1 { (j, v.abs()) } else { m },
            );
        let delay_ms = (pk.0 + at - 50 - at) as f64 * 1000.0 / SR_F;
        assert!(delay_ms < 2.0, "tick {k} peaks {delay_ms} ms late");
        peaks.push(pk.1);
    }
    let lo = peaks.iter().cloned().fold(f32::INFINITY, f32::min);
    let hi = peaks.iter().cloned().fold(0.0, f32::max);
    eprintln!("tick peaks {peaks:?}");
    assert!(hi / lo < 1.5, "tick level spread {lo}..{hi}");
}

#[test]
fn transport_stop_ends_every_preset_within_its_tail_without_a_click() {
    // `all_notes_off` half a second in (mid-build for the riser, mid-attack for the drone).
    for (p, ..) in ALL {
        let mut inst = create(SFX, p, &[]);
        let tail = inst.tail_samples();
        let mut n = note(1, 0, A3, 0.8, HELD).on;
        n.seed = 3;
        inst.note_on(&n);
        let stop = 24_000;
        let total = stop + tail + 4800;
        let mut l = vec![0f32; total];
        let mut r = vec![0f32; total];
        let mut t = 0;
        while t < total {
            if t == stop {
                inst.all_notes_off();
            }
            let end = if t < stop {
                (t + 128).min(stop)
            } else {
                (t + 128).min(total)
            };
            inst.process(&mut l[t..end], &mut r[t..end]);
            t = end;
        }
        let end = sound_end(&l).max(sound_end(&r));
        let jump = l
            .windows(2)
            .skip(stop.saturating_sub(10))
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        let before = l[stop - 2400..stop]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        eprintln!(
            "{p:<7} stopped at {stop}: silent from {end} (tail {tail}), largest step after {jump:.4} vs before {before:.4}"
        );
        assert!(end <= stop + tail, "{p}: sounds until {end}, tail {tail}");
        assert!(
            jump <= before * 1.5 + 1e-6,
            "{p}: step {jump} after the stop"
        );
    }
}
