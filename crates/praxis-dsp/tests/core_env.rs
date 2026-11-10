//! Envelope timing (within one sample at 48 kHz), curve shape and click-free retriggering.

use praxis_dsp::env::{Adsr, Ar, Dahdsr, Stage};

const SR: f32 = 48_000.0;

fn samples(s: f32) -> usize {
    (s * SR).round() as usize
}

/// Calls `next()` until `pred` holds; returns how many calls it took (1-based).
fn calls_until(mut next: impl FnMut() -> (f32, Stage), pred: impl Fn(f32, Stage) -> bool) -> usize {
    for n in 1..10_000_000 {
        let (v, s) = next();
        if pred(v, s) {
            return n;
        }
    }
    panic!("condition never reached");
}

fn assert_within_one(got: usize, want: usize, what: &str) {
    println!("{what}: {got} samples (expected {want})");
    assert!(
        got.abs_diff(want) <= 1,
        "{what}: {got} samples, expected {want}"
    );
}

#[test]
fn adsr_stage_timing_within_one_sample() {
    for (a, d, s, r) in [
        (0.010f32, 0.020f32, 0.5f32, 0.030f32),
        (0.0013, 0.25, 0.8, 1.7),
        (0.5, 0.001, 0.2, 0.0021),
    ] {
        let mut e = Adsr::new(SR);
        e.set_params(a, d, s, r);
        e.gate(true);
        let peak = calls_until(
            || {
                let v = e.next();
                (v, e.stage())
            },
            |v, _| v == 1.0,
        );
        assert_within_one(peak, samples(a), "attack to peak");
        let sustain = calls_until(
            || {
                let v = e.next();
                (v, e.stage())
            },
            |_, st| st == Stage::Sustain,
        );
        assert_within_one(sustain, samples(d), "decay to sustain");
        assert!((e.level() - s).abs() < 1e-6);
        for _ in 0..1000 {
            assert!((e.next() - s).abs() < 1e-6);
        }
        e.gate(false);
        let release = calls_until(
            || {
                let v = e.next();
                (v, e.stage())
            },
            |_, st| st == Stage::Idle,
        );
        assert_within_one(release, samples(r), "release to silence");
        assert_eq!(e.level(), 0.0);
    }
}

#[test]
fn attack_is_linear_and_decay_release_are_exponential() {
    let mut e = Adsr::new(SR);
    e.set_params(0.01, 0.1, 0.25, 0.2);
    e.gate(true);
    let att: Vec<f32> = (0..samples(0.01)).map(|_| e.next()).collect();
    // Linear: constant increments.
    let step = att[1] - att[0];
    for w in att.windows(2) {
        assert!(((w[1] - w[0]) - step).abs() < 1e-6);
    }
    let dec: Vec<f32> = (0..samples(0.1)).map(|_| e.next()).collect();
    // Curve 1: the distance to the sustain level shrinks by a constant factor, i.e. the decay
    // is a straight line in dB down to -80 dB of the step.
    let d = |i: usize| (dec[i] - 0.25) as f64;
    let r1 = d(1000) / d(500);
    let r2 = d(1500) / d(1000);
    assert!((r1 - r2).abs() < 1e-3, "{r1} vs {r2}");
    // Halfway through the decay it is far below the straight-line midpoint.
    assert!(dec[2400] < 0.3);
    // Curve 0 is close to a straight line.
    let mut lin = Adsr::new(SR);
    lin.set_params(0.0, 0.1, 0.0, 0.1);
    lin.set_curve(0.0);
    lin.gate(true);
    let v: Vec<f32> = (0..samples(0.1)).map(|_| lin.next()).collect();
    assert!((v[2399] - 0.5).abs() < 0.01, "{}", v[2399]);
}

#[test]
fn dahdsr_stage_boundaries_within_one_sample() {
    let (delay, attack, hold, decay, sustain, release) = (0.005, 0.010, 0.007, 0.020, 0.3, 0.030);
    let mut e = Dahdsr::new(SR);
    e.set_delay(delay);
    e.set_attack(attack);
    e.set_hold(hold);
    e.set_decay(decay);
    e.set_sustain(sustain);
    e.set_release(release);
    e.set_start(0.1);
    e.gate(true);
    assert_eq!(e.stage(), Stage::Delay);
    let mut transitions = Vec::new();
    let mut last = e.stage();
    let mut first = None;
    for n in 1..=samples(0.1) {
        let v = e.next();
        if first.is_none() {
            first = Some(v);
        }
        if e.stage() != last {
            transitions.push((n, e.stage()));
            last = e.stage();
        }
    }
    assert!(
        (first.unwrap() - 0.1).abs() < 1e-6,
        "start level held during delay"
    );
    let want = [
        (samples(delay), Stage::Attack),
        (samples(delay) + samples(attack * 0.9), Stage::Hold),
        (
            samples(delay) + samples(attack * 0.9) + samples(hold),
            Stage::Decay,
        ),
        (
            samples(delay) + samples(attack * 0.9) + samples(hold) + samples(decay),
            Stage::Sustain,
        ),
    ];
    assert_eq!(transitions.len(), want.len(), "{transitions:?}");
    for ((n, st), (wn, wst)) in transitions.iter().zip(want.iter()) {
        assert_eq!(st, wst);
        // The attack starts from 0.1, so it takes 90 % of the attack time.
        assert_within_one(*n, *wn, &format!("{st:?} starts"));
    }
    e.gate(false);
    let n = calls_until(
        || {
            let v = e.next();
            (v, e.stage())
        },
        |_, st| st == Stage::Idle,
    );
    assert_within_one(n, samples(release), "release");
}

