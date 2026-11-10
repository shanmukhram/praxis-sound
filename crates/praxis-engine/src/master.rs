//! Master bus: loudness normalisation (or frozen gain) and the true-peak limiter (FR-25,
//! docs/contracts/engine.md, ADR-0007).
//!
//! The graph produces the *bus*: the sum of the stems in score order through the master effects,
//! latency-compensated and captured over the render window (render.rs). This module turns it into
//! the master:
//!
//! 1. **Measure** the bus's integrated loudness (BS.1770, `praxis_dsp::meter::LoudnessMeter`)
//!    over the captured cycle (loops) or the whole render (one-shots). Loops shorter than 3 s are
//!    measured over enough repeats of the cycle to fill 3 s, as a looping player would play them.
//! 2. **Gain**: `master.frozen_gain_db` when set (no measurement decides anything, so a scoped edit
//!    cannot move the level of untouched bars); otherwise `target - measured` when `normalize` is
//!    on; otherwise 0 dB. Gains are limited to [`MIN_GAIN_DB`]..=[`MAX_GAIN_DB`].
//! 3. **Limit** with `praxis_dsp::fx::TruePeakLimiter` at `master.loudness.true_peak_dbtp`
//!    (clamped to the limiter's -12..=0 dBTP range). The limiter is always on (also with
//!    `normalize: false`, and with a frozen gain).
//! 4. **Re-measure**; when normalising and the limiter pulled the loudness more than
//!    [`ITERATE_BELOW_LU`] below the target, raise the gain by the shortfall and limit again (at
//!    most [`MAX_ITERATIONS`] times). A target still missed by more than 0.5 LU is a warning.
//!
//! **Loops**: the limiter runs in steady state, like the layers. The bus cycle is already steady
//! state (warm-up cycles in render.rs), so the limiter is fed the cycle periodically for
//! [`limiter_warmup_cycles`] cycles plus its latency before the captured cycle; its gain at sample
//! 0 is then the gain it had at the end of the previous cycle and the seam stays continuous.
//! **One-shots**: the limiter starts from rest and is fed its latency in silence after the bus.
//! Either way the output is aligned with the grid (latency compensated).
//!
//! **Isolation** (docs/contracts/patch.md): with a frozen gain the master differs from an unedited
//! render only inside the edit window widened by [`isolation_tail_samples`] after it (master
//! chain tail, limiter latency and release settling) and by the limiter lookahead before it.

use crate::effects::{BuiltinEffects, EffectChain, build_effect};
use praxis_dsp::meter::{LoudnessMeter, true_peak_dbtp};
use praxis_dsp::util::db_to_gain_f64;
use praxis_dsp::{Processor, StereoBuffer};
use praxis_score::params::effect_param;
use praxis_score::{Effect, EffectKind, Loudness, Master};
use std::collections::BTreeMap;

/// Re-measure and correct when the limited master is this far below the target (LU).
pub const ITERATE_BELOW_LU: f64 = 0.3;
/// Correction passes after the first limiting pass.
pub const MAX_ITERATIONS: u32 = 3;
/// Loudness tolerance (PRD section 2), for the warning.
pub const TOLERANCE_LU: f64 = 0.5;
pub const MIN_GAIN_DB: f64 = -60.0;
pub const MAX_GAIN_DB: f64 = 48.0;
/// Loops shorter than this are measured over repeats of the cycle.
pub const MIN_MEASURE_SECONDS: f64 = 3.0;
/// The limiter's release state converges to the bit within this many release time constants
/// (`2^-24` is about `e^-16.6`).
pub const RELEASE_SETTLE_TIME_CONSTANTS: f64 = 17.0;
/// The limiter re-sums its gain box filter every this many samples, which ends any rounding
/// difference an edit left in the running sum (praxis_dsp::fx::limiter).
pub const LIMITER_RESYNC_SAMPLES: u64 = 65_536;

/// What the master stage did.
#[derive(Clone, Debug, PartialEq)]
pub struct MasterResult {
    pub audio: StereoBuffer,
    /// Integrated loudness of the bus before gain and limiter (LUFS).
    pub loudness_before_lufs: f64,
    /// Integrated loudness of the final master (LUFS).
    pub loudness_lufs: f64,
    /// True peak of the final master (dBTP).
    pub true_peak_dbtp: f64,
    /// Gain applied before the limiter (dB).
    pub applied_gain_db: f64,
    /// Correction passes run (0 when the first pass met the target or the gain was frozen).
    pub iterations: u32,
}

/// The score's master limiter as an effect spec (ceiling from `loudness.true_peak_dbtp`,
/// release and lookahead at their defaults).
pub fn limiter_spec(loudness: &Loudness) -> Effect {
    let mut params = BTreeMap::new();
    params.insert("ceiling_dbtp".to_string(), ceiling_dbtp(loudness));
    Effect {
        id: "master_limiter".into(),
        kind: EffectKind::Limiter,
        params,
        bypass: false,
        sidechain: None,
        impulse: None,
    }
}

