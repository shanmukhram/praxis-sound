//! The character of a reference, read from its Sound Profile: the facts the matcher turns into a
//! Score (tempo, key, groove, brightness, envelope, space, density, sections). Numbers only; a
//! profile never holds audio (FR-18).

use praxis_analyze::SoundProfile;
use praxis_analyze::profile::ClipKind;
use praxis_score::{Grid, Key, Mode, PitchClass, TimeSignature};

/// Tempo confidence below which the detected tempo is not trusted.
pub const MIN_TEMPO_CONFIDENCE: f32 = 0.15;
/// Key confidence below which the detected key is not trusted.
pub const MIN_KEY_CONFIDENCE: f32 = 0.2;

/// What the matcher reads from one profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Character {
    pub kind: ClipKind,
    pub duration_s: f64,
    /// Detected tempo when it is trusted.
    pub bpm: Option<f64>,
    pub beats_per_bar: u8,
    /// Detected key when it is trusted.
    pub key: Option<Key>,
    pub groove: GrooveShape,
    pub centroid_hz: f32,
    pub flatness: f32,
    pub tonality: f32,
    /// Share of the energy in the octave bands at or below 125 Hz, 0..1.
    pub low_share: f32,
    pub attack_s: f32,
    pub release_s: f32,
    pub rt60_s: Option<f32>,
    pub width: f32,
    pub lufs: f64,
    /// Median voiced pitch (MIDI), when the clip has a pitch.
    pub median_midi: Option<f32>,
    pub sections: usize,
    /// Time the clip is audibly sounding (envelope within 40 dB of its peak), seconds.
    pub active_s: f64,
    /// Where the loudest moment sits within the sounding part, 0 (start) .. 1 (end).
    pub peak_pos: f32,
    /// Spectral centroid at the end of the sounding part over the centroid at its start.
    pub centroid_rise: f32,
    pub onsets_s: Vec<f64>,
}

/// The rhythm of a clip on a step grid.
#[derive(Clone, Debug, PartialEq)]
pub struct GrooveShape {
    pub grid: Grid,
    /// Onset strength per step of one bar, max 1.
    pub step_weights: Vec<f32>,
    /// 0 straight .. 1 full triplet swing.
    pub swing: f32,
    pub onsets_per_bar: f32,
}

impl GrooveShape {
    /// A plain groove when the profile has none: accents on the beats of an eighth-note bar.
    pub fn plain(ts: TimeSignature) -> GrooveShape {
        let steps = ts.steps_per_bar(Grid::Eighth).unwrap_or(8) as usize;
        let step_weights = (0..steps)
            .map(|i| match i {
                0 => 1.0,
                i if i % 2 == 0 => 0.6,
                _ => 0.25,
            })
            .collect();
        GrooveShape {
            grid: Grid::Eighth,
            step_weights,
            swing: 0.0,
            onsets_per_bar: (steps / 2) as f32,
        }
    }
}

