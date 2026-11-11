//! Real-time transport: live mute/solo, restart after stop, stop and seek with a declick, loop
//! ranges, seamless `playback: loop`, graph swaps with a crossfade, and rejected swaps.

mod common;

use common::*;
use praxis_dsp::StereoBuffer;
use praxis_engine::realtime::{
    Command, DECLICK_MS, EngineControl, LoopRange, RealtimeEngine, RealtimeOptions,
    SWAP_CROSSFADE_MS, Status, prepare,
};
use praxis_engine::testing::TestFactory;
use praxis_score::timeline::score_length_samples;
use praxis_score::*;

const CB: usize = 256;

fn one_shot(mut s: Score) -> Score {
    s.playback = Playback::OneShot { tail_seconds: 2.0 };
    s
}

fn frozen(score: &Score) -> Score {
    let out = render(score, &opts(false));
    let mut s = score.clone();
    s.master.frozen_gain_db = Some(out.report.applied_gain_db);
    s
}

fn engine(score: &Score) -> (RealtimeEngine, EngineControl) {
    let g = prepare(score, &TestFactory::new(), &RealtimeOptions::default()).expect("prepare");
    RealtimeEngine::with_graph(g)
}

struct Run {
    audio: StereoBuffer,
    status: Vec<Status>,
}

/// Runs `frames` frames in CB-frame callbacks, recording audio and every status.
fn run(engine: &mut RealtimeEngine, control: &mut EngineControl, frames: usize) -> Run {
    let mut audio = StereoBuffer::silent(frames);
    let mut status = Vec::new();
    let (mut l, mut r) = ([0.0f32; CB], [0.0f32; CB]);
    let mut i = 0;
    while i < frames {
        engine.process(&mut l, &mut r);
        let n = CB.min(frames - i);
        audio.left[i..i + n].copy_from_slice(&l[..n]);
        audio.right[i..i + n].copy_from_slice(&r[..n]);
        control.drain_status(&mut status);
        control.collect_garbage();
        i += n;
    }
    Run { audio, status }
}

fn max_step(x: &[f32]) -> f32 {
    x.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max)
}

#[test]
fn live_mute_and_solo_equal_offline_renders_of_the_edited_score() {
    let base = frozen(&one_shot(bell_score()));
    let len = render(&base, &opts(false)).master.len();
    let kick = 1usize; // score order: piano, kick
    assert_eq!(base.layers[kick].id, "kick");

    let mut muted = base.clone();
    muted.layers[kick].muted = true;
    let (mut e, mut c) = engine(&base);
    c.send(Command::SetLayerMute {
        layer: kick,
        muted: true,
    })
    .unwrap();
    c.play();
    let rt = run(&mut e, &mut c, len);
    let off = render(&muted, &opts(false));
    assert!(buffers_equal(&rt.audio, &off.master), "live mute");

    let mut solo = base.clone();
    solo.layers[kick].solo = true;
    let (mut e, mut c) = engine(&base);
    c.send(Command::SetLayerSolo {
        layer: kick,
        solo: true,
    })
    .unwrap();
    c.play();
    let rt = run(&mut e, &mut c, len);
    let off = render(&solo, &opts(false));
    assert!(buffers_equal(&rt.audio, &off.master), "live solo");

    // A score that is muted plays muted, and unmuting live brings the layer in: the muted layer
    // was built (silently) and runs, so it sounds exactly as if it had never been muted.
    let (mut e, mut c) = engine(&muted);
    c.play();
    let a = run(&mut e, &mut c, len);
    assert!(buffers_equal(&a.audio, &off_master(&muted)), "score mute");
    let (mut e, mut c) = engine(&muted);
    c.send(Command::SetLayerMute {
        layer: kick,
        muted: false,
    })
    .unwrap();
    c.play();
    let b = run(&mut e, &mut c, len);
    assert!(buffers_equal(&b.audio, &off_master(&base)), "live unmute");
}

fn off_master(score: &Score) -> StereoBuffer {
    render(score, &opts(false)).master
}

