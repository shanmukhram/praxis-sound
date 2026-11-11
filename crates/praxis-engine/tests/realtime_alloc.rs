//! Proves the audio thread never allocates or frees: a counting global allocator is armed while
//! the real-time engine processes callbacks and applies every command, including a graph swap
//! with its crossfade, a stop and a fresh restart, seeks, loop wraps, odd callback sizes and the
//! end of a one-shot. Graphs and boxes are built before arming (that is the control thread's
//! job); the retired graph is dropped after disarming, by the control side.
//!
//! This file holds a single test so no other test thread allocates while the counter is armed.
//! It needs `unsafe` only to implement `GlobalAlloc` (a test-only allocator that forwards to the
//! system allocator); the library itself stays `#![forbid(unsafe_code)]`.

#![allow(unsafe_code)]

mod common;

use common::*;
use praxis_engine::realtime::{
    Command, EngineControl, LoopRange, RealtimeEngine, RealtimeOptions, prepare,
};
use praxis_engine::testing::TestFactory;
use praxis_score::timeline::score_length_samples;
use praxis_score::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Counting;

static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);

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
        if ARMED.load(Ordering::Relaxed) {
            FREES.fetch_add(1, Ordering::Relaxed);
        }
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

/// Runs `callbacks` callbacks of `frames` frames with the counter armed; returns
/// (allocations, frees) seen meanwhile. Then polls the control side (disarmed).
fn armed(
    engine: &mut RealtimeEngine,
    control: &mut EngineControl,
    l: &mut [f32],
    r: &mut [f32],
    frames: usize,
    callbacks: usize,
) -> (usize, usize) {
    ALLOCS.store(0, Ordering::SeqCst);
    FREES.store(0, Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
    for _ in 0..callbacks {
        engine.process(&mut l[..frames], &mut r[..frames]);
    }
    ARMED.store(false, Ordering::SeqCst);
    let counts = (ALLOCS.load(Ordering::SeqCst), FREES.load(Ordering::SeqCst));
    control.poll(); // the control side drains statuses and drops retired graphs
    counts
}

fn send(c: &mut EngineControl, cmd: Command) {
    c.send(cmd).expect("queue has room");
}

#[test]
fn the_audio_thread_never_allocates_or_frees() {
    // ---- control thread: build everything first ----
    let mut score = bell_score();
    score.playback = Playback::OneShot { tail_seconds: 1.0 };
    score.master.frozen_gain_db = Some(0.0);
    // Exercise the master chain, a layer with latency and a sidechain too.
    let mut lim = score.layers[0].effects[0].clone();
    lim.id = "lim".into();
    lim.kind = EffectKind::Limiter;
    lim.params.clear();
    score.layers[1].effects.push(lim.clone());
    score.master.effects.push(lim);
    let bar = score_length_samples(&score, SR) / score.bars as u64;
    let a = prepare(&score, &TestFactory::new(), &RealtimeOptions::default()).unwrap();
    let mut edited = score.clone();
    edited.layers[1].gain_db -= 6.0;
    let b = Box::new(prepare(&edited, &TestFactory::new(), &RealtimeOptions::default()).unwrap());
    let rejected = Box::new(
        prepare(
            &score,
            &TestFactory::new(),
            &RealtimeOptions {
                sample_rate: 44_100,
                master_gain_db: None,
            },
        )
        .unwrap(),
    );
    let (mut e, mut c) = RealtimeEngine::with_graph(a);
    let mut l = vec![0.0f32; 1024];
    let mut r = vec![0.0f32; 1024];
    let mut total = (0usize, 0usize);
    let mut add = |x: (usize, usize), what: &str| {
        println!("{what}: {} allocations, {} frees", x.0, x.1);
        total.0 += x.0;
        total.1 += x.1;
    };

    // ---- audio thread (armed) ----
    send(&mut c, Command::Play);
    add(
        armed(&mut e, &mut c, &mut l, &mut r, 256, 100),
        "play + process",
    );
    for cmd in [
        Command::SetLayerGain {
            layer: 0,
            gain_db: -3.0,
        },
        Command::SetLayerMute {
            layer: 1,
            muted: true,
        },
        Command::SetLayerSolo {
            layer: 0,
            solo: true,
        },
        Command::SetMasterGain { gain_db: -2.0 },
        Command::SetLayerSolo {
            layer: 0,
            solo: false,
        },
        Command::SetLayerMute {
            layer: 1,
            muted: false,
        },
        Command::SetLoop(Some(LoopRange {
            start: bar,
            end: 2 * bar,
        })),
        Command::Seek(bar),
    ] {
        send(&mut c, cmd);
        add(armed(&mut e, &mut c, &mut l, &mut r, 97, 3), "command");
    }
    add(
        armed(&mut e, &mut c, &mut l, &mut r, 256, 3 * bar as usize / 256),
        "loop wraps",
    );
    send(&mut c, Command::Swap(b));
    add(
        armed(&mut e, &mut c, &mut l, &mut r, 256, 40),
        "swap + crossfade",
    );
    send(&mut c, Command::Swap(rejected));
    add(
        armed(&mut e, &mut c, &mut l, &mut r, 256, 2),
        "rejected swap",
    );
    send(&mut c, Command::Stop);
    add(armed(&mut e, &mut c, &mut l, &mut r, 1024, 4), "stop");
    send(&mut c, Command::SetLoop(None));
    send(&mut c, Command::Seek(14 * bar));
    send(&mut c, Command::Play);
    add(
        armed(&mut e, &mut c, &mut l, &mut r, 1000, 30),
        "restart (reset)",
    );
    add(
        armed(&mut e, &mut c, &mut l, &mut r, 512, 4 * bar as usize / 512),
        "play to the end",
    );

    // ---- control thread again ----
    let s = c.poll();
    println!("status: {s:?}");
    assert!(s.ended && !s.playing, "played to the end");
    assert_eq!((s.swaps, s.rejected_swaps), (2, 1));
    assert_eq!(total, (0, 0), "allocations and frees on the audio thread");
}
