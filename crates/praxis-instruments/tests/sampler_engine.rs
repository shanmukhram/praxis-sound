//! Sampler engine tests on a synthetic in-memory pack (`Packs::insert`), so CI covers the real
//! sampler code path without downloading anything: preset building, pitch, velocity layers and
//! crossfades, release samples with rt_decay, seeded round robin, loops, voice stealing, robustness.

mod sampler_common;

use praxis_engine::{Instrument, InstrumentContext, InstrumentFactory, NoteOn};
use praxis_instruments::Registry;
use praxis_instruments::sampler::engine::{SamplerInstrument, Voicing};
use praxis_instruments::sampler::presets::PIANO_PACK;
use praxis_instruments::sampler::program::Program;
use praxis_score::{InstrumentKind, InstrumentRef, Key, Mode, PitchClass};
use sampler_common::*;
use std::sync::Arc;

fn program() -> Arc<Program> {
    static PROGRAM: std::sync::OnceLock<Arc<Program>> = std::sync::OnceLock::new();
    PROGRAM
        .get_or_init(|| {
            let packs = synth_pack("synth");
            Arc::new(Program::load(&packs, "synth/sfz/synth_piano.sfz").unwrap())
        })
        .clone()
}

fn sampler(voicing: Voicing, voices: usize) -> SamplerInstrument {
    SamplerInstrument::new(program(), SR, voices, voicing, None, &[])
}

fn ctx() -> InstrumentContext {
    InstrumentContext {
        sample_rate: SR,
        max_block: BLOCK,
        key: Key {
            tonic: PitchClass::A,
            mode: Mode::Minor,
        },
        bpm: 128.0,
        seed: 1,
    }
}

#[test]
fn presets_build_from_an_in_memory_pack() {
    // The synthetic pack stands in for Upright Piano KW under the same name, so the real
    // `felt_piano` / `upright_piano` build functions run.
    let reg = Registry::new(synth_pack(PIANO_PACK));
    for preset in ["felt_piano", "upright_piano"] {
        let spec = InstrumentRef {
            kind: InstrumentKind::Sampler,
            preset: preset.into(),
            params: Default::default(),
        };
        let mut inst = reg.create(&spec, &ctx()).unwrap();
        for (n, expected) in [
            (57u8, 220.0f32),
            (60, 220.0 * 2f32.powf(0.25)),
            (50, 220.0 * 2f32.powf(-7.0 / 12.0)),
        ] {
            inst.reset();
            let len = (1.0 * SR) as usize;
            let (l, r) = render(inst.as_mut(), &[note(1, 0, n, 0.5, len)], len);
            let x = mono(&l, &r);
            let f = pitch_hz(&x[2400..len - 2400], SR, expected * 0.7, expected * 1.4);
            let err = cents(f, expected);
            println!("{preset} note {n}: {f:.3} Hz ({err:+.3} cents)");
            assert!(err.abs() < 5.0, "{preset} note {n}: {err:+.2} cents");
        }
        // Unknown parameters are rejected by the registry.
        let mut bad = spec.clone();
        bad.params.insert("nope".into(), 1.0);
        assert!(reg.create(&bad, &ctx()).is_err());
    }
}

#[test]
fn velocity_selects_layers_with_crossfade() {
    let mut s = sampler(Voicing::default(), 16);
    let on = |v: f32| NoteOn {
        id: 1,
        pitch: 57,
        velocity: v,
        brightness: 0.0,
        seed: 3,
        length_samples: 1000,
    };
    // Soft layer is 0..=64, hard 65..=127; the crossfade spans 64.5 +- 8.
    for (v, voices) in [
        (0.3f32, 1usize),
        (0.42, 1),
        (0.47, 2),
        (0.51, 2),
        (0.6, 1),
        (1.0, 1),
    ] {
        s.reset();
        s.note_on(&on(v));
        assert_eq!(s.active_voices(), voices, "velocity {v}");
    }
    // Without the crossfade there is always exactly one layer.
    let mut hard = sampler(
        Voicing {
            auto_xfade: false,
            ..Voicing::default()
        },
        16,
    );
    hard.note_on(&on(0.51));
    assert_eq!(hard.active_voices(), 1);

    // Sweeping velocity: the crossfade keeps level and brightness continuous.
    let sweep = |voicing: Voicing| {
        let mut out = Vec::new();
        for k in 0..=40 {
            let v = 0.35 + 0.3 * k as f32 / 40.0;
            let mut s = sampler(voicing, 16);
            let len = 9600;
            let (l, r) = render(&mut s, &[note(1, 0, 57, v, len)], len);
            out.push(rms(&mono(&l, &r)));
        }
        out.windows(2).map(|w| w[1] / w[0]).fold(0f32, f32::max)
    };
    let smooth = sweep(Voicing::default());
    let stepped = sweep(Voicing {
        auto_xfade: false,
        ..Voicing::default()
    });
    println!(
        "largest level step between adjacent velocities: crossfade {smooth:.3}, hard switch {stepped:.3}"
    );
    assert!(smooth < 1.3);
    assert!(stepped > smooth * 1.3);
    // The hard layer is brighter.
    let c = |v: f32| {
        let mut s = sampler(Voicing::default(), 16);
        let len = 16384;
        let (l, r) = render(&mut s, &[note(1, 0, 57, v, len)], len);
        centroid(&mono(&l, &r), SR, 16384)
    };
    assert!(c(0.95) > 1.5 * c(0.3));
}

