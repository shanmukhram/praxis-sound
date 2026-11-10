//! Shared helpers for the synth and drum tests: building presets through the registry, rendering
//! like the engine, an FFT, spectral measures, a fine pitch estimator and click metrics.
#![allow(dead_code)]

use praxis_engine::{Instrument, InstrumentContext, InstrumentFactory, NoteOn, PresetInfo};
use praxis_instruments::{Packs, Registry};
use praxis_score::{InstrumentKind, InstrumentRef, Key, Mode, PitchClass};
use std::collections::BTreeMap;
use std::f64::consts::PI;

pub const SR: f32 = 48_000.0;
pub const BLOCK: usize = 128;

pub fn registry() -> Registry {
    Registry::new(Packs::none())
}

pub fn ctx() -> InstrumentContext {
    InstrumentContext {
        sample_rate: SR,
        max_block: 512,
        key: Key {
            tonic: PitchClass::A,
            mode: Mode::Minor,
        },
        bpm: 120.0,
        seed: 7,
    }
}

pub fn spec(kind: InstrumentKind, preset: &str, params: &[(&str, f64)]) -> InstrumentRef {
    InstrumentRef {
        kind,
        preset: preset.into(),
        params: params
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect::<BTreeMap<_, _>>(),
    }
}

pub fn create(kind: InstrumentKind, preset: &str, params: &[(&str, f64)]) -> Box<dyn Instrument> {
    registry()
        .create(&spec(kind, preset, params), &ctx())
        .unwrap_or_else(|e| panic!("create {preset}: {e}"))
}

pub fn info(kind: InstrumentKind, preset: &str) -> PresetInfo {
    registry()
        .describe(kind, preset)
        .unwrap_or_else(|| panic!("no preset {preset}"))
}

/// Names of every preset of `kind`.
pub fn names(kind: InstrumentKind) -> Vec<String> {
    registry()
        .presets()
        .into_iter()
        .filter(|p| p.kind == kind)
        .map(|p| p.name)
        .collect()
}

/// A note to render: start sample and the note (note-off after `length_samples`).
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

/// Renders like the engine: blocks of at most `block`, split at note-on/off so events land on
/// the first sample of a `process` call.
pub fn render_with(
    inst: &mut dyn Instrument,
    notes: &[Note],
    total: usize,
    block: usize,
) -> (Vec<f32>, Vec<f32>) {
    let mut events: Vec<(usize, u8, usize)> = Vec::new();
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
        let mut end = (t + block).min(total);
        if ei < events.len() && events[ei].0 < end {
            end = events[ei].0.max(t + 1);
        }
        inst.process(&mut l[t..end], &mut r[t..end]);
        t = end;
    }
    (l, r)
}

pub fn render(inst: &mut dyn Instrument, notes: &[Note], total: usize) -> (Vec<f32>, Vec<f32>) {
    render_with(inst, notes, total, BLOCK)
}

/// Mono mix of a stereo render.
pub fn mono(lr: &(Vec<f32>, Vec<f32>)) -> Vec<f32> {
    lr.0.iter()
        .zip(lr.1.iter())
        .map(|(a, b)| 0.5 * (a + b))
        .collect()
}

pub fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

pub fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / x.len() as f64).sqrt() as f32
}

/// In-place radix-2 complex FFT (`re.len()` must be a power of two).
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
        let ang = -2.0 * PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (ang * k as f64).sin_cos();
                let a = start + k;
                let b = a + len / 2;
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

/// Power spectrum (bins `0..n/2`) summed over Hann-windowed frames of `n` samples, hop `n/2`,
/// covering `x`.
pub fn power_spectrum(x: &[f32], n: usize) -> Vec<f64> {
    let mut acc = vec![0.0; n / 2];
    let hop = n / 2;
    let mut start = 0;
    loop {
        let mut re = vec![0.0; n];
        let mut im = vec![0.0; n];
        for i in 0..n {
            let v = x.get(start + i).copied().unwrap_or(0.0) as f64;
            let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos();
            re[i] = v * w;
        }
        fft(&mut re, &mut im);
        for k in 0..n / 2 {
            acc[k] += re[k] * re[k] + im[k] * im[k];
        }
        start += hop;
        if start + hop >= x.len() {
            break;
        }
    }
    acc
}

