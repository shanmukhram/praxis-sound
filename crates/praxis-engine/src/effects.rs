//! Effect chains for layers and the master bus (FR-23, FR-24 wiring).
//!
//! Processors come from `praxis_dsp::fx` (WP-B2) through [`BuiltinEffects`]; tests and later
//! packages can inject their own through an [`EffectBuilder`]. Every effect that cannot be built
//! is listed in `RenderReport::warnings`, never silently dropped.
//!
//! Building one effect ([`EffectChain::build`]):
//! 1. bypassed effects are left out;
//! 2. `convolution_reverb` asks the builder for its impulse response ([`EffectBuilder::impulse`]);
//!    the built-in one knows the synthetic IRs [`IMPULSE_NAMES`] (an unknown name is a warning and
//!    the effect is skipped; no name uses [`DEFAULT_IMPULSE`]);
//! 3. the processor is created, `prepare`d, given every parameter from the spec (defaults from
//!    `praxis_score::params`, clamped to the spec range) and the score tempo (`set_tempo`, used by
//!    the delay), then the impulse response, then `reset` so the first sample already runs at the
//!    score's settings (no start-up glide from the processor's defaults).
//!
//! The chain reports `tail_samples` (each processor's tail capped at [`MAX_EFFECT_TAIL_SECONDS`],
//! because a delay at high feedback can report minutes) for loop warm-up planning and
//! `latency_samples` for latency compensation (see `latency.rs`). Compressors with a `sidechain`
//! are listed by [`EffectChain::sidechains`]; the graph feeds their key (see `sidechain.rs`).

use praxis_dsp::fx;
use praxis_dsp::rng::{Pcg32, derive};
use praxis_dsp::{Processor, StereoBuffer};
use praxis_score::params::{effect_param_value, effect_params};
use praxis_score::timeline::stable_hash;
use praxis_score::{Effect, EffectKind};

/// Upper bound for one effect's tail in warm-up and isolation planning.
pub const MAX_EFFECT_TAIL_SECONDS: f64 = 30.0;

/// Built-in synthetic impulse responses for `convolution_reverb`.
pub const IMPULSE_NAMES: [&str; 3] = ["room_small", "room_medium", "hall"];

/// The impulse response used when a convolution effect names none.
pub const DEFAULT_IMPULSE: &str = "room_medium";

/// Builds a processor for one effect spec.
pub trait EffectBuilder: Send + Sync {
    /// Returns `None` when the kind is not available.
    fn build(
        &self,
        effect: &Effect,
        sample_rate: f32,
        max_block: usize,
    ) -> Option<Box<dyn Processor>>;

    /// The impulse response for a `convolution_reverb` effect: `Ok(None)` for none (other kinds),
    /// `Err(reason)` to skip the effect with a warning. Called off the audio thread.
    fn impulse(&self, _effect: &Effect, _sample_rate: f32) -> Result<Option<StereoBuffer>, String> {
        Ok(None)
    }
}

/// The engine's built-in effects: `praxis_dsp::fx` and the synthetic impulse responses.
#[derive(Clone, Copy, Debug, Default)]
pub struct BuiltinEffects;

impl EffectBuilder for BuiltinEffects {
    fn build(
        &self,
        effect: &Effect,
        sample_rate: f32,
        max_block: usize,
    ) -> Option<Box<dyn Processor>> {
        fx::create(kind_index(effect.kind), sample_rate, max_block)
    }

    fn impulse(&self, effect: &Effect, sample_rate: f32) -> Result<Option<StereoBuffer>, String> {
        if effect.kind != EffectKind::ConvolutionReverb {
            return Ok(None);
        }
        let name = effect.impulse.as_deref().unwrap_or(DEFAULT_IMPULSE);
        builtin_impulse(name, sample_rate).map(Some).ok_or_else(|| {
            format!(
                "unknown impulse response `{name}` (built in: {})",
                IMPULSE_NAMES.join(", ")
            )
        })
    }
}

/// Builds and configures the processor for `effect` with the built-in effects (parameters from
/// the spec, tempo, impulse response). `Err` says why it could not be built.
pub fn build_effect(
    effect: &Effect,
    sample_rate: f32,
    max_block: usize,
    bpm: f32,
) -> Result<Box<dyn Processor>, String> {
    build_with(&BuiltinEffects, effect, sample_rate, max_block, bpm)
}

