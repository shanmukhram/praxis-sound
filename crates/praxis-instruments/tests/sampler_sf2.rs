//! SoundFont 2 tests on a tiny SF2 generated here: RIFF/hydra parsing, generator semantics
//! (global zones, preset offsets, range intersection, root key, tuning, envelope, loops), errors,
//! and playback through the sampler at the right pitch.

mod sampler_common;

use praxis_instruments::sampler::engine::{SamplerInstrument, Voicing};
use praxis_instruments::sampler::program::Program;
use praxis_instruments::sampler::region::{LoopMode, SampleRef};
use praxis_instruments::sampler::sf2;
use sampler_common::*;
use std::sync::Arc;

fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(body);
    if body.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn list(kind: &[u8; 4], subs: &[Vec<u8>]) -> Vec<u8> {
    let mut body = kind.to_vec();
    for s in subs {
        body.extend_from_slice(s);
    }
    chunk(b"LIST", &body)
}

fn name20(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.resize(20, 0);
    v
}

fn gen_rec(oper: u16, amount: i32) -> Vec<u8> {
    let mut v = oper.to_le_bytes().to_vec();
    v.extend_from_slice(&(amount as i16 as u16).to_le_bytes());
    v
}

fn range(lo: u8, hi: u8) -> i32 {
    (lo as i32) | ((hi as i32) << 8)
}

const SF_RATE: u32 = 22_000;
const LEN: u32 = 22_000;
const GAP: u32 = 46;

/// Builds the test SoundFont: two looping sines (220 Hz root 57, 440 Hz with root key
/// overridden to 69), one instrument with a global zone, one preset whose zone narrows the key
/// range and adds attenuation.
fn tiny_sf2() -> Vec<u8> {
    let sine = |f: f32| -> Vec<i16> {
        (0..LEN)
            .map(|i| {
                let t = i as f32 / SF_RATE as f32;
                ((2.0 * std::f32::consts::PI * f * t).sin() * 0.5 * 32767.0) as i16
            })
            .collect()
    };
    let mut smpl = Vec::new();
    for s in [sine(220.0), sine(440.0)] {
        for v in s {
            smpl.extend_from_slice(&v.to_le_bytes());
        }
        smpl.extend(std::iter::repeat_n(0u8, GAP as usize * 2));
    }
    let info = list(
        b"INFO",
        &[
            chunk(b"ifil", &[2, 0, 1, 0]),
            chunk(b"INAM", b"tiny test bank\0"),
        ],
    );
    let sdta = list(b"sdta", &[chunk(b"smpl", &smpl)]);

    // Preset: global zone (coarse tune 0), zone 1 = instrument 0, keys 40..100, +60 cB.
    let mut phdr = Vec::new();
    for (name, prog, bag) in [("Test Preset", 5u16, 0u16), ("EOP", 0, 2)] {
        phdr.extend(name20(name));
        phdr.extend(prog.to_le_bytes());
        phdr.extend(0u16.to_le_bytes());
        phdr.extend(bag.to_le_bytes());
        phdr.extend([0u8; 12]);
    }
    let pbag: Vec<u8> = [0u16, 1, 4]
        .iter()
        .flat_map(|g| [g.to_le_bytes(), 0u16.to_le_bytes()].concat())
        .collect();
    let pgen: Vec<u8> = [
        gen_rec(51, 0),
        gen_rec(43, range(40, 100)),
        gen_rec(48, 60),
        gen_rec(41, 0),
        gen_rec(0, 0),
    ]
    .concat();
    // Instrument: global zone (release 0.5 s, sustain -6 dB), zone A (keys 0..63, sample 0,
    // loop), zone B (keys 64..127, fine tune +10, root key 69, sample 1, loop until release).
    let mut inst = Vec::new();
    for (name, bag) in [("Test Inst", 0u16), ("EOI", 3)] {
        inst.extend(name20(name));
        inst.extend(bag.to_le_bytes());
    }
    let ibag: Vec<u8> = [0u16, 2, 5, 10]
        .iter()
        .flat_map(|g| [g.to_le_bytes(), 0u16.to_le_bytes()].concat())
        .collect();
    let igen: Vec<u8> = [
        gen_rec(38, -1200),
        gen_rec(37, 60),
        gen_rec(43, range(0, 63)),
        gen_rec(54, 1),
        gen_rec(53, 0),
        gen_rec(43, range(64, 127)),
        gen_rec(52, 10),
        gen_rec(58, 69),
        gen_rec(54, 3),
        gen_rec(53, 1),
        gen_rec(0, 0),
    ]
    .concat();
    let mut shdr = Vec::new();
    let s1_start = LEN + GAP;
    for (name, start, pitch) in [("sine220", 0u32, 57u8), ("sine440", s1_start, 60)] {
        shdr.extend(name20(name));
        for v in [start, start + LEN, start + 2000, start + 12_000, SF_RATE] {
            shdr.extend(v.to_le_bytes());
        }
        shdr.push(pitch);
        shdr.push(0);
        shdr.extend(0u16.to_le_bytes());
        shdr.extend(1u16.to_le_bytes());
    }
    shdr.extend(name20("EOS"));
    shdr.extend([0u8; 26]);
    let pdta = list(
        b"pdta",
        &[
            chunk(b"phdr", &phdr),
            chunk(b"pbag", &pbag),
            chunk(b"pmod", &[0u8; 10]),
            chunk(b"pgen", &pgen),
            chunk(b"inst", &inst),
            chunk(b"ibag", &ibag),
            chunk(b"imod", &[0u8; 10]),
            chunk(b"igen", &igen),
            chunk(b"shdr", &shdr),
        ],
    );
    let mut body = b"sfbk".to_vec();
    body.extend(info);
    body.extend(sdta);
    body.extend(pdta);
    chunk(b"RIFF", &body)
}

