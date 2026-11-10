//! Compressor, limiter, saturation and gain behaviour.

mod fx_support;
use fx_support::*;

use ebur128::{EbuR128, Mode};
use praxis_dsp::Processor;
use praxis_dsp::fx::compressor::static_curve;
use praxis_dsp::fx::{Compressor, GainFx, Saturation, TruePeakLimiter};
use praxis_dsp::meter::true_peak_dbtp;

#[test]
fn compressor_static_curve_matches_theory() {
    let (thr, ratio, knee, makeup) = (-20.0f32, 4.0f32, 6.0f32, 3.0f32);
    let mut worst = 0.0f64;
    for in_rms_db in [-40.0f32, -26.0, -22.0, -20.0, -18.0, -12.0, -6.0, -3.0] {
        let mut c = Compressor::new(SR, BLOCK);
        c.set_param(0, thr);
        c.set_param(1, ratio);
        c.set_param(2, 5.0);
        c.set_param(3, 50.0);
        c.set_param(4, knee);
        c.set_param(5, makeup);
        c.reset();
        let amp = 10f32.powf(in_rms_db / 20.0) * std::f32::consts::SQRT_2;
        let n = (SR * 1.0) as usize;
        let x = sine(1000.0, amp, n, SR);
        let mut l = x.clone();
        let mut r = x.clone();
        run(&mut c, &mut l, &mut r, BLOCK);
        let got = db(rms(&l[n / 2..]));
        let want = (static_curve(in_rms_db, thr, ratio, knee) + makeup) as f64;
        let err = (got - want).abs();
        worst = worst.max(err);
        assert!(
            err < 0.5,
            "input {in_rms_db} dB RMS: got {got:.3} want {want:.3}"
        );
    }
    println!("compressor: worst static-curve error {worst:.4} dB");
}

#[test]
fn compressor_parallel_mix_and_ratio_one_are_transparent() {
    let n = (SR * 0.5) as usize;
    let x = sine(500.0, 0.8, n, SR);
    let mut c = Compressor::new(SR, BLOCK);
    c.set_param(1, 1.0);
    c.reset();
    let mut l = x.clone();
    let mut r = x.clone();
    run(&mut c, &mut l, &mut r, BLOCK);
    assert!(l.iter().zip(x.iter()).all(|(a, b)| (a - b).abs() < 1e-5));
    let mut c = Compressor::new(SR, BLOCK);
    c.set_param(6, 0.0);
    c.reset();
    let mut l = x.clone();
    let mut r = x.clone();
    run(&mut c, &mut l, &mut r, BLOCK);
    assert!(l.iter().zip(x.iter()).all(|(a, b)| (a - b).abs() < 1e-6));
}

