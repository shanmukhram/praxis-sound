//! EQ and filter effect responses.

mod fx_support;
use fx_support::*;

use praxis_dsp::Processor;
use praxis_dsp::biquad::{BiquadShape, Coeffs};
use praxis_dsp::fx::{Eq4, FilterFx, Width};

/// Steady-state gain (dB) of a processor for a sine at `hz`, left channel.
fn measure_gain(p: &mut dyn Processor, hz: f32) -> f64 {
    p.reset();
    let n = (SR * 0.6) as usize;
    let x = sine(hz, 0.25, n, SR);
    let mut l = x.clone();
    let mut r = x.clone();
    run(p, &mut l, &mut r, BLOCK);
    let tail = n / 2;
    db(rms(&l[tail..]) / rms(&x[tail..]))
}

fn eq_with(settings: &[(usize, f32)]) -> (Eq4, [f32; 12]) {
    let mut eq = Eq4::new(SR, BLOCK);
    let mut p: [f32; 12] = std::array::from_fn(|i| praxis_dsp::fx::params(0)[i].default);
    for &(i, v) in settings {
        eq.set_param(i, v);
        p[i] = v;
    }
    eq.reset(); // settle ramps
    (eq, p)
}

#[test]
fn eq_magnitude_matches_biquad_designs() {
    let cases: Vec<Vec<(usize, f32)>> = vec![
        vec![],
        vec![(0, 200.0)],
        vec![(1, 4000.0)],
        vec![(2, 150.0), (3, 6.0)],
        vec![(4, 800.0), (5, -9.0), (6, 2.0)],
        vec![(7, 5000.0), (8, 12.0), (9, 0.7)],
        vec![(10, 6000.0), (11, -6.0)],
        vec![
            (0, 60.0),
            (1, 15000.0),
            (2, 200.0),
            (3, -4.0),
            (4, 400.0),
            (5, 3.0),
            (6, 1.4),
            (7, 2500.0),
            (8, -5.0),
            (9, 3.0),
            (10, 9000.0),
            (11, 4.0),
        ],
    ];
    let freqs = [
        40.0f32, 100.0, 250.0, 700.0, 1500.0, 3000.0, 6000.0, 12000.0,
    ];
    let mut worst = 0.0f64;
    for case in &cases {
        let (mut eq, p) = eq_with(case);
        for &f in &freqs {
            let want = db(Eq4::magnitude(&p, f as f64, SR));
            let got = measure_gain(&mut eq, f);
            let err = (got - want).abs();
            worst = worst.max(err);
            assert!(
                err < 0.1,
                "case {case:?} at {f} Hz: got {got:.3} dB want {want:.3} dB"
            );
        }
    }
    println!("eq: worst deviation from design {worst:.4} dB");
}

#[test]
fn eq_stage_designs_are_the_rbj_shapes() {
    let mut p: [f32; 12] = std::array::from_fn(|i| praxis_dsp::fx::params(0)[i].default);
    p[0] = 100.0;
    p[5] = 6.0;
    let c = Eq4::stage_coeffs(&p, SR);
    let q = std::f64::consts::FRAC_1_SQRT_2;
    assert_eq!(
        c[0],
        Coeffs::design(BiquadShape::HighPass, 100.0, q, SR as f64)
    );
    assert_eq!(c[1], Coeffs::IDENTITY); // lp at 22 kHz is off
    assert_eq!(
        c[3],
        Coeffs::design(BiquadShape::Peak { gain_db: 6.0 }, 500.0, 1.0, SR as f64)
    );
    // hp at 10 Hz is off
    p[0] = 10.0;
    assert_eq!(Eq4::stage_coeffs(&p, SR)[0], Coeffs::IDENTITY);
}

