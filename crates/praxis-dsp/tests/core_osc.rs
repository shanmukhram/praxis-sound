//! Oscillator quality: pitch accuracy, aliasing, FM sideband levels, wavetable mip-mapping.
//! Run with `--nocapture` to see the measured numbers.

use praxis_dsp::osc::{
    BLEP_LATENCY, FmOperator, FmPair, PolyBlepSaw, PolyBlepSquare, Sine, Triangle, Wavetable,
};

const SR: f32 = 48_000.0;

/// Frequency from rising zero crossings (linearly interpolated) after a settling time.
fn zero_crossing_freq(x: &[f32], sr: f64, skip: usize) -> f64 {
    let mut first = None;
    let mut last = 0.0;
    let mut count = 0usize;
    for n in (skip + 1)..x.len() {
        let (a, b) = (x[n - 1] as f64, x[n] as f64);
        if a < 0.0 && b >= 0.0 {
            let t = (n - 1) as f64 + a / (a - b);
            if first.is_none() {
                first = Some(t);
            }
            last = t;
            count += 1;
        }
    }
    let first = first.expect("no zero crossings");
    (count - 1) as f64 * sr / (last - first)
}

fn cents(measured: f64, target: f64) -> f64 {
    1200.0 * (measured / target).log2()
}

fn render(mut f: impl FnMut() -> f32, n: usize) -> Vec<f32> {
    (0..n).map(|_| f()).collect()
}

/// Power spectrum (rectangular window, bin k = k * sr / n) by direct DFT.
fn dft_power(x: &[f32]) -> Vec<f64> {
    let n = x.len();
    let mut out = vec![0.0; n / 2 + 1];
    for (k, o) in out.iter_mut().enumerate() {
        let w = std::f64::consts::TAU * k as f64 / n as f64;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, &v) in x.iter().enumerate() {
            let (s, c) = (w * i as f64).sin_cos();
            re += v as f64 * c;
            im -= v as f64 * s;
        }
        *o = (re * re + im * im) / (n as f64 * n as f64);
    }
    out
}

/// Amplitude of the component at `hz` (least-squares sine/cosine fit).
fn amplitude_at(x: &[f32], hz: f64, sr: f64) -> f64 {
    let w = std::f64::consts::TAU * hz / sr;
    let (mut ss, mut cc, mut sc, mut xs, mut xc) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        let (s, c) = (w * i as f64).sin_cos();
        ss += s * s;
        cc += c * c;
        sc += s * c;
        xs += v as f64 * s;
        xc += v as f64 * c;
    }
    let det = ss * cc - sc * sc;
    let a = (xs * cc - xc * sc) / det;
    let b = (xc * ss - xs * sc) / det;
    (a * a + b * b).sqrt()
}

fn saw_harmonics(n: usize) -> Vec<f32> {
    (1..=n).map(|k| 1.0 / k as f32).collect()
}

#[test]
fn pitch_accuracy_within_a_tenth_of_a_cent() {
    let n = (SR * 10.0) as usize;
    let sr = SR as f64;
    let mut worst: f64 = 0.0;
    for hz in [55.0f32, 440.0, 1234.5, 3999.0] {
        let mut sine = Sine::new(SR);
        sine.set_freq(hz);
        let mut saw = PolyBlepSaw::new(SR);
        saw.set_freq(hz);
        let mut sq = PolyBlepSquare::new(SR);
        sq.set_freq(hz);
        let mut pulse = PolyBlepSquare::new(SR);
        pulse.set_freq(hz);
        pulse.set_pulse_width(0.3);
        let mut tri = Triangle::new(SR);
        tri.set_freq(hz);
        let mut wt = Wavetable::from_harmonics(SR, &saw_harmonics(64));
        wt.set_freq(hz);
        let mut op = FmOperator::new(SR);
        op.set_freq(hz);
        let mut pair = FmPair::new(SR);
        pair.set_freq(hz);
        pair.set_index(1.0);
        let signals: Vec<(&str, Vec<f32>)> = vec![
            ("sine", render(|| sine.next(), n)),
            ("saw", render(|| saw.next(), n)),
            ("square", render(|| sq.next(), n)),
            ("pulse30", render(|| pulse.next(), n)),
            ("triangle", render(|| tri.next(), n)),
            ("wavetable", render(|| wt.next(), n)),
            ("fm-op", render(|| op.next(), n)),
            ("fm-pair", render(|| pair.next(), n)),
        ];
        for (name, x) in signals {
            let f = zero_crossing_freq(&x, sr, 1000);
            let c = cents(f, hz as f64);
            println!("{name:>10} {hz:>7} Hz: measured {f:.6} Hz, error {c:+.6} cent");
            assert!(c.abs() < 0.1, "{name} at {hz} Hz is {c} cents off");
            worst = worst.max(c.abs());
        }
    }
    println!("worst pitch error {worst:.6} cent");
}

