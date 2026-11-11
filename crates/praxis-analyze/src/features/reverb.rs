//! Reverb decay (RT60) from the energy after transients, by Schroeder backward integration.
//!
//! The clip is cut into *decay segments*: from the loudest 10 ms frame within 100 ms after each
//! onset (and after the start of the clip) up to the next onset (or the end of the clip). A
//! segment is used when it is at least 250 ms long and its energy really falls: the mean of its
//! last 50 ms is at least 20 dB below its first frame. For each such segment the energy decay
//! curve `EDC(t) = sum_{u >= t} E(u)` (within the segment) is fitted by least squares between
//! -5 dB and -25 dB (T20; -5 to -15 dB, T10, when the curve does not reach -25 dB before the
//! segment's last 20 %), and `RT60 = -60 / slope`.
//!
//! The estimate is the median of the segment values weighted by the dB range each fitted.
//! Confidence = mean fit r^2 x `min(1, segments / 3)` x `min(1, mean fitted range / 20 dB)`.
//! A clip with no usable segment (dense music that never decays) has no estimate.
//!
//! What this measures is how the clip's sound dies away, room and instrument together: a dry
//! piano note decaying on its own also reads as a decay time.

use crate::profile::ReverbEstimate;

/// One segment's estimate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecayFit {
    pub start_s: f64,
    pub rt60_s: f64,
    pub r2: f64,
    pub range_db: f64,
}

fn fit_line(x: &[f64], y: &[f64]) -> (f64, f64) {
    let n = x.len() as f64;
    if x.len() < 3 {
        return (0.0, 0.0);
    }
    let mx = x.iter().sum::<f64>() / n;
    let my = y.iter().sum::<f64>() / n;
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (a, b) in x.iter().zip(y.iter()) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx) * (a - mx);
        syy += (b - my) * (b - my);
    }
    if sxx <= 0.0 || syy <= 0.0 {
        return (0.0, 0.0);
    }
    let slope = sxy / sxx;
    let r2 = (sxy * sxy) / (sxx * syy);
    (slope, r2)
}

/// Fits one segment of frame energies (`hop_s` apart). `None` when it does not decay enough.
pub fn fit_segment(energy: &[f64], hop_s: f64) -> Option<(f64, f64, f64)> {
    let n = energy.len();
    if n as f64 * hop_s < 0.25 {
        return None;
    }
    let first = energy[0];
    let tail_n = ((0.05 / hop_s).round() as usize).clamp(1, n);
    let tail = energy[n - tail_n..].iter().sum::<f64>() / tail_n as f64;
    if first <= 0.0 || tail > first * 0.01 {
        return None;
    }
    let mut edc = vec![0.0f64; n + 1];
    for i in (0..n).rev() {
        edc[i] = edc[i + 1] + energy[i];
    }
    if edc[0] <= 0.0 {
        return None;
    }
    let db: Vec<f64> = edc[..n]
        .iter()
        .map(|e| 10.0 * (e.max(1e-30) / edc[0]).log10())
        .collect();
    let limit = (n as f64 * 0.8) as usize;
    let i5 = db.iter().position(|v| *v <= -5.0)?;
    let (end_db, range) = if db.iter().take(limit).any(|v| *v <= -25.0) {
        (-25.0, 20.0)
    } else if db.iter().take(limit).any(|v| *v <= -15.0) {
        (-15.0, 10.0)
    } else {
        return None;
    };
    let ie = db.iter().position(|v| *v <= end_db)?;
    if ie <= i5 + 2 {
        return None;
    }
    let x: Vec<f64> = (i5..=ie).map(|i| i as f64 * hop_s).collect();
    let y: Vec<f64> = db[i5..=ie].to_vec();
    let (slope, r2) = fit_line(&x, &y);
    if slope >= -1e-6 {
        return None;
    }
    Some(((-60.0 / slope).clamp(0.01, 60.0), r2, range))
}

/// Decay fits of every usable segment. `frames` are 10 ms energies (sum of squares or mean
/// square, any scale), `onsets_s` onset times.
pub fn segment_fits(frames: &[f64], hop_s: f64, onsets_s: &[f64]) -> Vec<DecayFit> {
    let n = frames.len();
    let mut starts: Vec<usize> = std::iter::once(0usize)
        .chain(
            onsets_s
                .iter()
                .map(|t| (t / hop_s).round().max(0.0) as usize),
        )
        .filter(|i| *i < n)
        .collect();
    starts.sort_unstable();
    starts.dedup();
    let look = ((0.1 / hop_s).round() as usize).max(1);
    let mut out = Vec::new();
    for (k, s) in starts.iter().enumerate() {
        let next = starts.get(k + 1).copied().unwrap_or(n);
        let lim = (*s + look).min(next);
        let mut peak = *s;
        for i in *s..lim {
            if frames[i] > frames[peak] {
                peak = i;
            }
        }
        if next <= peak + 1 {
            continue;
        }
        if let Some((rt, r2, range)) = fit_segment(&frames[peak..next], hop_s) {
            out.push(DecayFit {
                start_s: peak as f64 * hop_s,
                rt60_s: rt,
                r2,
                range_db: range,
            });
        }
    }
    out
}

/// RT60 estimate from 10 ms frame energies and onsets (see the module docs).
pub fn estimate(frames: &[f64], hop_s: f64, onsets_s: &[f64]) -> Option<ReverbEstimate> {
    let fits = segment_fits(frames, hop_s, onsets_s);
    if fits.is_empty() {
        return None;
    }
    let mut v: Vec<(f64, f64)> = fits.iter().map(|f| (f.rt60_s, f.range_db)).collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f64 = v.iter().map(|x| x.1).sum();
    let mut acc = 0.0;
    let mut rt = v[0].0;
    for (r, w) in &v {
        acc += w;
        if acc >= total / 2.0 {
            rt = *r;
            break;
        }
    }
    let r2 = fits.iter().map(|f| f.r2).sum::<f64>() / fits.len() as f64;
    let range = total / fits.len() as f64;
    let confidence =
        (r2 * (fits.len() as f64 / 3.0).min(1.0) * (range / 20.0).min(1.0)).clamp(0.0, 1.0);
    Some(ReverbEstimate {
        rt60_s: rt as f32,
        confidence: confidence as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_decay_reads_its_rt60() {
        let hop = 0.01;
        for rt in [0.5, 1.5, 3.0] {
            // Energy falls 60 dB in `rt` seconds.
            let frames: Vec<f64> = (0..(rt * 1.5 / hop) as usize)
                .map(|i| 10f64.powf(-6.0 * i as f64 * hop / rt))
                .collect();
            let e = estimate(&frames, hop, &[]).expect("decays");
            assert!(
                (e.rt60_s as f64 - rt).abs() / rt < 0.1,
                "rt {rt} read {}",
                e.rt60_s
            );
        }
    }

    #[test]
    fn steady_sound_has_no_estimate() {
        let frames = vec![1.0; 500];
        assert!(estimate(&frames, 0.01, &[]).is_none());
    }
}
