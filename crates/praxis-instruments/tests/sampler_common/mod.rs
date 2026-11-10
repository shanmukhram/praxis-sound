//! Shared helpers for the sampler tests: rendering, a pitch estimator, spectral centroid, click
//! metrics, and synthetic WAV/SF2 builders.
#![allow(dead_code)]

use praxis_engine::{Instrument, NoteOn};
use std::path::PathBuf;

pub const SR: f32 = 48_000.0;
pub const BLOCK: usize = 128;

/// A note to render: start sample, note, length in samples (note-off after it).
#[derive(Clone, Copy, Debug)]
pub struct Note {
    pub at: usize,
    pub on: NoteOn,
}

pub fn note(id: u64, at: usize, pitch: u8, velocity: f32, len: usize) -> Note {
    Note {
        at,
        on: NoteOn {
            id,
            pitch,
            velocity,
            brightness: 0.0,
            seed: 0x5EED_0000 + id,
            length_samples: len as u64,
        },
    }
}

/// Renders like the engine: blocks of at most `BLOCK`, split at note-on/off so events land on
/// the first sample of a `process` call.
pub fn render(inst: &mut dyn Instrument, notes: &[Note], total: usize) -> (Vec<f32>, Vec<f32>) {
    let mut events: Vec<(usize, u8, usize)> = Vec::new(); // (time, kind 0=off 1=on, index)
    for (i, n) in notes.iter().enumerate() {
        events.push((n.at, 1, i));
        events.push((n.at + n.on.length_samples as usize, 0, i));
    }
    events.sort();
    let mut l = vec![0f32; total];
    let mut r = vec![0f32; total];
    let mut t = 0;
    let mut ei = 0;
    while t < total {
        while ei < events.len() && events[ei].0 <= t {
            let (_, kind, i) = events[ei];
            if kind == 1 {
                inst.note_on(&notes[i].on);
            } else {
                inst.note_off(notes[i].on.id);
            }
            ei += 1;
        }
        let mut end = (t + BLOCK).min(total);
        if ei < events.len() && events[ei].0 < end {
            end = events[ei].0.max(t + 1);
        }
        inst.process(&mut l[t..end], &mut r[t..end]);
        t = end;
    }
    (l, r)
}

pub fn mono(l: &[f32], r: &[f32]) -> Vec<f32> {
    l.iter().zip(r).map(|(a, b)| 0.5 * (a + b)).collect()
}

pub fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt() as f32
}