/// Energy in the band above 20 kHz (where a 5 kHz saw sampled at 48 kHz can only have alias
/// products) and in the non-harmonic bins below 16 kHz, both relative to the total.
fn alias_report(x: &[f32], f0: f64) -> (f64, f64) {
    let p = dft_power(x);
    let n = x.len() as f64;
    let total: f64 = p.iter().sum();
    let bin_hz = SR as f64 / n;
    let mut high = 0.0;
    let mut low_alias = 0.0;
    for (k, &v) in p.iter().enumerate() {
        let f = k as f64 * bin_hz;
        if f > 20_500.0 {
            high += v;
        }
        let h = f / f0;
        let harmonic = (h - h.round()).abs() * f0 < 1.0;
        if f > 50.0 && f < 16_000.0 && !harmonic {
            low_alias += v;
        }
    }
    (
        10.0 * (high / total).log10(),
        10.0 * (low_alias.max(1e-30) / total).log10(),
    )
}

#[test]
fn blep_saw_alias_energy_far_below_naive() {
    // 5 kHz at 48 kHz: the sampled waveform repeats every 48 samples, so 4800 samples hold an
    // exact number of periods and every spectral line falls on a DFT bin.
    let f0 = 5000.0;
    let n = 4800;
    let skip = 1000;
    let mut saw = PolyBlepSaw::new(SR);
    saw.set_freq(f0 as f32);
    let blep: Vec<f32> = render(|| saw.next(), n + skip)[skip..].to_vec();
    let mut phase = 0.0f64;
    let naive: Vec<f32> = (0..n + skip)
        .map(|_| {
            let v = (2.0 * phase - 1.0) as f32;
            phase += f0 / SR as f64;
            if phase >= 1.0 {
                phase -= 1.0;
            }
            v
        })
        .collect::<Vec<_>>()[skip..]
        .to_vec();
    let (naive_high, naive_low) = alias_report(&naive, f0);
    let (blep_high, blep_low) = alias_report(&blep, f0);
    println!(
        "naive saw: energy above 20.5 kHz {naive_high:.1} dB, in-band alias {naive_low:.1} dB"
    );
    println!("BLEP saw : energy above 20.5 kHz {blep_high:.1} dB, in-band alias {blep_low:.1} dB");
    assert!(
        blep_high < naive_high - 15.0,
        "high-band alias only {:.1} dB below naive",
        naive_high - blep_high
    );
    assert!(blep_low < -80.0, "in-band alias {blep_low:.1} dB");
    assert!(
        naive_low > -30.0,
        "sanity: the naive saw must alias audibly"
    );

    // The pulse and the triangle get the same treatment.
    let mut sq = PolyBlepSquare::new(SR);
    sq.set_freq(f0 as f32);
    sq.set_pulse_width(0.3);
    let x: Vec<f32> = render(|| sq.next(), n + skip)[skip..].to_vec();
    let (h, l) = alias_report(&x, f0);
    println!("BLEP pulse (30 %): above 20.5 kHz {h:.1} dB, in-band alias {l:.1} dB");
    assert!(l < -80.0);
    let mut tri = Triangle::new(SR);
    tri.set_freq(f0 as f32);
    let x: Vec<f32> = render(|| tri.next(), n + skip)[skip..].to_vec();
    let (h, l) = alias_report(&x, f0);
    println!("BLAMP triangle: above 20.5 kHz {h:.1} dB, in-band alias {l:.1} dB");
    assert!(l < -90.0);
}

#[test]
fn blep_saw_keeps_its_harmonics() {
    // The correction must not dull the sound: harmonics of a 1 kHz saw up to 15 kHz stay within
    // 0.3 dB of the ideal 1/k series.
    let f0 = 1000.0;
    let mut saw = PolyBlepSaw::new(SR);
    saw.set_freq(f0 as f32);
    let x: Vec<f32> = render(|| saw.next(), 48_000 + 1000)[1000..].to_vec();
    let a1 = amplitude_at(&x, f0, SR as f64);
    for k in [2usize, 3, 5, 8, 12, 15] {
        let ak = amplitude_at(&x, f0 * k as f64, SR as f64);
        let db = 20.0 * (ak * k as f64 / a1).log10();
        println!("saw harmonic {k:>2}: {db:+.3} dB vs ideal");
        assert!(db.abs() < 0.3, "harmonic {k} off by {db} dB");
    }
    assert!(
        (a1 - 2.0 / std::f64::consts::PI).abs() < 0.01,
        "fundamental {a1}"
    );
}

#[test]
fn blep_latency_is_as_documented() {
    let mut saw = PolyBlepSaw::new(SR);
    saw.set_freq(1000.0);
    saw.reset(0.5);
    assert_eq!(saw.latency_samples(), BLEP_LATENCY);
    let x = render(|| saw.next(), 64);
    // Phase 0.5 is the zero crossing of the rising ramp; it appears BLEP_LATENCY samples late.
    assert!(x[BLEP_LATENCY].abs() < 0.05, "{}", x[BLEP_LATENCY]);
    assert!((x[BLEP_LATENCY + 10] - (2.0 * (0.5 + 10.0 / 48.0) - 1.0)).abs() < 0.02);
}

