//! Delay, reverb, convolution and chorus: timing, decay and equivalence.

mod fx_support;
use fx_support::*;

use praxis_dsp::biquad::{BiquadShape, Coeffs};
use praxis_dsp::fx::{Chorus, Convolver, Delay, FdnReverb};
use praxis_dsp::rng::Pcg32;
use praxis_dsp::{Processor, StereoBuffer};

fn impulse_response(p: &mut dyn Processor, n: usize, amp: f32) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0f32; n];
    let mut r = vec![0.0f32; n];
    l[0] = amp;
    r[0] = amp;
    run(p, &mut l, &mut r, BLOCK);
    (l, r)
}

fn first_nonzero(x: &[f32], from: usize) -> Option<usize> {
    x.iter()
        .enumerate()
        .skip(from)
        .find(|(_, v)| v.abs() > 1e-6)
        .map(|(i, _)| i)
}

#[test]
fn delay_timing_is_exact_in_samples() {
    for ms in [1.0f32, 10.0, 125.0, 350.0, 2000.0] {
        let mut d = Delay::new(SR, BLOCK);
        d.set_param(0, 0.0); // not synced
        d.set_param(1, ms);
        d.set_param(2, 0.5);
        d.set_param(4, 20000.0);
        d.set_param(5, 1.0);
        d.reset();
        let want = (ms * SR / 1000.0).round() as usize;
        let (l, _) = impulse_response(&mut d, want * 3 + 10, 1.0);
        assert_eq!(first_nonzero(&l, 0), Some(want), "{ms} ms");
        assert!((l[want] - 1.0).abs() < 1e-6, "first echo is the input");
        let second = first_nonzero(&l, want + 1).unwrap();
        assert!(
            second.abs_diff(2 * want) <= 1,
            "{ms} ms: second echo at {second}"
        );
    }
}

#[test]
fn delay_tempo_sync_is_correct() {
    for (bpm, beats) in [(120.0f32, 0.75f32), (128.0, 1.0), (90.0, 0.5), (20.0, 4.0)] {
        let mut d = Delay::new(SR, BLOCK);
        d.set_tempo(bpm);
        d.set_param(0, beats);
        d.set_param(2, 0.0);
        d.set_param(5, 1.0);
        d.reset();
        let want = (beats * 60.0 / bpm * SR).round() as usize;
        assert_eq!(d.target_samples() as usize, want);
        let (l, _) = impulse_response(&mut d, want + 100, 1.0);
        assert_eq!(first_nonzero(&l, 0), Some(want), "{bpm} bpm x {beats}");
        // feedback 0: exactly one echo, tail is one delay
        assert!(d.tail_samples() >= want && d.tail_samples() <= want + 2);
    }
    // through AnyFx as well
    let mut any = praxis_dsp::fx::create_any(4, SR, BLOCK).unwrap();
    any.set_tempo(60.0);
    any.set_param(0, 1.0);
    any.set_param(2, 0.0);
    any.set_param(5, 1.0);
    any.reset();
    let (l, _) = impulse_response(&mut any, 48100, 1.0);
    assert_eq!(first_nonzero(&l, 0), Some(48000));
}

#[test]
fn delay_ping_pong_alternates() {
    let mut d = Delay::new(SR, BLOCK);
    d.set_param(0, 0.0);
    d.set_param(1, 100.0);
    d.set_param(2, 0.8);
    d.set_param(3, 1.0);
    d.set_param(4, 20000.0);
    d.set_param(5, 1.0);
    d.reset();
    let t = 4800;
    let (l, r) = impulse_response(&mut d, t * 4 + 10, 1.0);
    // echo 1 left, echo 2 right, echo 3 left
    assert!(l[t].abs() > 0.5 && r[t].abs() < 1e-6);
    assert!(r[2 * t].abs() > 0.3 && l[2 * t].abs() < 1e-3);
    assert!(l[3 * t].abs() > 0.2 && r[3 * t].abs() < 1e-3);
}

