//! Audio-thread rule check for every sound-effect preset: `note_on`, `note_off`, `set_param`,
//! `all_notes_off`, `reset` and `process` never allocate after construction. A counting wrapper
//! around the system allocator (test-only; `GlobalAlloc` is an unsafe trait) counts allocations
//! while a dense, overlapping, voice-stealing note stream plays (the pattern of
//! `tests/synth_alloc.rs`).
#![allow(unsafe_code)]

mod synth_common;

use praxis_engine::{Instrument, NoteOn};
use praxis_score::InstrumentKind;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use synth_common::*;

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
    let pitches = [33u8, 45, 52, 57, 60, 64, 69, 76];
    let mut l = vec![0f32; BLOCK];
    let mut r = vec![0f32; BLOCK];
    ALLOCS.store(0, Ordering::SeqCst);
    COUNTING.store(true, Ordering::SeqCst);
    for b in 0..1500u64 {
        // A note every 3 blocks (dense and overlapping, so voices get stolen), released 40
        // blocks later; macros swept while voices play.
        if b % 3 == 0 {
            inst.note_on(&NoteOn {
                id: b,
                pitch: pitches[(b / 3) as usize % pitches.len()],
                velocity: 0.3 + 0.07 * (b % 10) as f32,
                brightness: if b % 8 == 0 { 1.0 } else { 0.0 },
                seed: b.wrapping_mul(0x9E37_79B9),
                length_samples: 40 * BLOCK as u64,
            });
        }
        if b >= 40 && (b - 40) % 3 == 0 {
            inst.note_off(b - 40);
        }
        if b % 250 == 125 {
            for i in 0..4 {
                inst.set_param(i, ((b / 125) % 5) as f32 * 0.25);
            }
        }
        if b == 1000 {
            inst.all_notes_off();
        }
        if b == 1300 {
            inst.reset();
        }
        inst.process(&mut l, &mut r);
    }
    let _ = inst.tail_samples();
    let n = ALLOCS.load(Ordering::SeqCst);
    let v = std::hint::black_box(vec![1u8; 64]);
    drop(v);
    let control = ALLOCS.load(Ordering::SeqCst) - n;
    COUNTING.store(false, Ordering::SeqCst);
    assert!(control >= 1, "allocation counter is not counting");
    n
}

#[test]
fn sfx_audio_calls_do_not_allocate() {
    let presets = names(InstrumentKind::Sfx);
    assert_eq!(presets.len(), 6, "{presets:?}");
    for preset in presets {
        let mut inst = create(InstrumentKind::Sfx, &preset, &[]);
        let n = play(inst.as_mut());
        println!("{preset}: {n} allocations on the audio path");
        assert_eq!(n, 0, "{preset} allocated on the audio path");
    }
}
