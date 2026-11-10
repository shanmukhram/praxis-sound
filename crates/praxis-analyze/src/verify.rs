//! Objective checks of a render against its Score. FROZEN CONTRACT for the types and the
//! `verify_render` signature; WP-D implements the body. The CLI `praxis check` prints this report.

use praxis_dsp::StereoBuffer;
use praxis_score::Score;
use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Check {
    /// "tempo", "key", "grid", "loudness", "true_peak", "loop_seam", "clipping", "silence".
    pub name: String,
    pub passed: bool,
    pub measured: String,
    pub expected: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct VerifyReport {
    pub checks: Vec<Check>,
    /// Detected tempo, BPM.
    pub tempo_bpm: Option<f64>,
    /// Detected key, e.g. "a minor".
    pub key: Option<String>,
    /// Median absolute onset deviation from the nearest grid step, ms.
    pub grid_deviation_ms: Option<f64>,
    /// Fraction of detected onsets within 10 ms of a grid step.
    pub onsets_on_grid: Option<f64>,
    pub integrated_lufs: Option<f64>,
    pub true_peak_dbtp: Option<f64>,
    /// Sample-to-sample jump at the loop point relative to the typical jump (1.0 = ordinary).
    pub seam_jump_ratio: Option<f64>,
    /// Seam discontinuity minus the local noise floor, dB (<= 0 passes).
    pub seam_excess_db: Option<f64>,
}

impl VerifyReport {
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed)
    }
}

/// Tolerances for [`verify_render`]. Defaults follow PRD section 2.
#[derive(Clone, Debug, PartialEq)]
pub struct Tolerances {
    pub tempo_bpm: f64,
    pub loudness_lu: f64,
    pub onset_ms: f64,
    pub min_on_grid: f64,
}

impl Default for Tolerances {
    fn default() -> Self {
        Tolerances {
            tempo_bpm: 2.0,
            loudness_lu: 0.5,
            onset_ms: 10.0,
            min_on_grid: 0.9,
        }
    }
}

/// Measures `audio` (the master render of `score`) and compares it with what the score says.
pub fn verify_render(
    score: &Score,
    audio: &StereoBuffer,
    sample_rate: u32,
    tol: &Tolerances,
) -> VerifyReport {
    imp::verify(score, audio, sample_rate, tol)
}

// ---- WP-D1 implementation: private helpers (the types and signature above are frozen) ----
mod imp {
    use super::{Check, Tolerances, VerifyReport};
    use crate::features::key::mode_family;
    use crate::features::onset::{self, Onset};
    use crate::features::{self, amp_to_db, loudness, median, seam, tempo};
    use praxis_dsp::StereoBuffer;
    use praxis_score::timeline::{NoteEvent, TimeMap, expand_notes};
    use praxis_score::{GeneratorClass, Grid, Mode, Playback, Score};

    /// Bars quieter than this (RMS, dBFS) count as silent.
    const SILENCE_DBFS: f64 = -60.0;
    /// With fewer distinct note starts than this, tempo is not measurable; the check is skipped.
    const MIN_NOTE_STARTS_FOR_TEMPO: usize = 4;
    /// Loops are analysed with this much of their end prepended, so the downbeat at sample 0 is
    /// heard after the material that really precedes it.
    const LOOP_PREROLL_S: f64 = 0.3;

    fn check(name: &str, passed: bool, measured: String, expected: String) -> Check {
        Check {
            name: name.to_string(),
            passed,
            measured,
            expected,
        }
    }

    fn skipped(name: &str, measured: String, why: &str) -> Check {
        check(name, true, measured, format!("n/a ({why})"))
    }

    /// Pitched layers per contracts/score.md: every class except drums, percussion and tick.
    fn is_pitched(class: GeneratorClass) -> bool {
        !matches!(
            class,
            GeneratorClass::Drums | GeneratorClass::Percussion | GeneratorClass::Tick
        )
    }

    /// Layers the engine plays: not blocked by the avoid-list; solo wins over mute.
    fn audible_layers(score: &Score) -> Vec<bool> {
        let any_solo = score.layers.iter().any(|l| l.solo);
        score
            .layers
            .iter()
            .map(|l| {
                let on = if any_solo { l.solo } else { !l.muted };
                on && !score.constraints.avoid.contains(&l.class)
            })
            .collect()
    }

