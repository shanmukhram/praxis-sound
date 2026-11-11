//! Measurements an agent can use instead of ears: the verify report of the master plus, per
//! layer (its stem: after the layer's effects, gain and pan, before the master), the spectral
//! centroid, the share of energy below [`LOW_BAND_HZ`] and above [`HIGH_BAND_HZ`], and the RMS
//! level.
//!
//! "Muffled" shows as a lower centroid and less high-band energy, "less boomy" as less low-band
//! energy, "quieter" as a lower RMS. Every number is a pure function of the render, which is deterministic.

use crate::OpError;
use crate::ops::render::{RenderSettings, render_score_output};
use praxis_analyze::features::stft::FrameAnalyzer;
use praxis_score::Score;
use serde_json::{Value, json};
use std::path::Path;

/// Upper edge of the low band, Hz (kick and bass fundamentals sit below it).
pub const LOW_BAND_HZ: f64 = 150.0;
/// Lower edge of the high band, Hz (where a felt or a low-pass takes the brightness away).
pub const HIGH_BAND_HZ: f64 = 2000.0;
/// FFT frame (and hop) for the layer measurements.
const FRAME: usize = 4096;
/// Level reported for silence, dB.
const FLOOR_DB: f64 = -120.0;

/// One layer's numbers. `None` when the stem is silent.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerMeasure {
    pub id: String,
    pub centroid_hz: Option<f64>,
    pub low_band_db: Option<f64>,
    /// Share of the energy at or above [`HIGH_BAND_HZ`], dB.
    pub high_band_db: Option<f64>,
    pub rms_dbfs: f64,
}

impl LayerMeasure {
    pub fn to_json(&self) -> Value {
        json!({
            "layer": self.id,
            "centroid_hz": self.centroid_hz.map(round1),
            "low_band_db": self.low_band_db.map(round1),
            "high_band_db": self.high_band_db.map(round1),
            "rms_dbfs": round1(self.rms_dbfs),
        })
    }
}

/// A rendered and measured score.
#[derive(Clone, Debug)]
pub struct Measured {
    /// The render payload (`docs/contracts/cli.md`), with the checks under `check`.
    pub render: Value,
    pub layers: Vec<LayerMeasure>,
    pub master: praxis_dsp::StereoBuffer,
    pub sample_rate: u32,
}

pub fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn db(x: f64) -> f64 {
    if x > 0.0 {
        (10.0 * x.log10()).max(FLOOR_DB)
    } else {
        FLOOR_DB
    }
}

/// Centroid, low-band share and RMS of a stereo stem (measured on its mono sum).
pub fn measure_audio(id: &str, left: &[f32], right: &[f32], sample_rate: u32) -> LayerMeasure {
    let n = left.len().min(right.len());
    let mono: Vec<f32> = (0..n).map(|i| 0.5 * (left[i] + right[i])).collect();
    let rms = if n == 0 {
        0.0
    } else {
        mono.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>() / n as f64
    };
    let mut fft = FrameAnalyzer::new(FRAME);
    let bins = fft.bins();
    let mut mags = vec![0.0f32; bins];
    let hz = sample_rate as f64 / FRAME as f64;
    // Energy (power) for the band shares; magnitude for the centroid, its usual definition,
    // which weights the upper partials enough to follow the brightness people hear.
    let (mut total, mut low, mut high) = (0.0f64, 0.0f64, 0.0f64);
    let (mut mag_sum, mut mag_weighted) = (0.0f64, 0.0f64);
    let mut start = 0;
    while start < n {
        fft.magnitudes(&mono[start..(start + FRAME).min(n)], &mut mags);
        for (k, m) in mags.iter().enumerate().skip(1) {
            let m = *m as f64;
            let p = m * m;
            let f = k as f64 * hz;
            total += p;
            mag_sum += m;
            mag_weighted += m * f;
            if f < LOW_BAND_HZ {
                low += p;
            }
            if f >= HIGH_BAND_HZ {
                high += p;
            }
        }
        start += FRAME;
    }
    let audible = total > 1e-12 && mag_sum > 0.0;
    LayerMeasure {
        id: id.to_string(),
        centroid_hz: audible.then_some(mag_weighted / mag_sum),
        low_band_db: audible.then_some(db(low / total)),
        high_band_db: audible.then_some(db(high / total)),
        rms_dbfs: db(rms),
    }
}

/// Renders `score` to `out` (24-bit WAV, 48 kHz), checks it and measures every rendered layer.
pub fn render_measure(score: &Score, out: &Path, skip_missing: bool) -> Result<Measured, OpError> {
    let settings = RenderSettings {
        skip_missing,
        ..RenderSettings::default()
    };
    let (render, output) = render_score_output(score, out, &settings, true)?;
    let layers = output
        .stems
        .iter()
        .map(|s| {
            measure_audio(
                &s.layer_id,
                &s.audio.left,
                &s.audio.right,
                settings.sample_rate,
            )
        })
        .collect();
    Ok(Measured {
        render,
        layers,
        master: output.master,
        sample_rate: settings.sample_rate,
    })
}

/// Each layer's numbers next to the same layer's numbers in `before`, with the change.
pub fn compare(after: &[LayerMeasure], before: &[LayerMeasure]) -> Vec<Value> {
    after
        .iter()
        .map(|a| {
            let mut v = a.to_json();
            if let Some(b) = before.iter().find(|b| b.id == a.id) {
                v["before"] = b.to_json();
                let delta = |x: Option<f64>, y: Option<f64>| match (x, y) {
                    (Some(x), Some(y)) => json!(round1(x - y)),
                    _ => Value::Null,
                };
                v["change"] = json!({
                    "centroid_hz": delta(a.centroid_hz, b.centroid_hz),
                    "low_band_db": delta(a.low_band_db, b.low_band_db),
                    "high_band_db": delta(a.high_band_db, b.high_band_db),
                    "rms_db": round1(a.rms_dbfs - b.rms_dbfs),
                });
            } else {
                v["before"] = Value::Null;
            }
            v
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (0.5 * (2.0 * std::f64::consts::PI * f * i as f64 / 48_000.0).sin()) as f32)
            .collect()
    }

    #[test]
    fn centroid_and_low_band_follow_the_spectrum() {
        let low = sine(80.0, 48_000);
        let high = sine(2_000.0, 48_000);
        let a = measure_audio("a", &low, &low, 48_000);
        let b = measure_audio("b", &high, &high, 48_000);
        assert!(a.centroid_hz.unwrap() < 200.0, "{a:?}");
        assert!((b.centroid_hz.unwrap() - 2_000.0).abs() < 50.0, "{b:?}");
        assert!(a.low_band_db.unwrap() > -1.0);
        assert!(b.low_band_db.unwrap() < -40.0);
        assert!((a.rms_dbfs - (-9.0)).abs() < 0.2, "{}", a.rms_dbfs);
        let silent = measure_audio("s", &[0.0; 100], &[0.0; 100], 48_000);
        assert_eq!(silent.centroid_hz, None);
        assert_eq!(silent.rms_dbfs, FLOOR_DB);
        let c = compare(
            &[b.clone()],
            &[LayerMeasure {
                id: "b".into(),
                ..a
            }],
        );
        assert!(c[0]["change"]["centroid_hz"].as_f64().unwrap() > 1_000.0);
    }
}