impl Character {
    pub fn of(p: &SoundProfile) -> Character {
        let beats_per_bar = p
            .tempo
            .as_ref()
            .map(|t| t.beats_per_bar)
            .filter(|b| (2..=7).contains(b))
            .unwrap_or(4);
        let ts = TimeSignature {
            numerator: beats_per_bar,
            denominator: 4,
        };
        let bpm = p
            .tempo
            .as_ref()
            .filter(|t| t.confidence >= MIN_TEMPO_CONFIDENCE && t.bpm.is_finite())
            .map(|t| t.bpm.clamp(40.0, 220.0));
        let key = p
            .key
            .as_ref()
            .filter(|k| k.confidence >= MIN_KEY_CONFIDENCE)
            .map(|k| k.key);
        let groove = match &p.groove {
            Some(g) if ts.steps_per_bar(g.grid) == Some(g.step_weights.len() as u32) => {
                let max = g.step_weights.iter().copied().fold(0.0f32, f32::max);
                if max > 0.0 {
                    GrooveShape {
                        grid: g.grid,
                        step_weights: g.step_weights.iter().map(|w| finite(*w) / max).collect(),
                        swing: finite(g.swing).clamp(0.0, 1.0),
                        onsets_per_bar: finite(g.onsets_per_bar).max(0.0),
                    }
                } else {
                    GrooveShape::plain(ts)
                }
            }
            _ => GrooveShape::plain(ts),
        };
        let low_share = band_share(&p.spectral.bands_db, 0..3);
        let (active_s, peak_pos) = active_span(&p.envelope.frames_db, p.envelope.hop_s);
        let voiced: Vec<f32> = p
            .pitch
            .midi
            .iter()
            .filter_map(|m| *m)
            .filter(|m| m.is_finite())
            .collect();
        let median_midi = if voiced.len() >= 5 {
            let mut v = voiced;
            v.sort_by(f32::total_cmp);
            Some(v[v.len() / 2])
        } else {
            None
        };
        Character {
            kind: p.class.kind,
            duration_s: p.source.duration_s.max(0.0),
            bpm,
            beats_per_bar,
            key,
            groove,
            centroid_hz: finite(p.spectral.centroid_hz).max(20.0),
            flatness: finite(p.spectral.flatness).clamp(0.0, 1.0),
            tonality: finite(p.tonality).clamp(0.0, 1.0),
            low_share,
            attack_s: finite(p.envelope.attack_s).max(0.0),
            release_s: finite(p.envelope.release_s).max(0.0),
            rt60_s: p
                .reverb
                .as_ref()
                .filter(|r| r.confidence > 0.2 && r.rt60_s.is_finite())
                .map(|r| r.rt60_s.clamp(0.05, 20.0)),
            width: finite(p.stereo.width).max(0.0),
            lufs: if p.loudness.integrated_lufs.is_finite() {
                p.loudness.integrated_lufs
            } else {
                -18.0
            },
            median_midi,
            sections: p.sections.len(),
            active_s: if active_s > 0.0 {
                active_s
            } else {
                p.source.duration_s
            },
            peak_pos,
            centroid_rise: centroid_rise(&p.spectral.centroid_track_hz, &p.envelope.frames_db),
            onsets_s: p.onsets_s.clone(),
        }
    }

    pub fn time_signature(&self) -> TimeSignature {
        TimeSignature {
            numerator: self.beats_per_bar,
            denominator: 4,
        }
    }

    /// The key to write in: detected, else from the median pitch, else A minor.
    pub fn key_or_default(&self) -> Key {
        if let Some(k) = self.key {
            return k;
        }
        let tonic = self
            .median_midi
            .map(|m| PitchClass::from_semitone(m.round() as i32))
            .unwrap_or(PitchClass::A);
        Key {
            tonic,
            mode: Mode::Minor,
        }
    }

    /// Brightness on a 0..1 scale: log of the centroid between 150 Hz and 8 kHz.
    pub fn brightness(&self) -> f64 {
        log_unit(self.centroid_hz as f64, 150.0, 8000.0)
    }
}

/// `log(v)` mapped from `[lo, hi]` to `[0, 1]`, clamped.
pub fn log_unit(v: f64, lo: f64, hi: f64) -> f64 {
    if !(v.is_finite() && v > 0.0) {
        return 0.0;
    }
    ((v / lo).ln() / (hi / lo).ln()).clamp(0.0, 1.0)
}

/// Inverse of [`log_unit`]: `0..1` to `[lo, hi]` exponentially.
pub fn exp_map(x: f64, lo: f64, hi: f64) -> f64 {
    lo * (hi / lo).powf(x.clamp(0.0, 1.0))
}

fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

