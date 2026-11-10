//! Proves the dsp-core building blocks never allocate after construction: a counting global
//! allocator is armed while every type runs, with parameter changes, gates and resets.
//!
//! This file holds a single test so no other test thread allocates while the counter is armed.
//! It needs `unsafe` only to implement `GlobalAlloc` (a test-only allocator that forwards to the
//! system allocator); the library itself stays `#![forbid(unsafe_code)]`.

#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use praxis_dsp::env::{Adsr, Ar, Dahdsr};
use praxis_dsp::filter::{DcBlocker, Ladder, OnePole, Svf, SvfMode};
use praxis_dsp::interp::{Quality, Resampler};
use praxis_dsp::noise::{Brown, Pink, White};
use praxis_dsp::osc::{
    Fm4, FmAlgorithm, FmOperator, FmPair, PolyBlepSaw, PolyBlepSquare, Sine, Triangle, Wavetable,
};
use praxis_dsp::smooth::Smoothed;

struct Counting;

static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: forwarded unchanged to the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: forwarded unchanged to the system allocator.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded unchanged to the system allocator.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: forwarded unchanged to the system allocator.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn nothing_allocates_after_construction() {
    let sr = 48_000.0;
    // ---- construction (allowed to allocate) ----
    let mut sine = Sine::new(sr);
    let mut saw = PolyBlepSaw::new(sr);
    let mut sq = PolyBlepSquare::new(sr);
    let mut tri = Triangle::new(sr);
    let harmonics: Vec<f32> = (1..=300).map(|k| 1.0 / k as f32).collect();
    let wt_proto = Wavetable::from_frames(sr, &[vec![1.0], harmonics]);
    let mut op = FmOperator::new(sr);
    let mut pair = FmPair::new(sr);
    let mut fm4 = Fm4::new(sr);
    let mut adsr = Adsr::new(sr);
    let mut ar = Ar::new(sr);
    let mut dahdsr = Dahdsr::new(sr);
    let mut svf = Svf::new(sr);
    let mut one = OnePole::new(sr);
    let mut ladder = Ladder::new(sr);
    let mut dc = DcBlocker::new(sr);
    let mut white = White::new(1);
    let mut pink = Pink::new(sr, 2);
    let mut brown = Brown::new(sr, 3);
    let mut smooth = Smoothed::exponential(sr, 0.01, 0.0);
    let sample: Vec<f32> = (0..10_000).map(|i| (i as f32 * 0.01).sin()).collect();
    let mut readers = [
        Resampler::new(Quality::Linear),
        Resampler::new(Quality::Hermite),
        Resampler::new(Quality::Sinc),
    ];

    ARMED.store(true, Ordering::SeqCst);
    // Cloning a wavetable per voice shares its tables.
    let mut wt = wt_proto.clone();
    let mut acc = 0.0f32;
    for n in 0..20_000usize {
        let t = n as f32 / sr;
        let hz = 110.0 + 2000.0 * (t * 3.0).sin().abs();
        if n % 4000 == 0 {
            for r in readers.iter_mut() {
                r.start(0.0);
                r.set_rate(0.5 + (n / 4000) as f64 * 0.4);
                r.set_loop(Some((2000, 6000)));
            }
            adsr.gate(true);
            ar.gate(true);
            dahdsr.gate(true);
            sine.reset(0.0);
            saw.reset(0.25);
            sq.reset(0.0);
            tri.reset(0.0);
            wt.reset(0.0);
            pair.reset(0.0);
            fm4.reset(0.0);
            pink.reseed(n as u64);
            fm4.algorithm = if n % 8000 == 0 {
                FmAlgorithm::Stack
            } else {
                FmAlgorithm::TwoPairs
            };
            svf.set_mode(if n % 8000 == 0 {
                SvfMode::LowPass
            } else {
                SvfMode::Bell { gain_db: 3.0 }
            });
        }
        if n % 4000 == 3000 {
            adsr.gate(false);
            dahdsr.gate(false);
        }
        sine.set_freq(hz);
        saw.set_freq(hz);
        sq.set_freq(hz);
        sq.set_pulse_width(0.5 + 0.4 * (t * 2.0).sin());
        tri.set_freq(hz);
        wt.set_freq(hz);
        wt.set_position(0.5 + 0.5 * (t * 5.0).sin());
        op.set_freq(hz);
        pair.set_freq(hz);
        pair.set_index(2.0);
        fm4.set_freq(hz);
        smooth.set_target(if n % 1000 < 500 { 1.0 } else { 0.0 });
        let cutoff = 200.0 + 5000.0 * smooth.next();
        svf.set_cutoff_q(cutoff, 2.0);
        one.set_cutoff(cutoff);
        ladder.set_cutoff(cutoff);
        ladder.set_resonance(0.8);
        let src = sine.next() + saw.next() + sq.next() + tri.next() + wt.next();
        let src = src + op.next() + pair.next() + fm4.next();
        let src = src + white.next() + pink.next() + brown.next();
        let env = adsr.next() + ar.next() + dahdsr.next();
        let filtered = svf.tick(src) + svf.tick_all(src).low + one.tick(src) + ladder.tick(src);
        let mut sampled = 0.0;
        for r in readers.iter_mut() {
            sampled += r.next(&sample);
        }
        acc += dc.tick(filtered * env + sampled);
    }
    ARMED.store(false, Ordering::SeqCst);
    black_box(acc);
    let count = ALLOCS.load(Ordering::SeqCst);
    assert_eq!(count, 0, "{count} allocations on the processing path");
}