#[test]
fn ar_is_one_shot() {
    let mut e = Ar::new(SR);
    e.set_attack(0.002);
    e.set_release(0.1);
    e.trigger();
    e.gate(false); // ignored
    let peak = calls_until(
        || {
            let v = e.next();
            (v, e.stage())
        },
        |v, _| v == 1.0,
    );
    assert_within_one(peak, samples(0.002), "AR attack");
    let end = calls_until(
        || {
            let v = e.next();
            (v, e.stage())
        },
        |_, st| st == Stage::Idle,
    );
    assert_within_one(end, samples(0.1), "AR release");
}

/// Largest sample-to-sample change over a run.
fn max_jump(v: &[f32]) -> f32 {
    v.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

#[test]
fn retrigger_never_jumps() {
    let (a, d, s, r) = (0.01, 0.02, 0.5, 0.03);
    // The steepest legitimate slopes: the linear attack, and the first sample of an
    // exponential decay or release (which falls fastest at its start).
    let attack_slope = 1.0 / samples(a) as f32;
    let decay_slope = (1.0 - s) * (1.0 - (1e-4f32).powf(1.0 / samples(d) as f32));
    let release_slope = 1.0 * (1.0 - (1e-4f32).powf(1.0 / samples(r) as f32));
    let bound = attack_slope.max(decay_slope).max(release_slope) * 1.01;

    // Retrigger in every stage: mid-attack, mid-decay, sustain, mid-release, end of release.
    for retrigger_at in [200usize, 700, 2000, 2600, 3500] {
        let mut e = Adsr::new(SR);
        e.set_params(a, d, s, r);
        e.gate(true);
        let mut out = Vec::new();
        for n in 0..6000 {
            if n == 2400 {
                e.gate(false);
            }
            if n == retrigger_at {
                e.gate(true);
            }
            out.push(e.next());
        }
        let j = max_jump(&out);
        println!("retrigger at {retrigger_at}: max jump {j:.5} (bound {bound:.5})");
        assert!(j <= bound, "retrigger at {retrigger_at}: jump {j}");
    }

    // A restart from zero would jump by the whole level; ours continues from it.
    let mut e = Adsr::new(SR);
    e.set_params(a, d, s, r);
    e.gate(true);
    for _ in 0..3000 {
        e.next();
    }
    e.gate(false);
    for _ in 0..100 {
        e.next();
    }
    let before = e.level();
    e.gate(true);
    let after = e.next();
    assert!(before > 0.1);
    assert!(after >= before && after - before <= attack_slope * 1.01);

    // Sustain changes glide instead of stepping.
    let mut e = Adsr::new(SR);
    e.set_params(0.001, 0.001, 0.8, 0.1);
    e.gate(true);
    for _ in 0..1000 {
        e.next();
    }
    e.set_sustain(0.2);
    let out: Vec<f32> = (0..2000).map(|_| e.next()).collect();
    assert!(max_jump(&out) < 0.01);
    assert!((out[1999] - 0.2).abs() < 1e-4);
}

#[test]
fn sustain_lowered_to_zero_glides_out_then_frees_the_voice() {
    let mut e = Adsr::new(SR);
    e.set_params(0.001, 0.001, 0.8, 0.1);
    e.gate(true);
    for _ in 0..1000 {
        e.next();
    }
    e.set_sustain(0.0);
    let out: Vec<f32> = (0..4000).map(|_| e.next()).collect();
    assert!(max_jump(&out) < 0.01, "{}", max_jump(&out));
    assert!(e.is_idle());
}

#[test]
fn release_from_any_level_takes_the_release_time() {
    let mut e = Adsr::new(SR);
    e.set_params(0.1, 0.1, 0.5, 0.05);
    e.gate(true);
    for _ in 0..1000 {
        e.next(); // partway up the attack
    }
    assert_eq!(e.stage(), Stage::Attack);
    e.gate(false);
    let n = calls_until(
        || {
            let v = e.next();
            (v, e.stage())
        },
        |_, st| st == Stage::Idle,
    );
    assert_within_one(n, samples(0.05), "release from mid-attack");
}

#[test]
fn envelopes_are_deterministic_and_finite() {
    let run = || {
        let mut e = Dahdsr::new(SR);
        e.set_delay(0.001);
        e.set_attack(0.003);
        e.set_hold(0.002);
        e.set_decay(0.05);
        e.set_sustain(0.4);
        e.set_release(0.07);
        let mut out = Vec::new();
        for n in 0..20_000 {
            if n % 5000 == 0 {
                e.gate(true);
            }
            if n % 5000 == 3000 {
                e.gate(false);
            }
            let v = e.next();
            assert!(v.is_finite() && (0.0..=1.0).contains(&v));
            out.push(v.to_bits());
        }
        out
    };
    assert_eq!(run(), run());
}