#[test]
fn play_after_stop_starts_again_from_fresh_state() {
    let score = frozen(&one_shot(bell_score()));
    let offline = off_master(&score);
    let len = offline.len();
    let (mut e, mut c) = engine(&score);
    c.play();
    run(&mut e, &mut c, SR as usize * 3);
    c.stop();
    let stopped = run(&mut e, &mut c, SR as usize / 10);
    let last = stopped.status.last().unwrap();
    assert!(!last.playing);
    let declick = (DECLICK_MS * 0.001 * SR as f64) as usize;
    assert!(
        stopped.audio.left[declick + 1..].iter().all(|v| *v == 0.0),
        "silent after the declick"
    );
    c.seek(0);
    c.play();
    let again = run(&mut e, &mut c, len);
    let differ = (0..len)
        .filter(|&i| again.audio.left[i].to_bits() != offline.left[i].to_bits())
        .count();
    let max = again
        .audio
        .left
        .iter()
        .zip(&offline.left)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    println!(
        "replay after stop: {differ} of {len} samples differ from offline, max |diff| {max:e}"
    );
    assert!(
        buffers_equal(&again.audio, &offline),
        "reset state equals a fresh graph"
    );
}

#[test]
fn stop_fades_out_and_reports_the_position() {
    let score = frozen(&one_shot(bell_score()));
    let (mut e, mut c) = engine(&score);
    c.play();
    let played = 2 * SR as usize; // bell is loud by then
    let before = run(&mut e, &mut c, played);
    assert_eq!(before.status.last().unwrap().position, played as u64);
    c.stop();
    let after = run(&mut e, &mut c, 4 * CB);
    let declick = (DECLICK_MS * 0.001 * SR as f64) as usize;
    let s = after.status.last().unwrap();
    assert!(!s.playing);
    assert_eq!(
        s.position,
        (played + declick) as u64,
        "stops after the fade"
    );
    // The fade is a ramp: no step larger than the music's own.
    let music = max_step(&before.audio.left[played - 4 * CB..]);
    let fade = max_step(&after.audio.left[..declick + 2]);
    assert!(fade <= music * 1.5 + 1e-4, "fade {fade} vs music {music}");
}

#[test]
fn seek_while_playing_restarts_at_the_target() {
    let score = frozen(&one_shot(bell_score()));
    let bar = score_length_samples(&score, SR) / score.bars as u64;
    let (mut e, mut c) = engine(&score);
    c.play();
    run(&mut e, &mut c, SR as usize);
    c.seek(8 * bar);
    let declick = (DECLICK_MS * 0.001 * SR as f64) as usize;
    let r = run(&mut e, &mut c, 4 * CB);
    let pos = r.status.last().unwrap().position;
    assert_eq!(pos, 8 * bar + (4 * CB - declick) as u64);
    assert!(
        rms(&r.audio.left[declick + 200..]) > 1e-4,
        "plays after the seek"
    );
}

#[test]
fn a_loop_range_wraps_with_tails_ringing_on() {
    let score = frozen(&one_shot(bell_score()));
    let bar = score_length_samples(&score, SR) / score.bars as u64;
    let range = LoopRange {
        start: 2 * bar,
        end: 4 * bar,
    };
    let (mut e, mut c) = engine(&score);
    c.set_loop(Some(range));
    c.seek(2 * bar);
    c.play();
    let len = (2 * bar * 3) as usize; // three passes
    let r = run(&mut e, &mut c, len);
    assert!(
        r.status
            .iter()
            .all(|s| s.position >= range.start && s.position < range.end)
    );
    // Each pass sounds; the wrap leaves no gap (tails ring into the next pass).
    let pass = (2 * bar) as usize;
    for k in 0..3 {
        assert!(rms(&r.audio.left[k * pass..(k + 1) * pass]) > 1e-3);
    }
    let around = &r.audio.left[pass - 2000..pass + 2000];
    assert!(rms(around) > 1e-4, "no silence at the wrap");
    // The second and third passes match (steady state after one pass), within float noise of
    // the limiter's running sum.
    let d = r.audio.left[pass..2 * pass]
        .iter()
        .zip(&r.audio.left[2 * pass..3 * pass])
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    println!("loop range: pass 2 vs pass 3 max |diff| {d:e}");
    assert!(d < 1e-3, "{d}");
}

#[test]
fn loop_scores_repeat_seamlessly_and_converge_to_the_offline_loop() {
    let score = frozen(&bell_score()); // playback: loop, 30 s
    let off = render(&score, &opts(false));
    let l = off.master.len();
    let (mut e, mut c) = engine(&score);
    c.play();
    let r = run(&mut e, &mut c, 3 * l);
    // The third cycle has the previous cycles' tails, like the offline capture.
    let cyc = &r.audio.left[2 * l..3 * l];
    let d = cyc
        .iter()
        .zip(&off.master.left)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    let peak = off.master.left.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    println!("loop score: cycle 3 vs offline loop max |diff| {d:e} (peak {peak})");
    assert!(d < 1e-3 * peak.max(1e-3), "{d}");
    // Position wraps.
    assert!(r.status.iter().all(|s| s.position < l as u64));
    // No click at the seams.
    for k in 1..3 {
        let seam = &r.audio.left[k * l - 64..k * l + 64];
        assert!(max_step(seam) <= max_step(&r.audio.left[k * l - 4800..k * l - 64]) * 2.0 + 1e-4);
    }
}

