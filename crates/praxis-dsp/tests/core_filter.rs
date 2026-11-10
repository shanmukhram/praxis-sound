//! Filter magnitude responses against theory (within 0.5 dB), stability under fast modulation
//! and at maximum resonance. Run with `--nocapture` to see the measured numbers.

use praxis_dsp::filter::{DcBlocker, Ladder, OnePole, OnePoleMode, Svf, SvfMode};
use praxis_dsp::noise::White;

const SR: f32 = 48_000.0;

#[derive(Clone, Copy)]
struct C(f64, f64);
impl C {
    fn add(self, o: C) -> C {
        C(self.0 + o.0, self.1 + o.1)
    }
    fn mul(self, o: C) -> C {
        C(self.0 * o.0 - self.1 * o.1, self.0 * o.1 + self.1 * o.0)
    }
    fn scale(self, k: f64) -> C {
        C(self.0 * k, self.1 * k)
    }
    fn abs(self) -> f64 {
        self.0.hypot(self.1)
    }
}

/// Normalised analog frequency the bilinear transform (pre-warped at `fc`) maps `f` to.
fn omega(f: f64, fc: f64) -> f64 {
    let sr = SR as f64;
    (std::f64::consts::PI * f / sr).tan() / (std::f64::consts::PI * fc / sr).tan()
}

/// A theoretical magnitude response, linear gain as a function of Hz.
type Theory = Box<dyn Fn(f64) -> f64>;

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-300).log10()
}

/// Steady-state gain of `tick` at `f` Hz: settle, then least-squares sine fit.
fn measure(mut tick: impl FnMut(f32) -> f32, f: f64, amp: f64) -> f64 {
    let sr = SR as f64;
    let w = std::f64::consts::TAU * f / sr;
    let settle = 24_000;
    let n = 24_000;
    for i in 0..settle {
        tick((amp * (w * i as f64).sin()) as f32);
    }
    let (mut ss, mut cc, mut sc, mut ys, mut yc) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for i in settle..settle + n {
        let (s, c) = (w * i as f64).sin_cos();
        let y = tick((amp * s) as f32) as f64;
        ss += s * s;
        cc += c * c;
        sc += s * c;
        ys += y * s;
        yc += y * c;
    }
    let det = ss * cc - sc * sc;
    let a = (ys * cc - yc * sc) / det;
    let b = (yc * ss - ys * sc) / det;
    (a * a + b * b).sqrt() / amp
}

const FREQS: [f64; 9] = [
    40.0, 200.0, 500.0, 900.0, 1000.0, 1300.0, 3000.0, 8000.0, 16000.0,
];

/// Compares measured and theoretical gain in dB at every test frequency; skips points where
/// theory is below -80 dB.
fn check(
    name: &str,
    mut make: impl FnMut() -> Box<dyn FnMut(f32) -> f32>,
    theory: impl Fn(f64) -> f64,
) {
    let mut worst: f64 = 0.0;
    for f in FREQS {
        let t = db(theory(f));
        if t < -80.0 {
            continue;
        }
        let mut tick = make();
        let m = db(measure(&mut tick, f, 0.25));
        worst = worst.max((m - t).abs());
        assert!(
            (m - t).abs() < 0.5,
            "{name} at {f} Hz: measured {m:.3} dB, theory {t:.3} dB"
        );
    }
    println!("{name:>28}: worst deviation from theory {worst:.4} dB");
}

