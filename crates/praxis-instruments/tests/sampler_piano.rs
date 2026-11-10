//! Renders through the real sampler from the installed Upright Piano KW pack. Each test skips
//! with a printed message when the pack is not installed (CI installs it first).

mod sampler_common;

use praxis_engine::{Instrument, InstrumentContext, InstrumentError, InstrumentFactory};
use praxis_instruments::sampler::presets::{PIANO_PACK, PIANO_RANGE, load_pack_preset};
use praxis_instruments::sampler::region::{SampleRef, Trigger};
use praxis_instruments::{Packs, Registry};
use praxis_score::{InstrumentKind, InstrumentRef, Key, Mode, PitchClass};
use sampler_common::*;
use std::collections::BTreeMap;

fn ctx() -> InstrumentContext {
    InstrumentContext {
        sample_rate: SR,
        max_block: BLOCK,
        key: Key {
            tonic: PitchClass::A,
            mode: Mode::Minor,
        },
        bpm: 128.0,
        seed: 7,
    }
}

fn create(reg: &Registry, preset: &str, params: &[(&str, f64)]) -> Box<dyn Instrument> {
    let spec = InstrumentRef {
        kind: InstrumentKind::Sampler,
        preset: preset.to_string(),
        params: params
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect::<BTreeMap<_, _>>(),
    };
    reg.create(&spec, &ctx()).expect("create instrument")
}

#[test]
fn missing_pack_is_reported() {
    let reg = Registry::new(Packs::none());
    for preset in ["felt_piano", "upright_piano"] {
        let spec = InstrumentRef {
            kind: InstrumentKind::Sampler,
            preset: preset.into(),
            params: BTreeMap::new(),
        };
        match reg.create(&spec, &ctx()) {
            Err(InstrumentError::PackMissing(p)) => assert_eq!(p, PIANO_PACK),
            Err(e) => panic!("expected PackMissing, got {e}"),
            Ok(_) => panic!("expected PackMissing, got an instrument"),
        }
    }
}

#[test]
fn pack_range_and_layers() {
    let Some(packs) = installed_piano("pack_range_and_layers") else {
        return;
    };
    let t0 = std::time::Instant::now();
    let prog = load_pack_preset(&packs, PIANO_PACK, "felt_piano").unwrap();
    println!("load + decode: {:.2} s", t0.elapsed().as_secs_f64());
    println!(
        "regions {} samples {} decoded MB {:.1} warnings {:?}",
        prog.regions.len(),
        prog.samples.len(),
        prog.sample_bytes() as f64 / 1e6,
        prog.warnings
    );
    println!(
        "sample rate {} stereo {}",
        prog.samples[0].sample_rate,
        prog.samples[0].is_stereo()
    );
    assert_eq!(prog.key_range(), Some(PIANO_RANGE));
    assert_eq!(prog.regions.len(), 66);
    assert!(prog.regions.iter().all(|r| r.trigger == Trigger::Attack));
    // Every key has exactly one soft (<= 80) and one hard (>= 81) region.
    for k in PIANO_RANGE.0..=PIANO_RANGE.1 {
        let soft = prog
            .regions
            .iter()
            .filter(|r| r.matches_key(k) && r.hivel == 80)
            .count();
        let hard = prog
            .regions
            .iter()
            .filter(|r| r.matches_key(k) && r.lovel == 81)
            .count();
        assert_eq!((soft, hard), (1, 1), "key {k}");
    }
    let reg = Registry::new(packs);
    let info = reg.describe(InstrumentKind::Sampler, "felt_piano").unwrap();
    assert_eq!(info.range, PIANO_RANGE);
}