#[test]
fn release_samples_follow_note_off_with_rt_decay() {
    let tail_level = |held_s: f32| {
        let mut s = sampler(Voicing::default(), 16);
        let held = (held_s * SR) as usize;
        let total = held + (1.0 * SR) as usize;
        let (l, r) = render(&mut s, &[note(1, 0, 57, 0.3, held)], total);
        // 0.5 s after note-off the 0.3 s main release is over (-90 dB at 0.45 s); what remains
        // is the release sample.
        let a = held + (0.5 * SR) as usize;
        rms(&mono(&l, &r)[a..a + 4800])
    };
    let short = tail_level(0.2);
    let long = tail_level(1.0);
    let db = 20.0 * (long / short).log10();
    println!("release sample after 0.2 s vs 1.0 s held: {db:+.2} dB (rt_decay 6 dB/s -> -4.8)");
    assert!(short > 1e-3);
    assert!((db + 4.8).abs() < 0.3);
    // The release voice starts at note-off, not before.
    let mut s = sampler(Voicing::default(), 16);
    s.note_on(&note(1, 0, 57, 0.3, 100).on);
    assert_eq!(s.active_voices(), 1);
    s.note_off(1);
    assert_eq!(s.active_voices(), 2);
    s.note_off(1); // a second note-off does nothing
    assert_eq!(s.active_voices(), 2);
}

#[test]
fn round_robin_uses_the_note_seed() {
    let c = |seed: u64| {
        let mut s = sampler(Voicing::default(), 16);
        let mut n = note(1, 0, 72, 0.7, 9600);
        n.on.seed = seed;
        let (l, r) = render(&mut s, &[n], 16384);
        (centroid(&mono(&l, &r), SR, 16384) * 10.0).round() as i32
    };
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..16 {
        let a = c(seed);
        assert_eq!(a, c(seed), "same seed, same sample");
        seen.insert(a);
    }
    assert_eq!(seen.len(), 2, "both round-robin samples are used: {seen:?}");
    // No running counter: a note sounds the same whether or not other notes came before it.
    let probe = note(9, 3 * SR as usize, 72, 0.7, 9600);
    let total = 4 * SR as usize;
    let mut s = sampler(Voicing::default(), 16);
    let alone = render(&mut s, &[probe], total);
    let mut s = sampler(Voicing::default(), 16);
    let mut before = vec![
        note(1, 0, 72, 0.7, 4800),
        note(2, 9600, 72, 0.7, 4800),
        note(3, 19200, 72, 0.7, 4800),
    ];
    before.push(probe);
    let after = render(&mut s, &before, total);
    let from = 3 * SR as usize;
    assert_eq!(alone.0[from..], after.0[from..]);
}