#[test]
fn delay_tail_reaches_minus_90_db() {
    let mut d = Delay::new(SR, BLOCK);
    d.set_param(0, 0.0);
    d.set_param(1, 50.0);
    d.set_param(2, 0.9);
    d.set_param(4, 20000.0);
    d.set_param(5, 1.0);
    d.reset();
    let tail = d.tail_samples();
    let (l, r) = impulse_response(&mut d, tail + 4800, 1.0);
    let after = peak(&l[tail..]).max(peak(&r[tail..]));
    assert!(after < 10f32.powf(-90.0 / 20.0), "after tail {after}");
    // and the tail is not wildly pessimistic: something is still audible at 80% of it
    let at = tail * 8 / 10;
    assert!(peak(&l[at - 2400..at + 2400]) > 1e-6);
}

/// Schroeder energy-decay RT60 (T20 extrapolated) of a stereo impulse response.
fn measured_rt60(l: &[f32], r: &[f32], sr: f32) -> f64 {
    let e: Vec<f64> = l
        .iter()
        .zip(r.iter())
        .map(|(a, b)| (*a as f64).powi(2) + (*b as f64).powi(2))
        .collect();
    let total: f64 = e.iter().sum();
    let mut rem = total;
    let mut t5 = None;
    let mut t25 = None;
    for (i, v) in e.iter().enumerate() {
        let d = 10.0 * (rem / total).log10();
        if t5.is_none() && d <= -5.0 {
            t5 = Some(i);
        }
        if t25.is_none() && d <= -25.0 {
            t25 = Some(i);
            break;
        }
        rem -= v;
    }
    3.0 * (t25.unwrap() - t5.unwrap()) as f64 / sr as f64
}

fn reverb(decay: f32, size: f32, damp: f32) -> FdnReverb {
    let mut r = FdnReverb::new(SR, BLOCK);
    r.set_param(0, decay);
    r.set_param(1, 0.0);
    r.set_param(2, size);
    r.set_param(3, damp);
    r.set_param(5, 1.0);
    r.reset();
    r
}

/// RT60 in an octave band around `hz` (band-pass Q 1.41, T20).
fn band_rt60(l: &[f32], r: &[f32], hz: f64) -> f64 {
    let c = Coeffs::design(
        BiquadShape::BandPass,
        hz,
        std::f64::consts::SQRT_2,
        SR as f64,
    );
    let bl: Vec<f32> = biquad_f64(&c, &l.iter().map(|&v| v as f64).collect::<Vec<_>>())
        .iter()
        .map(|&v| v as f32)
        .collect();
    let br: Vec<f32> = biquad_f64(&c, &r.iter().map(|&v| v as f64).collect::<Vec<_>>())
        .iter()
        .map(|&v| v as f32)
        .collect();
    measured_rt60(&bl, &br, SR)
}

#[test]
fn reverb_decays_at_the_configured_rt60() {
    // RT60 is the mid-band figure (ISO 3382 T_mid: mean of the 500 Hz and 1 kHz octaves).
    // Damping shortens the highs by design, so the broadband figure of a white impulse (half
    // its energy above 12 kHz) is checked more loosely.
    for (decay, size, damp) in [
        (0.5f32, 0.3f32, 20000.0f32),
        (2.2, 0.6, 7000.0),
        (5.0, 1.0, 20000.0),
        (1.2, 0.0, 3000.0),
        (12.0, 0.8, 7000.0),
    ] {
        let mut rv = reverb(decay, size, damp);
        let n = (SR * (decay * 1.2 + 0.5)) as usize;
        let (l, r) = impulse_response(&mut rv, n, 1.0);
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()));
        let mid = 0.5 * (band_rt60(&l, &r, 500.0) + band_rt60(&l, &r, 1000.0));
        let broad = measured_rt60(&l, &r, SR);
        let err = (mid - decay as f64).abs() / decay as f64;
        println!(
            "reverb: RT60 {decay} s, size {size}, damp {damp}: T_mid {mid:.3} s ({:.1}%), broadband {broad:.3} s",
            err * 100.0
        );
        assert!(err < 0.15, "RT60 {decay}: measured T_mid {mid}");
        if damp >= 20000.0 {
            let e = (broad - decay as f64).abs() / decay as f64;
            assert!(e < 0.2, "RT60 {decay}: broadband {broad}");
        }
    }
}

