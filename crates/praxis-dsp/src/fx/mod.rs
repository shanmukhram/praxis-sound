//! WP-B2 dsp-effects: one file per effect kind. See docs/contracts/dsp.md.
//!
//! Every effect implements [`Processor`], is built with `new(sample_rate, max_block)` and takes
//! parameters by index. The index is the row order of `praxis_score::params::effect_params(kind)`
//! (praxis-dsp cannot depend on praxis-score, so the table is mirrored in [`PARAMS`]; a
//! cross-crate test in the engine compares the two). Units are exactly those of the score table.
//!
//! | index | kind               | type                | parameters (index order)                                   |
//! |-------|--------------------|---------------------|------------------------------------------------------------|
//! | 0     | eq                 | [`Eq4`]             | hp_hz, lp_hz, low_hz, low_db, mid1_hz, mid1_db, mid1_q, mid2_hz, mid2_db, mid2_q, high_hz, high_db |
//! | 1     | filter             | [`FilterFx`]        | mode, cutoff_hz, resonance, mix                            |
//! | 2     | compressor         | [`Compressor`]      | threshold_db, ratio, attack_ms, release_ms, knee_db, makeup_db, mix |
//! | 3     | saturation         | [`Saturation`]      | drive_db, tone, mix, output_db                             |
//! | 4     | delay              | [`Delay`]           | time_beats, time_ms, feedback, ping_pong, damp_hz, mix     |
//! | 5     | reverb             | [`FdnReverb`]       | decay_s, predelay_ms, size, damp_hz, width, mix            |
//! | 6     | convolution_reverb | [`Convolver`]       | predelay_ms, decay_scale, hp_hz, mix                       |
//! | 7     | chorus             | [`Chorus`]          | rate_hz, depth_ms, delay_ms, mix                           |
//! | 8     | width              | [`Width`]           | width, bass_mono_hz                                        |
//! | 9     | limiter            | [`TruePeakLimiter`] | ceiling_dbtp, release_ms, lookahead_ms                     |
//! | 10    | gain               | [`GainFx`]          | gain_db                                                    |
//!
//! [`Delay::set_tempo`] and [`Convolver::set_impulse`] are not part of the frozen `Processor`
//! trait, so a `Box<dyn Processor>` from [`create`] cannot reach them. [`create_any`] returns an
//! [`AnyFx`] enum that implements `Processor` and also forwards those two calls.

use crate::buffer::StereoBuffer;
use crate::traits::Processor;

pub mod chorus;
pub mod compressor;
pub mod convolver;
pub mod delay;
pub mod eq;
pub mod gain;
pub mod limiter;
pub(crate) mod ramp;
pub mod reverb;
pub mod saturation;
pub mod svf;
pub mod width;

pub use chorus::Chorus;
pub use compressor::Compressor;
pub use convolver::Convolver;
pub use delay::Delay;
pub use eq::Eq4;
pub use gain::GainFx;
pub use limiter::TruePeakLimiter;
pub use reverb::FdnReverb;
pub use saturation::Saturation;
pub use svf::FilterFx;
pub use width::Width;

/// One parameter row, mirroring `praxis_score::params::ParamSpec` (name, range, default, unit).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxParam {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: &'static str,
}

const fn fp(name: &'static str, min: f32, max: f32, default: f32, unit: &'static str) -> FxParam {
    FxParam {
        name,
        min,
        max,
        default,
        unit,
    }
}

/// Number of effect kinds.
pub const KIND_COUNT: usize = 11;

/// Kind names in index order (the serde names of `EffectKind`).
pub const KIND_NAMES: [&str; KIND_COUNT] = [
    "eq",
    "filter",
    "compressor",
    "saturation",
    "delay",
    "reverb",
    "convolution_reverb",
    "chorus",
    "width",
    "limiter",
    "gain",
];

