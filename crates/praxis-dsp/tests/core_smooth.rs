//! Parameter smoothing reaches its target in exactly the specified time, without steps.

use praxis_dsp::smooth::{SmoothMode, Smoothed};

const SR: f32 = 48_000.0;

#[test]
fn linear_reaches_target_in_the_specified_time() {
    for time in [0.001f32, 0.01, 0.0333, 0.25] {
        let steps = (time * SR).round() as usize;
        let mut s = Smoothed::linear(SR, time, 0.0);
        s.set_target(1.0);
        let out: Vec<f32> = (0..steps).map(|_| s.next()).collect();
        assert!(s.is_settled());
        assert_eq!(out[steps - 1], 1.0, "{time}");
        if steps > 1 {
            assert!(out[steps - 2] < 1.0);
        }
        // A straight ramp: equal increments.
        let inc = 1.0 / steps as f32;
        for (i, v) in out.iter().enumerate() {
            assert!((v - inc * (i + 1) as f32).abs() < 1e-5, "{time} at {i}");
        }
        assert_eq!(s.next(), 1.0);
    }
}

#[test]
fn exponential_reaches_target_in_the_specified_time() {
    let time = 0.02;
    let steps = (time * SR).round() as usize;
    let mut s = Smoothed::exponential(SR, time, 1.0);
    s.set_target(0.0);
    let out: Vec<f32> = (0..steps).map(|_| s.next()).collect();
    assert!(s.is_settled());
    assert_eq!(out[steps - 1], 0.0);
    // Within -80 dB of the change one sample earlier, so the final snap is inaudible.
    assert!(out[steps - 2].abs() < 1.1e-4, "{}", out[steps - 2]);
    // Monotonic and fastest at the start (exponential, not linear).
    assert!(out.windows(2).all(|w| w[1] <= w[0]));
    assert!(out[steps / 4] < 0.2);
}

#[test]
fn retargeting_mid_glide_is_continuous() {
    for mode in [SmoothMode::Linear, SmoothMode::Exponential] {
        let mut s = Smoothed::new(SR, 0.01, mode, 0.0);
        s.set_target(1.0);
        let mut out = Vec::new();
        for n in 0..2000 {
            if n == 200 {
                s.set_target(-0.5);
            }
            if n == 300 {
                s.set_target(0.8);
            }
            out.push(s.next());
        }
        let max_jump = out
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        println!("{mode:?}: max step {max_jump:.5}");
        // At most the steepest legitimate step: the whole change over the ramp (linear) or
        // its first exponential step.
        let bound = match mode {
            SmoothMode::Linear => 1.5 / 480.0,
            SmoothMode::Exponential => 1.5 * (1.0 - 1e-4f32.powf(1.0 / 480.0)),
        };
        assert!(max_jump <= bound * 1.01, "{mode:?} {max_jump}");
        assert_eq!(*out.last().unwrap(), 0.8);
    }
}

#[test]
fn snap_and_same_target() {
    let mut s = Smoothed::linear(SR, 0.05, 0.3);
    assert!(s.is_settled());
    assert_eq!(s.next(), 0.3);
    s.set_target(0.3);
    assert!(s.is_settled());
    s.set_target(0.9);
    assert!(!s.is_settled());
    s.snap(0.1);
    assert!(s.is_settled());
    assert_eq!(s.value(), 0.1);
    assert_eq!(s.target(), 0.1);
    assert_eq!(s.next(), 0.1);
    s.set_target(f32::NAN);
    assert_eq!(s.next(), 0.1);
}