#[test]
fn reverb_is_energy_stable_at_max_decay() {
    let mut rv = reverb(20.0, 1.0, 20000.0);
    let n = (SR * 1.0) as usize;
    let mut prev = f64::INFINITY;
    let mut peak_all = 0.0f32;
    for k in 0..24 {
        let (mut l, mut r) = if k == 0 {
            (white(7, 0.5, n), white(8, 0.5, n))
        } else {
            (vec![0.0; n], vec![0.0; n])
        };
        run(&mut rv, &mut l, &mut r, BLOCK);
        assert!(
            l.iter().chain(r.iter()).all(|v| v.is_finite()),
            "NaN at {k}"
        );
        peak_all = peak_all.max(peak(&l)).max(peak(&r));
        let e = rms(&l).powi(2) + rms(&r).powi(2);
        if k >= 2 {
            assert!(e <= prev * 1.001, "energy grew at second {k}: {e} > {prev}");
        }
        if k >= 4 {
            // 20 s RT60 = -3 dB per second
            let drop = 10.0 * (prev / e).log10();
            println!("second {k}: decay {drop:.2} dB/s");
            // high frequencies decay faster first; the body settles at 3 dB/s
            if k >= 8 {
                assert!((drop - 3.0).abs() < 0.4, "second {k}: decay {drop:.2} dB/s");
            } else {
                assert!(drop > 2.4 && drop < 4.2, "second {k}: decay {drop:.2} dB/s");
            }
        }
        prev = e;
    }
    assert!(peak_all < 4.0, "peak {peak_all}");
}

#[test]
fn reverb_damping_darkens_the_tail() {
    let n = (SR * 1.5) as usize;
    let hf = |damp: f32| {
        let mut rv = reverb(2.0, 0.6, damp);
        let (l, _) = impulse_response(&mut rv, n, 1.0);
        let seg = &l[(SR * 0.5) as usize..];
        let diff: Vec<f32> = seg.windows(2).map(|w| w[1] - w[0]).collect();
        rms(&diff) / rms(seg)
    };
    assert!(hf(1500.0) < 0.5 * hf(20000.0));
}

#[test]
fn reverb_width_zero_is_mono_and_predelay_is_exact() {
    let mut rv = reverb(1.0, 0.5, 8000.0);
    rv.set_param(4, 0.0);
    rv.set_param(1, 50.0);
    rv.reset();
    let n = 48000;
    let mut l = vec![0.0f32; n];
    let mut r = vec![0.0f32; n];
    l[0] = 1.0;
    run(&mut rv, &mut l, &mut r, BLOCK);
    assert!(l.iter().zip(r.iter()).all(|(a, b)| (a - b).abs() < 1e-6));
    // predelay 50 ms plus the shortest diffuser chain: nothing before 2400 samples
    assert!(peak(&l[..2400]) == 0.0);
    assert!(peak(&l[2400..]) > 0.0);
}

fn direct_convolution(x: &[f32], h: &[f32]) -> Vec<f64> {
    let mut y = vec![0.0f64; x.len()];
    for (n, out) in y.iter_mut().enumerate() {
        let mut acc = 0.0f64;
        for (m, &hm) in h.iter().enumerate().take(n + 1) {
            acc += hm as f64 * x[n - m] as f64;
        }
        *out = acc;
    }
    y
}

