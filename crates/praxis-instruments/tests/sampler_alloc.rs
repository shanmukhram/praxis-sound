//! Audio-thread rule check: `note_on`, `note_off`, `set_param`, `all_notes_off`, `reset` and
//! `process` never allocate. A counting wrapper around the system allocator (test-only; the one
//! place this crate's tests need `unsafe`, because `GlobalAlloc` is an unsafe trait) counts
//! allocations while a dense, overlapping, voice-stealing note stream is played.
#![allow(unsafe_code)]

mod sampler_common;

use praxis_engine::{Instrument, InstrumentContext, InstrumentFactory, NoteOn};
use praxis_instruments::Registry;
use praxis_instruments::sampler::presets::PIANO_PACK;
use praxis_score::{InstrumentKind, InstrumentRef, Key, Mode, PitchClass};
use sampler_common::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Counting;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn play(inst: &mut dyn Instrument) -> usize {
    let mut l = vec![0f32; BLOCK];
    let mut r = vec![0f32; BLOCK];
    let pitches = [50u8, 57, 60, 64, 69, 72, 55, 62];
    ALLOCS.store(0, Ordering::SeqCst);
    COUNTING.store(true, Ordering::SeqCst);
    for b in 0..2000u64 {
        // A note every other block (overlapping, so voices get stolen), released 12 blocks later.
        if b % 2 == 0 {
            inst.note_on(&NoteOn {
                id: b,
                pitch: pitches[(b / 2) as usize % pitches.len()],
                velocity: 0.3 + 0.07 * (b % 10) as f32,
                brightness: if b % 8 == 0 { 1.0 } else { 0.0 },
                seed: b.wrapping_mul(0x9E37_79B9),
                length_samples: 12 * BLOCK as u64,
            });
        }
        if b >= 12 && b % 2 == 0 {
            inst.note_off(b - 12);
        }
        if b % 500 == 250 {
            inst.set_param(0, 0.3);
            inst.set_param(2, 0.8);
        }
        if b == 1500 {
            inst.all_notes_off();
        }
        if b == 1800 {
            inst.reset();
        }
        inst.process(&mut l, &mut r);
    }
    let _ = inst.tail_samples();
    let n = ALLOCS.load(Ordering::SeqCst);
    // Control: the counter does see allocations.
    let v = std::hint::black_box(vec![1u8; 64]);
    drop(v);
    let control = ALLOCS.load(Ordering::SeqCst) - n;
    COUNTING.store(false, Ordering::SeqCst);
    assert!(control >= 1, "allocation counter is not counting");
    n
}

#[test]
fn audio_thread_calls_do_not_allocate() {
    let reg = Registry::new(synth_pack(PIANO_PACK));
    let ctx = InstrumentContext {
        sample_rate: SR,
        max_block: BLOCK,
        key: Key {
            tonic: PitchClass::A,
            mode: Mode::Minor,
        },
        bpm: 128.0,
        seed: 1,
    };
    let mut packs_to_test = vec![("synthetic", reg)];
    if let Some(real) = installed_piano("audio_thread_calls_do_not_allocate (real pack part)") {
        packs_to_test.push(("Upright Piano KW", Registry::new(real)));
    }
    for (label, reg) in &packs_to_test {
        for preset in ["felt_piano", "upright_piano"] {
            let spec = InstrumentRef {
                kind: InstrumentKind::Sampler,
                preset: preset.into(),
                params: Default::default(),
            };
            let mut inst = reg.create(&spec, &ctx).unwrap();
            let n = play(inst.as_mut());
            println!("{label} {preset}: {n} allocations on the audio path");
            assert_eq!(n, 0, "{label} {preset} allocated on the audio path");
        }
    }
}