/// Share of linear power in the given octave bands (dB relative to the total), 0..1.
fn band_share(bands_db: &[f32], range: std::ops::Range<usize>) -> f32 {
    let total: f64 = bands_db
        .iter()
        .filter(|b| b.is_finite())
        .map(|b| 10f64.powf(*b as f64 / 10.0))
        .sum();
    if total <= 0.0 {
        return 0.0;
    }
    let part: f64 = bands_db
        .iter()
        .enumerate()
        .filter(|(i, b)| range.contains(i) && b.is_finite())
        .map(|(_, b)| 10f64.powf(*b as f64 / 10.0))
        .sum();
    (part / total).clamp(0.0, 1.0) as f32
}

/// Sounding span of an RMS envelope (frames within 40 dB of the peak) and where its peak sits.
fn active_span(frames_db: &[f32], hop_s: f32) -> (f64, f32) {
    let (first, last, peak) = match active_frames(frames_db) {
        Some(x) => x,
        None => return (0.0, 0.5),
    };
    let span = (last - first + 1) as f64 * hop_s.max(0.0) as f64;
    let pos = if last > first {
        (peak - first) as f32 / (last - first) as f32
    } else {
        0.0
    };
    (span, pos)
}

/// First and last frame within 40 dB of the peak, and the peak frame.
fn active_frames(frames_db: &[f32]) -> Option<(usize, usize, usize)> {
    let (peak, max) = frames_db
        .iter()
        .enumerate()
        .filter(|(_, v)| v.is_finite())
        .max_by(|a, b| a.1.total_cmp(b.1))?;
    let floor = max - 40.0;
    let first = frames_db
        .iter()
        .position(|v| v.is_finite() && *v >= floor)?;
    let last = frames_db
        .iter()
        .rposition(|v| v.is_finite() && *v >= floor)?;
    Some((first, last, peak))
}

/// Centroid over the last fifth of the sounding part divided by the first fifth's.
fn centroid_rise(track: &[f32], frames_db: &[f32]) -> f32 {
    let Some((first, last, _)) = active_frames(frames_db) else {
        return 1.0;
    };
    let last = last.min(track.len().saturating_sub(1));
    if track.is_empty() || last <= first + 4 {
        return 1.0;
    }
    let n = (last - first + 1) / 5;
    let mean = |r: std::ops::Range<usize>| {
        let v: Vec<f32> = track[r]
            .iter()
            .copied()
            .filter(|c| c.is_finite() && *c > 0.0)
            .collect();
        if v.is_empty() {
            None
        } else {
            Some(v.iter().sum::<f32>() / v.len() as f32)
        }
    };
    match (mean(first..first + n), mean(last + 1 - n..last + 1)) {
        (Some(a), Some(b)) if a > 0.0 => (b / a).clamp(0.05, 20.0),
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_round_trip() {
        for x in [0.0, 0.25, 0.5, 1.0] {
            let v = exp_map(x, 150.0, 8000.0);
            assert!((log_unit(v, 150.0, 8000.0) - x).abs() < 1e-9);
        }
    }

    #[test]
    fn band_share_counts_low_bands() {
        // Equal energy in all ten bands: the lowest three hold 30%.
        let bands = vec![-10.0; 10];
        assert!((band_share(&bands, 0..3) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn active_span_and_peak_position() {
        // Silence, a ramp up to a peak at frame 8, a fall, silence.
        let mut f = vec![-120.0f32; 4];
        f.extend([-30.0, -20.0, -10.0, -5.0, 0.0, -10.0, -30.0]);
        f.extend(vec![-120.0f32; 4]);
        let (span, pos) = active_span(&f, 0.01);
        assert!((span - 0.07).abs() < 1e-9, "{span}");
        assert!((pos - 4.0 / 6.0).abs() < 1e-6, "{pos}");
    }

    #[test]
    fn rising_centroid_reads_above_one() {
        let frames = vec![-6.0f32; 50];
        let track: Vec<f32> = (0..50).map(|i| 300.0 + 60.0 * i as f32).collect();
        assert!(centroid_rise(&track, &frames) > 3.0);
        let flat = vec![1000.0f32; 50];
        assert!((centroid_rise(&flat, &frames) - 1.0).abs() < 1e-6);
    }
}