/// The frozen biquad design run in f64 (transposed direct form II).
fn biquad_f64(c: &Coeffs, x: &[f64]) -> Vec<f64> {
    let (b0, b1, b2, a1, a2) = (
        c.b0 as f64,
        c.b1 as f64,
        c.b2 as f64,
        c.a1 as f64,
        c.a2 as f64,
    );
    let (mut z1, mut z2) = (0.0, 0.0);
    x.iter()
        .map(|&v| {
            let y = b0 * v + z1;
            z1 = b1 * v - a1 * y + z2;
            z2 = b2 * v - a2 * y;
            y
        })
        .collect()
}

#[test]
fn convolution_equals_direct_convolution() {
    // IR long enough to use the head plus several FFT partitions
    for ir_len in [300usize, 512, 513, 2000, 5000] {
        let mut rng = Pcg32::new(ir_len as u64);
        let ir = StereoBuffer {
            left: (0..ir_len)
                .map(|i| rng.next_bipolar() * (-(i as f32) / 800.0).exp())
                .collect(),
            right: (0..ir_len)
                .map(|i| rng.next_bipolar() * (-(i as f32) / 600.0).exp())
                .collect(),
        };
        let mut c = Convolver::new(SR, BLOCK);
        c.set_param(2, 20.0);
        c.set_param(3, 1.0);
        c.set_impulse(&ir);
        c.reset();
        let n = 12000;
        let xl = white(11, 0.5, n);
        let xr = white(12, 0.5, n);
        let mono: Vec<f32> = xl
            .iter()
            .zip(xr.iter())
            .map(|(a, b)| 0.5 * (a + b))
            .collect();
        let mut l = xl.clone();
        let mut r = xr.clone();
        // odd block sizes to exercise the partition buffering
        run(&mut c, &mut l, &mut r, 77);
        let hp = Coeffs::design(
            BiquadShape::HighPass,
            20.0,
            std::f64::consts::FRAC_1_SQRT_2,
            SR as f64,
        );
        for (ch, out) in [(&ir.left, &l), (&ir.right, &r)] {
            let want = direct_convolution(&mono, ch);
            let want: Vec<f32> = biquad_f64(&hp, &want).iter().map(|&v| v as f32).collect();
            let scale = peak(&want).max(1e-6);
            let err = out
                .iter()
                .zip(want.iter())
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            println!(
                "convolution IR {ir_len}: max relative error {:.2e}",
                err / scale
            );
            assert!(
                err / scale < 1e-4,
                "IR {ir_len}: max error {err} (relative {})",
                err / scale
            );
        }
    }
}

#[test]
fn convolution_decay_scale_reshapes_the_ir() {
    // an exponential-noise IR with RT60 0.5 s
    let n = (SR * 1.5) as usize;
    let mut rng = Pcg32::new(3);
    let k = 3.0 * std::f32::consts::LN_10 / (0.5 * SR);
    let ir = StereoBuffer {
        left: (0..n)
            .map(|i| rng.next_bipolar() * (-k * i as f32).exp())
            .collect(),
        right: (0..n)
            .map(|i| rng.next_bipolar() * (-k * i as f32).exp())
            .collect(),
    };
    for scale in [1.0f32, 0.5, 1.5] {
        let mut c = Convolver::new(SR, BLOCK);
        c.set_param(2, 20.0);
        c.set_param(3, 1.0);
        c.set_param(1, scale);
        c.set_impulse(&ir);
        c.reset();
        let est = c.impulse_rt60_seconds();
        assert!((est - 0.5).abs() < 0.05, "estimate {est}");
        let (l, r) = impulse_response(&mut c, n + 4800, 1.0);
        let rt = measured_rt60(&l, &r, SR);
        let want = 0.5 * scale as f64;
        assert!(
            (rt - want).abs() / want < 0.15,
            "scale {scale}: measured {rt}"
        );
    }
    // a change while running crossfades in without NaN or a jump
    let mut c = Convolver::new(SR, BLOCK);
    c.set_param(3, 1.0);
    c.set_impulse(&ir);
    let total = (SR * 2.0) as usize;
    let mut l = white(4, 0.3, total);
    let mut r = white(5, 0.3, total);
    let mut i = 0;
    while i < total {
        let e = (i + BLOCK).min(total);
        if i == 24000 {
            c.set_param(1, 0.3);
        }
        c.process(&mut l[i..e], &mut r[i..e]);
        i = e;
    }
    assert!(l.iter().all(|v| v.is_finite()));
}