pub fn max_jump(x: &[f32]) -> f32 {
    x.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

/// Click metric at an onset: largest sample-to-sample jump in the first 2 ms after `at`,
/// relative to the RMS of the 20 ms after `at`. A hard cut-in of a sustained tone scores ~1.4;
/// a ramped onset scores far lower.
pub fn onset_click(x: &[f32], at: usize) -> f32 {
    let a = at.saturating_sub(1);
    let j = max_jump(&x[a..(at + 96).min(x.len())]);
    let r = rms(&x[at..(at + 960).min(x.len())]);
    j / r.max(1e-12)
}

/// Largest cubic-prediction residual |x[n] - (3x[n-1] - 3x[n-2] + x[n-3])| in `x[a..b]`. Smooth
/// audio predicts well; a discontinuity (click) does not.
pub fn max_residual(x: &[f32], a: usize, b: usize) -> f32 {
    (a.max(3)..b.min(x.len()))
        .map(|n| (x[n] - (3.0 * x[n - 1] - 3.0 * x[n - 2] + x[n - 3])).abs())
        .fold(0.0, f32::max)
}

/// Click metric at a note-off: largest prediction residual in [off, off + 10 ms] relative to the
/// largest in the 20 ms before. Release only lowers the level, so a clean note-off stays at or
/// below about 1; a hard cut scores far higher.
pub fn release_click(x: &[f32], off: usize) -> f32 {
    let after = max_residual(x, off, off + 480);
    let before = max_residual(x, off.saturating_sub(960), off);
    after / before.max(1e-12)
}

/// `x` cut to silence at the loudest sample within 5 ms after `off` (worst-case hard cut), for
/// negative controls.
pub fn hard_cut(x: &[f32], off: usize) -> (Vec<f32>, usize) {
    let c = (off..off + 240)
        .max_by(|&a, &b| x[a].abs().partial_cmp(&x[b].abs()).unwrap())
        .unwrap();
    let mut y = x[..=c].to_vec();
    y.resize(x.len(), 0.0);
    (y, c + 1)
}

/// YIN fundamental estimate with parabolic refinement, then a spectral-peak refinement near
/// the YIN estimate (Hann window, golden-section search on the DFT magnitude).
pub fn pitch_hz(x: &[f32], sr: f32, fmin: f32, fmax: f32) -> f32 {
    let max_lag = (sr / fmin).ceil() as usize + 2;
    let min_lag = (sr / fmax).floor().max(2.0) as usize;
    let w = x.len().saturating_sub(max_lag + 1).min(16384);
    assert!(w > 1024, "signal too short for pitch estimation");
    let mut d = vec![0f64; max_lag + 2];
    for tau in 1..=max_lag + 1 {
        let mut s = 0f64;
        for j in 0..w {
            let e = (x[j] - x[j + tau]) as f64;
            s += e * e;
        }
        d[tau] = s;
    }
    let mut cm = vec![1f64; max_lag + 2];
    let mut run = 0f64;
    for tau in 1..=max_lag + 1 {
        run += d[tau];
        cm[tau] = d[tau] * tau as f64 / run.max(1e-30);
    }
    let mut tau = None;
    let mut t = min_lag;
    while t <= max_lag {
        if cm[t] < 0.15 {
            while t < max_lag && cm[t + 1] < cm[t] {
                t += 1;
            }
            tau = Some(t);
            break;
        }
        t += 1;
    }
    let tau = tau.unwrap_or_else(|| {
        (min_lag..=max_lag)
            .min_by(|&a, &b| cm[a].partial_cmp(&cm[b]).unwrap())
            .unwrap()
    });
    let (a, b, c) = (d[tau - 1], d[tau], d[tau + 1]);
    let den = a - 2.0 * b + c;
    let shift = if den.abs() > 1e-30 {
        0.5 * (a - c) / den
    } else {
        0.0
    };
    let f_yin = sr as f64 / (tau as f64 + shift.clamp(-1.0, 1.0));
    refine_peak(x, sr as f64, f_yin) as f32
}

fn dft_mag(x: &[f32], win: &[f64], sr: f64, f: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * f / sr;
    let (mut re, mut im) = (0f64, 0f64);
    let (mut c, mut s) = (1f64, 0f64);
    let (cw, sw) = (w.cos(), w.sin());
    for (i, &v) in x.iter().enumerate() {
        let v = v as f64 * win[i];
        re += v * c;
        im -= v * s;
        let nc = c * cw - s * sw;
        s = s * cw + c * sw;
        c = nc;
        if i % 1024 == 1023 {
            // Re-anchor the recurrence to avoid drift.
            let ph = w * (i + 1) as f64;
            c = ph.cos();
            s = ph.sin();
        }
    }
    (re * re + im * im).sqrt()
}

fn refine_peak(x: &[f32], sr: f64, f0: f64) -> f64 {
    let n = x.len().min(32768);
    let x = &x[..n];
    let win: Vec<f64> = (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos())
        .collect();
    // Coarse scan within +-1.5% for the strongest peak, then golden-section.
    let (lo, hi) = (f0 * 0.985, f0 * 1.015);
    let steps = 60;
    let mut best = (f0, 0.0);
    for k in 0..=steps {
        let f = lo + (hi - lo) * k as f64 / steps as f64;
        let m = dft_mag(x, &win, sr, f);
        if m > best.1 {
            best = (f, m);
        }
    }
    let step = (hi - lo) / steps as f64;
    let (mut a, mut b) = (best.0 - step, best.0 + step);
    let g = (5f64.sqrt() - 1.0) / 2.0;
    for _ in 0..40 {
        let c = b - g * (b - a);
        let d = a + g * (b - a);
        if dft_mag(x, &win, sr, c) > dft_mag(x, &win, sr, d) {
            b = d;
        } else {
            a = c;
        }
    }
    0.5 * (a + b)
}

pub fn cents(f: f32, reference: f32) -> f32 {
    1200.0 * (f / reference).log2()
}

pub fn midi_hz(n: f32) -> f32 {
    440.0 * 2f32.powf((n - 69.0) / 12.0)
}

/// In-place radix-2 FFT (re, im), length a power of two.
pub fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (ang * k as f64).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// Spectral centroid (Hz) of `x[..n]` (n a power of two), Hann-windowed magnitude spectrum.
pub fn centroid(x: &[f32], sr: f32, n: usize) -> f32 {
    let mut re: Vec<f64> = (0..n)
        .map(|i| {
            let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos();
            x.get(i).copied().unwrap_or(0.0) as f64 * w
        })
        .collect();
    let mut im = vec![0f64; n];
    fft(&mut re, &mut im);
    let (mut num, mut den) = (0f64, 0f64);
    for k in 1..n / 2 {
        let m = (re[k] * re[k] + im[k] * im[k]).sqrt();
        num += m * k as f64 * sr as f64 / n as f64;
        den += m;
    }
    (num / den.max(1e-30)) as f32
}

/// A decaying harmonic tone ("struck string") at `f0`, `secs` long.
pub fn struck_tone(f0: f32, sr: u32, secs: f32, harmonics: usize, amp: f32) -> Vec<f32> {
    let n = (secs * sr as f32) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / sr as f32;
            let mut v = 0.0;
            for k in 1..=harmonics {
                let kf = k as f32;
                v += (2.0 * std::f32::consts::PI * f0 * kf * t).sin() / kf.powf(1.1)
                    * (-t * (0.7 + 0.5 * kf)).exp();
            }
            // 2 ms attack so the source itself has no click.
            v * amp * (t / 0.002).min(1.0)
        })
        .collect()
}