#[test]
fn svf_responses_match_theory() {
    let fc = 1000.0;
    for q in [0.707f64, 3.0] {
        let k = 1.0 / q;
        let den = move |f: f64| {
            let s = C(0.0, omega(f, fc));
            s.mul(s).add(s.scale(k)).add(C(1.0, 0.0))
        };
        let modes: Vec<(SvfMode, Theory)> = vec![
            (SvfMode::LowPass, Box::new(move |f| 1.0 / den(f).abs())),
            (
                SvfMode::HighPass,
                Box::new(move |f| omega(f, fc).powi(2) / den(f).abs()),
            ),
            (
                SvfMode::BandPass,
                Box::new(move |f| k * omega(f, fc) / den(f).abs()),
            ),
            (
                SvfMode::Notch,
                Box::new(move |f| (1.0 - omega(f, fc).powi(2)).abs() / den(f).abs()),
            ),
            (
                SvfMode::Peak,
                Box::new(move |f| (1.0 + omega(f, fc).powi(2)) / den(f).abs()),
            ),
            (SvfMode::AllPass, Box::new(|_| 1.0)),
        ];
        for (mode, theory) in modes {
            check(
                &format!("SVF {mode:?} Q {q}"),
                || {
                    let mut f = Svf::new(SR);
                    f.set_mode(mode);
                    f.set_cutoff_q(fc as f32, q as f32);
                    Box::new(move |x| f.tick(x))
                },
                &theory,
            );
        }
    }
    // Bell: (s^2 + k A^2 s + 1) / (s^2 + k s + 1) with k = 1 / (Q A).
    let (gain_db, q) = (9.0f32, 2.0f64);
    let a = 10f64.powf(gain_db as f64 / 40.0);
    let k = 1.0 / (q * a);
    check(
        "SVF Bell +9 dB Q 2",
        || {
            let mut f = Svf::new(SR);
            f.set_mode(SvfMode::Bell { gain_db });
            f.set_cutoff_q(fc as f32, q as f32);
            Box::new(move |x| f.tick(x))
        },
        |f| {
            let s = C(0.0, omega(f, fc));
            let num = s.mul(s).add(s.scale(k * a * a)).add(C(1.0, 0.0));
            let den = s.mul(s).add(s.scale(k)).add(C(1.0, 0.0));
            num.abs() / den.abs()
        },
    );
    // Shelves: plateau gains.
    for (mode, lo_db, hi_db) in [
        (SvfMode::LowShelf { gain_db: 6.0 }, 6.0, 0.0),
        (SvfMode::HighShelf { gain_db: -6.0 }, 0.0, -6.0),
    ] {
        let mut f = Svf::new(SR);
        f.set_mode(mode);
        f.set_cutoff_q(1000.0, 0.707);
        let lo = db(measure(|x| f.tick(x), 30.0, 0.25));
        f.reset();
        let hi = db(measure(|x| f.tick(x), 15_000.0, 0.25));
        println!("{mode:?}: {lo:.3} dB at 30 Hz, {hi:.3} dB at 15 kHz");
        assert!((lo - lo_db).abs() < 0.2 && (hi - hi_db).abs() < 0.2);
    }
}

#[test]
fn one_pole_responses_match_theory() {
    for fc in [100.0f64, 1000.0, 6000.0] {
        check(
            &format!("OnePole LP {fc} Hz"),
            || {
                let mut f = OnePole::new(SR);
                f.set_cutoff(fc as f32);
                Box::new(move |x| f.tick(x))
            },
            |f| 1.0 / (1.0 + omega(f, fc).powi(2)).sqrt(),
        );
        check(
            &format!("OnePole HP {fc} Hz"),
            || {
                let mut f = OnePole::new(SR);
                f.set_mode(OnePoleMode::HighPass);
                f.set_cutoff(fc as f32);
                Box::new(move |x| f.tick(x))
            },
            |f| {
                let w = omega(f, fc);
                w / (1.0 + w * w).sqrt()
            },
        );
    }
}

#[test]
fn ladder_response_matches_theory() {
    // Small signals (-68 dBFS) keep the tanh linear: |H| = (1 + k) / |(1 + s)^4 + k|. Near
    // resonance the loop gain is high, so even gentle saturation shifts the peak: at -48 dBFS
    // the 500 Hz / 0.9 case already reads 0.28 dB low, which is the saturation, not an error.
    for fc in [500.0f64, 2000.0] {
        for res in [0.0f32, 0.5, 0.9] {
            let mut probe = Ladder::new(SR);
            probe.set_resonance(res);
            let k = probe.feedback() as f64;
            check(
                &format!("Ladder {fc} Hz res {res}"),
                || {
                    let mut f = Ladder::new(SR);
                    f.set_cutoff(fc as f32);
                    f.set_resonance(res);
                    Box::new(move |x| f.tick(x * 0.0004) / 0.0004)
                },
                |f| {
                    let s = C(1.0, omega(f, fc));
                    let s2 = s.mul(s);
                    (1.0 + k) / s2.mul(s2).add(C(k, 0.0)).abs()
                },
            );
        }
    }
}