/// Fundamental of the source sample a key plays (at its own rate), and the region's keycenter.
fn source_pitch(packs: &Packs, key: u8, vel: u8) -> (f32, u8) {
    let prog = load_pack_preset(packs, PIANO_PACK, "upright_piano").unwrap();
    let i = prog
        .regions
        .iter()
        .position(|r| r.matches_key(key) && r.matches_vel(vel))
        .unwrap();
    let r = &prog.regions[i];
    let s = &prog.samples[prog.sample_of[i]];
    let sr = s.sample_rate;
    let a = (0.15 * sr) as usize;
    let b = (a + (0.85 * sr) as usize).min(s.frames());
    let x: Vec<f32> = (a..b)
        .map(|j| {
            let (l, rr) = s.frame(j);
            0.5 * (l + rr)
        })
        .collect();
    let f_expected = midi_hz(r.pitch_keycenter as f32);
    (
        pitch_hz(&x, sr, f_expected * 0.7, f_expected * 1.4),
        r.pitch_keycenter,
    )
}

#[test]
fn pitch_within_5_cents_across_range() {
    let Some(packs) = installed_piano("pitch_within_5_cents_across_range") else {
        return;
    };
    let reg = Registry::new(packs.clone());
    let notes = [24u8, 33, 40, 47, 52, 57, 62, 66, 70, 76, 83, 89, 96, 101];
    let mut worst = 0f32;
    for &n in &notes {
        let (f_src, kc) = source_pitch(&packs, n, 50);
        let expected = f_src * 2f32.powf((n as f32 - kc as f32) / 12.0);
        let mut inst = create(&reg, "upright_piano", &[]);
        let len = (1.3 * SR) as usize;
        let (l, r) = render(inst.as_mut(), &[note(1, 0, n, 50.0 / 127.0, len)], len);
        let x = mono(&l, &r);
        let seg = &x[(0.15 * SR) as usize..(1.0 * SR) as usize];
        let f = pitch_hz(seg, SR, expected * 0.7, expected * 1.4);
        let err = cents(f, expected);
        println!(
            "note {n:3} (keycenter {kc:3}): measured {f:9.3} Hz, expected {expected:9.3} Hz, \
             error {err:+.2} cents; vs equal temperament {:+.1} cents",
            cents(f, midi_hz(n as f32))
        );
        worst = worst.max(err.abs());
        assert!(err.abs() < 5.0, "note {n}: {err:+.2} cents");
    }
    println!("worst pitch error {worst:.2} cents");
}

#[test]
fn felt_is_darker_than_upright() {
    let Some(packs) = installed_piano("felt_is_darker_than_upright") else {
        return;
    };
    let reg = Registry::new(packs);
    for &(n, v) in &[(48u8, 0.6f32), (60, 0.6), (72, 0.6), (60, 0.95)] {
        let len = (1.0 * SR) as usize;
        let mut c = [0f32; 2];
        for (i, preset) in ["felt_piano", "upright_piano"].iter().enumerate() {
            let mut inst = create(&reg, preset, &[]);
            let (l, r) = render(inst.as_mut(), &[note(1, 0, n, v, len)], len);
            c[i] = centroid(&mono(&l, &r), SR, 32768);
        }
        println!(
            "note {n} velocity {v}: centroid felt {:.0} Hz, upright {:.0} Hz",
            c[0], c[1]
        );
        assert!(c[0] < 0.92 * c[1], "felt not darker at note {n}");
    }
}

#[test]
fn bright_accent_opens_tone() {
    let Some(packs) = installed_piano("bright_accent_opens_tone") else {
        return;
    };
    let reg = Registry::new(packs);
    let len = (0.8 * SR) as usize;
    let mut c = [0f32; 2];
    for (i, b) in [0.0f32, 1.0].iter().enumerate() {
        let mut inst = create(&reg, "felt_piano", &[]);
        let mut nt = note(1, 0, 64, 0.8, len);
        nt.on.brightness = *b;
        let (l, r) = render(inst.as_mut(), &[nt], len);
        c[i] = centroid(&mono(&l, &r), SR, 32768);
    }
    println!("felt centroid normal {:.0} Hz, bright {:.0} Hz", c[0], c[1]);
    assert!(c[1] > 1.05 * c[0]);
}