#[test]
fn compressor_sidechain_ducks_a_piano_under_a_kick() {
    let n = (SR * 2.0) as usize;
    let piano = sine(440.0, 0.05, n, SR); // -29 dB RMS, below threshold
    let beat = (SR * 0.5) as usize;
    let mut kick = vec![0.0f32; n];
    for (i, k) in kick.iter_mut().enumerate() {
        let t = (i % beat) as f32 / SR;
        if t < 0.12 {
            *k = 0.9 * (-t * 10.0).exp() * (2.0 * std::f32::consts::PI * 60.0 * t).sin();
        }
    }
    let setup = || {
        let mut c = Compressor::new(SR, BLOCK);
        c.set_param(0, -20.0);
        c.set_param(1, 8.0);
        c.set_param(2, 2.0);
        c.set_param(3, 100.0);
        c.set_param(4, 0.0);
        c.reset();
        c
    };
    // self-keyed: the quiet piano passes untouched
    let mut c = setup();
    let mut l = piano.clone();
    let mut r = piano.clone();
    run(&mut c, &mut l, &mut r, BLOCK);
    assert!((db(rms(&l) / rms(&piano))).abs() < 0.05);
    // keyed by the kick
    let mut c = setup();
    let mut l = piano.clone();
    let mut r = piano.clone();
    let mut i = 0;
    while i < n {
        let e = (i + BLOCK).min(n);
        c.set_sidechain(&kick[i..e], &kick[i..e]);
        c.process(&mut l[i..e], &mut r[i..e]);
        i = e;
    }
    for b in 1..4 {
        let s = b * beat;
        let during = db(rms(&l[s + 960..s + 2880]) / rms(&piano[s + 960..s + 2880]));
        let before = db(rms(&l[s - 2400..s]) / rms(&piano[s - 2400..s]));
        println!("beat {b}: ducked {during:.2} dB, recovered {before:.2} dB");
        assert!(during < -8.0, "kick must duck the piano: {during}");
        assert!(before > -1.5, "piano must recover between kicks: {before}");
    }
    // the sidechain is consumed by one process call
    let mut l2 = piano[..BLOCK].to_vec();
    let mut r2 = piano[..BLOCK].to_vec();
    let mut c = setup();
    c.set_sidechain(&kick[..BLOCK], &kick[..BLOCK]);
    c.process(&mut l2, &mut r2);
    let mut l3 = piano[..BLOCK].to_vec();
    let mut r3 = piano[..BLOCK].to_vec();
    let mut c2 = setup();
    c2.process(&mut l3, &mut r3);
    assert_ne!(l2, l3);
}

fn ebur128_true_peak(l: &[f32], r: &[f32]) -> f64 {
    let mut m = EbuR128::new(2, SR as u32, Mode::TRUE_PEAK).unwrap();
    m.add_frames_planar_f32(&[l, r]).unwrap();
    let p = m.true_peak(0).unwrap().max(m.true_peak(1).unwrap());
    20.0 * p.log10()
}

fn limit(
    sig_l: &[f32],
    sig_r: &[f32],
    ceiling: f32,
    release: f32,
    la: f32,
) -> (Vec<f32>, Vec<f32>) {
    let mut lim = TruePeakLimiter::new(SR, BLOCK);
    lim.set_param(0, ceiling);
    lim.set_param(1, release);
    lim.set_param(2, la);
    lim.reset();
    let pad = lim.latency_samples() + 64;
    let mut l = sig_l.to_vec();
    let mut r = sig_r.to_vec();
    l.extend(std::iter::repeat_n(0.0, pad));
    r.extend(std::iter::repeat_n(0.0, pad));
    run(&mut lim, &mut l, &mut r, 100);
    (l, r)
}

