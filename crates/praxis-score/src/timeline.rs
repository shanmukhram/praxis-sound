//! Time mapping and note expansion. FROZEN CONTRACT (signatures); see docs/contracts/engine.md.
//!
//! All sample positions are computed from exact integer step counts and the constant tempo with
//! `f64` arithmetic and rounded once, so the same Score gives the same sample positions on every run.

use crate::types::{Accent, Grid, Playback, Score};

/// Duration solver (FR-2).
pub mod duration {
    use crate::types::TimeSignature;

    /// Tempo (quarter-note BPM) that makes `bars` bars last `seconds`.
    /// 16 bars of 4/4 in 30 s gives 128 BPM.
    pub fn solve_tempo(bars: u32, ts: TimeSignature, seconds: f64) -> f64 {
        bars as f64 * ts.bar_quarters() * 60.0 / seconds
    }

    /// Whole bars at `bpm` closest to `seconds` (at least 1), and the resulting exact length.
    pub fn solve_bars(bpm: f64, ts: TimeSignature, seconds: f64) -> (u32, f64) {
        let bar_s = ts.bar_quarters() * 60.0 / bpm;
        let bars = ((seconds / bar_s).round() as u32).max(1);
        (bars, bars as f64 * bar_s)
    }
}

/// Converts musical positions to seconds and samples for one score.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeMap {
    pub sample_rate: u32,
    pub bpm: f64,
    pub bar_whole_notes: f64,
}

impl TimeMap {
    pub fn new(score: &Score, sample_rate: u32) -> TimeMap {
        TimeMap {
            sample_rate,
            bpm: score.tempo.bpm,
            bar_whole_notes: score.time_signature.bar_whole_notes(),
        }
    }

    /// Seconds per whole note (four quarter-note beats).
    pub fn whole_note_s(&self) -> f64 {
        240.0 / self.bpm
    }

    pub fn bar_s(&self) -> f64 {
        self.bar_whole_notes * self.whole_note_s()
    }

    /// Start of 1-based fractional bar position (1.0 = time zero) in seconds.
    pub fn bar_pos_to_s(&self, bar: f64) -> f64 {
        (bar - 1.0) * self.bar_s()
    }

    pub fn s_to_bar_pos(&self, s: f64) -> f64 {
        s / self.bar_s() + 1.0
    }

    pub fn s_to_samples(&self, s: f64) -> u64 {
        (s * self.sample_rate as f64).round().max(0.0) as u64
    }

    /// Sample index of the start of 1-based bar `bar`.
    pub fn bar_start_sample(&self, bar: u32) -> u64 {
        self.s_to_samples(self.bar_pos_to_s(bar as f64))
    }

    pub fn step_s(&self, grid: Grid) -> f64 {
        grid.step_whole_notes() * self.whole_note_s()
    }
}

/// One note, resolved to absolute time. Produced by [`expand_notes`].
#[derive(Clone, Debug, PartialEq)]
pub struct NoteEvent {
    /// Index of the layer in `score.layers`.
    pub layer: usize,
    /// Stable identity: hash of (layer id, absolute bar, step within bar, pitch). Used to seed
    /// per-note randomness so editing one bar never shifts random draws in another.
    pub key: u64,
    /// 1-based bar the note starts in.
    pub bar: u32,
    pub start_s: f64,
    pub length_s: f64,
    pub start_sample: u64,
    /// Length in samples, at least 1.
    pub length_samples: u64,
    /// After transpose, clamped to 0..=127.
    pub pitch: u8,
    /// Velocity after clip scale and accent, 0..=1.
    pub velocity: f32,
    pub accent: Accent,
}

/// Velocity multiplier applied for each accent before clamping.
pub fn accent_velocity(accent: Accent) -> f32 {
    match accent {
        Accent::Normal => 1.0,
        Accent::Strong => 1.25,
        Accent::Soft => 0.7,
        Accent::Bright => 1.1,
    }
}