#[test]
fn no_clicks_at_note_start_or_note_off() {
    let Some(packs) = installed_piano("no_clicks_at_note_start_or_note_off") else {
        return;
    };
    let reg = Registry::new(packs);
    let pre = 2400;
    let mut worst_on = 0f32;
    let mut worst_off = 0f32;
    let mut worst_control = f32::INFINITY;
    let mut worst_control_on = f32::INFINITY;
    for preset in ["felt_piano", "upright_piano"] {
        for &(n, v) in &[
            (33u8, 0.5f32),
            (45, 0.9),
            (57, 0.3),
            (69, 1.0),
            (81, 0.6),
            (96, 0.9),
        ] {
            let held = (0.35 * SR) as usize;
            let total = pre + held + (2.5 * SR) as usize;
            let mut inst = create(&reg, preset, &[]);
            let (l, r) = render(inst.as_mut(), &[note(1, pre, n, v, held)], total);
            for x in [&l, &r] {
                assert!(x[..pre].iter().all(|&s| s == 0.0));
                let on = onset_click(x, pre);
                let off = release_click(x, pre + held);
                worst_on = worst_on.max(on);
                worst_off = worst_off.max(off);
                assert!(on < 0.5, "{preset} note {n}: onset click metric {on:.3}");
                assert!(
                    off < 1.5,
                    "{preset} note {n}: release click metric {off:.3}"
                );
            }
            // Negative controls: the same audio cut in hard and cut off hard must fail.
            let x = &l;
            // Worst-case hard cut-in: the sustained tone switched on at its loudest sample.
            let c = (pre + 4800..pre + 4800 + 240)
                .max_by(|&a, &b| x[a].abs().partial_cmp(&x[b].abs()).unwrap())
                .unwrap();
            let mut cut_in = vec![0f32; pre];
            cut_in.extend_from_slice(&x[c..c + 4800]);
            let control_on = onset_click(&cut_in, pre);
            worst_control_on = worst_control_on.min(control_on);
            assert!(
                control_on > 0.5,
                "control: hard cut-in not detected ({control_on:.2})"
            );
            let (cut_off, at) = hard_cut(x, pre + held);
            let control = release_click(&cut_off, at);
            worst_control = worst_control.min(control);
            assert!(
                control > 3.0,
                "control: hard cut not detected ({control:.2})"
            );
        }
    }
    println!(
        "worst onset metric {worst_on:.3} (limit 0.5; weakest hard cut-in control {worst_control_on:.2}), worst note-off metric {worst_off:.3} (limit 1.5; weakest hard-cut control {worst_control:.2}, must exceed 3)"
    );
}

#[test]
fn release_tail_decays_to_silence() {
    let Some(packs) = installed_piano("release_tail_decays_to_silence") else {
        return;
    };
    let reg = Registry::new(packs);
    for (preset, release) in [("felt_piano", 1.2f32), ("upright_piano", 0.6)] {
        let mut inst = create(&reg, preset, &[]);
        let held = (0.5 * SR) as usize;
        let tail = inst.tail_samples();
        let total = held + tail + 4800;
        let (l, r) = render(inst.as_mut(), &[note(1, 0, 52, 0.7, held)], total);
        let x = mono(&l, &r);
        let win = 2400;
        let before = rms(&x[held - win..held]);
        let mut prev = before;
        let mut t = held;
        while t + win <= held + (release * SR) as usize {
            let now = rms(&x[t..t + win]);
            assert!(now <= prev * 1.05, "{preset}: tail grew at {t}");
            prev = now;
            t += win;
        }
        let at_release =
            rms(&x[held + (release * SR) as usize - win..held + (release * SR) as usize]);
        let end = rms(&x[held + tail - win..held + tail]);
        println!(
            "{preset}: tail {:.2} s, level at release time {:.1} dB, at tail end {:.1} dB (re pre-off)",
            tail as f32 / SR,
            20.0 * (at_release / before).log10(),
            20.0 * (end.max(1e-12) / before).log10()
        );
        assert!(
            at_release < before * 0.01,
            "{preset}: not -40 dB by release time"
        );
        assert!(end < before * 1e-3, "{preset}: tail end not -60 dB");
        assert!(
            x[held + tail..].iter().all(|&s| s == 0.0),
            "{preset}: sound after tail"
        );
    }
}