#[test]
fn pulse_width_modulation_is_bounded_and_clean() {
    let mut sq = PolyBlepSquare::new(SR);
    sq.set_freq(220.0);
    let mut lfo = Sine::new(SR);
    lfo.set_freq(7.0);
    let mut max_abs: f32 = 0.0;
    for _ in 0..96_000 {
        sq.set_pulse_width(0.5 + 0.45 * lfo.next());
        let v = sq.next();
        assert!(v.is_finite());
        max_abs = max_abs.max(v.abs());
    }
    println!("PWM peak {max_abs}");
    // A band-limited edge overshoots by at most about 9 % of its height (2): never more.
    assert!(max_abs < 1.2, "PWM peak {max_abs}");

    // Even a width jumping between extremes every few samples stays bounded and finite.
    let mut sq = PolyBlepSquare::new(SR);
    sq.set_freq(1000.0);
    for n in 0..48_000 {
        sq.set_pulse_width(if (n / 3) % 2 == 0 { 0.05 } else { 0.95 });
        let v = sq.next();
        assert!(v.is_finite() && v.abs() < 2.0, "{v}");
    }
}

#[test]
fn wavetable_never_aliases() {
    // A 1023-harmonic saw table played at 5 kHz: every bin that is not a harmonic is empty.
    let f0 = 5000.0;
    let mut wt = Wavetable::from_harmonics(SR, &saw_harmonics(1023));
    wt.set_freq(f0 as f32);
    assert!(wt.top_harmonic() as f64 * f0 < SR as f64 / 2.0);
    let x = render(|| wt.next(), 4800);
    let (high, low) = alias_report(&x, f0);
    println!(
        "wavetable saw at 5 kHz: top harmonic {}, above 20.5 kHz {high:.1} dB, in-band alias {low:.1} dB",
        wt.top_harmonic()
    );
    assert!(high < -100.0 && low < -100.0);
}

#[test]
fn wavetable_morphs_between_frames() {
    let frames = vec![vec![1.0f32], vec![1.0, 0.5]];
    let mut wt = Wavetable::from_frames(SR, &frames);
    assert_eq!(wt.frames(), 2);
    wt.set_freq(1000.0);
    let x = render(|| wt.next(), 48_000);
    let h2 = amplitude_at(&x, 2000.0, SR as f64);
    assert!(
        h2 < 1e-5,
        "position 0 must be the pure first frame, got {h2}"
    );
    wt.set_position(1.0);
    let x = render(|| wt.next(), 48_000);
    let r = amplitude_at(&x, 2000.0, SR as f64) / amplitude_at(&x, 1000.0, SR as f64);
    assert!((r - 0.5).abs() < 1e-3, "harmonic ratio {r}");
    wt.set_position(0.5);
    let x = render(|| wt.next(), 48_000);
    let r = amplitude_at(&x, 2000.0, SR as f64) / amplitude_at(&x, 1000.0, SR as f64);
    assert!(r > 0.1 && r < 0.4, "midway ratio {r}");
}

/// Bessel function of the first kind, integer order, by its integral form.
fn bessel_j(n: i32, x: f64) -> f64 {
    let steps = 20_000;
    let mut s = 0.0;
    for i in 0..steps {
        let t = std::f64::consts::PI * (i as f64 + 0.5) / steps as f64;
        s += (n as f64 * t - x * t.sin()).cos();
    }
    s / steps as f64
}

#[test]
fn fm_sidebands_follow_bessel_amplitudes() {
    // Carrier f, modulator 4f, index 1: sin(wt + I sin 4wt) = sum J_n(I) sin((1 + 4n) wt), so
    // the line at f has J0(I) and the lines at 3f and 5f have J1(I).
    let f0 = 500.0;
    let index = 1.0;
    let mut pair = FmPair::new(SR);
    pair.set_freq(f0 as f32);
    pair.set_ratio(4.0);
    pair.set_index(index as f32);
    let x = render(|| pair.next(), 48_000);
    let sr = SR as f64;
    for (hz, want) in [
        (f0, bessel_j(0, index)),
        (3.0 * f0, bessel_j(1, index)),
        (5.0 * f0, bessel_j(1, index)),
        (7.0 * f0, bessel_j(2, index)),
        (9.0 * f0, bessel_j(2, index)),
    ] {
        let got = amplitude_at(&x, hz, sr);
        println!("FM line {hz:>6} Hz: {got:.5} (Bessel {want:.5})");
        assert!((got - want).abs() < 2e-3, "{hz}: {got} vs {want}");
    }
}

#[test]
fn oscillators_are_deterministic() {
    let run = || {
        let mut saw = PolyBlepSaw::new(SR);
        let mut fm = FmPair::new(SR);
        saw.set_freq(333.3);
        fm.set_freq(333.3);
        fm.set_index(2.0);
        fm.set_feedback(0.7);
        (0..10_000)
            .map(|_| saw.next().to_bits() ^ fm.next().to_bits())
            .collect::<Vec<u32>>()
    };
    assert_eq!(run(), run());
}
