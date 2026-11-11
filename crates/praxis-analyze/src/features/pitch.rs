//! Pitch track by YIN (de Cheveigne & Kawahara 2002) at a 10 ms hop.
//!
//! 1. The mono signal is low-passed (63-tap Blackman windowed sinc) and decimated by
//!    `floor(sample_rate / 16 kHz)` (48 kHz -> 16 kHz, 44.1 kHz -> 22.05 kHz).
//! 2. Each frame compares a 25 ms window with itself shifted by every lag up to 25 ms (40 Hz):
//!    the difference function `d(tau) = e(0) + e(tau) - 2 r(tau)`, with the cross-correlation
//!    `r` computed by FFT and the energies `e` from prefix sums.
//! 3. Cumulative mean normalised difference `d'(tau) = d(tau) tau / sum_{1..tau} d`; the first
//!    lag from 0.5 ms (2 kHz) where `d'` drops below [`YinConfig::dip`] is followed to its local
//!    minimum (or the global minimum is taken when none does), refined by parabolic interpolation.
//! 4. A frame is voiced when `d'` at the chosen lag is at most [`YinConfig::threshold`] and the
//!    window is louder than -50 dBFS and within 40 dB of the loudest window.

use realfft::RealFftPlanner;
use realfft::num_complex::Complex;

/// YIN settings.
#[derive(Clone, Debug, PartialEq)]
pub struct YinConfig {
    pub hop_s: f64,
    pub window_s: f64,
    pub min_hz: f64,
    pub max_hz: f64,
    /// First-dip threshold on `d'`.
    pub dip: f64,
    /// Voicing threshold on `d'` at the chosen lag.
    pub threshold: f64,
}

impl Default for YinConfig {
    fn default() -> Self {
        YinConfig {
            hop_s: 0.01,
            window_s: 0.025,
            min_hz: 40.0,
            max_hz: 2000.0,
            dip: 0.12,
            threshold: 0.25,
        }
    }
}

/// Low-pass FIR (Blackman windowed sinc) with cutoff `fc` as a fraction of the sample rate.
fn lowpass_taps(fc: f64, taps: usize) -> Vec<f64> {
    let m = (taps - 1) as f64;
    let mut h: Vec<f64> = (0..taps)
        .map(|i| {
            let x = i as f64 - m / 2.0;
            let sinc = if x == 0.0 {
                2.0 * fc
            } else {
                (2.0 * std::f64::consts::PI * fc * x).sin() / (std::f64::consts::PI * x)
            };
            let w = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / m).cos()
                + 0.08 * (4.0 * std::f64::consts::PI * i as f64 / m).cos();
            sinc * w
        })
        .collect();
    let s: f64 = h.iter().sum();
    for v in h.iter_mut() {
        *v /= s;
    }
    h
}

/// Low-passes and decimates `signal` by `factor`.
pub fn decimate(signal: &[f32], factor: usize) -> Vec<f32> {
    if factor <= 1 {
        return signal.to_vec();
    }
    let h = lowpass_taps(0.45 / factor as f64, 63);
    let half = h.len() / 2;
    let n = signal.len().div_ceil(factor);
    (0..n)
        .map(|j| {
            let c = j * factor;
            let mut s = 0.0f64;
            for (t, w) in h.iter().enumerate() {
                let idx = c as isize + t as isize - half as isize;
                if idx >= 0
                    && let Some(x) = signal.get(idx as usize)
                {
                    s += w * *x as f64;
                }
            }
            s as f32
        })
        .collect()
}