#[test]
fn loops_sustain_without_clicks() {
    let mut s = sampler(Voicing::default(), 16);
    let held = (4.0 * SR) as usize;
    let (l, r) = render(&mut s, &[note(1, 0, 69, 0.8, held)], held + 4800);
    let x = mono(&l, &r);
    // The sample is 2 s long; the loop keeps the level steady well past it.
    let early = rms(&x[(0.5 * SR) as usize..(0.7 * SR) as usize]);
    let late = rms(&x[(3.5 * SR) as usize..(3.7 * SR) as usize]);
    assert!((late / early - 1.0).abs() < 0.02, "{early} vs {late}");
    // Loop start is at 1.0 s of source (0.917 s at 48 kHz playback of a 44 kHz file); compare the
    // largest prediction residual before the first wrap with the whole held note.
    let before = max_residual(&x, 4800, (0.9 * SR) as usize);
    let all = max_residual(&x, 4800, held - 1);
    println!("loop residual: before first wrap {before:.2e}, whole note {all:.2e}");
    assert!(all < 1.2 * before);
    let f = pitch_hz(
        &x[(2.5 * SR) as usize..(3.5 * SR) as usize],
        SR,
        300.0,
        600.0,
    );
    assert!(cents(f, 440.0).abs() < 1.0);
}

#[test]
fn voice_stealing_fades_instead_of_cutting() {
    // 16 overlapping sustained notes through 4 voices: most are stolen.
    let notes: Vec<Note> = (0..16)
        .map(|i| note(i + 1, i as usize * 2400, 63 + (i % 8) as u8, 0.8, 48_000))
        .collect();
    let total = 16 * 2400 + 60_000;
    let mut few = sampler(Voicing::default(), 4);
    let (l, r) = render(&mut few, &notes, total);
    assert!(few.active_voices() <= 4 + 8);
    let x = mono(&l, &r);
    let mut many = sampler(Voicing::default(), 48);
    let (l2, r2) = render(&mut many, &notes, total);
    let y = mono(&l2, &r2);
    let stolen = max_residual(&x, 1, total);
    let free = max_residual(&y, 1, total);
    println!("max prediction residual: with stealing {stolen:.3e}, without {free:.3e}");
    assert!(stolen <= 1.2 * free, "stealing adds discontinuities");
}

#[test]
fn robust_against_odd_input() {
    let mut s = sampler(Voicing::default(), 4);
    s.note_off(12345); // unknown id
    let mut bad = note(1, 0, 57, f32::NAN, 100).on;
    bad.brightness = f32::INFINITY;
    s.note_on(&bad);
    s.note_on(&note(2, 0, 127, 1.0, 100).on); // no region for this key
    s.note_on(&note(3, 0, 0, 1.0, 100).on);
    s.set_param(99, 1.0);
    let mut l = vec![1.0f32; 256];
    let mut r = vec![1.0f32; 256];
    s.process(&mut l, &mut r);
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
    s.process(&mut [], &mut []);
    // all_notes_off releases; reset silences at once.
    s.note_on(&note(4, 0, 57, 0.8, 100).on);
    s.process(&mut l, &mut r);
    s.all_notes_off();
    s.process(&mut l, &mut r);
    assert!(l.iter().any(|v| *v != 0.0));
    s.reset();
    s.process(&mut l, &mut r);
    assert!(l.iter().chain(&r).all(|v| *v == 0.0));
    assert_eq!(s.active_voices(), 0);
}

#[test]
fn tail_covers_the_release() {
    let mut s = sampler(Voicing::default(), 8);
    let tail = s.tail_samples();
    // Release sample (1 s, may play an octave down) dominates: >= 2 s.
    assert!(tail >= (2.0 * SR) as usize, "{tail}");
    let held = 4800;
    let (l, r) = render(&mut s, &[note(1, 0, 57, 0.8, held)], held + tail + 4800);
    assert!(
        l[held + tail..]
            .iter()
            .chain(&r[held + tail..])
            .all(|v| *v == 0.0)
    );
    assert_eq!(s.active_voices(), 0);
}

#[test]
fn deterministic_renders() {
    let notes: Vec<Note> = (0..12)
        .map(|i| {
            note(
                i + 1,
                i as usize * 3000,
                [57u8, 60, 64, 69, 72][i as usize % 5],
                0.3 + 0.05 * i as f32,
                4500,
            )
        })
        .collect();
    let a = render(
        &mut sampler(Voicing::default(), 48),
        &notes,
        3 * SR as usize,
    );
    let b = render(
        &mut sampler(Voicing::default(), 48),
        &notes,
        3 * SR as usize,
    );
    assert!(
        a.0.iter()
            .zip(&b.0)
            .all(|(x, y)| x.to_bits() == y.to_bits())
    );
    assert!(
        a.1.iter()
            .zip(&b.1)
            .all(|(x, y)| x.to_bits() == y.to_bits())
    );
}