fn build_with(
    builder: &dyn EffectBuilder,
    effect: &Effect,
    sample_rate: f32,
    max_block: usize,
    bpm: f32,
) -> Result<Box<dyn Processor>, String> {
    let ir = builder.impulse(effect, sample_rate)?;
    let mut p = builder
        .build(effect, sample_rate, max_block)
        .ok_or_else(|| "no processor is available for this kind".to_string())?;
    p.prepare(sample_rate, max_block);
    for (i, spec) in effect_params(effect.kind).iter().enumerate() {
        let v =
            effect_param_value(effect.kind, &effect.params, spec.name).clamp(spec.min, spec.max);
        p.set_param(i, v as f32);
    }
    p.set_tempo(bpm);
    if let Some(ir) = &ir {
        p.set_impulse(ir);
    }
    p.reset();
    Ok(p)
}

/// Position of `kind` in `praxis_dsp::fx` (eq = 0 .. gain = 10).
pub fn kind_index(kind: EffectKind) -> usize {
    match kind {
        EffectKind::Eq => 0,
        EffectKind::Filter => 1,
        EffectKind::Compressor => 2,
        EffectKind::Saturation => 3,
        EffectKind::Delay => 4,
        EffectKind::Reverb => 5,
        EffectKind::ConvolutionReverb => 6,
        EffectKind::Chorus => 7,
        EffectKind::Width => 8,
        EffectKind::Limiter => 9,
        EffectKind::Gain => 10,
    }
}

/// Score name of an effect kind, for messages.
pub fn kind_name(kind: EffectKind) -> &'static str {
    fx::KIND_NAMES[kind_index(kind)]
}

/// Reverberation time (RT60, seconds) of a built-in impulse response.
pub fn impulse_rt60(name: &str) -> Option<f64> {
    match name {
        "room_small" => Some(0.45),
        "room_medium" => Some(1.1),
        "hall" => Some(2.6),
        _ => None,
    }
}

/// A built-in synthetic impulse response at `sample_rate`: decorrelated seeded noise per channel
/// with an exponential decay reaching -60 dB at the RT60, high frequencies decaying faster (a
/// one-pole low-pass closing over time), a 2 ms onset ramp, and a raised-cosine fade over the last
/// 10 % so the truncation at 1.5 x RT60 (-90 dB) is smooth. Each channel is scaled to unit energy,
/// so the wet signal is about as loud as the dry one. Deterministic: seeded from the name.
pub fn builtin_impulse(name: &str, sample_rate: f32) -> Option<StereoBuffer> {
    let rt60 = impulse_rt60(name)?;
    let sr = sample_rate as f64;
    let n = ((rt60 * 1.5 * sr).round() as usize).max(1);
    let seed = stable_hash(&[b"praxis-ir", name.as_bytes()]);
    let onset = (0.002 * sr).max(1.0);
    let fade_from = n as f64 * 0.9;
    let decay = -3.0 * std::f64::consts::LN_10 / (rt60 * sr); // ln(1e-3) per RT60
    let chan = |salt: u64| -> Vec<f32> {
        let mut rng = Pcg32::new(derive(seed, salt));
        let mut lp = 0.0f64;
        let mut out = Vec::with_capacity(n);
        let mut energy = 0.0f64;
        for i in 0..n {
            let t = i as f64;
            let x = rng.next_bipolar() as f64;
            // Low-pass coefficient from bright (0.95) to dark (0.15) across the RT60.
            let a = 0.15 + 0.8 * (-(t / (0.6 * rt60 * sr))).exp();
            lp += a * (x - lp);
            let mut g = (decay * t).exp();
            if t < onset {
                g *= t / onset;
            }
            if t > fade_from {
                let f = (t - fade_from) / (n as f64 - fade_from);
                g *= 0.5 * (1.0 + (std::f64::consts::PI * f).cos());
            }
            let v = lp * g;
            energy += v * v;
            out.push(v);
        }
        let k = if energy > 0.0 {
            1.0 / energy.sqrt()
        } else {
            0.0
        };
        out.into_iter().map(|v| (v * k) as f32).collect()
    };
    let left = chan(1);
    let right = chan(2);
    Some(StereoBuffer { left, right })
}