#[test]
fn eq_parameter_moves_do_not_click() {
    let mut eq = Eq4::new(SR, BLOCK);
    let n = (SR * 0.5) as usize;
    let x = sine(200.0, 0.5, n, SR);
    let mut l = x.clone();
    let mut r = x.clone();
    let mut i = 0;
    let mut k = 0;
    while i < n {
        let e = (i + BLOCK).min(n);
        if k == 20 {
            eq.set_param(4, 200.0);
            eq.set_param(5, 18.0);
            eq.set_param(0, 150.0);
        }
        eq.process(&mut l[i..e], &mut r[i..e]);
        i = e;
        k += 1;
    }
    // A 200 Hz sine at <= 8x gain changes by at most 2*pi*200/48000*4 per sample; a click
    // would be a jump far above that.
    let max_step = l
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    let bound = 2.0 * std::f32::consts::PI * 200.0 / SR * 4.5;
    assert!(max_step < bound, "max step {max_step} bound {bound}");
}

#[test]
fn svf_modes_have_expected_responses() {
    let mut f = FilterFx::new(SR, BLOCK);
    f.set_param(1, 1000.0);
    f.set_param(2, 0.0); // Q 0.5
    // low-pass
    f.set_param(0, 0.0);
    f.reset();
    assert!(measure_gain(&mut f, 100.0).abs() < 0.2);
    assert!(measure_gain(&mut f, 10000.0) < -30.0);
    // high-pass
    f.set_param(0, 1.0);
    f.reset();
    assert!(measure_gain(&mut f, 10000.0).abs() < 0.2);
    assert!(measure_gain(&mut f, 100.0) < -30.0);
    // band-pass: unity at the centre
    f.set_param(0, 2.0);
    f.reset();
    assert!(measure_gain(&mut f, 1000.0).abs() < 0.2);
    assert!(measure_gain(&mut f, 100.0) < -12.0);
    // notch: deep at the centre
    f.set_param(0, 3.0);
    f.reset();
    assert!(measure_gain(&mut f, 1000.0) < -30.0);
    assert!(measure_gain(&mut f, 100.0).abs() < 0.5);
    // TPT low-pass at Q 0.5 is -6 dB at the cutoff
    f.set_param(0, 0.0);
    f.reset();
    assert!((measure_gain(&mut f, 1000.0) + 6.02).abs() < 0.2);
    // mix 0 is dry
    f.set_param(3, 0.0);
    f.reset();
    assert!(measure_gain(&mut f, 10000.0).abs() < 0.01);
}

#[test]
fn width_mono_and_bass_mono() {
    let n = (SR * 0.5) as usize;
    // side-only signal: L = s, R = -s
    let s = sine(1000.0, 0.3, n, SR);
    let mut w = Width::new(SR, BLOCK);
    w.set_param(0, 0.0);
    w.reset();
    let mut l = s.clone();
    let mut r: Vec<f32> = s.iter().map(|v| -v).collect();
    run(&mut w, &mut l, &mut r, BLOCK);
    assert!(peak(&l[n / 2..]) < 1e-6, "width 0 must be mono");
    // width 2 doubles the side
    let mut w = Width::new(SR, BLOCK);
    w.set_param(0, 2.0);
    w.reset();
    let mut l = s.clone();
    let mut r: Vec<f32> = s.iter().map(|v| -v).collect();
    run(&mut w, &mut l, &mut r, BLOCK);
    assert!((db(rms(&l[n / 2..]) / rms(&s[n / 2..])) - 6.02).abs() < 0.05);
    // bass mono at 200 Hz removes 40 Hz side content, keeps 2 kHz side content
    for (hz, want_db, tol) in [(40.0, -40.0, 40.0), (2000.0, 0.0, 0.1)] {
        let s = sine(hz, 0.3, n, SR);
        let mut w = Width::new(SR, BLOCK);
        w.set_param(1, 200.0);
        w.reset();
        let mut l = s.clone();
        let mut r: Vec<f32> = s.iter().map(|v| -v).collect();
        run(&mut w, &mut l, &mut r, BLOCK);
        let g = db(rms(&l[n / 2..]) / rms(&s[n / 2..]));
        if want_db < 0.0 {
            assert!(g < want_db + 0.0 && g > want_db - tol, "{hz} Hz: {g}");
        } else {
            assert!((g - want_db).abs() < tol, "{hz} Hz: {g}");
        }
    }
}
