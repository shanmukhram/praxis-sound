//! Section boundaries from the novelty of a chroma + MFCC self-similarity matrix (Foote 2000).
//!
//! 1. Features are averaged into blocks of 0.5 s (longer for clips over 5 minutes, so there are
//!    at most 600 blocks): the 12 chroma bins (L2-normalised) and MFCC 1..12 divided by
//!    [`MFCC_SCALE`].
//! 2. Each block's feature is averaged over +-2 s around it (texture, not single notes); the
//!    self-similarity of two blocks is `exp(-d^2 / 0.2)` with `d` their Euclidean distance,
//!    so a steady sound (noise bed, drone) is self-similar everywhere instead of having its tiny
//!    fluctuations blown up.
//! 3. A checkerboard kernel (Gaussian taper, half-width 4 s) slid along the diagonal gives the
//!    novelty curve, normalised to `mean within-side similarity - mean cross similarity`
//!    (-1..1). Peaks at least 4 s apart that reach [`MIN_NOVELTY`], stand above `mean + 0.5 sd`
//!    of the curve and above 10 % of its maximum are boundaries.
//! 4. A boundary is the middle of the region around its peak where the novelty stays above
//!    60 % of the peak (a change shows as twin peaks around it); boundaries snap to the nearest
//!    beat (within half a beat) when there are beats.
//! 5. Sections are labelled A, B, C ... in order of appearance; a section whose mean feature is
//!    at least 0.8 similar (the same measure) to an earlier section's reuses that letter.
//!
//! A clip shorter than two kernel widths (16 s) is one section; so is one without clear peaks.

use crate::profile::SectionBoundary;

const BLOCK_S: f64 = 0.5;
const MAX_BLOCKS: usize = 600;
const KERNEL_HALF_S: f64 = 4.0;
const SAME_SECTION: f64 = 0.8;
/// MFCCs (log10 mel power, orthonormal DCT) are divided by this before distances.
pub const MFCC_SCALE: f64 = 2.0;
/// Weakest normalised novelty that can mark a boundary.
pub const MIN_NOVELTY: f64 = 0.15;
const SIM_TAU: f64 = 0.2;
/// Features are averaged over +- this many seconds before comparing.
const SMOOTH_S: f64 = 2.0;

fn similar(a: &[f64], b: &[f64]) -> f64 {
    let d2: f64 = a.iter().zip(b.iter()).map(|(x, y)| (x - y) * (x - y)).sum();
    (-d2 / SIM_TAU).exp()
}

fn mean_rows<const N: usize>(rows: &[[f32; N]], from: usize, to: usize) -> [f64; N] {
    let mut acc = [0.0f64; N];
    let to = to.min(rows.len());
    if from >= to {
        return acc;
    }
    for r in &rows[from..to] {
        for (a, v) in acc.iter_mut().zip(r.iter()) {
            *a += *v as f64;
        }
    }
    let n = (to - from) as f64;
    for a in acc.iter_mut() {
        *a /= n;
    }
    acc
}