#[test]
fn limiter_never_exceeds_the_ceiling_on_adversarial_inputs() {
    let n = (SR * 0.5) as usize;
    let mut signals: Vec<(&str, Vec<f32>, Vec<f32>)> = Vec::new();
    // full-scale square waves
    let sq = |period: usize, amp: f32| -> Vec<f32> {
        (0..n)
            .map(|i| {
                if (i / (period / 2)).is_multiple_of(2) {
                    amp
                } else {
                    -amp
                }
            })
            .collect()
    };
    signals.push(("square 1k", sq(48, 1.0), sq(48, 1.0)));
    signals.push(("square 3.4k +6dB", sq(14, 2.0), sq(14, -2.0)));
    // inter-sample peaks: sine at fs/4 with 45 degree phase offset (samples 0.707 * amp)
    let fs4 = |amp: f32, ph: f32| -> Vec<f32> {
        (0..n)
            .map(|i| amp * (std::f32::consts::FRAC_PI_2 * i as f32 + ph).sin())
            .collect()
    };
    signals.push((
        "fs/4 45deg",
        fs4(1.0, std::f32::consts::FRAC_PI_4),
        fs4(1.0, std::f32::consts::FRAC_PI_4),
    ));
    signals.push((
        "fs/4 45deg +3dB",
        fs4(std::f32::consts::SQRT_2, std::f32::consts::FRAC_PI_4),
        fs4(std::f32::consts::SQRT_2, 0.3),
    ));
    // a sudden full-scale onset after silence
    let mut onset = vec![0.0f32; n];
    onset[n / 2..]
        .copy_from_slice(&fs4(std::f32::consts::SQRT_2, std::f32::consts::FRAC_PI_4)[n / 2..]);
    signals.push(("fs/4 onset", onset.clone(), onset));
    // impulses and doublets
    let mut imp = vec![0.0f32; n];
    for k in 0..40 {
        let at = 1000 + k * 571;
        imp[at] = 1.0;
        if k % 2 == 0 {
            imp[at + 1] = -1.0;
        }
        if k % 3 == 0 {
            imp[at + 2] = 1.0;
        }
    }
    let imp_r: Vec<f32> = imp.iter().map(|v| v * 1.5).collect();
    signals.push(("impulses/doublets", imp, imp_r));
    // loud noise and a hot music-like signal
    signals.push(("white +6dB", white(1, 2.0, n), white(2, 2.0, n)));
    let (ml, mr) = music(2.0, SR);
    let ml: Vec<f32> = ml.iter().map(|v| v * 4.0).collect();
    let mr: Vec<f32> = mr.iter().map(|v| v * 4.0).collect();
    signals.push(("music +12dB", ml, mr));
    // nasty: alternating Nyquist bursts and sine
    let nyq: Vec<f32> = (0..n)
        .map(|i| {
            let b = if (i / 300) % 2 == 0 { 1.0 } else { 0.0 };
            b * if i % 2 == 0 { 1.2 } else { -1.2 } + (1.0 - b) * 1.3 * (0.7 * i as f32).sin()
        })
        .collect();
    signals.push(("nyquist bursts", nyq.clone(), nyq));

    let mut worst = f64::NEG_INFINITY;
    let mut worst_eb = f64::NEG_INFINITY;
    for (name, l, r) in &signals {
        for &(ceiling, release, la) in &[
            (-1.0f32, 80.0f32, 3.0f32),
            (-0.1, 5.0, 0.5),
            (-6.0, 300.0, 10.0),
            (0.0, 1.0, 1.0),
        ] {
            let (ol, or) = limit(l, r, ceiling, release, la);
            assert!(ol.iter().chain(or.iter()).all(|v| v.is_finite()));
            let tp = true_peak_dbtp(&ol, &or, SR);
            let tp_eb = ebur128_true_peak(&ol, &or);
            worst = worst.max(tp - ceiling as f64);
            worst_eb = worst_eb.max(tp_eb - ceiling as f64);
            assert!(
                tp <= ceiling as f64 + 0.05,
                "{name}: ceiling {ceiling} got {tp:.4} dBTP"
            );
            assert!(
                tp_eb <= ceiling as f64 + 0.05,
                "{name}: ceiling {ceiling} ebur128 measured {tp_eb:.4} dBTP"
            );
        }
    }
    println!(
        "limiter: worst true peak relative to ceiling {worst:.4} dB (ebur128 {worst_eb:.4} dB)"
    );
}

#[test]
fn limiter_latency_is_exact_and_quiet_signals_pass_unchanged() {
    for la in [0.5f32, 3.0, 10.0] {
        let mut lim = TruePeakLimiter::new(SR, BLOCK);
        lim.set_param(2, la);
        let d = lim.latency_samples();
        assert_eq!(d, ((la * 48.0).round() as usize).max(12));
        let x = sine(1000.0, 0.3, 4800, SR);
        let mut l = x.clone();
        l.extend(std::iter::repeat_n(0.0, d));
        let mut r = l.clone();
        run(&mut lim, &mut l, &mut r, BLOCK);
        for i in 0..x.len() {
            assert_eq!(l[i + d], x[i], "lookahead {la} sample {i}");
        }
    }
}

