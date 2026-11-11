//! Builds a complete [`SoundProfile`] from decoded audio (FR-12, FR-13). Pure: no files, no
//! clocks, no threads; the same samples always give the same profile.
//!
//! The methods of every field are in docs/contracts/sound-profile.md and in the module docs of
//! [`crate::features`]. This module wires them together and fills the fixed feature vector
//! ([`crate::profile::FEATURE_NAMES`]).

use crate::features::onset::{self, Onset};
use crate::features::{
    self, FeatureError, chroma, classify, envelope, groove, key, loudness, pitch, reverb, sections,
    spectral, stereo, tempo,
};
use crate::profile::{
    Envelope, FEATURE_NAMES, Groove, KeyEstimate, PROFILE_VERSION, PitchTrack, SoundProfile,
    SourceInfo, Spectral, TempoEstimate,
};

/// What the caller knows about the clip besides its samples.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileOptions {
    /// File name only (no directories), for `source.file_name`.
    pub file_name: String,
    /// Channels of the source as stored (1 for a mono file that was duplicated to stereo).
    pub channels: u16,
    /// Beats per bar for the downbeat and the groove (4 for 4/4).
    pub beats_per_bar: u8,
}

impl Default for ProfileOptions {
    fn default() -> Self {
        ProfileOptions {
            file_name: String::new(),
            channels: 2,
            beats_per_bar: 4,
        }
    }
}

/// Tempo settings for profiles: the estimator's defaults, except that an octave relative within
/// 10 % (not 3 %) of the strongest prefers the faster level. Chosen on the labelled set
/// (`crates/praxis-analyze/tests/clips`): eighth-note music at 155-175 BPM was read at half
/// tempo with the 3 % rule. Render verification keeps its own default (`verify.rs`).
pub fn profile_tempo_config() -> tempo::TempoConfig {
    tempo::TempoConfig {
        tie_ratio: 0.9,
        ..tempo::TempoConfig::default()
    }
}

/// Onsets less regular than this (see [`groove::regularity`]) have no groove. Random onsets
/// read about 0.4 to 0.7 (noise beds, grains), sequenced music 0.99 or more.
pub const MIN_GROOVE_REGULARITY: f64 = 0.75;

/// Fewer onsets than this: no tempo is reported (there is no pulse to measure).
const MIN_ONSETS_FOR_TEMPO: usize = 4;
/// The active span: frames within this many dB of the loudest 10 ms frame.
const ACTIVE_RANGE_DB: f64 = 45.0;

fn tempo_estimate(
    signal: &[f32],
    sample_rate: u32,
    env: &onset::OnsetEnvelope,
    onsets: &[Onset],
    beats_per_bar: u8,
) -> Option<TempoEstimate> {
    if onsets.len() < MIN_ONSETS_FOR_TEMPO {
        return None;
    }
    let t = tempo::estimate_tempo(env, &profile_tempo_config())?;
    let frames = tempo::track_beats(env, t.bpm, 100.0);
    let beats_s = tempo::beat_times(env, &frames, onsets, t.bpm);
    let phase = tempo::downbeat_phase(signal, sample_rate, &beats_s, beats_per_bar as usize);
    let downbeat_s = beats_s.get(phase).copied().unwrap_or(0.0);
    Some(TempoEstimate {
        bpm: t.bpm,
        confidence: t.confidence,
        beats_s,
        downbeat_s,
        beats_per_bar: beats_per_bar.max(1),
    })
}

fn sd(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let m = values.iter().sum::<f64>() / values.len() as f64;
    (values.iter().map(|v| (v - m).powi(2)).sum::<f64>() / values.len() as f64).sqrt()
}

/// Weighted mean of groups of `k` values (weights `w`); groups with no weight read 0.
fn decimate_weighted(values: &[f64], w: &[f64], k: usize) -> Vec<f64> {
    let k = k.max(1);
    values
        .chunks(k)
        .zip(w.chunks(k))
        .map(|(v, w)| {
            let sw: f64 = w.iter().sum();
            if sw > 0.0 {
                v.iter().zip(w).map(|(a, b)| a * b).sum::<f64>() / sw
            } else {
                0.0
            }
        })
        .collect()
}

/// Mean weight on the off-beat steps relative to the on-beat steps (0 with no on-beat weight).
fn offbeat_weight(g: &Groove, beats_per_bar: u8) -> f64 {
    let steps = g.step_weights.len();
    let per_beat = (steps / beats_per_bar.max(1) as usize).max(1);
    let (mut on, mut off, mut n_on, mut n_off) = (0.0f64, 0.0f64, 0usize, 0usize);
    for (i, w) in g.step_weights.iter().enumerate() {
        if i % per_beat == 0 {
            on += *w as f64;
            n_on += 1;
        } else {
            off += *w as f64;
            n_off += 1;
        }
    }
    if n_on == 0 || on <= 0.0 || n_off == 0 {
        return 0.0;
    }
    (off / n_off as f64) / (on / n_on as f64)
}