/// Section boundaries covering `0 .. duration_s`.
///
/// `chroma` frames are `chroma_hop_s` apart, `mfcc` frames `mfcc_hop_s` apart (coefficient 0
/// is ignored); `beats_s` (possibly empty) are used for snapping.
pub fn analyze<const M: usize>(
    chroma: &[[f32; 12]],
    chroma_hop_s: f64,
    mfcc: &[[f32; M]],
    mfcc_hop_s: f64,
    duration_s: f64,
    beats_s: &[f64],
) -> Vec<SectionBoundary> {
    let whole = || {
        vec![SectionBoundary {
            start_s: 0.0,
            end_s: duration_s.max(0.0),
            label: "A".to_string(),
        }]
    };
    if duration_s < 4.0 * KERNEL_HALF_S || chroma_hop_s <= 0.0 || mfcc_hop_s <= 0.0 || M < 2 {
        return whole();
    }
    let block_s = BLOCK_S.max(duration_s / MAX_BLOCKS as f64);
    let blocks = (duration_s / block_s).floor() as usize;
    if blocks < 4 {
        return whole();
    }
    let dims = 12 + M - 1;
    let mut feats: Vec<Vec<f64>> = Vec::with_capacity(blocks);
    for b in 0..blocks {
        let t0 = b as f64 * block_s;
        let t1 = t0 + block_s;
        let c = mean_rows(
            chroma,
            (t0 / chroma_hop_s) as usize,
            ((t1 / chroma_hop_s) as usize).max((t0 / chroma_hop_s) as usize + 1),
        );
        let m = mean_rows(
            mfcc,
            (t0 / mfcc_hop_s) as usize,
            ((t1 / mfcc_hop_s) as usize).max((t0 / mfcc_hop_s) as usize + 1),
        );
        let cn = c.iter().map(|v| v * v).sum::<f64>().sqrt();
        let mut f = Vec::with_capacity(dims);
        f.extend(c.iter().map(|v| if cn > 0.0 { v / cn } else { 0.0 }));
        f.extend(m.iter().skip(1).map(|v| v / MFCC_SCALE));
        feats.push(f);
    }
    // Texture, not single notes: each block's feature is the mean over +-2 s around it.
    let r = ((SMOOTH_S / block_s).round() as usize).max(1);
    let feats: Vec<Vec<f64>> = (0..blocks)
        .map(|i| {
            let a = i.saturating_sub(r);
            let b = (i + r + 1).min(blocks);
            let mut m = vec![0.0; dims];
            for f in &feats[a..b] {
                for (x, v) in m.iter_mut().zip(f.iter()) {
                    *x += v / (b - a) as f64;
                }
            }
            m
        })
        .collect();
    let ssm: Vec<Vec<f64>> = feats
        .iter()
        .map(|a| feats.iter().map(|b| similar(a, b)).collect())
        .collect();
    let k = ((KERNEL_HALF_S / block_s).round() as usize).max(2);
    let sigma = k as f64 / 2.0;
    let mut novelty = vec![0.0f64; blocks];
    for (i, nv) in novelty.iter_mut().enumerate() {
        let mut s = 0.0;
        let mut wsum = 0.0;
        for u in 0..k {
            for v in 0..k {
                let g = (-((u * u + v * v) as f64) / (2.0 * sigma * sigma)).exp();
                // Same-side pairs (before-before, after-after) +, cross pairs -.
                let (bu, au) = (i as isize - 1 - u as isize, i + u);
                let (bv, av) = (i as isize - 1 - v as isize, i + v);
                if bu < 0 || bv < 0 || au >= blocks || av >= blocks {
                    continue;
                }
                let (bu, bv) = (bu as usize, bv as usize);
                s += g * (ssm[bu][bv] + ssm[au][av] - ssm[bu][av] - ssm[au][bv]);
                wsum += 2.0 * g;
            }
        }
        *nv = if wsum > 0.0 { (s / wsum).max(0.0) } else { 0.0 };
    }
    let mean = novelty.iter().sum::<f64>() / blocks as f64;
    let sd = (novelty.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / blocks as f64).sqrt();
    let max = novelty.iter().fold(0.0f64, |m, v| m.max(*v));
    let thr = (mean + 0.5 * sd).max(0.1 * max).max(MIN_NOVELTY);
    let mut peaks: Vec<usize> = Vec::new();
    for i in k..blocks.saturating_sub(k) {
        let lo = i.saturating_sub(k);
        let hi = (i + k).min(blocks - 1);
        let is_max = (lo..=hi).all(|j| novelty[j] <= novelty[i]);
        if is_max && novelty[i] > thr && novelty[i] > 0.0 {
            if let Some(p) = peaks.last()
                && i - p < k
            {
                continue;
            }
            peaks.push(i);
        }
    }
    // Smoothing makes the blocks of a transition unlike both sides, so a change shows as twin
    // peaks around it (or a flat top): the boundary is the middle of the region around the
    // peak that stays above 60 % of it.
    let mut bounds: Vec<f64> = peaks
        .iter()
        .map(|p| {
            let top = 0.6 * novelty[*p];
            let mut a = *p;
            while a > 0 && novelty[a - 1] >= top {
                a -= 1;
            }
            let mut b = *p;
            while b + 1 < blocks && novelty[b + 1] >= top {
                b += 1;
            }
            0.5 * (a + b) as f64 * block_s
        })
        .collect();
    if beats_s.len() >= 2 {
        let period = (beats_s[beats_s.len() - 1] - beats_s[0]) / (beats_s.len() - 1) as f64;
        for b in bounds.iter_mut() {
            if let Some(n) = beats_s
                .iter()
                .min_by(|x, y| (*x - *b).abs().total_cmp(&(*y - *b).abs()))
                && (n - *b).abs() <= period / 2.0
            {
                *b = *n;
            }
        }
    }
    let mut edges = vec![0.0];
    for b in bounds {
        if b > edges[edges.len() - 1] + 1.0 && b < duration_s - 1.0 {
            edges.push(b);
        }
    }
    edges.push(duration_s);
    // Label by similarity of mean block features.
    let means: Vec<Vec<f64>> = edges
        .windows(2)
        .map(|w| {
            let a = ((w[0] / block_s) as usize).min(blocks - 1);
            let b = ((w[1] / block_s) as usize).clamp(a + 1, blocks);
            let mut m = vec![0.0; dims];
            for f in &feats[a..b] {
                for (x, v) in m.iter_mut().zip(f.iter()) {
                    *x += v;
                }
            }
            m
        })
        .collect();
    let mut labels: Vec<char> = Vec::with_capacity(means.len());
    let mut next = b'A';
    for i in 0..means.len() {
        let reuse = (0..i).find(|j| similar(&means[i], &means[*j]) >= SAME_SECTION);
        match reuse {
            Some(j) => labels.push(labels[j]),
            None => {
                labels.push(next as char);
                next = next.saturating_add(1).min(b'Z');
            }
        }
    }
    edges
        .windows(2)
        .zip(labels)
        .map(|(w, l)| SectionBoundary {
            start_s: w[0],
            end_s: w[1],
            label: l.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_clip_is_one_section() {
        let s = analyze::<13>(&[], 0.04, &[], 0.01, 10.0, &[]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].label, "A");
        assert_eq!(s[0].end_s, 10.0);
    }

    #[test]
    fn a_change_of_harmony_and_timbre_is_a_boundary() {
        // 40 s: C-major chroma and one timbre, then 40 s of F#-major chroma and another.
        let hop_c = 0.05;
        let hop_m = 0.01;
        let chroma: Vec<[f32; 12]> = (0..1600)
            .map(|i| {
                let mut c = [0.05f32; 12];
                let pcs: [usize; 3] = if i < 800 { [0, 4, 7] } else { [6, 10, 1] };
                for p in pcs {
                    c[p] = 1.0;
                }
                c
            })
            .collect();
        let mfcc: Vec<[f32; 13]> = (0..8000)
            .map(|i| {
                let mut m = [0.0f32; 13];
                let w = (i as f32 * 0.37).sin() * 0.05;
                for (c, v) in m.iter_mut().enumerate() {
                    *v = if i < 4000 {
                        c as f32 * 0.1
                    } else {
                        -(c as f32) * 0.1
                    } + w;
                }
                m
            })
            .collect();
        let s = analyze(&chroma, hop_c, &mfcc, hop_m, 80.0, &[]);
        assert_eq!(s.len(), 2, "{s:?}");
        assert!((s[0].end_s - 40.0).abs() <= 1.0, "{s:?}");
        assert_eq!(s[0].label, "A");
        assert_eq!(s[1].label, "B");
    }
}