/// Mirror of `praxis_score::params::effect_params`, indexed by kind index.
pub const PARAMS: [&[FxParam]; KIND_COUNT] = [
    &[
        fp("hp_hz", 10.0, 2000.0, 20.0, "Hz"),
        fp("lp_hz", 1000.0, 22000.0, 22000.0, "Hz"),
        fp("low_hz", 30.0, 1000.0, 120.0, "Hz"),
        fp("low_db", -24.0, 24.0, 0.0, "dB"),
        fp("mid1_hz", 80.0, 8000.0, 500.0, "Hz"),
        fp("mid1_db", -24.0, 24.0, 0.0, "dB"),
        fp("mid1_q", 0.1, 10.0, 1.0, ""),
        fp("mid2_hz", 200.0, 16000.0, 3000.0, "Hz"),
        fp("mid2_db", -24.0, 24.0, 0.0, "dB"),
        fp("mid2_q", 0.1, 10.0, 1.0, ""),
        fp("high_hz", 1000.0, 20000.0, 8000.0, "Hz"),
        fp("high_db", -24.0, 24.0, 0.0, "dB"),
    ],
    &[
        fp("mode", 0.0, 3.0, 0.0, "enum"),
        fp("cutoff_hz", 20.0, 20000.0, 8000.0, "Hz"),
        fp("resonance", 0.0, 1.0, 0.1, ""),
        fp("mix", 0.0, 1.0, 1.0, ""),
    ],
    &[
        fp("threshold_db", -60.0, 0.0, -18.0, "dB"),
        fp("ratio", 1.0, 20.0, 3.0, ":1"),
        fp("attack_ms", 0.1, 200.0, 10.0, "ms"),
        fp("release_ms", 5.0, 2000.0, 150.0, "ms"),
        fp("knee_db", 0.0, 24.0, 6.0, "dB"),
        fp("makeup_db", -12.0, 24.0, 0.0, "dB"),
        fp("mix", 0.0, 1.0, 1.0, ""),
    ],
    &[
        fp("drive_db", 0.0, 36.0, 6.0, "dB"),
        fp("tone", -1.0, 1.0, 0.0, ""),
        fp("mix", 0.0, 1.0, 1.0, ""),
        fp("output_db", -24.0, 12.0, 0.0, "dB"),
    ],
    &[
        fp("time_beats", 0.0, 4.0, 0.75, "beats"),
        fp("time_ms", 1.0, 2000.0, 350.0, "ms"),
        fp("feedback", 0.0, 0.95, 0.35, ""),
        fp("ping_pong", 0.0, 1.0, 0.0, "bool"),
        fp("damp_hz", 500.0, 20000.0, 6000.0, "Hz"),
        fp("mix", 0.0, 1.0, 0.2, ""),
    ],
    &[
        fp("decay_s", 0.1, 20.0, 2.2, "s"),
        fp("predelay_ms", 0.0, 200.0, 15.0, "ms"),
        fp("size", 0.0, 1.0, 0.6, ""),
        fp("damp_hz", 500.0, 20000.0, 7000.0, "Hz"),
        fp("width", 0.0, 1.0, 1.0, ""),
        fp("mix", 0.0, 1.0, 0.18, ""),
    ],
    &[
        fp("predelay_ms", 0.0, 200.0, 0.0, "ms"),
        fp("decay_scale", 0.1, 2.0, 1.0, ""),
        fp("hp_hz", 20.0, 1000.0, 120.0, "Hz"),
        fp("mix", 0.0, 1.0, 0.18, ""),
    ],
    &[
        fp("rate_hz", 0.01, 8.0, 0.4, "Hz"),
        fp("depth_ms", 0.0, 20.0, 3.0, "ms"),
        fp("delay_ms", 1.0, 40.0, 12.0, "ms"),
        fp("mix", 0.0, 1.0, 0.3, ""),
    ],
    &[
        fp("width", 0.0, 2.0, 1.0, ""),
        fp("bass_mono_hz", 0.0, 500.0, 0.0, "Hz"),
    ],
    &[
        fp("ceiling_dbtp", -12.0, 0.0, -1.0, "dBTP"),
        fp("release_ms", 1.0, 1000.0, 80.0, "ms"),
        fp("lookahead_ms", 0.5, 10.0, 3.0, "ms"),
    ],
    &[fp("gain_db", -60.0, 24.0, 0.0, "dB")],
];

