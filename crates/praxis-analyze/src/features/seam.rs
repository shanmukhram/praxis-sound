//! Loop-seam analysis: is the wrap from the last sample back to the first an ordinary moment of
//! the music, or a discontinuity?
//!
//! The clip is treated as circular. Four measures are taken at the seam and compared with the
//! same measure elsewhere:
//!
//! | Measure | At the seam | Compared with | Catches |
//! | --- | --- | --- | --- |
//! | sample jump | `max_ch abs(x[0] - x[N-1])` | the largest `abs(x[n] - x[n-1])` within 5 ms either side | steps, phase breaks |
//! | first-difference energy | sum of `dx^2`, both channels, 1 ms window centred on the wrap | the largest value over 1 ms windows throughout the clip | clicks |
//! | transient flux | short frames (~5 ms): log-spectral energy in the frame centred on the wrap that neither neighbour frame (half a frame earlier, later) has | the largest value at all other positions | broadband clicks |
//! | drop flux | long frames (~43 ms): median over bins of the log-spectral decrease from the frame ending at the wrap to the frame starting there | the largest value at all other positions | cut-off reverb tails |
//!
//! Positions within `exclude_s` of the seam are left out of the comparison distributions.
//! Musical onsets raise the transient and energy measures everywhere in the clip, so an onset on
//! the downbeat at the seam is not an outlier; a click larger than any transient in the music is.
//! Tails that decay naturally never cause a sudden broadband drop, and a note-off empties only
//! the bins of its partials (the median ignores it), so the drop measure isolates broadband tails
//! that were cut instead of wrapped. Ringing tonal tails that are cut stop abruptly, which the
//! transient and jump measures see.
//!
//! Known limit: when the downbeat at the seam brings its own broadband energy (e.g. a dense
//! reverb that builds up within a few ms) louder than the cut tail, nothing measurable is missing
//! at the seam and a cut tail is not flagged.
//!
//! The seam passes when the jump is at most `jump_margin` times the local maximum and every
//! spectral/energy measure is at most `margin_db` above its reference (`excess_db <= 0`).
//! Ratios are floored at -120 dB.

use super::stft::{FrameAnalyzer, frame_len_for};

/// Seam analysis settings.
#[derive(Clone, Debug, PartialEq)]
pub struct SeamConfig {
    /// Half-width of the local window for the sample-jump comparison, seconds.
    pub local_s: f64,
    /// The jump may be this many times the local maximum jump.
    pub jump_margin: f64,
    /// Window of the first-difference energy measure, seconds.
    pub diff_window_s: f64,
    /// Frame of the transient-flux measure, seconds.
    pub short_frame_s: f64,
    /// Frame of the drop-flux measure, seconds.
    pub long_frame_s: f64,
    /// Log compression of spectra, `ln(1 + compression * |X|)`.
    pub compression: f32,
    /// Positions this close to the seam are excluded from the reference distributions, seconds.
    pub exclude_s: f64,
    /// Allowed excess over the reference, dB.
    pub margin_db: f64,
}

impl Default for SeamConfig {
    fn default() -> Self {
        SeamConfig {
            local_s: 0.005,
            jump_margin: 1.25,
            diff_window_s: 0.001,
            short_frame_s: 0.005,
            long_frame_s: 0.04,
            compression: 1000.0,
            exclude_s: 0.05,
            margin_db: 2.0,
        }
    }
}

/// Result of [`analyze_seam`].
#[derive(Clone, Debug, PartialEq)]
pub struct SeamAnalysis {
    /// Largest jump across the wrap over both channels.
    pub jump: f64,
    /// Largest ordinary sample-to-sample jump within `local_s` of the seam.
    pub local_max_jump: f64,
    /// RMS of the ordinary jumps within `local_s` of the seam.
    pub local_rms_jump: f64,
    /// `jump / local_rms_jump` (1.0 = a typical jump at that point in the music).
    pub jump_ratio: f64,
    /// `jump <= jump_margin * local_max_jump`.
    pub jump_ok: bool,
    /// First-difference energy at the seam over the clip maximum, dB.
    pub diff_energy_db: f64,
    /// Transient flux at the seam over the clip maximum, dB.
    pub transient_db: f64,
    /// Drop flux at the seam over the clip maximum, dB.
    pub drop_db: f64,
    /// Largest of the three, minus `margin_db` (`<= 0` passes).
    pub excess_db: f64,
    pub passed: bool,
}

