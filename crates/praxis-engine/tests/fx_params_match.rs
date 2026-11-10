//! Cross-crate contract check: praxis-dsp's effect parameter mirror must equal the score's table
//! (names, order, ranges, defaults, units) for every effect kind, and every kind must build.

use praxis_dsp::fx;
use praxis_score::EffectKind;
use praxis_score::params::effect_params;

const KINDS: [EffectKind; 11] = [
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

#[test]
fn dsp_param_table_matches_score_table() {
    for (i, kind) in KINDS.iter().enumerate() {
        let score = effect_params(*kind);
        let dsp = fx::PARAMS[i];
        assert_eq!(score.len(), dsp.len(), "{kind:?}: parameter count");
        assert_eq!(fx::param_count(i), score.len(), "{kind:?}: param_count");
        for (s, d) in score.iter().zip(dsp.iter()) {
            assert_eq!(s.name, d.name, "{kind:?}: parameter order");
            assert_eq!(s.unit, d.unit, "{kind:?}.{}: unit", s.name);
            for (what, a, b) in [
                ("min", s.min, d.min),
                ("max", s.max, d.max),
                ("default", s.default, d.default),
            ] {
                assert!(
                    (a - b as f64).abs() <= 1e-6 * a.abs().max(1.0),
                    "{kind:?}.{}: {what} {a} vs {b}",
                    s.name
                );
            }
        }
        assert!(fx::create(i, 48_000.0, 128).is_some(), "{kind:?}: create");
    }
    assert!(fx::create(KINDS.len(), 48_000.0, 128).is_none());
}