/// A repeating eighth-note motif with accents and overlapping sustain (acceptance test 1 shape).
fn motif() -> Vec<Note> {
    let step = (60.0 / 128.0 / 2.0 * SR) as usize; // eighth notes at 128 BPM
    let pitches = [57u8, 64, 60, 64, 69, 64, 60, 62];
    let vels = [0.95f32, 0.45, 0.6, 0.45, 0.85, 0.45, 0.6, 0.5];
    let mut out = Vec::new();
    for rep in 0..2 {
        for (i, (&p, &v)) in pitches.iter().zip(&vels).enumerate() {
            let k = rep * 8 + i;
            let mut n = note(k as u64 + 1, k * step, p, v, step * 3 / 2);
            n.on.brightness = if i == 4 { 1.0 } else { 0.0 };
            out.push(n);
        }
    }
    out
}

/// Onset error (ms, positive = late) of every note of an eighth-note motif at 128 BPM with
/// overlapping sustain. Each note is isolated from the motif render by subtraction (the motif
/// minus the motif without that note; voices sum linearly and none are stolen), then its onset
/// is found with a simple envelope detector: the first time a centred 1 ms RMS envelope reaches
/// -15 dB of the note's peak in its first 80 ms (the relative threshold at which listeners place
/// the onset of a tone, Vos & Rasch 1981).
fn motif_onset_errors(reg: &Registry, preset: &str) -> Vec<(u8, f32, f32)> {
    let step = (60.0 / 128.0 / 2.0 * SR) as usize;
    let pre = 4800;
    // Two passes over the keyboard's middle and the motif register, soft to accented.
    let pitches = [
        45u8, 57, 60, 64, 69, 64, 60, 62, 48, 52, 55, 59, 67, 72, 76, 81, 50, 53, 58, 63, 66, 70,
        74, 84,
    ];
    let vels = [0.95f32, 0.45, 0.6, 0.45, 0.85, 0.45, 0.6, 0.5];
    let notes: Vec<Note> = pitches
        .iter()
        .enumerate()
        .map(|(k, &p)| {
            let mut n = note(k as u64 + 1, pre + k * step, p, vels[k % 8], step * 3 / 2);
            n.on.brightness = if k % 8 == 4 { 1.0 } else { 0.0 };
            n
        })
        .collect();
    let total = pre + notes.len() * step + (2.5 * SR) as usize;
    let full = {
        let mut inst = create(reg, preset, &[]);
        let (l, r) = render(inst.as_mut(), &notes, total);
        mono(&l, &r)
    };
    let mut out = Vec::new();
    for k in 0..notes.len() {
        let others: Vec<Note> = notes
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != k)
            .map(|(_, n)| *n)
            .collect();
        let mut inst = create(reg, preset, &[]);
        let (l, r) = render(inst.as_mut(), &others, total);
        let rest = mono(&l, &r);
        let alone: Vec<f32> = full.iter().zip(&rest).map(|(a, b)| a - b).collect();
        let t = notes[k].at;
        let onset = envelope_onset(&alone, t, SR);
        out.push((
            notes[k].on.pitch,
            notes[k].on.velocity,
            (onset as f32 - t as f32) / SR * 1000.0,
        ));
    }
    out
}

#[test]
fn notes_sound_on_their_note_times() {
    let Some(packs) = installed_piano("notes_sound_on_their_note_times") else {
        return;
    };
    let reg = Registry::new(packs);
    let mut results = Vec::new();
    for preset in ["felt_piano", "upright_piano"] {
        let errs = motif_onset_errors(&reg, preset);
        let mut abs: Vec<f32> = errs.iter().map(|e| e.2.abs()).collect();
        abs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = abs[abs.len() / 2];
        let worst = *abs.last().unwrap();
        let line: Vec<String> = errs
            .iter()
            .map(|(p, _, e)| format!("{p}:{e:+.1}"))
            .collect();
        println!("{preset} onset error ms (note:error): {}", line.join(" "));
        println!("{preset}: median |error| {median:.2} ms, worst {worst:.2} ms");
        results.push((preset, median, worst));
    }
    for (preset, median, worst) in results {
        assert!(median < 3.0, "{preset}: median onset error {median:.2} ms");
        assert!(worst < 6.0, "{preset}: worst onset error {worst:.2} ms");
    }
}

