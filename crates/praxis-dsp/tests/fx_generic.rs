//! Properties every effect must have: reset to silence, determinism, silence after the
//! reported tail, robustness against NaN/inf and extreme parameters.

mod fx_support;
use fx_support::*;

use praxis_dsp::fx::{AnyFx, KIND_COUNT, KIND_NAMES, create_any, params};
use praxis_dsp::rng::Pcg32;
use praxis_dsp::{Processor, StereoBuffer};

fn test_ir() -> StereoBuffer {
    let n = (SR * 0.4) as usize;
    let mut rng = Pcg32::new(77);
    let k = 3.0 * std::f32::consts::LN_10 / (0.3 * SR);
    StereoBuffer {
        left: (0..n)
            .map(|i| rng.next_bipolar() * (-k * i as f32).exp() * 0.3)
            .collect(),
        right: (0..n)
            .map(|i| rng.next_bipolar() * (-k * i as f32).exp() * 0.3)
            .collect(),
    }
}

/// An effect of every kind with settings that make it do something audible.
fn make(kind: usize) -> AnyFx {
    let mut fx = create_any(kind, SR, BLOCK).unwrap();
    fx.set_tempo(128.0);
    fx.set_impulse(&test_ir());
    match kind {
        0 => {
            fx.set_param(0, 80.0);
            fx.set_param(5, 6.0);
            fx.set_param(11, -3.0);
        }
        1 => {
            fx.set_param(1, 900.0);
            fx.set_param(2, 0.8);
        }
        2 => {
            fx.set_param(0, -30.0);
            fx.set_param(1, 6.0);
        }
        3 => fx.set_param(0, 18.0),
        4 => {
            fx.set_param(2, 0.6);
            fx.set_param(5, 0.5);
        }
        5 => {
            fx.set_param(0, 1.0);
            fx.set_param(5, 0.5);
        }
        6 => fx.set_param(3, 0.5),
        7 => fx.set_param(3, 0.5),
        8 => {
            fx.set_param(0, 1.6);
            fx.set_param(1, 150.0);
        }
        9 => fx.set_param(0, -6.0),
        10 => fx.set_param(0, -4.0),
        _ => unreachable!(),
    }
    fx.reset();
    fx
}

fn render(fx: &mut AnyFx, l: &[f32], r: &[f32], automate: bool) -> (Vec<f32>, Vec<f32>) {
    let mut l = l.to_vec();
    let mut r = r.to_vec();
    let n = l.len();
    let mut i = 0;
    let mut b = 0;
    while i < n {
        let e = (i + BLOCK).min(n);
        if automate && b == 40 {
            // move every parameter to its midpoint mid-render
            for (k, p) in params(fx.kind_index()).iter().enumerate() {
                if p.unit != "enum" && p.name != "lookahead_ms" {
                    fx.set_param(k, 0.5 * (p.min + p.max));
                }
            }
        }
        fx.process(&mut l[i..e], &mut r[i..e]);
        i = e;
        b += 1;
    }
    (l, r)
}

#[test]
fn every_effect_is_deterministic() {
    let (ml, mr) = music(3.0, SR);
    for kind in 0..KIND_COUNT {
        let a = render(&mut make(kind), &ml, &mr, true);
        let b = render(&mut make(kind), &ml, &mr, true);
        assert!(
            a.0.iter()
                .zip(b.0.iter())
                .all(|(x, y)| x.to_bits() == y.to_bits())
                && a.1
                    .iter()
                    .zip(b.1.iter())
                    .all(|(x, y)| x.to_bits() == y.to_bits()),
            "{} is not bit-identical across runs",
            KIND_NAMES[kind]
        );
    }
}

#[test]
fn reset_returns_every_effect_to_silence() {
    let (ml, mr) = music(2.0, SR);
    let z = vec![0.0f32; (SR * 2.0) as usize];
    for kind in 0..KIND_COUNT {
        let mut fx = make(kind);
        let first = render(&mut fx, &ml, &mr, false);
        fx.reset();
        let (sl, sr) = render(&mut fx, &z, &z, false);
        assert!(
            sl.iter().chain(sr.iter()).all(|v| *v == 0.0),
            "{}: output after reset with silent input",
            KIND_NAMES[kind]
        );
        fx.reset();
        let again = render(&mut fx, &ml, &mr, false);
        assert!(
            first.0 == again.0 && first.1 == again.1,
            "{}: reset does not restore the initial state",
            KIND_NAMES[kind]
        );
    }
}

#[test]
fn no_output_for_silent_input_after_the_tail() {
    let (ml, mr) = music(1.5, SR);
    for kind in 0..KIND_COUNT {
        let mut fx = make(kind);
        let tail = fx.tail_samples();
        let lat = fx.latency_samples();
        let quiet = lat + tail;
        let extra = 9600;
        let mut l = ml.clone();
        let mut r = mr.clone();
        l.extend(std::iter::repeat_n(0.0, quiet + extra));
        r.extend(std::iter::repeat_n(0.0, quiet + extra));
        let in_peak = peak(&ml).max(peak(&mr));
        let (ol, or) = render(&mut fx, &l, &r, false);
        let from = ml.len() + quiet;
        let after = peak(&ol[from..]).max(peak(&or[from..])) / in_peak;
        // reverb tails are reported as RT60 (-60 dB); everything else decays to -90 dB
        let limit = match kind {
            5 | 6 => 1e-3,
            _ => 10f32.powf(-90.0 / 20.0),
        };
        println!(
            "{:>18}: tail {tail:>7} latency {lat:>3}, peak after tail {:.1} dB re input",
            KIND_NAMES[kind],
            20.0 * after.max(1e-12).log10()
        );
        assert!(
            after <= limit,
            "{}: output {after} after tail {tail} + latency {lat}",
            KIND_NAMES[kind]
        );
    }
}