/// Encodes mono or stereo f32 audio as a 16-bit PCM WAV.
pub fn wav_bytes(channels: &[Vec<f32>], sr: u32) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: channels.len() as u16,
        sample_rate: sr,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cur = std::io::Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut cur, spec).unwrap();
        for i in 0..channels[0].len() {
            for ch in channels {
                w.write_sample((ch[i].clamp(-1.0, 1.0) * 32767.0) as i16)
                    .unwrap();
            }
        }
        w.finalize().unwrap();
    }
    cur.into_inner()
}

/// A stationary tone (no decay) with `harmonics` partials, for seamless loops.
pub fn steady_tone(f0: f32, sr: u32, secs: f32, harmonics: usize, amp: f32) -> Vec<f32> {
    let n = (secs * sr as f32) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / sr as f64;
            let v: f64 = (1..=harmonics)
                .map(|k| {
                    // Phase reduced in f64 so the tone is exactly periodic over long spans.
                    let cycles = (f0 as f64 * k as f64 * i as f64 / sr as f64).fract();
                    (2.0 * std::f64::consts::PI * cycles).sin() / k as f64
                })
                .sum();
            (v * amp as f64 * (t / 0.002).min(1.0)) as f32
        })
        .collect()
}

/// The synthetic test pack's SFZ (also a fixture: tests/fixtures/sampler/synth_piano.sfz).
pub const SYNTH_SFZ: &str = include_str!("../fixtures/sampler/synth_piano.sfz");
/// Loop points of `a4_loop.wav` (44 kHz, 440 Hz: exactly 100 frames per period).
pub const LOOP_START: usize = 44_000;
pub const LOOP_END: usize = 87_999;

/// An in-memory pack named `name` with `synth_piano.sfz` and its generated samples:
/// two velocity layers on A3 (soft mono / hard stereo), a looping A4, two round-robin samples on
/// C5, and a release sample. `pack.json` maps both piano presets to the SFZ.
pub fn synth_pack(name: &str) -> praxis_instruments::Packs {
    let mut p = praxis_instruments::Packs::none();
    let manifest = format!(
        r#"{{"name":"{name}","version":"test","license":"CC0-1.0","source_url":"test",
        "presets":{{"felt_piano":"sfz/synth_piano.sfz","upright_piano":"sfz/synth_piano.sfz"}}}}"#
    );
    p.insert(&format!("{name}/pack.json"), manifest.into_bytes());
    p.insert(
        &format!("{name}/sfz/synth_piano.sfz"),
        SYNTH_SFZ.as_bytes().to_vec(),
    );
    p.insert(
        &format!("{name}/sfz/common.sfz"),
        include_bytes!("../fixtures/sampler/common.sfz").to_vec(),
    );
    let sr = 44_100;
    let soft = struck_tone(220.0, sr, 2.0, 3, 0.3);
    let hard_l = struck_tone(220.0, sr, 2.0, 10, 0.6);
    let hard_r: Vec<f32> = hard_l.iter().map(|v| v * 0.9).collect();
    let files: Vec<(&str, Vec<u8>)> = vec![
        ("soft_a3.wav", wav_bytes(&[soft], sr)),
        ("hard_a3.wav", wav_bytes(&[hard_l, hard_r], sr)),
        (
            "a4_loop.wav",
            wav_bytes(&[steady_tone(440.0, 44_000, 2.0, 4, 0.4)], 44_000),
        ),
        (
            "rr1_c5.wav",
            wav_bytes(&[struck_tone(523.251, sr, 1.0, 2, 0.4)], sr),
        ),
        (
            "rr2_c5.wav",
            wav_bytes(&[struck_tone(523.251, sr, 1.0, 8, 0.4)], sr),
        ),
        (
            "release.wav",
            wav_bytes(&[struck_tone(880.0, sr, 1.0, 1, 0.2)], sr),
        ),
    ];
    for (f, b) in files {
        p.insert(&format!("{name}/samples/{f}"), b);
    }
    p
}

/// The repository's `packs/` directory, or `$PRAXIS_PACKS`.
pub fn packs_dir() -> PathBuf {
    std::env::var("PRAXIS_PACKS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs"))
}

/// `Some(packs)` when Upright Piano KW is installed; otherwise prints why the test is skipped.
pub fn installed_piano(test: &str) -> Option<praxis_instruments::Packs> {
    let packs = praxis_instruments::Packs::at(packs_dir());
    if packs.has_pack(praxis_instruments::sampler::presets::PIANO_PACK) {
        Some(packs)
    } else {
        println!(
            "SKIPPED {test}: pack `upright-piano-kw` is not installed in {}. Install it with \
             `praxis packs install upright-piano-kw` (or run the ignored test \
             `sampler_install::install_upright_piano_kw_from_network`).",
            packs_dir().display()
        );
        None
    }
}
