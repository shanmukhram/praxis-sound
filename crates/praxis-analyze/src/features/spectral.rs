//! Spectral balance, tonality and MFCCs from one streaming STFT pass.
//!
//! One Hann STFT of the mono signal (frame about 85 ms: 4096 samples at 44.1/48 kHz, hop 10 ms,
//! frames centred like [`super::stft`]). Nothing but small per-frame results is stored, so long
//! clips stay cheap in memory.
//!
//! | Measure | Method |
//! | --- | --- |
//! | centroid | power-weighted mean frequency of the summed power spectrum (so it sits with the rolloff) |
//! | rolloff | frequency under which 85 % of the summed power lies |
//! | flatness | mean over sounding frames of geometric / arithmetic mean power, 20 Hz .. 16 kHz |
//! | bands | summed power in 10 octave bands centred 31.5 Hz .. 16 kHz (edges `fc / sqrt 2 .. fc * sqrt 2`), dB relative to the power of all ten |
//! | tilt | least-squares slope of the band levels against octave number, dB per octave |
//! | centroid track | per-frame power-weighted centroid (0 for silent frames) |
//! | tonality | per sounding frame `0.5 * (1 - whitened flatness / 0.5615) + 0.5 * peak ratio`, then the mean |
//! | MFCC | 40 mel bands 40 Hz .. 8 kHz, log power, DCT-II, coefficients 0..12 |
//!
//! *Whitened flatness* divides each bin's power by its local mean (+-16 bins) before the
//! geometric / arithmetic ratio (both weighted by the local mean power, so the empty space
//! between partials does not count), so the colour of a noise (white, pink, brown) does not matter:
//! any noise reads about `exp(-0.5772) = 0.5615`, a set of partials about 0. The *peak ratio* is
//! the fraction of power within +-2 bins of peaks standing at least 15 dB above their local
//! geometric mean (+-16 bins), 40 Hz .. 10 kHz: about 1 for tones, about 0 for noise.
//! A frame is *sounding* when its power is within 60 dB of the loudest frame and above -100 dBFS.

use super::stft::{FrameAnalyzer, frame_len_for};

/// Octave band centres, Hz.
pub const BAND_CENTRES_HZ: [f64; 10] = [
    31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
/// Number of MFCC coefficients kept (0..12).
pub const MFCC_COEFFS: usize = 13;
const MEL_BANDS: usize = 40;
/// Whitened flatness of any noise: `exp(-Euler-Mascheroni)`.
const NOISE_FLATNESS: f64 = 0.561_459_5;
const LOCAL_RADIUS: usize = 16;
const PEAK_DB: f64 = 15.0;

/// Everything measured in the STFT pass.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SpectrumAnalysis {
    pub hop_s: f64,
    pub centroid_hz: f64,
    pub rolloff_hz: f64,
    pub flatness: f64,
    pub bands_db: Vec<f64>,
    pub tilt_db_per_oct: f64,
    /// Per frame (10 ms): centroid, Hz (0 when silent).
    pub centroid_track_hz: Vec<f64>,
    /// Per frame: power (sum over bins), for weighting.
    pub frame_power: Vec<f64>,
    /// Per frame: tonality 0..1, `None` when not sounding.
    pub tonality_track: Vec<Option<f64>>,
    pub tonality: f64,
    /// Per frame MFCCs.
    pub mfcc: Vec<[f32; MFCC_COEFFS]>,
}

fn mel(hz: f64) -> f64 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn mel_inv(m: f64) -> f64 {
    700.0 * (10f64.powf(m / 2595.0) - 1.0)
}

