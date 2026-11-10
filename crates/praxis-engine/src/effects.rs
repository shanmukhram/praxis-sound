//! Effect-chain plumbing for layers and the master bus.
//!
//! Effect processors themselves come from `praxis_dsp::fx` (WP-B2), which is not wired yet:
//! [`build_effect`] returns `None` for every kind, and every effect that could not be built is
//! listed in `RenderReport::warnings` (never silently dropped). WP-F2 replaces the body of
//! [`build_effect`] with the real factory; nothing else in the engine changes.
//!
//! When a processor exists the chain honours its contract: `prepare` before the first block,
//! initial parameters set from the score (clamped to `praxis_score::params`), `tail_samples`
//! summed into the loop warm-up planning, and `latency_samples` summed for latency compensation.
//! Tests and later packages can inject processors through an [`EffectBuilder`].

use praxis_dsp::Processor;
use praxis_score::params::{effect_param_value, effect_params};
use praxis_score::{Effect, EffectKind};

/// Builds a processor for one effect spec. Returns `None` when the kind is not available.
pub trait EffectBuilder: Send + Sync {
    fn build(
        &self,
        effect: &Effect,
        sample_rate: f32,
        max_block: usize,
    ) -> Option<Box<dyn Processor>>;
}

/// The engine's built-in effects: currently none (see [`build_effect`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct BuiltinEffects;

impl EffectBuilder for BuiltinEffects {
    fn build(
        &self,
        effect: &Effect,
        sample_rate: f32,
        max_block: usize,
    ) -> Option<Box<dyn Processor>> {
        build_effect(effect, sample_rate, max_block)
    }
}

/// Builds the processor for `effect`. Phase 1: `praxis_dsp::fx` (WP-B2) is not available to the
/// engine yet, so this returns `None` for every kind and the caller reports the effect as skipped.
pub fn build_effect(
    effect: &Effect,
    sample_rate: f32,
    max_block: usize,
) -> Option<Box<dyn Processor>> {
    let _ = (effect, sample_rate, max_block);
    None
}

/// Score name of an effect kind, for messages.
pub fn kind_name(kind: EffectKind) -> &'static str {
    match kind {
        EffectKind::Eq => "eq",
        EffectKind::Filter => "filter",
        EffectKind::Compressor => "compressor",
        EffectKind::Saturation => "saturation",
        EffectKind::Delay => "delay",
        EffectKind::Reverb => "reverb",
        EffectKind::ConvolutionReverb => "convolution_reverb",
        EffectKind::Chorus => "chorus",
        EffectKind::Width => "width",
        EffectKind::Limiter => "limiter",
        EffectKind::Gain => "gain",
    }
}

/// One built effect in a chain.
pub struct ChainSlot {
    pub effect_id: String,
    pub kind: EffectKind,
    pub processor: Box<dyn Processor>,
}

/// Effects in order, processed in place.
#[derive(Default)]
pub struct EffectChain {
    slots: Vec<ChainSlot>,
}

impl EffectChain {
    pub fn empty() -> EffectChain {
        EffectChain { slots: Vec::new() }
    }

    /// Builds the chain for `effects`. Bypassed effects are left out. Effects the builder cannot
    /// make are skipped and reported in `warnings`, prefixed with `owner` ("layer `piano`").
    pub fn build(
        effects: &[Effect],
        builder: &dyn EffectBuilder,
        sample_rate: f32,
        max_block: usize,
        owner: &str,
        warnings: &mut Vec<String>,
    ) -> EffectChain {
        let mut slots = Vec::with_capacity(effects.len());
        for e in effects {
            if e.bypass {
                continue;
            }
            let Some(mut p) = builder.build(e, sample_rate, max_block) else {
                warnings.push(format!(
                    "{owner}: effect `{}` ({}) skipped: effect processors are not available in the engine yet (praxis_dsp::fx, WP-B2/WP-F2)",
                    e.id,
                    kind_name(e.kind)
                ));
                continue;
            };
            p.prepare(sample_rate, max_block);
            for (i, spec) in effect_params(e.kind).iter().enumerate() {
                let v = effect_param_value(e.kind, &e.params, spec.name).clamp(spec.min, spec.max);
                p.set_param(i, v as f32);
            }
            if let Some(sc) = &e.sidechain {
                warnings.push(format!(
                    "{owner}: effect `{}` sidechain from `{sc}` is not wired yet (WP-F2); it runs unkeyed",
                    e.id
                ));
            }
            slots.push(ChainSlot {
                effect_id: e.id.clone(),
                kind: e.kind,
                processor: p,
            });
        }
        EffectChain { slots }
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

    /// Sum of the processors' tails.
    pub fn tail_samples(&self) -> usize {
        self.slots
            .iter()
            .map(|s| s.processor.tail_samples())
            .fold(0usize, |a, b| a.saturating_add(b))
    }

    /// Sum of the processors' latencies.
    pub fn latency_samples(&self) -> usize {
        self.slots
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

    pub fn set_param(&mut self, slot: usize, index: usize, value: f32) {
        if let Some(s) = self.slots.get_mut(slot) {
            s.processor.set_param(index, value);
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

    #[test]
    fn unbuilt_effects_are_reported() {
        let fx = vec![
            Effect {
                id: "room".into(),
                kind: EffectKind::Reverb,
                params: Default::default(),
                bypass: false,
                sidechain: None,
                impulse: None,
            },
            Effect {
                id: "off".into(),
                kind: EffectKind::Eq,
                params: Default::default(),
                bypass: true,
                sidechain: None,
                impulse: None,
            },
        ];
        let mut w = Vec::new();
        let chain = EffectChain::build(&fx, &BuiltinEffects, 48_000.0, 128, "layer `x`", &mut w);
        assert!(chain.is_empty());
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("`room` (reverb) skipped"));
        assert_eq!(chain.tail_samples(), 0);
        assert_eq!(chain.latency_samples(), 0);
    }
}