fn ratio_db(seam: f64, reference: f64, power: bool) -> f64 {
    let eps = 1e-12;
    let r = (seam + eps) / (reference + eps);
    let db = if power {
        10.0 * r.log10()
    } else {
        20.0 * r.log10()
    };
    db.max(-120.0)
}

/// Circular distance of `p` from the seam (sample 0 / N), samples.
fn seam_distance(p: usize, n: usize) -> usize {
    let p = p % n.max(1);
    p.min(n - p)
}

/// Log spectra of circular frames centred at `centres` (positions may be negative or past the end).
fn circular_log_spectra(
    x: &[f32],
    frame_len: usize,
    centres: &[i64],
    compression: f32,
) -> Vec<Vec<f32>> {
    let n = x.len() as i64;
    let mut fa = FrameAnalyzer::new(frame_len);
    let half = frame_len as i64 / 2;
    centres
        .iter()
        .map(|c| {
            let mut m = vec![0.0f32; fa.bins()];
            let start = c - half;
            fa.magnitudes_with(|j| x[(start + j as i64).rem_euclid(n) as usize], &mut m);
            for v in m.iter_mut() {
                *v = (compression * *v).ln_1p();
            }
            m
        })
        .collect()
}

/// Mean over bins of the energy in `c` that neither `b` nor `a` has.
fn transient(b: &[f32], c: &[f32], a: &[f32]) -> f64 {
    let s: f64 = c
        .iter()
        .zip(b.iter().zip(a.iter()))
        .map(|(c, (b, a))| (*c - b.max(*a)).max(0.0) as f64)
        .sum();
    s / c.len().max(1) as f64
}

/// Broadband decrease from `b` to `a`: the median over bins of the positive drop. A note-off
/// empties only the bins of its partials (median unchanged); a cut reverb tail empties most bins.
fn drop(b: &[f32], a: &[f32]) -> f64 {
    let d: Vec<f64> = b
        .iter()
        .zip(a.iter())
        .map(|(b, a)| (*b - *a).max(0.0) as f64)
        .collect();
    super::median(&d)
}

/// Spectral measure at the seam and the maximum elsewhere: frames every half frame around the
/// circle; `f(prev, cur, next)` is evaluated at each centre.
fn spectral_measure(
    mono: &[f32],
    frame_len: usize,
    exclude: usize,
    compression: f32,
    f: impl Fn(&[f32], &[f32], &[f32]) -> f64,
) -> (f64, f64) {
    let n = mono.len();
    let hop = (frame_len / 2).max(1);
    let count = n / hop;
    let mut centres: Vec<i64> = (0..count as i64).map(|j| j * hop as i64).collect();
    // Frame half a frame before the seam, which `count * hop` does not reach when N % hop != 0.
    centres.push(-(hop as i64));
    let spectra = circular_log_spectra(mono, frame_len, &centres, compression);
    let last = spectra.len() - 1;
    let seam = if count >= 2 {
        f(&spectra[last], &spectra[0], &spectra[1])
    } else {
        0.0
    };
    let mut reference = 0.0f64;
    for j in 1..count.saturating_sub(1) {
        if seam_distance(j * hop, n) <= exclude {
            continue;
        }
        reference = reference.max(f(&spectra[j - 1], &spectra[j], &spectra[j + 1]));
    }
    (seam, reference)
}