fn quantile_f32(v: &mut [f32], q: f64) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let i = ((v.len() - 1) as f64 * q).round() as usize;
    v[i.min(v.len() - 1)]
}

/// Measures a stereo clip into a complete Sound Profile.
pub fn sound_profile(
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
    opts: &ProfileOptions,
) -> Result<SoundProfile, FeatureError> {
    if sample_rate == 0 {
        return Err(FeatureError::SampleRate);
    }
    if left.len() != right.len() {
        return Err(FeatureError::ChannelLength {
            left: left.len(),
            right: right.len(),
        });
    }
    let sr = sample_rate;
    let duration_s = left.len() as f64 / sr as f64;
    let mono = features::mono(left, right);
    let loud = loudness::measure(left, right, sr)?;
    let (env, onsets) = onset::detect_onsets(&mono, sr);
    let tempo_est = tempo_estimate(&mono, sr, &env, &onsets, opts.beats_per_bar.max(1));
    let chromagram = chroma::chromagram(&mono, sr);
    let key_est = key::estimate_key(&chromagram.mean()).map(|r| KeyEstimate {
        key: r.key,
        confidence: r.confidence,
        chroma: chromagram.mean(),
    });

    // Envelope (10 ms).
    let hop = envelope::hop_samples(sr);
    let hop_s = hop as f64 / sr as f64;
    let rms = envelope::rms_frames(&mono, hop);
    let adsr = envelope::fit_adsr(&rms, hop_s);
    let k = envelope::decimation(rms.len());
    let frames_db = envelope::to_db(&envelope::decimate_rms(&rms, k));

    // Spectrum.
    let spec = spectral::analyze(&mono, sr, hop);
    let centroid_track: Vec<f32> = decimate_weighted(&spec.centroid_track_hz, &spec.frame_power, k)
        .iter()
        .map(|v| *v as f32)
        .collect();

    // Pitch, stereo, reverb.
    let (midi, pitch_hop) = pitch::yin(&mono, sr, &pitch::YinConfig::default());
    let stereo_stats = stereo::measure(left, right);
    let energy: Vec<f64> = rms.iter().map(|a| a * a).collect();
    let onset_times: Vec<f64> = onsets.iter().map(|o| o.time_s).collect();
    let reverb_est = reverb::estimate(&energy, hop_s, &onset_times);

    // Groove and sections.
    // A groove needs onsets on the grid: irregular onsets (noise, grains) have none.
    let groove_est = tempo_est
        .as_ref()
        .filter(|t| groove::regularity(t, &onsets) >= MIN_GROOVE_REGULARITY)
        .and_then(|t| groove::analyze(t, &onsets));
    let beats: &[f64] = tempo_est.as_ref().map_or(&[], |t| &t.beats_s);
    let section_list = sections::analyze(
        &chromagram.frames,
        chromagram.hop_s,
        &spec.mfcc,
        spec.hop_s,
        duration_s,
        beats,
    );

    // Classification cues over the active span.
    let peak = rms.iter().fold(0.0f64, |m, v| m.max(*v));
    let floor = peak * 10f64.powf(-ACTIVE_RANGE_DB / 20.0);
    let a0 = rms
        .iter()
        .position(|v| *v >= floor && *v > 0.0)
        .unwrap_or(0);
    let a1 = rms
        .iter()
        .rposition(|v| *v >= floor && *v > 0.0)
        .unwrap_or(a0);
    let active_s = if peak > 0.0 {
        (a1 + 1 - a0) as f64 * hop_s
    } else {
        0.0
    };
    let (t0, t1) = (a0 as f64 * hop_s - 0.05, (a1 + 1) as f64 * hop_s + 0.05);
    let active_onsets: Vec<Onset> = onsets
        .iter()
        .filter(|o| o.time_s >= t0 && o.time_s <= t1)
        .cloned()
        .collect();
    let level_db: Vec<f64> = if rms.is_empty() {
        Vec::new()
    } else {
        rms[a0..=a1.max(a0).min(rms.len() - 1)]
            .chunks(40)
            .filter(|c| c.len() >= 20)
            .map(|c| {
                let p = c.iter().map(|a| a * a).sum::<f64>() / c.len() as f64;
                features::amp_to_db(p.sqrt())
            })
            .collect()
    };
    let bright: Vec<f64> = (a0..=a1)
        .filter_map(|i| spec.centroid_track_hz.get(i).copied())
        .filter(|c| *c > 20.0)
        .map(|c| c.log2())
        .collect();
    let regularity = tempo_est
        .as_ref()
        .map_or(0.0, |t| groove::regularity(t, &active_onsets));
    let cues = classify::Cues {
        active_s,
        tempo_confidence: tempo_est.as_ref().map_or(0.0, |t| t.confidence as f64),
        regularity,
        onsets: active_onsets.len(),
        onset_rate: active_onsets.len() as f64 / active_s.max(0.1),
        tonality: spec.tonality,
        level_iqr_db: features::quantile(&level_db, 0.75) - features::quantile(&level_db, 0.25),
        brightness_sd_oct: sd(&bright),
    };
    let class = classify::classify(&cues);

    let profile = SoundProfile {
        version: PROFILE_VERSION,
        source: SourceInfo {
            file_name: opts.file_name.clone(),
            duration_s,
            sample_rate: sr,
            channels: opts.channels,
            pcm_hash: crate::hash::pcm_hash(left, right),
        },
        class,
        tempo: tempo_est,
        key: key_est,
        loudness: loud,
        envelope: Envelope {
            attack_s: adsr.attack_s as f32,
            decay_s: adsr.decay_s as f32,
            sustain: adsr.sustain as f32,
            release_s: adsr.release_s as f32,
            frames_db,
            hop_s: (hop_s * k as f64) as f32,
        },
        spectral: Spectral {
            centroid_hz: spec.centroid_hz as f32,
            rolloff_hz: spec.rolloff_hz as f32,
            flatness: spec.flatness as f32,
            bands_db: spec.bands_db.iter().map(|v| *v as f32).collect(),
            tilt_db_per_oct: spec.tilt_db_per_oct as f32,
            centroid_track_hz: centroid_track,
        },
        groove: groove_est,
        onsets_s: onset_times,
        pitch: PitchTrack {
            hop_s: pitch_hop as f32,
            midi,
        },
        tonality: spec.tonality as f32,
        stereo: stereo_stats,
        reverb: reverb_est,
        sections: section_list,
        features: Vec::new(),
        tags: Vec::new(),
        style_name: None,
    };
    let features = feature_vector(&profile, &cues);
    Ok(SoundProfile {
        features,
        ..profile
    })
}