/// The parameter table of a kind (empty for an unknown index).
pub fn params(kind_index: usize) -> &'static [FxParam] {
    PARAMS.get(kind_index).copied().unwrap_or(&[])
}

/// Number of parameters of a kind (0 for an unknown index).
pub fn param_count(kind_index: usize) -> usize {
    params(kind_index).len()
}

/// Default values of a kind's parameters, in index order.
pub(crate) fn defaults<const N: usize>(kind_index: usize) -> [f32; N] {
    let mut out = [0.0; N];
    for (o, p) in out.iter_mut().zip(params(kind_index)) {
        *o = p.default;
    }
    out
}

/// Clamps `value` into the spec range of parameter `index` of `kind_index`; `None` for an
/// unknown index or a NaN value.
pub(crate) fn clamp_param(kind_index: usize, index: usize, value: f32) -> Option<f32> {
    let p = params(kind_index).get(index)?;
    if value.is_nan() {
        return None;
    }
    Some(value.clamp(p.min, p.max))
}

/// Every effect behind one enum: implements [`Processor`] and forwards the two calls that are
/// not part of the trait (`set_tempo`, `set_impulse`). Allocation-free dispatch.
pub enum AnyFx {
    Eq(Box<Eq4>),
    Filter(Box<FilterFx>),
    Compressor(Box<Compressor>),
    Saturation(Box<Saturation>),
    Delay(Box<Delay>),
    Reverb(Box<FdnReverb>),
    Convolution(Box<Convolver>),
    Chorus(Box<Chorus>),
    Width(Box<Width>),
    Limiter(Box<TruePeakLimiter>),
    Gain(Box<GainFx>),
}

macro_rules! each {
    ($self:ident, $p:ident => $e:expr) => {
        match $self {
            AnyFx::Eq($p) => $e,
            AnyFx::Filter($p) => $e,
            AnyFx::Compressor($p) => $e,
            AnyFx::Saturation($p) => $e,
            AnyFx::Delay($p) => $e,
            AnyFx::Reverb($p) => $e,
            AnyFx::Convolution($p) => $e,
            AnyFx::Chorus($p) => $e,
            AnyFx::Width($p) => $e,
            AnyFx::Limiter($p) => $e,
            AnyFx::Gain($p) => $e,
        }
    };
}

impl AnyFx {
    /// The kind index of this effect.
    pub fn kind_index(&self) -> usize {
        match self {
            AnyFx::Eq(_) => 0,
            AnyFx::Filter(_) => 1,
            AnyFx::Compressor(_) => 2,
            AnyFx::Saturation(_) => 3,
            AnyFx::Delay(_) => 4,
            AnyFx::Reverb(_) => 5,
            AnyFx::Convolution(_) => 6,
            AnyFx::Chorus(_) => 7,
            AnyFx::Width(_) => 8,
            AnyFx::Limiter(_) => 9,
            AnyFx::Gain(_) => 10,
        }
    }

    /// Tempo for tempo-synced effects (delay); ignored by others.
    pub fn set_tempo(&mut self, bpm: f32) {
        if let AnyFx::Delay(d) = self {
            d.set_tempo(bpm);
        }
    }

    /// Impulse response for the convolution reverb; ignored by others. Allocates: call off the
    /// audio thread.
    pub fn set_impulse(&mut self, ir: &StereoBuffer) {
        if let AnyFx::Convolution(c) = self {
            c.set_impulse(ir);
        }
    }
}