#[test]
fn renders_are_deterministic() {
    let Some(packs) = installed_piano("renders_are_deterministic") else {
        return;
    };
    let reg = Registry::new(packs);
    let total = (5.0 * SR) as usize;
    for preset in ["felt_piano", "upright_piano"] {
        let mut a = create(&reg, preset, &[]);
        let mut b = create(&reg, preset, &[]);
        let ra = render(a.as_mut(), &motif(), total);
        let rb = render(b.as_mut(), &motif(), total);
        assert!(
            ra.0.iter()
                .zip(&rb.0)
                .all(|(x, y)| x.to_bits() == y.to_bits())
        );
        assert!(
            ra.1.iter()
                .zip(&rb.1)
                .all(|(x, y)| x.to_bits() == y.to_bits())
        );
        // Reset gives the same output again.
        a.reset();
        let rc = render(a.as_mut(), &motif(), total);
        assert!(
            ra.0.iter()
                .zip(&rc.0)
                .all(|(x, y)| x.to_bits() == y.to_bits())
        );
        let peak = ra.0.iter().chain(&ra.1).fold(0f32, |m, v| m.max(v.abs()));
        println!("{preset}: motif peak {peak:.3}");
        assert!(peak > 0.01 && peak < 4.0);
    }
}

#[test]
fn params_change_the_sound() {
    let Some(packs) = installed_piano("params_change_the_sound") else {
        return;
    };
    let reg = Registry::new(packs);
    let len = (0.8 * SR) as usize;
    let c = |params: &[(&str, f64)]| {
        let mut inst = create(&reg, "felt_piano", params);
        let (l, r) = render(inst.as_mut(), &[note(1, 0, 60, 0.7, len)], len);
        centroid(&mono(&l, &r), SR, 32768)
    };
    let dark = c(&[("tone", 0.0)]);
    let bright = c(&[("tone", 1.0)]);
    let light_felt = c(&[("felt", 0.0)]);
    let heavy_felt = c(&[("felt", 1.0)]);
    println!(
        "tone 0 {dark:.0} Hz, tone 1 {bright:.0} Hz; felt 0 {light_felt:.0}, felt 1 {heavy_felt:.0}"
    );
    assert!(dark < bright);
    assert!(heavy_felt < light_felt);
    // set_param at run time matches the spec param.
    let mut a = create(&reg, "felt_piano", &[("release_s", 0.3)]);
    let mut b = create(&reg, "felt_piano", &[]);
    b.set_param(2, 0.3);
    assert_eq!(a.tail_samples(), b.tail_samples());
    let ra = render(a.as_mut(), &motif(), len);
    let rb = render(b.as_mut(), &motif(), len);
    assert_eq!(ra, rb);
}

/// Writes listening demos (8 bars of the motif, 128 BPM, eighth-note grid, accents, 150% note
/// length) for both presets to `$PRAXIS_SAMPLER_DEMO_DIR` (default: target tmp dir).
/// `cargo test -p praxis-instruments --test sampler_piano -- --ignored --nocapture write_demo_wavs`
#[test]
#[ignore]
fn write_demo_wavs() {
    let Some(packs) = installed_piano("write_demo_wavs") else {
        return;
    };
    let dir = std::env::var("PRAXIS_SAMPLER_DEMO_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")));
    std::fs::create_dir_all(&dir).unwrap();
    let reg = Registry::new(packs);
    let step = (60.0 / 128.0 / 2.0 * SR) as usize;
    // DON din din din TIN din din din, A minor motif.
    let pitches = [57u8, 60, 64, 60, 69, 64, 60, 64];
    let vels = [0.9f32, 0.45, 0.55, 0.45, 0.8, 0.45, 0.55, 0.5];
    let mut notes = Vec::new();
    for k in 0..64 {
        let i = k % 8;
        let mut n = note(k as u64 + 1, k * step, pitches[i], vels[i], step * 3 / 2);
        n.on.brightness = if i == 4 { 1.0 } else { 0.0 };
        notes.push(n);
    }
    for preset in ["felt_piano", "upright_piano"] {
        let mut inst = create(&reg, preset, &[]);
        let total = 64 * step + inst.tail_samples();
        let (l, r) = render(inst.as_mut(), &notes, total);
        let peak = l.iter().chain(&r).fold(0f32, |m, v| m.max(v.abs()));
        let g = 0.5 / peak.max(1e-6);
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: SR as u32,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let path = dir.join(format!("{preset}_motif.wav"));
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..total {
            w.write_sample(l[i] * g).unwrap();
            w.write_sample(r[i] * g).unwrap();
        }
        w.finalize().unwrap();
        println!("wrote {}", path.display());
    }
}