/// MIDI pitch (fractional) per frame, `None` when unvoiced, plus the hop in seconds.
pub fn yin(signal: &[f32], sample_rate: u32, cfg: &YinConfig) -> (Vec<Option<f32>>, f64) {
    let sr0 = sample_rate.max(1) as f64;
    let factor = ((sr0 / 16_000.0).floor() as usize).max(1);
    let x = decimate(signal, factor);
    let sr = sr0 / factor as f64;
    let hop0 = ((cfg.hop_s * sr0).round() as usize).max(1);
    let hop_s = hop0 as f64 / sr0;
    let frames = if signal.is_empty() {
        0
    } else {
        (signal.len() - 1) / hop0 + 1
    };
    let w = ((cfg.window_s * sr).round() as usize).max(8);
    let tau_max = ((sr / cfg.min_hz).ceil() as usize).max(4);
    let tau_min = ((sr / cfg.max_hz).floor() as usize).max(2);
    let seg = w + tau_max + 2;
    let nfft = (seg + w).next_power_of_two();
    let mut planner = RealFftPlanner::<f64>::new();
    let fwd = planner.plan_fft_forward(nfft);
    let inv = planner.plan_fft_inverse(nfft);
    let mut a_in = fwd.make_input_vec();
    let mut b_in = fwd.make_input_vec();
    let mut a_sp = fwd.make_output_vec();
    let mut b_sp = fwd.make_output_vec();
    let mut scratch = fwd.make_scratch_vec();
    let mut iscratch = inv.make_scratch_vec();
    let mut corr = inv.make_output_vec();
    let mut prefix = vec![0.0f64; seg + 1];
    let mut d = vec![0.0f64; tau_max + 1];
    let mut dn = vec![0.0f64; tau_max + 1];

    let mut energies = Vec::with_capacity(frames);
    let mut picks: Vec<Option<(f64, f64)>> = Vec::with_capacity(frames);
    for i in 0..frames {
        // Frame i is centred on decimated sample c; the window starts half a window earlier.
        let c = (i * hop0) as f64 / factor as f64;
        let start = c.round() as isize - (w / 2) as isize;
        let get = |j: usize| -> f64 {
            let idx = start + j as isize;
            if idx >= 0 {
                x.get(idx as usize).copied().unwrap_or(0.0) as f64
            } else {
                0.0
            }
        };
        for v in a_in.iter_mut() {
            *v = 0.0;
        }
        for v in b_in.iter_mut() {
            *v = 0.0;
        }
        for j in 0..seg {
            let v = get(j);
            b_in[j] = v;
            prefix[j + 1] = prefix[j] + v * v;
            if j < w {
                a_in[j] = v;
            }
        }
        let e0 = prefix[w];
        energies.push(e0 / w as f64);
        if e0 <= 1e-12 {
            picks.push(None);
            continue;
        }
        if fwd
            .process_with_scratch(&mut a_in, &mut a_sp, &mut scratch)
            .is_err()
            || fwd
                .process_with_scratch(&mut b_in, &mut b_sp, &mut scratch)
                .is_err()
        {
            picks.push(None);
            continue;
        }
        // r(tau) = sum_j a[j] b[j + tau]  <=>  IFFT(conj(A) B).
        for (bs, asp) in b_sp.iter_mut().zip(a_sp.iter()) {
            *bs = asp.conj() * *bs;
        }
        if let Some(first) = b_sp.first_mut() {
            *first = Complex::new(first.re, 0.0);
        }
        if let Some(last) = b_sp.last_mut() {
            *last = Complex::new(last.re, 0.0);
        }
        if inv
            .process_with_scratch(&mut b_sp, &mut corr, &mut iscratch)
            .is_err()
        {
            picks.push(None);
            continue;
        }
        let scale = 1.0 / nfft as f64;
        d[0] = 0.0;
        dn[0] = 1.0;
        let mut running = 0.0f64;
        for tau in 1..=tau_max {
            let et = prefix[tau + w] - prefix[tau];
            let v = (e0 + et - 2.0 * corr[tau] * scale).max(0.0);
            d[tau] = v;
            running += v;
            dn[tau] = if running > 0.0 {
                v * tau as f64 / running
            } else {
                1.0
            };
        }
        let mut chosen = None;
        let mut tau = tau_min;
        while tau < tau_max {
            if dn[tau] < cfg.dip {
                while tau + 1 < tau_max && dn[tau + 1] < dn[tau] {
                    tau += 1;
                }
                chosen = Some(tau);
                break;
            }
            tau += 1;
        }
        let tau = chosen.unwrap_or_else(|| {
            (tau_min..tau_max)
                .min_by(|a, b| dn[*a].total_cmp(&dn[*b]))
                .unwrap_or(tau_min)
        });
        // Parabolic interpolation.
        let (a, b, cc) = (dn[tau - 1], dn[tau], dn[tau + 1]);
        let den = a - 2.0 * b + cc;
        let off = if den.abs() > 1e-12 {
            (0.5 * (a - cc) / den).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        let t = tau as f64 + off;
        picks.push(Some((sr / t, b)));
    }
    let loudest = energies.iter().fold(0.0f64, |m, v| m.max(*v));
    let floor = (loudest * 1e-4).max(1e-5);
    let track = picks
        .iter()
        .zip(energies.iter())
        .map(|(p, e)| match p {
            Some((hz, dprime)) if *dprime <= cfg.threshold && *e >= floor && *hz > 0.0 => {
                Some((69.0 + 12.0 * (hz / 440.0).log2()) as f32)
            }
            _ => None,
        })
        .collect();
    (track, hop_s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(f: f64, secs: f64, sr: u32) -> Vec<f32> {
        (0..(secs * sr as f64) as usize)
            .map(|i| {
                let t = i as f64 / sr as f64;
                let mut s = 0.0;
                for h in 1..=5 {
                    s += (2.0 * std::f64::consts::PI * f * h as f64 * t).sin() / h as f64;
                }
                (0.3 * s) as f32
            })
            .collect()
    }

    #[test]
    fn harmonic_tone_reads_its_pitch() {
        for (sr, f, midi) in [
            (48_000, 220.0, 57.0),
            (44_100, 110.0, 45.0),
            (48_000, 880.0, 81.0),
        ] {
            let (t, hop) = yin(&tone(f, 0.5, sr), sr, &YinConfig::default());
            assert!((hop - 0.01).abs() < 1e-3);
            let voiced: Vec<f32> = t.iter().flatten().copied().collect();
            assert!(voiced.len() as f64 > 0.8 * t.len() as f64, "{sr} {f}");
            let mut v = voiced.clone();
            v.sort_by(|a, b| a.total_cmp(b));
            let med = v[v.len() / 2];
            assert!((med - midi).abs() < 0.1, "{f} Hz read {med}");
        }
    }

    #[test]
    fn silence_is_unvoiced() {
        let (t, _) = yin(&vec![0.0; 9600], 48_000, &YinConfig::default());
        assert!(t.iter().all(|p| p.is_none()));
    }
}
