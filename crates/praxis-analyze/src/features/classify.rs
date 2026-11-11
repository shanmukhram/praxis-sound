//! Clip class: music, sound effect or ambience (FR-13).
//!
//! A small hand-set linear model over cues the profile already has, then a softmax:
//!
//! | Cue | Meaning |
//! | --- | --- |
//! | `active_s` | seconds between the first and last 10 ms frame within 45 dB of the loudest |
//! | `tempo_confidence` | the tempo estimate's confidence (0 without a tempo) |
//! | `regularity` | onset strength on the sixteenth grid of the beats ([`super::groove::regularity`]) |
//! | `onset_rate` | onsets per second of active sound |
//! | `tonality` | 0 noise .. 1 tone |
//! | `level_iqr_db` | interquartile range of the 400 ms level (dB) over the active span |
//! | `brightness_sd_oct` | standard deviation of the spectral centroid (octaves) over the active span |
//!
//! Music has a confident, regular pulse of many onsets; a sound effect is short, or one event
//! whose level and brightness move a lot; ambience is long and steady with few or irregular
//! onsets. The weights were set by hand on the labelled set (`crates/praxis-analyze/tests/clips`,
//! accuracy in docs/contracts/sound-profile.md).

use crate::profile::{Classification, ClipKind};

/// The cues the classifier reads.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Cues {
    pub active_s: f64,
    pub tempo_confidence: f64,
    pub regularity: f64,
    pub onsets: usize,
    pub onset_rate: f64,
    pub tonality: f64,
    pub level_iqr_db: f64,
    pub brightness_sd_oct: f64,
}

fn ramp(x: f64, lo: f64, hi: f64) -> f64 {
    if hi <= lo {
        return if x >= hi { 1.0 } else { 0.0 };
    }
    ((x - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// Unnormalised scores (logits) for music, sfx, ambience.
pub fn logits(c: &Cues) -> [f64; 3] {
    let long = ramp(c.active_s, 3.0, 8.0);
    // Random onsets land on a sixteenth about 40 % of the time and noise can show a spurious
    // periodicity, so only a clearly regular pulse counts.
    let pulse = c.tempo_confidence.clamp(0.0, 1.0) * ramp(c.regularity, 0.7, 0.92);
    let many = ramp(c.onsets as f64, 4.0, 12.0);
    let steady = 1.0 - ramp(c.level_iqr_db, 1.5, 5.0);
    let moving = ramp(c.brightness_sd_oct, 0.2, 0.8);
    let music = 4.0 * pulse + 1.5 * many + 1.0 * c.tonality + 1.0 * long - 2.5;
    let sfx = 2.5 * (1.0 - long) + 1.5 * moving + 1.0 * (1.0 - steady) - 1.0 * pulse - 0.5;
    let ambience = 2.5 * long + 2.0 * steady - 3.0 * pulse - 0.5 * c.tonality - 1.0;
    [music, sfx, ambience]
}

/// Classifies from the cues.
pub fn classify(c: &Cues) -> Classification {
    let l = logits(c);
    let m = l.iter().fold(f64::NEG_INFINITY, |a, b| a.max(*b));
    let e: Vec<f64> = l.iter().map(|v| (v - m).exp()).collect();
    let s: f64 = e.iter().sum();
    let scores = [(e[0] / s) as f32, (e[1] / s) as f32, (e[2] / s) as f32];
    let (best, conf) =
        scores.iter().enumerate().fold(
            (0usize, -1.0f32),
            |b, (i, v)| if *v > b.1 { (i, *v) } else { b },
        );
    Classification {
        kind: [ClipKind::Music, ClipKind::Sfx, ClipKind::Ambience][best],
        confidence: conf,
        scores,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typical_cues() {
        let music = Cues {
            active_s: 30.0,
            tempo_confidence: 0.7,
            regularity: 0.95,
            onsets: 120,
            onset_rate: 4.0,
            tonality: 0.8,
            level_iqr_db: 3.0,
            brightness_sd_oct: 0.3,
        };
        assert_eq!(classify(&music).kind, ClipKind::Music);
        let hit = Cues {
            active_s: 1.5,
            onsets: 1,
            tonality: 0.3,
            level_iqr_db: 10.0,
            brightness_sd_oct: 0.5,
            ..Cues::default()
        };
        assert_eq!(classify(&hit).kind, ClipKind::Sfx);
        let bed = Cues {
            active_s: 20.0,
            tempo_confidence: 0.1,
            regularity: 0.4,
            onsets: 3,
            tonality: 0.1,
            level_iqr_db: 1.0,
            brightness_sd_oct: 0.1,
            ..Cues::default()
        };
        assert_eq!(classify(&bed).kind, ClipKind::Ambience);
        let c = classify(&bed);
        assert!((c.scores.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }
}