/// Triangular mel filters as `(first bin, weights)`.
fn mel_filters(bins: usize, bin_hz: f64, nyquist: f64) -> Vec<(usize, Vec<f64>)> {
    let lo = mel(40.0);
    let hi = mel(8000.0f64.min(nyquist * 0.95));
    let pts: Vec<f64> = (0..MEL_BANDS + 2)
        .map(|i| mel_inv(lo + (hi - lo) * i as f64 / (MEL_BANDS + 1) as f64))
        .collect();
    (0..MEL_BANDS)
        .map(|b| {
            let (f0, f1, f2) = (pts[b], pts[b + 1], pts[b + 2]);
            let k0 = ((f0 / bin_hz).floor() as usize).min(bins - 1);
            let k2 = ((f2 / bin_hz).ceil() as usize).min(bins - 1);
            let w: Vec<f64> = (k0..=k2)
                .map(|k| {
                    let f = k as f64 * bin_hz;
                    if f <= f0 || f >= f2 {
                        0.0
                    } else if f <= f1 {
                        (f - f0) / (f1 - f0).max(1e-9)
                    } else {
                        (f2 - f) / (f2 - f1).max(1e-9)
                    }
                })
                .collect();
            (k0, w)
        })
        .collect()
}

fn dct_ii(x: &[f64; MEL_BANDS]) -> [f32; MFCC_COEFFS] {
    let mut out = [0.0f32; MFCC_COEFFS];
    let n = MEL_BANDS as f64;
    for (c, o) in out.iter_mut().enumerate() {
        let mut s = 0.0;
        for (i, v) in x.iter().enumerate() {
            s += v * (std::f64::consts::PI * c as f64 * (i as f64 + 0.5) / n).cos();
        }
        *o = (s * (2.0 / n).sqrt()) as f32;
    }
    out
}

/// Least-squares slope of `y` against `x`.
pub fn slope(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len().min(y.len());
    if n < 2 {
        return 0.0;
    }
    let mx = x[..n].iter().sum::<f64>() / n as f64;
    let my = y[..n].iter().sum::<f64>() / n as f64;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for i in 0..n {
        sxy += (x[i] - mx) * (y[i] - my);
        sxx += (x[i] - mx) * (x[i] - mx);
    }
    if sxx > 0.0 { sxy / sxx } else { 0.0 }
}