impl Processor for AnyFx {
    fn set_tempo(&mut self, bpm: f32) {
        AnyFx::set_tempo(self, bpm);
    }

    fn set_impulse(&mut self, ir: &StereoBuffer) {
        AnyFx::set_impulse(self, ir);
    }

    fn prepare(&mut self, sample_rate: f32, max_block: usize) {
        each!(self, p => p.prepare(sample_rate, max_block))
    }
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        each!(self, p => p.process(left, right))
    }
    fn set_param(&mut self, index: usize, value: f32) {
        each!(self, p => p.set_param(index, value))
    }
    fn reset(&mut self) {
        each!(self, p => p.reset())
    }
    fn tail_samples(&self) -> usize {
        each!(self, p => p.tail_samples())
    }
    fn latency_samples(&self) -> usize {
        each!(self, p => p.latency_samples())
    }
    fn set_sidechain(&mut self, left: &[f32], right: &[f32]) {
        each!(self, p => p.set_sidechain(left, right))
    }
}

/// Builds the effect of kind `kind_index` (eq = 0 .. gain = 10) as an [`AnyFx`].
pub fn create_any(kind_index: usize, sample_rate: f32, max_block: usize) -> Option<AnyFx> {
    let sr = sample_rate;
    let mb = max_block;
    Some(match kind_index {
        0 => AnyFx::Eq(Box::new(Eq4::new(sr, mb))),
        1 => AnyFx::Filter(Box::new(FilterFx::new(sr, mb))),
        2 => AnyFx::Compressor(Box::new(Compressor::new(sr, mb))),
        3 => AnyFx::Saturation(Box::new(Saturation::new(sr, mb))),
        4 => AnyFx::Delay(Box::new(Delay::new(sr, mb))),
        5 => AnyFx::Reverb(Box::new(FdnReverb::new(sr, mb))),
        6 => AnyFx::Convolution(Box::new(Convolver::new(sr, mb))),
        7 => AnyFx::Chorus(Box::new(Chorus::new(sr, mb))),
        8 => AnyFx::Width(Box::new(Width::new(sr, mb))),
        9 => AnyFx::Limiter(Box::new(TruePeakLimiter::new(sr, mb))),
        10 => AnyFx::Gain(Box::new(GainFx::new(sr, mb))),
        _ => return None,
    })
}

/// Builds the effect of kind `kind_index` (eq = 0 .. gain = 10). `None` for an unknown index.
pub fn create(kind_index: usize, sample_rate: f32, max_block: usize) -> Option<Box<dyn Processor>> {
    let sr = sample_rate;
    let mb = max_block;
    Some(match kind_index {
        0 => Box::new(Eq4::new(sr, mb)),
        1 => Box::new(FilterFx::new(sr, mb)),
        2 => Box::new(Compressor::new(sr, mb)),
        3 => Box::new(Saturation::new(sr, mb)),
        4 => Box::new(Delay::new(sr, mb)),
        5 => Box::new(FdnReverb::new(sr, mb)),
        6 => Box::new(Convolver::new(sr, mb)),
        7 => Box::new(Chorus::new(sr, mb)),
        8 => Box::new(Width::new(sr, mb)),
        9 => Box::new(TruePeakLimiter::new(sr, mb)),
        10 => Box::new(GainFx::new(sr, mb)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_covers_every_kind() {
        for k in 0..KIND_COUNT {
            assert!(create(k, 48000.0, 256).is_some());
            assert_eq!(create_any(k, 48000.0, 256).map(|a| a.kind_index()), Some(k));
            assert!(param_count(k) > 0);
            for p in params(k) {
                assert!(p.min <= p.default && p.default <= p.max, "{}", p.name);
            }
        }
        assert!(create(KIND_COUNT, 48000.0, 256).is_none());
        assert_eq!(param_count(KIND_COUNT), 0);
    }
}