#[test]
fn a_swap_while_playing_crossfades_and_returns_the_old_graph() {
    let score = frozen(&one_shot(bell_score()));
    let mut edited = score.clone();
    edited.layers.retain(|l| l.id != "kick");
    let (mut e, mut c) = engine(&score);
    c.play();
    let before = run(&mut e, &mut c, SR as usize * 4);
    let g = prepare(&edited, &TestFactory::new(), &RealtimeOptions::default()).unwrap();
    c.swap(g).unwrap();
    let xfade = (SWAP_CROSSFADE_MS * 0.001 * SR as f64) as usize;
    let mut status = Vec::new();
    let mut audio = Vec::new();
    let (mut l, mut r) = ([0.0f32; CB], [0.0f32; CB]);
    let mut retired = 0;
    for _ in 0..(SR as usize / CB) {
        e.process(&mut l, &mut r);
        audio.extend_from_slice(&l);
        c.drain_status(&mut status);
        retired += c.collect_garbage();
    }
    assert_eq!(retired, 1, "the old graph comes back to be dropped here");
    let last = status.last().unwrap();
    assert_eq!(last.swaps, 2, "the initial load and the edit");
    assert!(!last.crossfading && last.playing);
    // Position continues through the swap.
    assert_eq!(last.position, (SR as usize * 4 + audio.len()) as u64);
    let music = max_step(&before.audio.left[SR as usize..]);
    let swap = max_step(&audio[..xfade + CB]);
    println!("swap: max step during crossfade {swap:e}, in the music {music:e}");
    assert!(swap <= music * 1.5 + 1e-4, "no click");
    assert!(rms(&audio[xfade..]) > 1e-3, "the new graph plays");
}

#[test]
fn a_graph_of_another_sample_rate_is_rejected() {
    let score = bell_score();
    let (mut e, mut c) = engine(&score);
    let g = prepare(
        &score,
        &TestFactory::new(),
        &RealtimeOptions {
            sample_rate: 44_100,
            master_gain_db: Some(0.0),
        },
    )
    .unwrap();
    c.swap(g).unwrap();
    let r = run(&mut e, &mut c, CB);
    let s = r.status.last().unwrap();
    // The first swap is the initial load (`with_graph`).
    assert_eq!((s.swaps, s.rejected_swaps), (1, 1));
}

#[test]
fn an_engine_without_a_graph_is_silent_and_starts_when_one_arrives() {
    let (mut e, mut c) = RealtimeEngine::new(SR);
    c.play();
    let silent = run(&mut e, &mut c, 4 * CB);
    assert!(silent.audio.left.iter().all(|v| *v == 0.0));
    let score = frozen(&one_shot(bell_score()));
    c.swap(prepare(&score, &TestFactory::new(), &RealtimeOptions::default()).unwrap())
        .unwrap();
    let r = run(&mut e, &mut c, SR as usize);
    assert!(rms(&r.audio.left) > 1e-3);
    let mut out = [0.0f32; 2 * CB];
    let (mut sl, mut sr) = ([0.0f32; CB], [0.0f32; CB]);
    e.process_interleaved(&mut out, 2, &mut sl, &mut sr);
    assert_eq!(out[0].to_bits(), sl[0].to_bits());
    assert_eq!(out[1].to_bits(), sr[0].to_bits());
}

#[test]
fn a_seek_during_the_stop_fade_sets_where_play_resumes() {
    let score = frozen(&one_shot(bell_score()));
    let (mut e, mut c) = engine(&score);
    c.play();
    run(&mut e, &mut c, SR as usize);
    c.stop();
    c.seek(5 * SR as u64);
    let r = run(&mut e, &mut c, 4 * CB);
    let s = r.status.last().unwrap();
    assert!(!s.playing);
    assert_eq!(s.position, 5 * SR as u64);
    c.play();
    let r = run(&mut e, &mut c, CB);
    assert_eq!(r.status.last().unwrap().position, 5 * SR as u64 + CB as u64);
}
