//! Interpolation accuracy and the variable-rate sample reader: pitch, loops, band-limiting.

use praxis_dsp::interp::{Quality, Resampler, hermite4, lagrange4};
use praxis_dsp::rng::Pcg32;

const SR: f64 = 48_000.0;

fn sine_table(hz: f64, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| (std::f64::consts::TAU * hz * i as f64 / SR).sin() as f32)
        .collect()
}

#[test]
fn hermite_interpolates_a_band_limited_sine_accurately() {
    let mut rng = Pcg32::new(1);
    for (hz, hermite_bound, lagrange_bound) in [(1000.0, 1e-4, 1e-4), (4000.0, 6e-3, 5e-3)] {
        let x = sine_table(hz, 4800);
        let (mut eh, mut el) = (0.0f64, 0.0f64);
        for _ in 0..20_000 {
            let pos = 2.0 + rng.next_f32() as f64 * 4790.0;
            let i = pos.floor() as usize;
            let t = (pos - i as f64) as f32;
            let want = (std::f64::consts::TAU * hz * pos / SR).sin();
            let h = hermite4(x[i - 1], x[i], x[i + 1], x[i + 2], t) as f64;
            let l = lagrange4(x[i - 1], x[i], x[i + 1], x[i + 2], t) as f64;
            eh = eh.max((h - want).abs());
            el = el.max((l - want).abs());
        }
        println!(
            "{hz} Hz sine: hermite max error {eh:.2e} ({:.1} dB), lagrange {el:.2e}",
            20.0 * eh.log10()
        );
        assert!(eh < hermite_bound, "hermite {eh}");
        assert!(el < lagrange_bound, "lagrange {el}");
    }
}

/// Frequency from rising zero crossings.
fn zero_crossing_freq(x: &[f32]) -> f64 {
    let mut first = None;
    let mut last = 0.0;
    let mut count = 0usize;
    for n in 1..x.len() {
        let (a, b) = (x[n - 1] as f64, x[n] as f64);
        if a < 0.0 && b >= 0.0 {
            let t = (n - 1) as f64 + a / (a - b);
            first.get_or_insert(t);
            last = t;
            count += 1;
        }
    }
    (count - 1) as f64 * SR / (last - first.unwrap())
}

#[test]
fn resampler_plays_at_the_requested_pitch() {
    let src = sine_table(500.0, 48_000 * 4);
    for quality in [Quality::Linear, Quality::Hermite, Quality::Sinc] {
        for semitones in [-7.0f64, 0.0, 3.5, 12.0] {
            let rate = 2f64.powf(semitones / 12.0);
            let mut r = Resampler::new(quality);
            r.set_rate(rate);
            r.start(0.0);
            let out: Vec<f32> = (0..96_000).map(|_| r.next(&src)).collect();
            let f = zero_crossing_freq(&out[100..]);
            let want = 500.0 * rate;
            let cents = 1200.0 * (f / want).log2();
            assert!(cents.abs() < 0.1, "{quality:?} {semitones}: {cents} cents");
        }
    }
}

#[test]
fn sustain_loop_is_seamless() {
    // 1 kHz sine with exactly 10 cycles in the loop 960..1440.
    let src = sine_table(1000.0, 2400);
    for quality in [Quality::Hermite, Quality::Sinc] {
        let mut r = Resampler::new(quality);
        r.set_rate(0.77);
        r.set_loop(Some((960, 1440)));
        r.start(0.0);
        let mut pos = 0.0f64;
        let mut worst = 0.0f64;
        for n in 0..48_000 {
            let v = r.next(&src) as f64;
            // The ideal continuous sine (the loop holds whole cycles, so its phase continues).
            let want = (std::f64::consts::TAU * 1000.0 * pos / SR).sin();
            pos += 0.77;
            if n > 20 {
                worst = worst.max((v - want).abs());
            }
        }
        assert!(!r.is_finished());
        println!("{quality:?}: loop playback max error vs continuous sine {worst:.2e}");
        assert!(worst < 2e-3, "{quality:?}: {worst}");
        // Releasing the loop plays on to the end and finishes.
        r.set_loop(None);
        let mut n = 0;
        while !r.is_finished() && n < 10_000 {
            r.next(&src);
            n += 1;
        }
        assert!(r.is_finished());
    }
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
}

#[test]
fn sinc_reader_does_not_alias_when_pitching_up() {
    // A 15 kHz partial played an octave up would land at 30 kHz, above Nyquist: it must vanish
    // instead of folding back to 18 kHz.
    let src = sine_table(15_000.0, 48_000);
    let mut sinc = Resampler::new(Quality::Sinc);
    let mut herm = Resampler::new(Quality::Hermite);
    for r in [&mut sinc, &mut herm] {
        r.set_rate(2.0);
        r.start(1000.0);
    }
    let a: Vec<f32> = (0..20_000).map(|_| sinc.next(&src)).collect();
    let b: Vec<f32> = (0..20_000).map(|_| herm.next(&src)).collect();
    let (ra, rb) = (rms(&a), rms(&b));
    println!(
        "15 kHz an octave up: sinc alias {:.1} dB, hermite alias {:.1} dB (re a full-scale sine)",
        20.0 * (ra * 2f64.sqrt()).log10(),
        20.0 * (rb * 2f64.sqrt()).log10()
    );
    assert!(20.0 * (ra * 2f64.sqrt()).log10() < -60.0);
    assert!(20.0 * (rb * 2f64.sqrt()).log10() > -20.0);

    // In the passband the sinc reader is transparent: 3 kHz -> 6 kHz keeps its level.
    let src = sine_table(3000.0, 48_000);
    let mut r = Resampler::new(Quality::Sinc);
    r.set_rate(2.0);
    r.start(1000.0);
    let out: Vec<f32> = (0..20_000).map(|_| r.next(&src)).collect();
    let level = 20.0 * (rms(&out) * 2f64.sqrt()).log10();
    println!("3 kHz an octave up through sinc: {level:.3} dB");
    assert!(level.abs() < 0.1);
}

#[test]
fn stereo_reads_stay_in_lockstep() {
    let l = sine_table(440.0, 4800);
    let rch: Vec<f32> = l.iter().map(|v| -v).collect();
    let mut r = Resampler::new(Quality::Hermite);
    r.set_rate(1.234);
    r.start(10.5);
    for _ in 0..3000 {
        let (a, b) = r.next_stereo(&l, &rch);
        assert_eq!(a, -b);
    }
    // An end point earlier than the slice stops playback there.
    let mut r = Resampler::new(Quality::Linear);
    r.set_end(Some(100));
    r.start(0.0);
    let n = (0..1000).take_while(|_| {
        r.next(&l);
        !r.is_finished()
    });
    assert_eq!(n.count(), 99);
}
