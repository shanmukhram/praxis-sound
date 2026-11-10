//! Noise colour (spectral slope per octave) and seed reproducibility.

use praxis_dsp::noise::{Brown, Pink, White};
use realfft::RealFftPlanner;

const SR: f32 = 48_000.0;

/// Welch power spectral density (Hann, 50 % overlap), one value per bin of `size`.
fn welch_psd(x: &[f32], size: usize) -> Vec<f64> {
    let mut planner = RealFftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(size);
    let mut input = fft.make_input_vec();
    let mut spec = fft.make_output_vec();
    let window: Vec<f64> = (0..size)
        .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / size as f64).cos())
        .collect();
    let mut psd = vec![0.0; size / 2 + 1];
    let mut segments = 0;
    let mut start = 0;
    while start + size <= x.len() {
        for i in 0..size {
            input[i] = x[start + i] as f64 * window[i];
        }
        fft.process(&mut input, &mut spec).unwrap();
        for (p, c) in psd.iter_mut().zip(spec.iter()) {
            *p += c.norm_sqr();
        }
        segments += 1;
        start += size / 2;
    }
    psd.iter_mut().for_each(|p| *p /= segments as f64);
    psd
}

/// Least-squares slope of the mean PSD level (dB) per octave over octave bands centred
/// between `lo` and `hi` Hz.
fn slope_db_per_octave(x: &[f32], lo: f64, hi: f64) -> (f64, Vec<(f64, f64)>) {
    let size = 8192;
    let psd = welch_psd(x, size);
    let bin = SR as f64 / size as f64;
    let mut points = Vec::new();
    let mut fc = lo;
    while fc <= hi * 1.0001 {
        let (a, b) = (fc / 2f64.sqrt(), fc * 2f64.sqrt());
        let (ka, kb) = ((a / bin).ceil() as usize, (b / bin).floor() as usize);
        let mean = psd[ka..=kb].iter().sum::<f64>() / (kb - ka + 1) as f64;
        points.push((fc.log2(), 10.0 * mean.log10()));
        fc *= 2.0;
    }
    let n = points.len() as f64;
    let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let sxy: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let sxx: f64 = points.iter().map(|p| (p.0 - mx).powi(2)).sum();
    let slope = sxy / sxx;
    // Residuals from the fit line show how straight the spectrum is.
    let resid = points
        .iter()
        .map(|p| (2f64.powf(p.0), p.1 - (my + slope * (p.0 - mx))))
        .collect();
    (slope, resid)
}

fn render(mut f: impl FnMut() -> f32, n: usize) -> Vec<f32> {
    (0..n).map(|_| f()).collect()
}

#[test]
fn pink_noise_falls_3_db_per_octave() {
    let mut p = Pink::new(SR, 12345);
    let x = render(|| p.next(), 1 << 22);
    let (slope, resid) = slope_db_per_octave(&x, 31.25, 16_000.0);
    println!("pink slope {slope:.3} dB/octave (ideal -3.010)");
    for (f, r) in &resid {
        println!("  {f:>8.1} Hz: {r:+.3} dB from the line");
    }
    assert!((slope + 3.01).abs() < 0.15, "pink slope {slope}");
    assert!(resid.iter().all(|(_, r)| r.abs() < 0.5));
}

#[test]
fn brown_noise_falls_6_db_per_octave() {
    let mut b = Brown::new(SR, 777);
    let x = render(|| b.next(), 1 << 22);
    let (slope, _) = slope_db_per_octave(&x, 125.0, 8000.0);
    println!("brown slope {slope:.3} dB/octave (ideal -6.02 above the 8 Hz corner)");
    assert!((slope + 6.02).abs() < 0.3, "brown slope {slope}");
}

#[test]
fn white_noise_is_flat() {
    let mut w = White::new(5);
    let x = render(|| w.next(), 1 << 21);
    let (slope, resid) = slope_db_per_octave(&x, 62.5, 16_000.0);
    println!("white slope {slope:.3} dB/octave");
    assert!(slope.abs() < 0.1);
    assert!(resid.iter().all(|(_, r)| r.abs() < 0.5));
    let mean = x.iter().map(|v| *v as f64).sum::<f64>() / x.len() as f64;
    assert!(mean.abs() < 2e-3);
    assert!(x.iter().all(|v| (-1.0..1.0).contains(v)));
}

#[test]
fn noise_is_reproducible_by_seed() {
    let take =
        |mut f: Box<dyn FnMut() -> f32>| (0..4096).map(|_| f().to_bits()).collect::<Vec<u32>>();
    for seed in [0u64, 1, 42, u64::MAX] {
        let a = take(Box::new({
            let mut n = White::new(seed);
            move || n.next()
        }));
        let b = take(Box::new({
            let mut n = White::new(seed);
            move || n.next()
        }));
        assert_eq!(a, b);
        let pa = take(Box::new({
            let mut n = Pink::new(SR, seed);
            move || n.next()
        }));
        let pb = take(Box::new({
            let mut n = Pink::new(SR, seed);
            move || n.next()
        }));
        assert_eq!(pa, pb);
        let ba = take(Box::new({
            let mut n = Brown::new(SR, seed);
            move || n.next()
        }));
        let bb = take(Box::new({
            let mut n = Brown::new(SR, seed);
            move || n.next()
        }));
        assert_eq!(ba, bb);
    }
    // Different seeds differ; reseed restarts the sequence exactly.
    let mut a = Pink::new(SR, 1);
    let mut b = Pink::new(SR, 2);
    let va: Vec<f32> = (0..64).map(|_| a.next()).collect();
    let vb: Vec<f32> = (0..64).map(|_| b.next()).collect();
    assert_ne!(va, vb);
    a.reseed(1);
    let again: Vec<f32> = (0..64).map(|_| a.next()).collect();
    assert_eq!(va, again);
    let mut br = Brown::new(SR, 3);
    let first: Vec<f32> = (0..64).map(|_| br.next()).collect();
    for _ in 0..1000 {
        br.next();
    }
    br.reseed(3);
    let second: Vec<f32> = (0..64).map(|_| br.next()).collect();
    assert_eq!(first, second);
}

#[test]
fn pink_slope_holds_at_other_sample_rates() {
    for sr in [44_100.0f32, 96_000.0] {
        let mut p = Pink::new(sr, 99);
        let x = render(|| p.next(), 1 << 22);
        // Re-use the 48 kHz helper by scaling the band centres to the same bins.
        let scale = sr as f64 / SR as f64;
        let (slope, _) = slope_db_per_octave(&x, 31.25, 8000.0);
        println!("pink at {sr} Hz: slope {slope:.3} dB/octave (bands scaled by {scale:.3})");
        assert!((slope + 3.01).abs() < 0.2, "{sr}: {slope}");
    }
}
