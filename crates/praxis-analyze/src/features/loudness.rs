//! Loudness: BS.1770-4 integrated loudness, loudness range and true peak via the `ebur128` crate
//! (praxis_dsp::meter is the long-term home; until it lands this module meters directly), plus
//! sample peak, RMS and crest factor. Silence reads [`FLOOR_DB`](super::FLOOR_DB) instead of
//! minus infinity.

use super::{FLOOR_DB, FeatureError, amp_to_db, finite_or_zero};
use crate::profile::LoudnessStats;
use ebur128::{EbuR128, Mode};

/// Measures a stereo signal. Non-finite samples are metered as 0.
pub fn measure(
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
) -> Result<LoudnessStats, FeatureError> {
    if sample_rate == 0 {
        return Err(FeatureError::SampleRate);
    }
    if left.len() != right.len() {
        return Err(FeatureError::ChannelLength {
            left: left.len(),
            right: right.len(),
        });
    }
    let l: Vec<f32> = left.iter().map(|x| finite_or_zero(*x)).collect();
    let r: Vec<f32> = right.iter().map(|x| finite_or_zero(*x)).collect();
    let mut m = EbuR128::new(
        2,
        sample_rate,
        Mode::I | Mode::LRA | Mode::TRUE_PEAK | Mode::SAMPLE_PEAK,
    )
    .map_err(|e| FeatureError::Meter(format!("{e:?}")))?;
    m.add_frames_planar_f32(&[&l, &r])
        .map_err(|e| FeatureError::Meter(format!("{e:?}")))?;
    let integrated = m.loudness_global().unwrap_or(f64::NEG_INFINITY);
    let integrated_lufs = if integrated.is_finite() {
        integrated.max(FLOOR_DB)
    } else {
        FLOOR_DB
    };
    let range = m.loudness_range().unwrap_or(0.0);
    let range_lu = if range.is_finite() {
        range.max(0.0)
    } else {
        0.0
    };
    let mut tp = 0.0f64;
    for ch in 0..2 {
        tp = tp.max(m.true_peak(ch).unwrap_or(0.0));
    }
    let mut peak = 0.0f64;
    let mut sum = 0.0f64;
    for x in l.iter().chain(r.iter()) {
        let a = x.abs() as f64;
        peak = peak.max(a);
        sum += a * a;
    }
    // ebur128's true peak is never below the sample peak; keep that invariant exactly.
    let tp = tp.max(peak);
    let count = (l.len() + r.len()).max(1) as f64;
    let rms = (sum / count).sqrt();
    let sample_peak_dbfs = amp_to_db(peak);
    let rms_dbfs = amp_to_db(rms);
    Ok(LoudnessStats {
        integrated_lufs,
        range_lu,
        true_peak_dbtp: amp_to_db(tp),
        sample_peak_dbfs,
        rms_dbfs,
        crest_db: if peak > 0.0 {
            sample_peak_dbfs - rms_dbfs
        } else {
            0.0
        },
    })
}