#[test]
fn limiter_gain_recovers_with_release() {
    let mut lim = TruePeakLimiter::new(SR, BLOCK);
    lim.set_param(1, 50.0);
    let n = (SR * 1.0) as usize;
    let mut x = vec![0.0f32; n];
    x[..4800].copy_from_slice(&sine(1000.0, 2.0, 4800, SR));
    x[4800..].copy_from_slice(&sine(1000.0, 0.1, n - 4800, SR));
    let mut l = x.clone();
    let mut r = x.clone();
    run(&mut lim, &mut l, &mut r, BLOCK);
    // after 10 release time constants the quiet part is untouched
    let d = lim.latency_samples();
    let from = 4800 + d + 24000;
    let g = db(rms(&l[from..]) / rms(&x[from - d..n - d]));
    assert!(g.abs() < 0.05, "gain {g}");
    assert!(lim.gain() > 0.999);
}

#[test]
fn saturation_stays_bounded_and_is_transparent_at_low_level() {
    let n = (SR * 0.5) as usize;
    for (drive, tone, out) in [
        (36.0f32, 1.0f32, 12.0f32),
        (36.0, -1.0, 0.0),
        (0.0, 0.0, 0.0),
    ] {
        let mut s = Saturation::new(SR, BLOCK);
        s.set_param(0, drive);
        s.set_param(1, tone);
        s.set_param(3, out);
        s.reset();
        let mut l = white(3, 10.0, n);
        let mut r = sine(50.0, 20.0, n, SR);
        run(&mut s, &mut l, &mut r, BLOCK);
        let bound = 1.2 * 2.0 * 2.0 * 10f32.powf(out / 20.0) + 1e-3;
        assert!(
            l.iter()
                .chain(r.iter())
                .all(|v| v.is_finite() && v.abs() <= bound),
            "drive {drive}: peak {} bound {bound}",
            peak(&l).max(peak(&r))
        );
    }
    // 0 dB drive, neutral tone: a quiet 1 kHz sine passes at unity gain, delayed by LATENCY
    let mut s = Saturation::new(SR, BLOCK);
    s.set_param(0, 0.0);
    s.reset();
    let x = sine(1000.0, 0.01, n, SR);
    let mut l = x.clone();
    let mut r = x.clone();
    run(&mut s, &mut l, &mut r, BLOCK);
    let d = s.latency_samples();
    assert_eq!(d, praxis_dsp::fx::saturation::LATENCY);
    let err = l[n / 2..]
        .iter()
        .zip(x[n / 2 - d..].iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(err < 0.01 * 0.02, "max error {err}");
    // drive adds harmonics: a hot sine gets a measurable 3rd harmonic
    let mut s = Saturation::new(SR, BLOCK);
    s.set_param(0, 18.0);
    s.reset();
    let x = sine(1000.0, 0.5, n, SR);
    let mut l = x.clone();
    let mut r = x.clone();
    run(&mut s, &mut l, &mut r, BLOCK);
    let seg = &l[n / 2..n / 2 + 4800];
    let h = |k: f64| {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, &v) in seg.iter().enumerate() {
            let w = 2.0 * std::f64::consts::PI * 1000.0 * k * i as f64 / SR as f64;
            re += v as f64 * w.cos();
            im += v as f64 * w.sin();
        }
        (re * re + im * im).sqrt()
    };
    assert!(h(3.0) / h(1.0) > 0.05);
    assert!(h(2.0) / h(1.0) > 0.005, "asymmetry makes even harmonics");
}

#[test]
fn gain_applies_db_and_ramps() {
    let mut g = GainFx::new(SR, BLOCK);
    g.set_param(0, -6.0);
    g.reset();
    let mut l = vec![1.0f32; 100];
    let mut r = vec![1.0f32; 100];
    g.process(&mut l, &mut r);
    assert!((l[50] - 10f32.powf(-6.0 / 20.0)).abs() < 1e-6);
    g.set_param(0, 0.0);
    let mut l = vec![1.0f32; 2000];
    let mut r = vec![1.0f32; 2000];
    g.process(&mut l, &mut r);
    assert!(l[0] < 0.6, "ramps instead of jumping");
    assert!((l[1999] - 1.0).abs() < 1e-6);
}