/// One built effect in a chain.
pub struct ChainSlot {
    pub effect_id: String,
    pub kind: EffectKind,
    pub processor: Box<dyn Processor>,
    /// Layer whose instrument output keys this slot (compressor sidechain).
    pub sidechain: Option<String>,
}

/// Effects in order, processed in place.
#[derive(Default)]
pub struct EffectChain {
    slots: Vec<ChainSlot>,
    /// Tail cap in samples ([`MAX_EFFECT_TAIL_SECONDS`] at the chain's sample rate).
    tail_cap: usize,
}

impl EffectChain {
    pub fn empty() -> EffectChain {
        EffectChain::default()
    }

    /// Builds the chain for `effects`. Bypassed effects are left out. Effects that cannot be built
    /// are skipped and reported in `warnings`, prefixed with `owner` ("layer `piano`").
    /// `bpm` is the score tempo for tempo-synced effects.
    pub fn build(
        effects: &[Effect],
        builder: &dyn EffectBuilder,
        sample_rate: f32,
        max_block: usize,
        bpm: f32,
        owner: &str,
        warnings: &mut Vec<String>,
    ) -> EffectChain {
        let mut slots = Vec::with_capacity(effects.len());
        for e in effects {
            if e.bypass {
                continue;
            }
            if e.kind == EffectKind::ConvolutionReverb && e.impulse.is_none() {
                warnings.push(format!(
                    "{owner}: effect `{}` (convolution_reverb) names no impulse response; using `{DEFAULT_IMPULSE}`",
                    e.id
                ));
            }
            let p = match build_with(builder, e, sample_rate, max_block, bpm) {
                Ok(p) => p,
                Err(why) => {
                    warnings.push(format!(
                        "{owner}: effect `{}` ({}) skipped: {why}",
                        e.id,
                        kind_name(e.kind)
                    ));
                    continue;
                }
            };
            let sidechain = match (&e.sidechain, e.kind) {
                (Some(sc), EffectKind::Compressor) => Some(sc.clone()),
                (Some(sc), k) => {
                    warnings.push(format!(
                        "{owner}: effect `{}` ({}) cannot take a sidechain (from `{sc}`); ignored",
                        e.id,
                        kind_name(k)
                    ));
                    None
                }
                (None, _) => None,
            };
            slots.push(ChainSlot {
                effect_id: e.id.clone(),
                kind: e.kind,
                processor: p,
                sidechain,
            });
        }
        let tail_cap = (MAX_EFFECT_TAIL_SECONDS * sample_rate as f64).round() as usize;
        EffectChain { slots, tail_cap }
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Processes one block in place, in chain order.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for s in &mut self.slots {
            s.processor.process(left, right);
        }
    }

    /// Sum of the processors' tails, each capped at [`MAX_EFFECT_TAIL_SECONDS`].
    pub fn tail_samples(&self) -> usize {
        self.slots
            .iter()
            .map(|s| s.processor.tail_samples().min(self.tail_cap))
            .fold(0usize, |a, b| a.saturating_add(b))
    }

    /// Sum of the processors' latencies.
    pub fn latency_samples(&self) -> usize {
        self.latency_before(self.slots.len())
    }

    /// Latency of the slots before `slot` (how far the signal reaching `slot` lags the chain
    /// input).
    pub fn latency_before(&self, slot: usize) -> usize {
        self.slots[..slot.min(self.slots.len())]
            .iter()
            .map(|s| s.processor.latency_samples())
            .fold(0usize, |a, b| a.saturating_add(b))
    }

    /// Index of the built slot for effect `id`, if it was built.
    pub fn slot_of(&self, id: &str) -> Option<usize> {
        self.slots.iter().position(|s| s.effect_id == id)
    }

    pub fn slot_kind(&self, slot: usize) -> Option<EffectKind> {
        self.slots.get(slot).map(|s| s.kind)
    }