/// Prints each Upright Piano KW sample's measured fundamental against equal temperament at its
/// keycenter, in the form of the `KW_TUNING` table in `sampler/presets.rs`. Run when the pack
/// changes: `cargo test -p praxis-instruments --test sampler_piano -- --ignored --nocapture
/// measure_kw_tuning`.
#[test]
#[ignore]
fn measure_kw_tuning() {
    let Some(packs) = installed_piano("measure_kw_tuning") else {
        return;
    };
    let prog = load_pack_preset(&packs, PIANO_PACK, "upright_piano").unwrap();
    for (i, r) in prog.regions.iter().enumerate() {
        let s = &prog.samples[prog.sample_of[i]];
        let sr = s.sample_rate;
        let a = (0.15 * sr) as usize;
        let b = (a + (0.85 * sr) as usize).min(s.frames());
        let x: Vec<f32> = (a..b)
            .map(|j| {
                let (l, rr) = s.frame(j);
                0.5 * (l + rr)
            })
            .collect();
        let et = midi_hz(r.pitch_keycenter as f32);
        let f = pitch_hz(&x, sr, et * 0.7, et * 1.4);
        let name = match &r.sample {
            SampleRef::File(f) => f.rsplit('/').next().unwrap_or("").to_string(),
            SampleRef::Sf2(_) => String::new(),
        };
        println!("    (\"{name}\", {:.1}),", cents(f, et));
    }
}

/// Prints the attack of every Upright Piano KW sample (centred 1 ms RMS envelope against the
/// peak of the first 150 ms): times to -40, -20, -6 and -1 dB, in ms, and the measured onset
/// skip the sampler applies.
#[test]
#[ignore]
fn measure_kw_attacks() {
    let Some(packs) = installed_piano("measure_kw_attacks") else {
        return;
    };
    let prog = load_pack_preset(&packs, PIANO_PACK, "upright_piano").unwrap();
    for (i, r) in prog.regions.iter().enumerate() {
        let s = &prog.samples[prog.sample_of[i]];
        let sr = s.sample_rate;
        let n = ((0.15 * sr) as usize).min(s.frames());
        let x: Vec<f32> = (0..n)
            .map(|j| {
                let (l, rr) = s.frame(j);
                0.5 * (l + rr)
            })
            .collect();
        let w = (0.0005 * sr) as usize;
        let env: Vec<f32> = (0..n)
            .map(|j| rms(&x[j.saturating_sub(w)..(j + w).min(n)]))
            .collect();
        let peak = env.iter().fold(0f32, |m, v| m.max(*v));
        let t = |db: f32| {
            let th = peak * 10f32.powf(db / 20.0);
            env.iter().position(|v| *v >= th).unwrap_or(n) as f32 / sr * 1000.0
        };
        let name = match &r.sample {
            SampleRef::File(f) => f.rsplit('/').next().unwrap_or("").to_string(),
            SampleRef::Sf2(_) => String::new(),
        };
        println!(
            "{name:12} -40 {:5.1}  -20 {:5.1}  -6 {:5.1}  -1 {:5.1} ms; playback starts at {:5.1} ms",
            t(-40.0),
            t(-20.0),
            t(-6.0),
            t(-1.0),
            s.onset() as f32 / sr * 1000.0
        );
    }
}