#[test]
fn nan_and_inf_never_escape() {
    let (mut l, mut r) = music(1.0, SR);
    for i in (1000..l.len()).step_by(3331) {
        l[i] = f32::NAN;
        r[i] = f32::INFINITY;
        if i + 1 < l.len() {
            l[i + 1] = f32::NEG_INFINITY;
        }
    }
    let (ml, mr) = music(1.0, SR);
    for kind in 0..KIND_COUNT {
        let mut fx = make(kind);
        let (ol, or) = render(&mut fx, &l, &r, false);
        assert!(
            ol.iter().chain(or.iter()).all(|v| v.is_finite()),
            "{}: non-finite output",
            KIND_NAMES[kind]
        );
        // and the state is not poisoned: clean audio afterwards still produces sound
        let (cl, _) = render(&mut fx, &ml, &mr, false);
        assert!(peak(&cl) > 1e-3, "{} went silent", KIND_NAMES[kind]);
    }
}

#[test]
fn extreme_and_random_parameters_stay_finite() {
    let (ml, mr) = music(2.0, SR);
    for kind in 0..KIND_COUNT {
        for setting in 0..3 {
            let mut fx = make(kind);
            let ps = params(kind);
            let mut rng = Pcg32::new(kind as u64 * 10 + setting);
            let mut l = ml.clone();
            let mut r = mr.clone();
            let n = l.len();
            let mut i = 0;
            while i < n {
                let e = (i + BLOCK).min(n);
                for (k, p) in ps.iter().enumerate() {
                    let v = match setting {
                        0 => p.min,
                        1 => p.max,
                        _ => rng.range(p.min, p.max),
                    };
                    if p.name != "lookahead_ms" || i == 0 {
                        fx.set_param(k, v);
                    }
                }
                fx.process(&mut l[i..e], &mut r[i..e]);
                i = e;
            }
            let pk = peak(&l).max(peak(&r));
            assert!(
                l.iter().chain(r.iter()).all(|v| v.is_finite()) && pk < 1000.0,
                "{} setting {setting}: peak {pk}",
                KIND_NAMES[kind]
            );
        }
    }
}

#[test]
fn out_of_range_indices_and_values_are_ignored_or_clamped() {
    for kind in 0..KIND_COUNT {
        let mut fx = make(kind);
        fx.set_param(99, 1.0);
        fx.set_param(0, f32::NAN);
        fx.set_param(0, 1e9);
        let (l, r) = render(&mut fx, &music(0.5, SR).0, &music(0.5, SR).1, false);
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()));
    }
}

#[test]
fn parameter_jumps_do_not_click() {
    // A 100 Hz sine at 0.4 moves at most 0.0053 per sample; with up to ~4x gain after the jump
    // a smooth output stays under 0.03 per sample. A click is a step far above that.
    let n = (SR * 0.6) as usize;
    let x = sine(100.0, 0.4, n, SR);
    let jumps: &[(usize, &[(usize, f32)])] = &[
        (0, &[(3, 12.0), (5, -12.0), (0, 60.0), (1, 2000.0)]),
        (1, &[(0, 1.0), (1, 200.0), (3, 0.3)]),
        (1, &[(0, 3.0)]),
        (2, &[(0, -50.0), (5, 12.0), (6, 0.2)]),
        (3, &[(0, 30.0), (1, 1.0), (3, -6.0)]),
        (8, &[(0, 0.0), (1, 300.0)]),
        (10, &[(0, 12.0)]),
        (9, &[(0, -12.0)]),
        (5, &[(5, 1.0), (4, 0.0)]),
        (6, &[(3, 1.0), (2, 800.0)]),
        (7, &[(3, 1.0), (1, 0.0)]),
    ];
    for (kind, set) in jumps {
        let mut fx = make(*kind);
        let mut l = x.clone();
        let mut r: Vec<f32> = x.iter().map(|v| v * 0.7).collect();
        let mut i = 0;
        let mut b = 0;
        while i < n {
            let e = (i + BLOCK).min(n);
            if b == 50 {
                for &(k, v) in set.iter() {
                    fx.set_param(k, v);
                }
            }
            fx.process(&mut l[i..e], &mut r[i..e]);
            i = e;
            b += 1;
        }
        let lat = fx.latency_samples();
        let jump_at = 50 * BLOCK + lat;
        let step = |a: usize, b: usize| {
            [&l, &r]
                .iter()
                .flat_map(|v| v[a..b].windows(2).map(|w| (w[1] - w[0]).abs()))
                .fold(0.0f32, f32::max)
        };
        // steady states before and well after the change bound what a smooth move can do
        let before = step(jump_at - 4000, jump_at - 100);
        let after = step(n - 6000, n - 1);
        let during = step(jump_at - 100, jump_at + 4000);
        let bound = 0.03f32.max(1.25 * before.max(after));
        assert!(
            during <= bound,
            "{} {set:?}: max step {during} (steady {before} / {after})",
            KIND_NAMES[*kind]
        );
    }
}