#[test]
fn dc_blocker_matches_its_transfer_function() {
    let r = (-std::f64::consts::TAU * 10.0 / SR as f64).exp();
    check(
        "DcBlocker 10 Hz",
        || {
            let mut d = DcBlocker::new(SR);
            Box::new(move |x| d.tick(x))
        },
        |f| {
            let w = std::f64::consts::TAU * f / SR as f64;
            let num = C(1.0 - w.cos(), w.sin());
            let den = C(1.0 - r * w.cos(), r * w.sin());
            num.abs() / den.abs()
        },
    );
}

#[test]
fn ladder_stable_at_max_resonance_with_noise() {
    for drive in [0.5f32, 1.0, 4.0] {
        for fc in [60.0f32, 1000.0, 12_000.0, 23_000.0] {
            let mut f = Ladder::new(SR);
            f.set_cutoff(fc);
            f.set_resonance(1.0);
            f.set_drive(drive);
            let mut noise = White::new(7);
            let mut peak: f32 = 0.0;
            for _ in 0..(SR as usize * 5) {
                let y = f.tick(noise.next());
                assert!(y.is_finite(), "NaN at {fc} Hz drive {drive}");
                peak = peak.max(y.abs());
            }
            println!("ladder res 1.0 drive {drive} at {fc} Hz: peak {peak:.3}");
            assert!(peak < 3.0 / drive.min(1.0), "peak {peak}");
        }
    }
    // Swept per sample at full resonance with loud noise.
    let mut f = Ladder::new(SR);
    f.set_resonance(1.0);
    let mut noise = White::new(9);
    for n in 0..(SR as usize * 5) {
        let t = n as f32 / SR;
        f.set_cutoff(20.0 * 1000f32.powf(0.5 + 0.5 * (t * 37.0).sin()));
        let y = f.tick(4.0 * noise.next());
        assert!(y.is_finite() && y.abs() < 3.0, "{y}");
    }
}

#[test]
fn ladder_self_oscillates_at_the_cutoff() {
    let fc = 880.0f32;
    let mut f = Ladder::new(SR);
    f.set_cutoff(fc);
    f.set_resonance(1.0);
    f.tick(0.01);
    let out: Vec<f32> = (0..(SR as usize * 3)).map(|_| f.tick(0.0)).collect();
    let tail = &out[out.len() - 48_000..];
    let rms = (tail.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
    let crossings = tail
        .windows(2)
        .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
        .count();
    println!("self-oscillation: rms {rms:.3}, {crossings} cycles per second");
    assert!(rms > 0.05, "does not sing: rms {rms}");
    assert!((crossings as f32 - fc).abs() < fc * 0.02);
    // Just below the edge it rings and dies away.
    let mut f = Ladder::new(SR);
    f.set_cutoff(fc);
    f.set_resonance(0.9);
    f.tick(0.5);
    let last = (0..48_000).map(|_| f.tick(0.0)).last().unwrap();
    assert!(last.abs() < 1e-6);
}

#[test]
fn svf_stable_under_per_sample_modulation() {
    for mode in [
        SvfMode::LowPass,
        SvfMode::HighPass,
        SvfMode::BandPass,
        SvfMode::Notch,
        SvfMode::Peak,
    ] {
        let mut f = Svf::new(SR);
        f.set_mode(mode);
        let mut noise = White::new(3);
        let mut peak: f32 = 0.0;
        for n in 0..(SR as usize * 5) {
            let t = n as f32 / SR;
            // 20 Hz .. 20 kHz, swept at audio rate (300 Hz), Q 20 .. 0.5.
            let m = (t * 300.0 * std::f32::consts::TAU).sin();
            f.set_cutoff_q(20.0 * 1000f32.powf(0.5 + 0.5 * m), 10.25 + 9.75 * m);
            let y = f.tick(noise.next());
            assert!(y.is_finite());
            peak = peak.max(y.abs());
        }
        println!("SVF {mode:?} under audio-rate modulation: peak {peak:.2}");
        assert!(peak < 60.0, "{mode:?} peak {peak}");
    }
    // A cutoff jump on a steady tone does not produce a transient beyond the signal level.
    let mut f = Svf::new(SR);
    f.set_cutoff_q(200.0, 0.707);
    let mut s = praxis_dsp::osc::Sine::new(SR);
    s.set_freq(100.0);
    let mut peak: f32 = 0.0;
    for n in 0..48_000 {
        if n == 24_000 {
            f.set_cutoff(8000.0);
        }
        peak = peak.max(f.tick(s.next()).abs());
    }
    assert!(peak < 1.2, "{peak}");
}