    /// (slot, key layer id) of every sidechained slot, in chain order.
    pub fn sidechains(&self) -> Vec<(usize, String)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.sidechain.clone().map(|k| (i, k)))
            .collect()
    }

    pub fn set_param(&mut self, slot: usize, index: usize, value: f32) {
        if let Some(s) = self.slots.get_mut(slot) {
            s.processor.set_param(index, value);
        }
    }

    /// Feeds the sidechain key of `slot` for its next `process` call.
    pub fn set_sidechain(&mut self, slot: usize, left: &[f32], right: &[f32]) {
        if let Some(s) = self.slots.get_mut(slot) {
            s.processor.set_sidechain(left, right);
        }
    }

    pub fn reset(&mut self) {
        for s in &mut self.slots {
            s.processor.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effect(id: &str, kind: EffectKind) -> Effect {
        Effect {
            id: id.into(),
            kind,
            params: Default::default(),
            bypass: false,
            sidechain: None,
            impulse: None,
        }
    }

    #[test]
    fn every_kind_builds_and_bypass_and_unknown_impulse_are_reported() {
        let all = [
            EffectKind::Eq,
            EffectKind::Filter,
            EffectKind::Compressor,
            EffectKind::Saturation,
            EffectKind::Delay,
            EffectKind::Reverb,
            EffectKind::ConvolutionReverb,
            EffectKind::Chorus,
            EffectKind::Width,
            EffectKind::Limiter,
            EffectKind::Gain,
        ];
        for (i, k) in all.iter().enumerate() {
            assert_eq!(kind_index(*k), i);
            assert!(build_effect(&effect("x", *k), 48_000.0, 128, 120.0).is_ok());
        }
        let mut off = effect("off", EffectKind::Eq);
        off.bypass = true;
        let mut bad = effect("cave", EffectKind::ConvolutionReverb);
        bad.impulse = Some("cave".into());
        let mut hall = effect("hall", EffectKind::ConvolutionReverb);
        hall.impulse = Some("hall".into());
        let fx = vec![effect("room", EffectKind::Reverb), off, bad, hall];
        let mut w = Vec::new();
        let chain = EffectChain::build(
            &fx,
            &BuiltinEffects,
            48_000.0,
            128,
            120.0,
            "layer `x`",
            &mut w,
        );
        assert_eq!(chain.len(), 2);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(
            w[0].contains("`cave` (convolution_reverb) skipped: unknown impulse response `cave`")
        );
        // Reverb RT60 2.2 s + hall IR 3.9 s (plus small filter ring-outs).
        let tail = chain.tail_samples() as f64 / 48_000.0;
        assert!(tail > 6.0 && tail < 6.5, "tail {tail}");
        assert_eq!(chain.latency_samples(), 0);
    }

    #[test]
    fn impulses_are_deterministic_and_decay() {
        for name in IMPULSE_NAMES {
            let a = builtin_impulse(name, 48_000.0).unwrap();
            let b = builtin_impulse(name, 48_000.0).unwrap();
            assert_eq!(a, b);
            assert_ne!(a.left, a.right, "channels are decorrelated");
            let e: f64 = a.left.iter().map(|v| (*v as f64).powi(2)).sum();
            assert!((e - 1.0).abs() < 1e-4);
            let rt = impulse_rt60(name).unwrap();
            let at = |s: f64| {
                let i = (s * 48_000.0) as usize;
                let w = &a.left[i..i + 2400];
                (w.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / w.len() as f64).sqrt()
            };
            // About 60 dB down after one RT60 (the low-pass darkens it a little more).
            let drop = 20.0 * (at(0.01) / at(rt)).log10();
            assert!(drop > 55.0 && drop < 75.0, "{name}: {drop} dB");
        }
        assert!(builtin_impulse("nope", 48_000.0).is_none());
    }

    #[test]
    fn delay_tails_are_capped() {
        let mut d = effect("echo", EffectKind::Delay);
        d.params.insert("feedback".into(), 0.95);
        d.params.insert("time_beats".into(), 4.0);
        let mut w = Vec::new();
        let chain = EffectChain::build(&[d], &BuiltinEffects, 48_000.0, 128, 60.0, "m", &mut w);
        assert!(chain.tail_samples() <= 30 * 48_000);
    }
}