/// Analyses the wrap of a loop. Channels of different lengths use the shorter length.
pub fn analyze_seam(
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
    cfg: &SeamConfig,
) -> SeamAnalysis {
    let n = left.len().min(right.len());
    let sr = sample_rate.max(1) as f64;
    if n < 4 {
        return SeamAnalysis {
            jump: 0.0,
            local_max_jump: 0.0,
            local_rms_jump: 0.0,
            jump_ratio: 1.0,
            jump_ok: true,
            diff_energy_db: 0.0,
            transient_db: 0.0,
            drop_db: 0.0,
            excess_db: -cfg.margin_db,
            passed: true,
        };
    }
    let chans: [Vec<f32>; 2] = [
        left[..n]
            .iter()
            .map(|x| super::finite_or_zero(*x))
            .collect(),
        right[..n]
            .iter()
            .map(|x| super::finite_or_zero(*x))
            .collect(),
    ];
    let at = |c: usize, i: i64| chans[c][i.rem_euclid(n as i64) as usize] as f64;
    // Circular first difference: d(i) = x[i] - x[i-1]; d(0) is the jump across the wrap.
    let d = |c: usize, i: i64| at(c, i) - at(c, i - 1);

    // 1. Sample jump against the local jumps.
    let jump = (0..2).map(|c| d(c, 0).abs()).fold(0.0, f64::max);
    let local = ((cfg.local_s * sr).round() as i64).clamp(1, (n as i64 - 1) / 2);
    let (mut lmax, mut lsum, mut lcount) = (0.0f64, 0.0f64, 0usize);
    for i in -local..=local {
        if i == 0 {
            continue;
        }
        for c in 0..2 {
            let v = d(c, i).abs();
            lmax = lmax.max(v);
            lsum += v * v;
            lcount += 1;
        }
    }
    let lrms = (lsum / lcount.max(1) as f64).sqrt();
    let jump_ok = jump <= cfg.jump_margin * lmax + 1e-7;
    let jump_ratio = if lrms > 0.0 {
        jump / lrms
    } else if jump > 0.0 {
        f64::INFINITY
    } else {
        1.0
    };

    // 2. First-difference energy, 1 ms windows.
    let half = ((cfg.diff_window_s * sr * 0.5).round() as i64).max(1);
    let energy = |centre: i64| -> f64 {
        let mut s = 0.0;
        for i in centre - half..centre + half {
            for c in 0..2 {
                let v = d(c, i);
                s += v * v;
            }
        }
        s
    };
    let exclude = (cfg.exclude_s * sr).round() as usize;
    let e_seam = energy(0);
    let mut e_ref = 0.0f64;
    let mut p = 2 * half;
    while p + half <= n as i64 {
        if seam_distance(p as usize, n) > exclude {
            e_ref = e_ref.max(energy(p));
        }
        p += 2 * half;
    }
    let diff_energy_db = ratio_db(e_seam, e_ref, true);

    // 3, 4. Spectral measures on the mono mix.
    let mono: Vec<f32> = chans[0]
        .iter()
        .zip(chans[1].iter())
        .map(|(l, r)| 0.5 * (l + r))
        .collect();
    let short = frame_len_for(cfg.short_frame_s, sample_rate.max(1)).min(n);
    let long = frame_len_for(cfg.long_frame_s, sample_rate.max(1)).min(n);
    let (t_seam, t_ref) = spectral_measure(&mono, short, exclude, cfg.compression, transient);
    let (d_seam, d_ref) =
        spectral_measure(&mono, long, exclude, cfg.compression, |b, _c, a| drop(b, a));
    let transient_db = ratio_db(t_seam, t_ref, false);
    let drop_db = ratio_db(d_seam, d_ref, false);

    let excess_db = diff_energy_db.max(transient_db).max(drop_db) - cfg.margin_db;
    SeamAnalysis {
        jump,
        local_max_jump: lmax,
        local_rms_jump: lrms,
        jump_ratio,
        jump_ok,
        diff_energy_db,
        transient_db,
        drop_db,
        excess_db,
        passed: jump_ok && excess_db <= 0.0,
    }
}