/// The feature vector of a profile, in [`FEATURE_NAMES`] order. `cues` are the classification
/// cues the profile was built with (active duration, regularity, level and brightness spread).
pub fn feature_vector(p: &SoundProfile, cues: &classify::Cues) -> Vec<f32> {
    let mut voiced: Vec<f32> = p.pitch.midi.iter().flatten().copied().collect();
    let voiced_fraction = if p.pitch.midi.is_empty() {
        0.0
    } else {
        voiced.len() as f64 / p.pitch.midi.len() as f64
    };
    let p10 = quantile_f32(&mut voiced, 0.1);
    let p50 = quantile_f32(&mut voiced, 0.5);
    let p90 = quantile_f32(&mut voiced, 0.9);
    let bpb = p.tempo.as_ref().map_or(4, |t| t.beats_per_bar);
    let mut v: Vec<f32> = vec![
        cues.active_s as f32,
        p.tempo.as_ref().map_or(0.0, |t| t.bpm as f32),
        p.tempo.as_ref().map_or(0.0, |t| t.confidence),
        p.key.as_ref().map_or(0.0, |k| k.confidence),
        p.key.as_ref().map_or(0.5, |k| {
            if k.key.mode == praxis_score::Mode::Minor {
                1.0
            } else {
                0.0
            }
        }),
        p.loudness.integrated_lufs as f32,
        p.loudness.range_lu as f32,
        p.loudness.crest_db as f32,
        p.envelope.attack_s,
        p.envelope.decay_s,
        p.envelope.sustain,
        p.envelope.release_s,
        p.spectral.centroid_hz,
        p.spectral.rolloff_hz,
        p.spectral.flatness,
    ];
    for i in 0..10 {
        v.push(p.spectral.bands_db.get(i).copied().unwrap_or(-120.0));
    }
    v.extend([
        p.spectral.tilt_db_per_oct,
        cues.brightness_sd_oct as f32,
        cues.onset_rate as f32,
        cues.regularity as f32,
        p.groove.as_ref().map_or(0.0, |g| g.swing),
        p.groove.as_ref().map_or(0.0, |g| g.onsets_per_bar),
        p.groove
            .as_ref()
            .map_or(0.0, |g| offbeat_weight(g, bpb) as f32),
        voiced_fraction as f32,
        if voiced.is_empty() { 0.0 } else { p50 },
        if voiced.is_empty() { 0.0 } else { p90 - p10 },
        p.tonality,
        p.stereo.width,
        p.stereo.correlation,
        p.reverb.as_ref().map_or(0.0, |r| r.rt60_s),
        p.reverb.as_ref().map_or(0.0, |r| r.confidence),
        cues.level_iqr_db as f32,
        p.sections.len() as f32,
        p.class.scores[0],
        p.class.scores[1],
        p.class.scores[2],
    ]);
    debug_assert_eq!(v.len(), FEATURE_NAMES.len());
    v.iter()
        .map(|x| if x.is_finite() { *x } else { 0.0 })
        .collect()
}

/// Index of a feature name in [`FEATURE_NAMES`].
pub fn feature_index(name: &str) -> Option<usize> {
    FEATURE_NAMES.iter().position(|n| *n == name)
}

/// A feature of `p` by name (`None` when the profile's vector is shorter, e.g. an old profile).
pub fn feature(p: &SoundProfile, name: &str) -> Option<f32> {
    feature_index(name).and_then(|i| p.features.get(i).copied())
}