/// The ceiling the limiter actually uses: `true_peak_dbtp` within the limiter's range.
pub fn ceiling_dbtp(loudness: &Loudness) -> f64 {
    let spec = effect_param(EffectKind::Limiter, "ceiling_dbtp");
    let (lo, hi) = spec.map(|s| (s.min, s.max)).unwrap_or((-12.0, 0.0));
    let c = loudness.true_peak_dbtp;
    if c.is_finite() {
        c.clamp(lo, hi)
    } else {
        hi.min(-1.0)
    }
}

fn default_param(name: &str, fallback: f64) -> f64 {
    effect_param(EffectKind::Limiter, name)
        .map(|s| s.default)
        .unwrap_or(fallback)
}

/// Samples after which the limiter has forgotten a change in its input: its latency, the release
/// settling time and one resync period of its running sum.
pub fn limiter_settle_samples(sample_rate: u32) -> u64 {
    let sr = sample_rate as f64;
    let release_s = default_param("release_ms", 80.0) * 0.001;
    let lookahead = (default_param("lookahead_ms", 3.0) * 0.001 * sr).round() as u64;
    let release = (release_s * sr * RELEASE_SETTLE_TIME_CONSTANTS).ceil() as u64;
    lookahead + 16 + release + LIMITER_RESYNC_SAMPLES
}

/// How far after an edit window the master can still differ with a frozen gain: the master
/// chain's effect tails plus [`limiter_settle_samples`]. The window also starts
/// [`limiter_latency_samples`] earlier (lookahead).
pub fn isolation_tail_samples(
    master: &Master,
    sample_rate: u32,
    max_block: usize,
    bpm: f32,
) -> u64 {
    let mut w = Vec::new();
    let chain = EffectChain::build(
        &master.effects,
        &BuiltinEffects,
        sample_rate as f32,
        max_block,
        bpm,
        "master",
        &mut w,
    );
    chain.tail_samples() as u64
        + chain.latency_samples() as u64
        + limiter_settle_samples(sample_rate)
}

/// The master limiter's latency (its lookahead) in samples.
pub fn limiter_latency_samples(loudness: &Loudness, sample_rate: u32, max_block: usize) -> u64 {
    build_effect(
        &limiter_spec(loudness),
        sample_rate as f32,
        max_block,
        120.0,
    )
    .map(|p| p.latency_samples() as u64)
    .unwrap_or(0)
}

/// Loop cycles the limiter runs before the captured one so its state is steady.
pub fn limiter_warmup_cycles(loop_len: u64, sample_rate: u32) -> u64 {
    limiter_settle_samples(sample_rate)
        .div_ceil(loop_len.max(1))
        .max(1)
}

/// Integrated loudness of `audio` (see the module docs for loops).
pub fn measure_lufs(audio: &StereoBuffer, sample_rate: u32, is_loop: bool) -> f64 {
    let n = audio.len();
    if n == 0 {
        return f64::NEG_INFINITY;
    }
    let mut m = LoudnessMeter::new(sample_rate as f32);
    let min = (MIN_MEASURE_SECONDS * sample_rate as f64).ceil() as usize;
    let reps = if is_loop && n < min {
        min.div_ceil(n)
    } else {
        1
    };
    for _ in 0..reps {
        m.process(&audio.left, &audio.right);
    }
    m.integrated_lufs()
}

/// The master stage for one render.
pub struct MasterBus {
    limiter: Box<dyn Processor>,
    latency: u64,
    ceiling_dbtp: f64,
    scratch_l: Vec<f32>,
    scratch_r: Vec<f32>,
    sample_rate: u32,
    block: usize,
    target_lufs: f64,
    normalize: bool,
    frozen_gain_db: Option<f64>,
}

impl MasterBus {
    /// Builds the limiter for `master`; problems with the settings go to `warnings`.
    pub fn new(
        master: &Master,
        sample_rate: u32,
        block: usize,
        warnings: &mut Vec<String>,
    ) -> Result<MasterBus, String> {
        let l = &master.loudness;
        let c = ceiling_dbtp(l);
        if c != l.true_peak_dbtp {
            warnings.push(format!(
                "master: true_peak_dbtp {} is outside the limiter's range; using {c} dBTP",
                l.true_peak_dbtp
            ));
        }
        let limiter = build_effect(&limiter_spec(l), sample_rate as f32, block, 120.0)
            .map_err(|e| format!("master limiter: {e}"))?;
        let latency = limiter.latency_samples() as u64;
        let frozen_gain_db = match master.frozen_gain_db {
            Some(g) if g.is_finite() => Some(g.clamp(MIN_GAIN_DB, MAX_GAIN_DB)),
            Some(_) => {
                warnings.push("master: frozen_gain_db is not finite; ignored".into());
                None
            }
            None => None,
        };
        if let (Some(g), Some(c)) = (master.frozen_gain_db, frozen_gain_db)
            && g != c
        {
            warnings.push(format!("master: frozen_gain_db {g} dB limited to {c} dB"));
        }
        let target_lufs = if l.target_lufs.is_finite() {
            l.target_lufs
        } else {
            warnings.push("master: target_lufs is not finite; normalisation off".into());
            f64::NAN
        };
        Ok(MasterBus {
            limiter,
            latency,
            ceiling_dbtp: c,
            scratch_l: vec![0.0; block.max(1)],
            scratch_r: vec![0.0; block.max(1)],
            sample_rate,
            block: block.max(1),
            target_lufs,
            normalize: l.normalize && l.target_lufs.is_finite(),
            frozen_gain_db,
        })
    }