/// FNV-1a 64-bit, used for stable identities (not cryptographic).
pub fn stable_hash(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in *p {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Expands every clip of every layer into absolute note events, sorted by
/// (start_sample, layer, pitch). Muted layers are included; the engine decides what to play.
/// Invalid references (unknown patterns) are skipped; run validation first.
pub fn expand_notes(score: &Score, sample_rate: u32) -> Vec<NoteEvent> {
    let tm = TimeMap::new(score, sample_rate);
    let mut out = Vec::new();
    for (li, layer) in score.layers.iter().enumerate() {
        for clip in &layer.clips {
            let Some(pat) = score.patterns.iter().find(|p| p.id == clip.pattern) else {
                continue;
            };
            let Some(spb) = score.time_signature.steps_per_bar(pat.grid) else {
                continue;
            };
            if pat.length_steps == 0 {
                continue;
            }
            let step_s = tm.step_s(pat.grid);
            let first = clip.bars.start.max(1);
            let last = clip.bars.end.min(score.bars);
            if last < first {
                continue;
            }
            let clip_steps = (last - first + 1) as u64 * spb as u64;
            let len = pat.length_steps as u64;
            let offset = clip.pattern_offset_steps as u64 % len;
            // Absolute step index of the clip's first step, so note times are computed from one
            // integer step count and splitting a clip cannot change their last bits.
            let clip_first_step = (first as u64 - 1) * spb as u64;
            // Clip-relative step `rel` plays pattern step `(rel + offset) % len`. Walk pattern
            // repetitions starting at -offset so the notes of the partial first cycle are included.
            let mut cycle_start: i64 = -(offset as i64);
            while cycle_start < clip_steps as i64 {
                for n in &pat.notes {
                    if n.step >= pat.length_steps {
                        continue;
                    }
                    let rel = cycle_start + n.step as i64;
                    if rel < 0 || rel >= clip_steps as i64 {
                        continue;
                    }
                    let abs_step = rel as u64;
                    let bar = first + (abs_step / spb as u64) as u32;
                    let step_in_bar = (abs_step % spb as u64) as u32;
                    let start_s =
                        (((clip_first_step + abs_step) as f64 + n.offset as f64) * step_s).max(0.0);
                    let length_s = n.length as f64 * step_s;
                    let pitch = (n.pitch as i32 + clip.transpose as i32).clamp(0, 127) as u8;
                    let velocity = (n.velocity * clip.velocity_scale * accent_velocity(n.accent))
                        .clamp(0.0, 1.0);
                    let key = stable_hash(&[
                        layer.id.as_bytes(),
                        &bar.to_le_bytes(),
                        &step_in_bar.to_le_bytes(),
                        &[pitch],
                    ]);
                    let start_sample = tm.s_to_samples(start_s);
                    let end_sample = tm.s_to_samples(start_s + length_s);
                    out.push(NoteEvent {
                        layer: li,
                        key,
                        bar,
                        start_s,
                        length_s,
                        start_sample,
                        length_samples: (end_sample.saturating_sub(start_sample)).max(1),
                        pitch,
                        velocity,
                        accent: n.accent,
                    });
                }
                cycle_start += len as i64;
            }
        }
    }
    out.sort_by(|a, b| {
        (a.start_sample, a.layer, a.pitch, a.key).cmp(&(b.start_sample, b.layer, b.pitch, b.key))
    });
    out
}

/// Musical length of the score in samples (bars only, no tail).
pub fn score_length_samples(score: &Score, sample_rate: u32) -> u64 {
    TimeMap::new(score, sample_rate).bar_start_sample(score.bars + 1)
}

/// Tail length rendered after the last bar for one-shots; zero for loops (tails wrap instead).
pub fn tail_samples(score: &Score, sample_rate: u32) -> u64 {
    match score.playback {
        Playback::Loop => 0,
        Playback::OneShot { tail_seconds } => {
            (tail_seconds.max(0.0) * sample_rate as f64).round() as u64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::duration::*;
    use crate::types::TimeSignature;

    #[test]
    fn acceptance_test_tempo() {
        let ts = TimeSignature {
            numerator: 4,
            denominator: 4,
        };
        assert!((solve_tempo(16, ts, 30.0) - 128.0).abs() < 1e-9);
        let (bars, len) = solve_bars(128.0, ts, 30.0);
        assert_eq!(bars, 16);
        assert!((len - 30.0).abs() < 1e-9);
    }
}
