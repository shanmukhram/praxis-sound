//! Shared helpers for the fx_* tests.
#![allow(dead_code)]

use praxis_dsp::Processor;
use praxis_dsp::rng::Pcg32;

pub const SR: f32 = 48000.0;
pub const BLOCK: usize = 256;

/// Runs a processor over a stereo signal in blocks of `block` samples (last block may be short).
pub fn run(p: &mut dyn Processor, l: &mut [f32], r: &mut [f32], block: usize) {
    let n = l.len();
    let mut i = 0;
    while i < n {
        let e = (i + block).min(n);
        p.process(&mut l[i..e], &mut r[i..e]);
        i = e;
    }
}

pub fn sine(freq: f32, amp: f32, n: usize, sr: f32) -> Vec<f32> {
    (0..n)
        .map(|i| {
            (2.0 * std::f64::consts::PI * freq as f64 * i as f64 / sr as f64).sin() as f32 * amp
        })
        .collect()
}

pub fn white(seed: u64, amp: f32, n: usize) -> Vec<f32> {
    let mut r = Pcg32::new(seed);
    (0..n).map(|_| r.next_bipolar() * amp).collect()
}

/// Pink noise (Paul Kellet's refined filter), roughly unit-ish peak scaled by `amp`.
pub fn pink(seed: u64, amp: f32, n: usize) -> Vec<f32> {
    let mut r = Pcg32::new(seed);
    let (mut b0, mut b1, mut b2, mut b3, mut b4, mut b5, mut b6) =
        (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
    (0..n)
        .map(|_| {
            let w = r.next_bipolar();
            b0 = 0.99886 * b0 + w * 0.0555179;
            b1 = 0.99332 * b1 + w * 0.0750759;
            b2 = 0.96900 * b2 + w * 0.153852;
            b3 = 0.86650 * b3 + w * 0.3104856;
            b4 = 0.55000 * b4 + w * 0.5329522;
            b5 = -0.7616 * b5 - w * 0.0168980;
            let p = b0 + b1 + b2 + b3 + b4 + b5 + b6 + w * 0.5362;
            b6 = w * 0.115926;
            p * 0.11 * amp
        })
        .collect()
}

pub fn rms(x: &[f32]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|&v| v as f64 * v as f64).sum::<f64>() / x.len() as f64).sqrt()
}

pub fn db(x: f64) -> f64 {
    20.0 * x.max(1e-30).log10()
}

pub fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// A music-like stereo test signal: plucked decaying notes on a grid, a soft kick, a quiet
/// passage and a noise bed, so gating and dynamics are exercised.
pub fn music(seconds: f32, sr: f32) -> (Vec<f32>, Vec<f32>) {
    let n = (seconds * sr) as usize;
    let mut l = vec![0.0f32; n];
    let mut r = vec![0.0f32; n];
    let notes = [
        220.0f32, 261.63, 329.63, 392.0, 440.0, 329.63, 293.66, 246.94,
    ];
    let step = (sr * 0.25) as usize;
    let mut rng = Pcg32::new(99);
    let mut k = 0;
    let mut start = 0;
    while start < n {
        let f = notes[k % notes.len()];
        // quiet passage between 40% and 55% of the signal
        let pos = start as f32 / n as f32;
        let amp = if (0.4..0.55).contains(&pos) {
            0.02
        } else {
            0.25
        };
        let pan = 0.3 * ((k % 3) as f32 - 1.0);
        for i in 0..(step * 3).min(n - start) {
            let t = i as f32 / sr;
            let env = (-t * 3.0).exp() * (1.0 - (-t * 400.0).exp());
            let v = amp
                * env
                * ((2.0 * std::f32::consts::PI * f * t).sin()
                    + 0.3 * (4.0 * std::f32::consts::PI * f * t).sin());
            l[start + i] += v * (1.0 - pan);
            r[start + i] += v * (1.0 + pan);
        }
        if k % 4 == 0 {
            for i in 0..(step).min(n - start) {
                let t = i as f32 / sr;
                let f = 50.0 + 80.0 * (-t * 30.0).exp();
                let v = amp * 1.6 * (-t * 12.0).exp() * (2.0 * std::f32::consts::PI * f * t).sin();
                l[start + i] += v;
                r[start + i] += v;
            }
        }
        k += 1;
        start += step;
    }
    for i in 0..n {
        l[i] += rng.next_bipolar() * 0.004;
        r[i] += rng.next_bipolar() * 0.004;
    }
    (l, r)
}