    /// The limiter's latency in samples (compensated inside [`Self::finish`]).
    pub fn latency_samples(&self) -> u64 {
        self.latency
    }

    /// Gain, limiter and measurement over the captured `bus`.
    pub fn finish(
        &mut self,
        bus: &StereoBuffer,
        is_loop: bool,
        warnings: &mut Vec<String>,
    ) -> MasterResult {
        let sr = self.sample_rate;
        let before = measure_lufs(bus, sr, is_loop);
        let normalising = self.frozen_gain_db.is_none() && self.normalize && before.is_finite();
        let mut gain = match self.frozen_gain_db {
            Some(g) => g,
            None if normalising => self.target_lufs - before,
            None => 0.0,
        };
        if gain > MAX_GAIN_DB || gain < MIN_GAIN_DB {
            warnings.push(format!(
                "master: normalisation gain {gain:.1} dB limited to {MIN_GAIN_DB}..={MAX_GAIN_DB} dB"
            ));
            gain = gain.clamp(MIN_GAIN_DB, MAX_GAIN_DB);
        }
        let mut out = StereoBuffer::silent(bus.len());
        self.limit(bus, gain, is_loop, &mut out);
        let mut after = measure_lufs(&out, sr, is_loop);
        let mut iterations = 0;
        if normalising {
            while after.is_finite()
                && after < self.target_lufs - ITERATE_BELOW_LU
                && iterations < MAX_ITERATIONS
                && gain < MAX_GAIN_DB
            {
                gain = (gain + self.target_lufs - after).min(MAX_GAIN_DB);
                self.limit(bus, gain, is_loop, &mut out);
                after = measure_lufs(&out, sr, is_loop);
                iterations += 1;
            }
            if (after - self.target_lufs).abs() > TOLERANCE_LU {
                warnings.push(format!(
                    "master: loudness {after:.2} LUFS misses the target {:.1} LUFS by more than {TOLERANCE_LU} LU (the true-peak limiter at {} dBTP holds it down; the material is too peaky)",
                    self.target_lufs,
                    self.ceiling_dbtp
                ));
            }
        }
        let tp = true_peak_dbtp(&out.left, &out.right, sr as f32);
        MasterResult {
            audio: out,
            loudness_before_lufs: before,
            loudness_lufs: after,
            true_peak_dbtp: tp,
            applied_gain_db: gain,
            iterations,
        }
    }

    /// Applies `gain_db` and the limiter to `bus` into `out` (same length), latency compensated.
    fn limit(&mut self, bus: &StereoBuffer, gain_db: f64, is_loop: bool, out: &mut StereoBuffer) {
        let n = bus.len();
        let g = db_to_gain_f64(gain_db) as f32;
        self.limiter.reset();
        if n == 0 {
            return;
        }
        let la = self.latency;
        let warm = if is_loop {
            limiter_warmup_cycles(n as u64, self.sample_rate) * n as u64
        } else {
            0
        };
        let skip = warm + la; // output samples before the captured window
        let total = skip + n as u64;
        let (bl, br) = (&mut self.scratch_l, &mut self.scratch_r);
        let mut t = 0u64;
        while t < total {
            let m = ((total - t) as usize).min(self.block);
            for k in 0..m {
                let i = t + k as u64;
                let (l, r) = if is_loop {
                    let j = (i % n as u64) as usize;
                    (bus.left[j], bus.right[j])
                } else if (i as usize) < n {
                    (bus.left[i as usize], bus.right[i as usize])
                } else {
                    (0.0, 0.0)
                };
                bl[k] = l * g;
                br[k] = r * g;
            }
            self.limiter.process(&mut bl[..m], &mut br[..m]);
            for k in 0..m {
                let j = t + k as u64;
                if j >= skip {
                    let o = (j - skip) as usize;
                    out.left[o] = bl[k];
                    out.right[o] = br[k];
                }
            }
            t += m as u64;
        }
    }
}