/// Runs the STFT pass over `signal` (mono) with a hop of `hop` samples.
pub fn analyze(signal: &[f32], sample_rate: u32, hop: usize) -> SpectrumAnalysis {
    let sr = sample_rate.max(1) as f64;
    let hop = hop.max(1);
    let mut fa = FrameAnalyzer::new(frame_len_for(0.085, sample_rate.max(1)));
    let n = fa.frame_len();
    let bins = fa.bins();
    let bin_hz = sr / n as f64;
    let nyquist = sr / 2.0;
    let frames = if signal.is_empty() {
        0
    } else {
        (signal.len() - 1) / hop + 1
    };
    let half = (n / 2) as isize;
    let mut mags = vec![0.0f32; bins];
    let mut power = vec![0.0f64; bins];
    let mut sum_pow = vec![0.0f64; bins];
    let filters = mel_filters(bins, bin_hz, nyquist);
    let k_of = |hz: f64| ((hz / bin_hz).round() as usize).clamp(1, bins - 1);
    let (flat_lo, flat_hi) = (k_of(20.0), k_of(16000.0f64.min(nyquist)));
    let (ton_lo, ton_hi) = (k_of(40.0), k_of(10000.0f64.min(nyquist)));

    let mut centroid_track = Vec::with_capacity(frames);
    let mut frame_power = Vec::with_capacity(frames);
    let mut flat_frames = Vec::with_capacity(frames);
    let mut ton_frames: Vec<f64> = Vec::with_capacity(frames);
    let mut mfcc = Vec::with_capacity(frames);
    let mut logp = vec![0.0f64; bins];
    let mut prefix_p = vec![0.0f64; bins + 1];
    let mut prefix_l = vec![0.0f64; bins + 1];

    for i in 0..frames {
        let start = (i * hop) as isize - half;
        fa.magnitudes_with(
            |j| {
                let s = start + j as isize;
                if s >= 0 {
                    signal.get(s as usize).copied().unwrap_or(0.0)
                } else {
                    0.0
                }
            },
            &mut mags,
        );
        let mut total = 0.0f64;
        let (mut cw, mut cm) = (0.0f64, 0.0f64);
        for k in 0..bins {
            let m = mags[k] as f64;
            let p = m * m;
            power[k] = p;
            total += p;
            sum_pow[k] += p;
            if k > 0 {
                cw += k as f64 * bin_hz * p;
                cm += p;
            }
        }
        frame_power.push(total);
        centroid_track.push(if cm > 1e-12 { cw / cm } else { 0.0 });

        // Raw flatness.
        let (mut sl, mut sp, mut cnt) = (0.0f64, 0.0f64, 0usize);
        for p in &power[flat_lo..=flat_hi.max(flat_lo)] {
            sl += (p + 1e-20).ln();
            sp += p;
            cnt += 1;
        }
        let am = sp / cnt.max(1) as f64;
        flat_frames.push(if am > 1e-20 {
            ((sl / cnt.max(1) as f64).exp() / am).clamp(0.0, 1.0)
        } else {
            0.0
        });

        // Tonality: whitened flatness and peak ratio.
        for k in 0..bins {
            logp[k] = (power[k] + 1e-20).ln();
            prefix_p[k + 1] = prefix_p[k] + power[k];
            prefix_l[k + 1] = prefix_l[k] + logp[k];
        }
        let (mut wl, mut ws, mut wsum) = (0.0f64, 0.0f64, 0.0f64);
        let (mut peak_e, mut band_e) = (0.0f64, 0.0f64);
        let thr = PEAK_DB * std::f64::consts::LN_10 / 10.0;
        let mut k = ton_lo;
        let mut last_peak_end = 0usize;
        while k <= ton_hi.max(ton_lo) {
            let a = k.saturating_sub(LOCAL_RADIUS);
            let b = (k + LOCAL_RADIUS + 1).min(bins);
            let cntw = (b - a) as f64;
            let local_mean = (prefix_p[b] - prefix_p[a]) / cntw;
            let local_logmean = (prefix_l[b] - prefix_l[a]) / cntw;
            if local_mean > 1e-20 {
                // Weighted by the local power, so empty regions between partials (which are
                // locally "flat") do not count as noise.
                let w = power[k] / local_mean;
                wl += local_mean * (w + 1e-20).ln();
                ws += power[k];
                wsum += local_mean;
            }
            band_e += power[k];
            let is_peak = k >= 2
                && k + 2 < bins
                && power[k] >= power[k - 1]
                && power[k] >= power[k + 1]
                && power[k] >= power[k - 2]
                && power[k] >= power[k + 2]
                && logp[k] - local_logmean >= thr;
            if is_peak {
                let lo = (k - 2).max(last_peak_end);
                let hi = (k + 2).min(ton_hi);
                for j in lo..=hi {
                    peak_e += power[j];
                }
                last_peak_end = hi + 1;
            }
            k += 1;
        }
        let wflat = if wsum > 0.0 && ws > 0.0 {
            ((wl / wsum).exp() / (ws / wsum)).clamp(0.0, 1.0)
        } else {
            NOISE_FLATNESS
        };
        let ratio = if band_e > 0.0 {
            (peak_e / band_e).clamp(0.0, 1.0)
        } else {
            0.0
        };
        ton_frames.push(
            (0.5 * (1.0 - wflat / NOISE_FLATNESS).clamp(0.0, 1.0) + 0.5 * ratio).clamp(0.0, 1.0),
        );

        // MFCC.
        let mut mb = [0.0f64; MEL_BANDS];
        for (b, (k0, w)) in filters.iter().enumerate() {
            let mut s = 0.0;
            for (j, wj) in w.iter().enumerate() {
                s += wj * power[k0 + j];
            }
            mb[b] = (s + 1e-10).log10();
        }
        mfcc.push(dct_ii(&mb));
    }

    // Sounding frames.
    let loudest = frame_power.iter().fold(0.0f64, |m, v| m.max(*v));
    let sounding = |p: f64| p > 0.0 && p >= loudest * 1e-6 && p > 1e-10;
    let mut tonality_track = Vec::with_capacity(frames);
    let (mut fl, mut fc, mut tn) = (0.0f64, 0usize, 0.0f64);
    for i in 0..frames {
        if sounding(frame_power[i]) {
            fl += flat_frames[i];
            tn += ton_frames[i];
            fc += 1;
            tonality_track.push(Some(ton_frames[i]));
        } else {
            tonality_track.push(None);
            centroid_track[i] = 0.0;
        }
    }
    let flatness = if fc > 0 { fl / fc as f64 } else { 0.0 };
    let tonality = if fc > 0 { tn / fc as f64 } else { 0.0 };

    // Long-term spectrum.
    let pow_sum: f64 = sum_pow[1..].iter().sum();
    let centroid_hz = if pow_sum > 0.0 {
        sum_pow
            .iter()
            .enumerate()
            .skip(1)
            .map(|(k, m)| k as f64 * bin_hz * m)
            .sum::<f64>()
            / pow_sum
    } else {
        0.0
    };
    let pow_total: f64 = sum_pow[1..].iter().sum();
    let rolloff_hz = if pow_total > 0.0 {
        let mut acc = 0.0;
        let mut r = 0.0;
        for (k, p) in sum_pow.iter().enumerate().skip(1) {
            acc += p;
            if acc >= 0.85 * pow_total {
                r = k as f64 * bin_hz;
                break;
            }
        }
        r
    } else {
        0.0
    };
    let mut band_pow = [0.0f64; 10];
    for (b, fc) in BAND_CENTRES_HZ.iter().enumerate() {
        let lo = fc / std::f64::consts::SQRT_2;
        let hi = fc * std::f64::consts::SQRT_2;
        for (k, p) in sum_pow.iter().enumerate().skip(1) {
            let f = k as f64 * bin_hz;
            if f >= lo && f < hi {
                band_pow[b] += p;
            }
        }
    }
    let bands_total: f64 = band_pow.iter().sum();
    let bands_db: Vec<f64> = band_pow
        .iter()
        .map(|p| {
            if bands_total > 0.0 && *p > 0.0 {
                (10.0 * (p / bands_total).log10()).max(-120.0)
            } else {
                -120.0
            }
        })
        .collect();
    // Tilt over the bands below Nyquist that hold any energy at all.
    let (xs, ys): (Vec<f64>, Vec<f64>) = BAND_CENTRES_HZ
        .iter()
        .zip(bands_db.iter())
        .enumerate()
        .filter(|(_, (fc, db))| **fc < nyquist && **db > -120.0)
        .map(|(i, (_, db))| (i as f64, *db))
        .unzip();
    let tilt_db_per_oct = slope(&xs, &ys);

    SpectrumAnalysis {
        hop_s: hop as f64 / sr,
        centroid_hz,
        rolloff_hz,
        flatness,
        bands_db,
        tilt_db_per_oct,
        centroid_track_hz: centroid_track,
        frame_power,
        tonality_track,
        tonality,
        mfcc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use praxis_dsp::rng::Pcg32;

    fn sine(f: f64, secs: f64, sr: u32) -> Vec<f32> {
        (0..(secs * sr as f64) as usize)
            .map(|i| (0.5 * (2.0 * std::f64::consts::PI * f * i as f64 / sr as f64).sin()) as f32)
            .collect()
    }

    fn noise(secs: f64, sr: u32, seed: u64) -> Vec<f32> {
        let mut r = Pcg32::new(seed);
        (0..(secs * sr as f64) as usize)
            .map(|_| 0.3 * r.next_bipolar())
            .collect()
    }

    #[test]
    fn tone_is_tonal_and_noise_is_not() {
        let sr = 48_000;
        let hop = 480;
        let t = analyze(&sine(440.0, 1.0, sr), sr, hop);
        let n = analyze(&noise(1.0, sr, 3), sr, hop);
        assert!(t.tonality > 0.85, "tone {}", t.tonality);
        assert!(n.tonality < 0.15, "noise {}", n.tonality);
        assert!((t.centroid_hz - 440.0).abs() < 40.0, "{}", t.centroid_hz);
        assert!(n.flatness > 0.4 && t.flatness < 0.05);
        // The 500 Hz band holds the 440 Hz tone.
        let best =
            t.bands_db.iter().enumerate().fold(
                (0, f64::MIN),
                |b, (i, v)| if *v > b.1 { (i, *v) } else { b },
            );
        assert_eq!(best.0, 4);
    }

    #[test]
    fn white_noise_tilts_up_three_db_per_octave() {
        let sr = 48_000;
        let a = analyze(&noise(2.0, sr, 9), sr, 480);
        assert!(
            (a.tilt_db_per_oct - 3.0).abs() < 0.8,
            "{}",
            a.tilt_db_per_oct
        );
    }
}