#[test]
fn parses_hydra_into_regions() {
    let bytes = tiny_sf2();
    let f = sf2::parse(&bytes).unwrap();
    assert_eq!(f.samples.len(), 2);
    assert_eq!(f.samples[1].name, "sine440");
    assert_eq!(f.samples[1].sample_rate, SF_RATE);
    assert_eq!(f.presets.len(), 1);
    let p = &f.presets[0];
    assert_eq!((p.name.as_str(), p.bank, p.program), ("Test Preset", 0, 5));
    assert_eq!(p.regions.len(), 2);
    let (a, b) = (&p.regions[0], &p.regions[1]);
    // Key ranges intersect with the preset zone's 40..100.
    assert_eq!((a.lokey, a.hikey), (40, 63));
    assert_eq!((b.lokey, b.hikey), (64, 100));
    assert_eq!(a.sample, SampleRef::Sf2(0));
    assert_eq!(b.sample, SampleRef::Sf2(1));
    // Root keys: sample's original pitch vs overridingRootKey.
    assert_eq!((a.pitch_keycenter, b.pitch_keycenter), (57, 69));
    assert_eq!((a.tune, b.tune), (0.0, 10.0));
    // Preset attenuation 60 cB = -6 dB.
    assert_eq!(a.volume, -6.0);
    // Global instrument zone: release 2^(-1200/1200) = 0.5 s, sustain 60 cB = -6 dB.
    assert!((a.ampeg.release - 0.5).abs() < 1e-6);
    assert!((b.ampeg.sustain - 10f32.powf(-0.3)).abs() < 1e-4);
    // Loops, relative to the sample start, SF2 end-exclusive -> SFZ inclusive.
    assert_eq!(a.loop_mode, Some(LoopMode::Continuous));
    assert_eq!(b.loop_mode, Some(LoopMode::Sustain));
    assert_eq!((b.loop_start, b.loop_end), (Some(2000), Some(11_999)));
    assert_eq!(b.end, Some(LEN as u64 - 1));
    let d = sf2::sample_data(&f, &bytes, 1).unwrap();
    assert_eq!(d.frames(), LEN as usize);
    assert!(
        (d.frame(10).0 - (2.0 * std::f32::consts::PI * 440.0 * 10.0 / 22_000.0).sin() * 0.5).abs()
            < 1e-3
    );
}

#[test]
fn rejects_malformed_files() {
    let bytes = tiny_sf2();
    assert!(sf2::parse(b"RIFF\0\0\0\0WAVE").is_err());
    assert!(sf2::parse(&bytes[..100]).is_err());
    let mut bad = bytes.clone();
    // Corrupt the first shdr record's end so it points past smpl.
    let pos = bad.windows(7).position(|w| w == b"sine220").unwrap();
    bad[pos + 24..pos + 28].copy_from_slice(&u32::MAX.to_le_bytes());
    let e = sf2::parse(&bad).unwrap_err();
    assert!(e.to_string().contains("outside smpl"), "{e}");
    for cut in (12..bytes.len()).step_by(37) {
        let _ = sf2::parse(&bytes[..cut]); // must not panic
    }
}

#[test]
fn plays_at_the_right_pitch() {
    let bytes = tiny_sf2();
    let prog = Arc::new(Program::from_sf2_bytes(&bytes, "tiny.sf2", None).unwrap());
    assert_eq!(prog.key_range(), Some((40, 100)));
    for (n, expected) in [
        (57u8, 220.0f32),
        (52, 220.0 * 2f32.powf(-5.0 / 12.0)),
        (76, 440.0 * 2f32.powf((7.0 + 0.1) / 12.0)),
    ] {
        let mut inst = SamplerInstrument::new(prog.clone(), SR, 8, Voicing::default(), None, &[]);
        // Hold past the loop end to cover looping.
        let len = (1.5 * SR) as usize;
        let (l, r) = render(&mut inst, &[note(1, 0, n, 0.8, len)], len);
        let x = mono(&l, &r);
        let f = pitch_hz(
            &x[(0.3 * SR) as usize..(1.4 * SR) as usize],
            SR,
            expected * 0.7,
            expected * 1.4,
        );
        let err = cents(f, expected);
        println!("sf2 note {n}: {f:.3} Hz, expected {expected:.3} Hz, {err:+.2} cents");
        assert!(err.abs() < 5.0);
        // The loop keeps it sounding past the sample's 1 s length.
        let early = rms(&x[(0.2 * SR) as usize..(0.4 * SR) as usize]);
        let late = rms(&x[(1.2 * SR) as usize..(1.4 * SR) as usize]);
        assert!(
            late > 0.01 && (late / early - 1.0).abs() < 0.05,
            "{early} {late}"
        );
        // Loop wraps (seamless: whole periods) add no discontinuity: the largest prediction
        // residual over the held note matches the one before the first wrap.
        let before_wrap = max_residual(&x, (0.05 * SR) as usize, (0.3 * SR) as usize);
        let held = max_residual(&x, (0.05 * SR) as usize, (1.45 * SR) as usize);
        assert!(held < 1.5 * before_wrap, "loop seam click at note {n}");
    }
    // Preset selection by bank/program.
    assert!(Program::from_sf2_bytes(&bytes, "tiny.sf2", Some((0, 5))).is_ok());
    assert!(Program::from_sf2_bytes(&bytes, "tiny.sf2", Some((1, 0))).is_err());
}