#[test]
fn chorus_delays_within_its_sweep() {
    let mut c = Chorus::new(SR, BLOCK);
    c.set_param(0, 0.5);
    c.set_param(1, 4.0);
    c.set_param(2, 10.0);
    c.set_param(3, 1.0);
    c.reset();
    let (l, _) = impulse_response(&mut c, 4800, 1.0);
    let first = first_nonzero(&l, 0).unwrap();
    // at phase 0 the delay is base + depth/2 = 12 ms
    assert!((570..=586).contains(&first), "first {first}");
    assert!(c.tail_samples() >= (14.0 * 48.0) as usize);
}

#[test]
fn reverb_wet_envelope_builds_up_smoothly_after_the_predelay() {
    // A natural room: the wet signal of a click starts right after the predelay (early
    // reflections) and builds up smoothly into the late field, with no gap and no step where
    // the feedback network's first arrivals come in (heard as a ghost attack).
    let w = 120; // 2.5 ms windows
    for (decay, size, damp, pd) in [
        (1.8f32, 0.35f32, 7000.0f32, 12.0f32),
        (1.0, 0.0, 20000.0, 0.0),
        (2.2, 0.6, 7000.0, 15.0),
        (3.0, 1.0, 7000.0, 40.0),
        (0.5, 0.3, 20000.0, 5.0),
    ] {
        let mut rv = reverb(decay, size, damp);
        rv.set_param(1, pd);
        rv.reset();
        let (l, r) = impulse_response(&mut rv, (SR * 0.3) as usize, 1.0);
        let start = (pd * SR * 0.001).round() as usize;
        let first = first_nonzero(&l, 0)
            .unwrap()
            .min(first_nonzero(&r, 0).unwrap());
        assert!(
            first >= start && first <= start + 48,
            "wet starts at {first}, predelay {start}"
        );
        let env: Vec<f64> = (0..(l.len() - start) / w)
            .map(|k| {
                let a = start + k * w;
                let e: f64 = l[a..a + w]
                    .iter()
                    .chain(&r[a..a + w])
                    .map(|v| (*v as f64).powi(2))
                    .sum();
                10.0 * e.max(1e-30).log10()
            })
            .collect();
        let peak = env.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        // Within 5 ms of the predelay the wet is already within 30 dB of its peak.
        assert!(
            env[1] > peak - 30.0,
            "{decay}/{size}: {:.1} vs peak {peak:.1}",
            env[1]
        );
        // From then on (first 200 ms) no window rises more than 4 dB above the previous one
        // and none falls more than 4 dB below it.
        let worst_rise = env[2..80]
            .windows(2)
            .map(|p| p[1] - p[0])
            .fold(f64::NEG_INFINITY, f64::max);
        let worst_fall = env[2..80]
            .windows(2)
            .map(|p| p[0] - p[1])
            .fold(f64::NEG_INFINITY, f64::max);
        println!(
            "reverb envelope: RT60 {decay} s, size {size}, predelay {pd} ms: first wet sample +{}              samples, -{:.1} dB at 5 ms, worst step +{worst_rise:.1} / -{worst_fall:.1} dB per 2.5 ms",
            first - start,
            peak - env[1]
        );
        assert!(
            worst_rise <= 4.0,
            "{decay}/{size}: step +{worst_rise:.1} dB"
        );
        assert!(
            worst_fall <= 4.0,
            "{decay}/{size}: step -{worst_fall:.1} dB"
        );
    }
}