    fn grid_name(g: Grid) -> &'static str {
        match g {
            Grid::Quarter => "quarter",
            Grid::Eighth => "eighth",
            Grid::Sixteenth => "sixteenth",
            Grid::EighthTriplet => "eighth-triplet",
            Grid::SixteenthTriplet => "sixteenth-triplet",
        }
    }

    fn opt_bpm(t: Option<&tempo::TempoResult>) -> String {
        t.map(|t| format!("{:.2} bpm", t.bpm))
            .unwrap_or_else(|| "none".to_string())
    }

    /// Onsets of a loop, analysed circularly: the end of the clip is prepended and onsets are
    /// kept in `[-10 ms, len - 10 ms)`, so each event is counted exactly once.
    fn loop_onsets(mono: &[f32], sample_rate: u32) -> Vec<Onset> {
        let n = mono.len();
        let pre = ((LOOP_PREROLL_S * sample_rate as f64).round() as usize).min(n);
        let mut ext = Vec::with_capacity(n + pre);
        ext.extend_from_slice(&mono[n - pre..]);
        ext.extend_from_slice(mono);
        let sr = sample_rate as f64;
        let shift = pre as f64 / sr;
        let end = n as f64 / sr - 0.01;
        onset::detect_onsets(&ext, sample_rate)
            .1
            .into_iter()
            .map(|mut o| {
                o.time_s -= shift;
                o
            })
            .filter(|o| o.time_s >= -0.01 && o.time_s < end)
            .collect()
    }

    fn tempo_check(
        report: &mut VerifyReport,
        env: &onset::OnsetEnvelope,
        score: &Score,
        note_starts: usize,
        tol: &Tolerances,
    ) -> Check {
        let est = tempo::estimate_tempo(env, &tempo::TempoConfig::default());
        report.tempo_bpm = est.as_ref().map(|t| t.bpm);
        let bpm = score.tempo.bpm;
        let expected = format!("{:.2} +- {:.2} bpm", bpm, tol.tempo_bpm);
        if note_starts < MIN_NOTE_STARTS_FOR_TEMPO {
            return skipped("tempo", opt_bpm(est.as_ref()), "fewer than 4 note starts");
        }
        match est {
            Some(t) => check(
                "tempo",
                (t.bpm - bpm).abs() <= tol.tempo_bpm,
                format!("{:.2} bpm (confidence {:.2})", t.bpm, t.confidence),
                expected,
            ),
            None => check("tempo", false, "no periodic onsets".to_string(), expected),
        }
    }

    fn key_check(
        report: &mut VerifyReport,
        mono: &[f32],
        sample_rate: u32,
        score: &Score,
        pitched_notes: usize,
    ) -> Check {
        let chroma = features::chroma::chromagram(mono, sample_rate).mean();
        let est = features::key::estimate_key(&chroma);
        report.key = est.as_ref().map(|k| k.key.display());
        let measured = report.key.clone().unwrap_or_else(|| "none".to_string());
        if pitched_notes == 0 {
            return skipped("key", measured, "no pitched layers");
        }
        if score.key.mode == Mode::Chromatic {
            return skipped("key", measured, "chromatic score");
        }
        let expected = score.key.display();
        match est {
            Some(k) => check(
                "key",
                k.key.tonic == score.key.tonic && mode_family(score.key.mode) == Some(k.key.mode),
                format!("{} (confidence {:.2})", k.key.display(), k.confidence),
                expected,
            ),
            None => check("key", false, "no tonal content".to_string(), expected),
        }
    }

    fn grid_check(
        report: &mut VerifyReport,
        onsets: &[Onset],
        score: &Score,
        sample_rate: u32,
        has_notes: bool,
        tol: &Tolerances,
    ) -> Check {
        let finest = score
            .patterns
            .iter()
            .map(|p| p.grid)
            .min_by(|a, b| a.step_whole_notes().total_cmp(&b.step_whole_notes()));
        let Some(grid) = finest else {
            return skipped("grid", format!("{} onsets", onsets.len()), "no patterns");
        };
        if !has_notes {
            return skipped("grid", format!("{} onsets", onsets.len()), "no notes");
        }
        let step_s = TimeMap::new(score, sample_rate).step_s(grid);
        let expected = format!(
            ">= {:.0}% of onsets within {:.1} ms of the {} grid ({:.2} ms steps)",
            tol.min_on_grid * 100.0,
            tol.onset_ms,
            grid_name(grid),
            step_s * 1000.0
        );
        if onsets.is_empty() || step_s.is_nan() || step_s <= 0.0 {
            return check("grid", false, "no onsets detected".to_string(), expected);
        }
        let devs: Vec<f64> = onsets
            .iter()
            .map(|o| (o.time_s - (o.time_s / step_s).round() * step_s).abs() * 1000.0)
            .collect();
        let on = devs.iter().filter(|d| **d <= tol.onset_ms).count();
        let frac = on as f64 / devs.len() as f64;
        let med = median(&devs);
        report.onsets_on_grid = Some(frac);
        report.grid_deviation_ms = Some(med);
        check(
            "grid",
            frac >= tol.min_on_grid,
            format!(
                "{:.1}% of {} onsets on grid, median deviation {:.2} ms",
                frac * 100.0,
                devs.len(),
                med
            ),
            expected,
        )
    }

    fn loudness_checks(
        report: &mut VerifyReport,
        left: &[f32],
        right: &[f32],
        sample_rate: u32,
        score: &Score,
        has_notes: bool,
        tol: &Tolerances,
    ) {
        let target = score.master.loudness;
        let ceiling = target.true_peak_dbtp + 0.05;
        let l = match loudness::measure(left, right, sample_rate) {
            Ok(l) => l,
            Err(e) => {
                let expected = format!("{:.2} LUFS", target.target_lufs);
                report.checks.push(check(
                    "loudness",
                    false,
                    format!("meter error: {e}"),
                    expected,
                ));
                let expected = format!("<= {ceiling:.2} dBTP");
                report.checks.push(check(
                    "true_peak",
                    false,
                    format!("meter error: {e}"),
                    expected,
                ));
                return;
            }
        };
        let silent = l.integrated_lufs <= features::FLOOR_DB;
        report.integrated_lufs = (!silent).then_some(l.integrated_lufs);
        report.true_peak_dbtp = Some(l.true_peak_dbtp);
        let measured = if silent {
            "silent (below the gate)".to_string()
        } else {
            format!("{:.2} LUFS (LRA {:.1} LU)", l.integrated_lufs, l.range_lu)
        };
        let loud = if !has_notes {
            skipped("loudness", measured, "no notes")
        } else if target.normalize {
            check(
                "loudness",
                !silent && (l.integrated_lufs - target.target_lufs).abs() <= tol.loudness_lu,
                measured,
                format!("{:.2} +- {:.2} LUFS", target.target_lufs, tol.loudness_lu),
            )
        } else {
            skipped("loudness", measured, "normalisation off")
        };
        report.checks.push(loud);
        report.checks.push(check(
            "true_peak",
            l.true_peak_dbtp <= ceiling,
            format!("{:.2} dBTP", l.true_peak_dbtp),
            format!("<= {ceiling:.2} dBTP"),
        ));
    }

    fn clipping_check(left: &[f32], right: &[f32]) -> Check {
        let (mut over, mut non_finite, mut peak) = (0usize, 0usize, 0.0f64);
        for x in left.iter().chain(right.iter()) {
            if !x.is_finite() {
                non_finite += 1;
                continue;
            }
            let a = x.abs() as f64;
            peak = peak.max(a);
            if a >= 1.0 {
                over += 1;
            }
        }
        let mut measured = format!(
            "{over} samples at or above 1.0 (sample peak {:.2} dBFS)",
            amp_to_db(peak)
        );
        if non_finite > 0 {
            measured.push_str(&format!(", {non_finite} non-finite samples"));
        }
        check(
            "clipping",
            over == 0 && non_finite == 0,
            measured,
            "no sample with |x| >= 1.0".to_string(),
        )
    }

    fn seam_check(
        report: &mut VerifyReport,
        left: &[f32],
        right: &[f32],
        sample_rate: u32,
    ) -> Check {
        let cfg = seam::SeamConfig::default();
        let s = seam::analyze_seam(left, right, sample_rate, &cfg);
        report.seam_jump_ratio = Some(s.jump_ratio);
        report.seam_excess_db = Some(s.excess_db);
        check(
            "loop_seam",
            s.passed,
            format!(
                "jump {:.5} ({:.2}x typical, local max {:.5}); excess {:.2} dB (first-difference \
                 energy {:.2} dB, transient flux {:.2} dB, drop flux {:.2} dB vs the clip maximum)",
                s.jump,
                s.jump_ratio,
                s.local_max_jump,
                s.excess_db,
                s.diff_energy_db,
                s.transient_db,
                s.drop_db
            ),
            format!(
                "jump <= {:.2}x the local maximum jump; excess <= 0 dB ({:.1} dB margin)",
                cfg.jump_margin, cfg.margin_db
            ),
        )
    }

    fn silence_check(
        left: &[f32],
        right: &[f32],
        sample_rate: u32,
        score: &Score,
        notes: &[NoteEvent],
    ) -> Check {
        let n = left.len().min(right.len());
        let mut bars: Vec<u32> = notes.iter().map(|e| e.bar).collect();
        bars.sort_unstable();
        bars.dedup();
        if bars.is_empty() {
            let s: f64 = left
                .iter()
                .chain(right.iter())
                .map(|x| (*x as f64).powi(2))
                .sum();
            let rms = (s / (2 * n).max(1) as f64).sqrt();
            return skipped(
                "silence",
                format!("render RMS {:.1} dBFS", amp_to_db(rms)),
                "no notes",
            );
        }
        let tm = TimeMap::new(score, sample_rate);
        let mut quietest = f64::INFINITY;
        let mut silent: Vec<u32> = Vec::new();
        for b in &bars {
            let a = (tm.bar_start_sample(*b) as usize).min(n);
            let e = (tm.bar_start_sample(*b + 1) as usize).min(n);
            let s: f64 = (a..e)
                .map(|i| (left[i] as f64).powi(2) + (right[i] as f64).powi(2))
                .sum();
            let rms = if e > a {
                (s / (2 * (e - a)) as f64).sqrt()
            } else {
                0.0
            };
            let db = amp_to_db(rms);
            quietest = quietest.min(db);
            if db.is_nan() || db <= SILENCE_DBFS {
                silent.push(*b);
            }
        }
        let mut measured = format!(
            "{}/{} bars with notes audible (quietest {:.1} dBFS)",
            bars.len() - silent.len(),
            bars.len(),
            quietest
        );
        if !silent.is_empty() {
            let list: Vec<String> = silent.iter().take(8).map(|b| b.to_string()).collect();
            measured.push_str(&format!("; silent bars {}", list.join(", ")));
        }
        check(
            "silence",
            silent.is_empty(),
            measured,
            format!("every bar with notes above {SILENCE_DBFS:.0} dBFS RMS"),
        )
    }

    pub(super) fn verify(
        score: &Score,
        audio: &StereoBuffer,
        sample_rate: u32,
        tol: &Tolerances,
    ) -> VerifyReport {
        let mut report = VerifyReport::default();
        let n = audio.left.len().min(audio.right.len());
        let left = &audio.left[..n];
        let right = &audio.right[..n];
        if sample_rate == 0 || n == 0 {
            let why = if sample_rate == 0 {
                "sample rate is 0"
            } else {
                "render is empty"
            };
            report.checks.push(check(
                "silence",
                false,
                why.to_string(),
                "a non-empty render".to_string(),
            ));
            return report;
        }
        let mono = features::mono(left, right);
        let is_loop = matches!(score.playback, Playback::Loop);

        let audible = audible_layers(score);
        let notes: Vec<NoteEvent> = expand_notes(score, sample_rate)
            .into_iter()
            .filter(|e| audible.get(e.layer).copied().unwrap_or(false))
            .collect();
        let mut starts: Vec<u64> = notes.iter().map(|e| e.start_sample).collect();
        starts.dedup();
        let pitched_notes = notes
            .iter()
            .filter(|e| is_pitched(score.layers[e.layer].class))
            .count();

        let (env, linear_onsets) = onset::detect_onsets(&mono, sample_rate);
        let c = tempo_check(&mut report, &env, score, starts.len(), tol);
        report.checks.push(c);
        let c = key_check(&mut report, &mono, sample_rate, score, pitched_notes);
        report.checks.push(c);
        let onsets = if is_loop {
            loop_onsets(&mono, sample_rate)
        } else {
            linear_onsets
        };
        let c = grid_check(
            &mut report,
            &onsets,
            score,
            sample_rate,
            !notes.is_empty(),
            tol,
        );
        report.checks.push(c);
        loudness_checks(
            &mut report,
            left,
            right,
            sample_rate,
            score,
            !notes.is_empty(),
            tol,
        );
        report.checks.push(clipping_check(left, right));
        if is_loop {
            let c = seam_check(&mut report, left, right, sample_rate);
            report.checks.push(c);
        }
        report
            .checks
            .push(silence_check(left, right, sample_rate, score, &notes));
        report
    }
}