/// Fraction of spectral energy between `lo` and `hi` Hz.
pub fn band_fraction(x: &[f32], lo: f64, hi: f64) -> f64 {
    let n = 4096;
    let p = power_spectrum(x, n);
    let bin = SR as f64 / n as f64;
    let total: f64 = p.iter().sum();
    let band: f64 = p
        .iter()
        .enumerate()
        .filter(|(k, _)| {
            let f = *k as f64 * bin;
            f >= lo && f < hi
        })
        .map(|(_, v)| v)
        .sum();
    band / total.max(1e-30)
}

/// Spectral centroid in Hz.
pub fn centroid(x: &[f32]) -> f64 {
    let n = 2048;
    let p = power_spectrum(x, n);
    let bin = SR as f64 / n as f64;
    let total: f64 = p.iter().sum();
    p.iter()
        .enumerate()
        .map(|(k, v)| k as f64 * bin * v)
        .sum::<f64>()
        / total.max(1e-30)
}

fn dtft_mag(x: &[f32], hz: f64) -> f64 {
    let n = x.len();
    let w = 2.0 * PI * hz / SR as f64;
    let (mut re, mut im) = (0.0, 0.0);
    let (mut c, mut s) = (1.0f64, 0.0f64);
    let (cw, sw) = (w.cos(), w.sin());
    for (i, v) in x.iter().enumerate() {
        let win = 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos();
        let v = *v as f64 * win;
        re += v * c;
        im -= v * s;
        let nc = c * cw - s * sw;
        s = c * sw + s * cw;
        c = nc;
    }
    (re * re + im * im).sqrt()
}

/// Frequency of the strongest spectral peak within +/-`span_cents` of `hz`, to ~0.02 cent.
pub fn peak_near(x: &[f32], hz: f64, span_cents: f64) -> f64 {
    let at = |c: f64| hz * 2f64.powf(c / 1200.0);
    let mut best = 0.0;
    let mut best_mag = -1.0;
    let mut c = -span_cents;
    while c <= span_cents {
        let m = dtft_mag(x, at(c));
        if m > best_mag {
            best_mag = m;
            best = c;
        }
        c += 1.0;
    }
    let (mut lo, mut hi) = (best - 1.0, best + 1.0);
    for _ in 0..40 {
        let m1 = lo + (hi - lo) / 3.0;
        let m2 = hi - (hi - lo) / 3.0;
        if dtft_mag(x, at(m1)) < dtft_mag(x, at(m2)) {
            lo = m1;
        } else {
            hi = m2;
        }
    }
    at(0.5 * (lo + hi))
}

pub fn cents(f: f64, reference: f64) -> f64 {
    1200.0 * (f / reference).log2()
}

pub fn midi_hz(p: u8) -> f64 {
    440.0 * 2f64.powf((p as f64 - 69.0) / 12.0)
}

/// Onset step: the first non-zero sample relative to the RMS of the next 5 ms.
pub fn onset_step(x: &[f32]) -> f32 {
    let Some(i) = x.iter().position(|v| *v != 0.0) else {
        return 0.0;
    };
    let w = &x[i..(i + 240).min(x.len())];
    x[i].abs() / rms(w).max(1e-12)
}

/// End step: the largest adjacent-sample jump in the last 10 ms of sound (including the step to
/// silence), relative to the note's peak.
pub fn end_step(x: &[f32]) -> f32 {
    let Some(j) = x.iter().rposition(|v| *v != 0.0) else {
        return 0.0;
    };
    let from = j.saturating_sub(480).max(1);
    let to = (j + 2).min(x.len());
    let mut m = x[j].abs();
    for k in from..to {
        m = m.max((x[k] - x[k - 1]).abs());
    }
    m / peak(x).max(1e-12)
}

/// Index of the last non-zero sample (+1), or 0.
pub fn sound_end(x: &[f32]) -> usize {
    x.iter().rposition(|v| *v != 0.0).map_or(0, |j| j + 1)
}
