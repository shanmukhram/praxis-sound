//! The app's audio service, driven exactly as the Tauri commands drive it, on a fake output (a
//! thread calling the real-time engine in 480-frame callbacks, recording what it plays) and, with
//! `PRAXIS_DEVICE_TEST=1`, on the default output device.
//!
//! - a one-shot plays sample for sample like its offline render;
//! - mute and solo from a new live score go in live (no rebuild), heard within one ramp;
//! - a new score is swapped in while playing without a click;
//! - seek and a loop range keep the position inside the range;
//! - no output device is reported plainly and nothing panics;
//! - a real sound made by the command layer (felt piano, `PRAXIS_PACKS`) plays through it.

use praxis_app::audio::{AudioService, FakeOutput, LayerLive, Tap, TransportEvent, cpal_opener};
use praxis_engine::testing::TestFactory;
use praxis_engine::{RenderOptions, render_offline};
use praxis_score::{BarRange, Playback, Score};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SR: u32 = 48_000;
const CALLBACK: usize = 480;

fn bell_score() -> Score {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/acceptance1_bell.score.json");
    let text = std::fs::read_to_string(&path).expect("fixture");
    praxis_score::from_json(&text).expect("valid fixture")
}

fn one_shot(mut s: Score) -> Score {
    s.playback = Playback::OneShot { tail_seconds: 2.0 };
    s
}

/// The offline render of `score` and the master gain it applied.
fn offline(score: &Score) -> (Vec<f32>, Vec<f32>, f64) {
    let out = render_offline(
        score,
        &TestFactory::new(),
        &RenderOptions {
            sample_rate: SR,
            ..RenderOptions::default()
        },
    )
    .expect("render");
    (
        out.master.left.clone(),
        out.master.right.clone(),
        out.report.applied_gain_db,
    )
}

type Events = Arc<Mutex<Vec<TransportEvent>>>;

struct Rig {
    service: AudioService,
    tap: Tap,
    events: Events,
}

fn rig(speed: f64) -> Rig {
    let tap: Tap = Arc::new(Mutex::new((Vec::new(), Vec::new())));
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let service = AudioService::start(
        FakeOutput {
            sample_rate: SR,
            callback_frames: CALLBACK,
            speed,
            tap: Some(tap.clone()),
        }
        .opener(),
        Arc::new(TestFactory::new()),
        Box::new(move |e| ev.lock().unwrap().push(e.clone())),
    );
    Rig {
        service,
        tap,
        events,
    }
}

fn wait_for(what: &str, secs: f64, mut ok: impl FnMut() -> bool) {
    let t0 = Instant::now();
    while !ok() {
        assert!(
            t0.elapsed().as_secs_f64() < secs,
            "timed out after {secs} s waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

impl Rig {
    fn loaded(&self, swaps: u64) {
        wait_for("the graph", 30.0, || {
            let s = self.service.status();
            s.loaded && s.event.swaps >= swaps && !s.event.preparing
        });
    }

    fn frames(&self) -> usize {
        self.tap.lock().unwrap().0.len()
    }

    /// Recorded audio from frame `from` on.
    fn audio_from(&self, from: usize) -> (Vec<f32>, Vec<f32>) {
        let t = self.tap.lock().unwrap();
        (
            t.0[from.min(t.0.len())..].to_vec(),
            t.1[from.min(t.1.len())..].to_vec(),
        )
    }

    fn wait_frames(&self, n: usize) {
        let target = self.frames() + n;
        wait_for("audio", 60.0, || self.frames() >= target);
    }

    fn last_event(&self) -> Option<TransportEvent> {
        self.events.lock().unwrap().last().cloned()
    }
}

fn first_sound(x: &[f32]) -> usize {
    x.iter().position(|v| *v != 0.0).unwrap_or(x.len())
}

fn max_step(x: &[f32]) -> f32 {
    x.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max)
}

fn rms(x: &[f32]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
}

#[test]
fn a_one_shot_plays_sample_for_sample_like_its_offline_render() {
    let score = one_shot(bell_score());
    let (left, right, gain) = offline(&score);
    let r = rig(40.0);
    r.service.load(score, Some(gain));
    r.loaded(1);
    let start = r.frames();
    r.service.play();
    wait_for("the one-shot to end", 60.0, || {
        r.last_event().is_some_and(|e| e.ended && !e.playing)
    });
    let (l, rr) = r.audio_from(start);
    let (a, b) = (first_sound(&l), first_sound(&left));
    assert!(a >= b, "real time sounded before the render did");
    let off = a - b;
    // Equal except the last limiter lookahead (docs: realtime/mod.rs "Equivalence").
    let n = left.len() - (SR as usize / 100);
    assert!(l.len() >= off + n, "recorded {} frames", l.len());
    let same_l = l[off..off + n]
        .iter()
        .zip(&left[..n])
        .all(|(x, y)| x.to_bits() == y.to_bits());
    let same_r = rr[off..off + n]
        .iter()
        .zip(&right[..n])
        .all(|(x, y)| x.to_bits() == y.to_bits());
    assert!(
        same_l && same_r,
        "real time differs from the offline render"
    );
    let ev = r.last_event().unwrap();
    assert!(ev.available);
    assert_eq!(ev.xruns, 0);
    assert_eq!(ev.min_callback_frames, CALLBACK as u64);
}

#[test]
fn mute_and_solo_in_a_new_live_score_go_in_live_without_a_rebuild() {
    let score = bell_score(); // playback: loop
    let (_, _, gain) = offline(&score);
    let r = rig(8.0);
    r.service.load(score.clone(), Some(gain));
    r.loaded(1);
    r.service.play();
    r.wait_frames(SR as usize);
    let before = r.audio_from(r.frames() - SR as usize / 2);
    assert!(rms(&before.0) > 1e-3, "the loop is not sounding");

    // Every layer muted in the score (as a mute patch saves it): live commands, no new graph.
    let mut muted = score.clone();
    for l in &mut muted.layers {
        l.muted = true;
    }
    r.service.load(muted, Some(gain));
    let s = r.service.status();
    assert!(s.loaded && !s.event.preparing, "a mute must not rebuild");
    let at = r.frames();
    r.wait_frames(SR as usize / 2);
    // The 10 ms ramp plus the limiter lookahead, then silence.
    let (l, _) = r.audio_from(at + CALLBACK * 2 + SR as usize / 20);
    assert!(
        rms(&l) < 1e-6,
        "still sounding after the mute: rms {}",
        rms(&l)
    );
    assert_eq!(r.service.status().event.swaps, 1);

    // A live override on top of the score: the piano (bars 1-16; the kick only plays in bars
    // 9-16) unmuted and soloed live is heard again, with no rebuild.
    r.service.set_layer(
        &score.layers[0].id,
        LayerLive {
            muted: Some(false),
            solo: Some(true),
            gain_db: None,
        },
    );
    let at = r.frames();
    r.wait_frames(SR as usize);
    let (l, _) = r.audio_from(at + SR as usize / 10);
    assert!(rms(&l) > 1e-4, "the live-unmuted piano is not heard");
    assert_eq!(r.service.status().event.swaps, 1);
}

#[test]
fn a_new_score_is_swapped_in_while_playing_without_a_click() {
    let score = bell_score();
    let (al, _, gain) = offline(&score);
    let mut edited = score.clone();
    edited.layers[0].gain_db -= 4.0;
    edited.layers[0].pan = -0.6; // not live: needs a rebuild
    let (bl, _, _) = offline(&edited);
    let r = rig(8.0);
    r.service.load(score, Some(gain));
    r.loaded(1);
    r.service.play();
    r.wait_frames(SR as usize * 2);
    let at = r.frames();
    r.service.load(edited, Some(gain));
    r.loaded(2);
    r.wait_frames(SR as usize);
    let (l, _) = r.audio_from(at.saturating_sub(SR as usize));
    let limit = max_step(&al).max(max_step(&bl)) * 1.25 + 1e-3;
    let got = max_step(&l);
    assert!(
        got <= limit,
        "a jump of {got} around the swap (renders jump at most {limit})"
    );
    assert!(rms(&l) > 1e-3);
    let ev = r.last_event().unwrap();
    assert!(ev.playing && ev.swaps == 2 && ev.xruns == 0);
}

#[test]
fn seek_and_a_loop_range_keep_the_position_in_the_range() {
    let score = bell_score(); // 128 BPM, 4/4: a bar is 1.875 s
    let (_, _, gain) = offline(&score);
    let bar = (SR as f64 * 1.875) as u64;
    let r = rig(10.0);
    r.service.load(score, Some(gain));
    r.loaded(1);
    r.service.set_loop(Some(BarRange { start: 5, end: 6 }));
    r.service.seek(4.0 * 1.875); // bar 5
    r.service.play();
    r.wait_frames(SR as usize * 6); // three passes of the two-bar range
    r.service.stop();
    wait_for("stop", 10.0, || r.last_event().is_some_and(|e| !e.playing));
    let events = r.events.lock().unwrap().clone();
    let playing: Vec<_> = events.iter().filter(|e| e.playing).collect();
    assert!(playing.len() > 10, "{} position events", playing.len());
    for e in &playing {
        assert!(
            e.position >= 4 * bar - 1 && e.position <= 6 * bar + 1,
            "position {} outside bars 5-6",
            e.position
        );
    }
    let late = playing
        .iter()
        .position(|e| e.position > 5 * bar + bar / 2)
        .expect("never reached the second half of bar 6");
    assert!(
        playing[late..]
            .iter()
            .any(|e| e.position < 4 * bar + bar / 2),
        "never wrapped back to bar 5"
    );
    let s = r.service.status();
    assert_eq!(s.loop_range, Some(BarRange { start: 5, end: 6 }));
}

#[test]
fn no_output_device_is_reported_plainly() {
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let service = AudioService::start(
        Box::new(|| Err("no default output device".into())),
        Arc::new(TestFactory::new()),
        Box::new(move |e| ev.lock().unwrap().push(e.clone())),
    );
    service.load(bell_score(), None);
    service.play();
    service.seek(3.0);
    service.set_loop(Some(BarRange { start: 1, end: 2 }));
    service.stop();
    let s = service.status();
    assert!(!s.event.available);
    assert!(!s.loaded);
    let err = s.event.error.unwrap_or_default();
    assert!(err.contains("No audio output"), "{err}");
    wait_for("an event", 5.0, || !events.lock().unwrap().is_empty());
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .all(|e| !e.available && !e.playing)
    );
}

// ---------------------------------------------------------------- with real instruments

fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("praxis-app-audio-{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::path::absolute(dir).unwrap()
}

fn have_piano(test: &str) -> bool {
    let ok = praxis_instruments::Packs::discover().has_pack("upright-piano-kw");
    if !ok {
        println!("SKIP {test}: the upright-piano-kw pack is not installed (set PRAXIS_PACKS)");
    }
    ok
}

/// A real sound folder (take 1 by the offline planner, felt piano), as the app makes it.
fn real_sound(test: &str) -> String {
    let sound = scratch(test).join("felt");
    let sound = sound.display().to_string();
    praxis_app::commands::ask(praxis_cli::ops::AskRequest {
        sound: sound.clone(),
        text: "a calm 30-second felt piano loop in A minor".into(),
        who: Some("You".into()),
        scope: None,
        patch: None,
        title: None,
        provider: Some(praxis_cli::ops::ProviderChoice::Offline),
        model: None,
        seed: None,
        skip_missing: false,
        include_score: false,
    })
    .expect("take 1");
    sound
}

#[test]
fn a_real_take_plays_through_the_service_with_the_felt_piano() {
    if !have_piano("a_real_take_plays_through_the_service_with_the_felt_piano") {
        return;
    }
    let sound = real_sound("real-take");
    let (score, gain) = praxis_app::commands::playback_score(&sound).expect("live score");
    assert!(gain.is_some(), "the take's render applied a gain");
    let tap: Tap = Arc::new(Mutex::new((Vec::new(), Vec::new())));
    let service = AudioService::start(
        FakeOutput {
            sample_rate: SR,
            callback_frames: CALLBACK,
            speed: 4.0,
            tap: Some(tap.clone()),
        }
        .opener(),
        Arc::new(praxis_instruments::Registry::with_default_packs()),
        Box::new(|_| {}),
    );
    service.load(score, gain);
    wait_for("the felt piano graph", 120.0, || service.status().loaded);
    let s = service.status();
    println!(
        "prepared in {:?} ms; warnings {:?}",
        s.prepare_ms, s.warnings
    );
    assert!(s.event.error.is_none(), "{:?}", s.event.error);
    service.play();
    let start = tap.lock().unwrap().0.len();
    wait_for("4 s of audio", 60.0, || {
        tap.lock().unwrap().0.len() >= start + 4 * SR as usize
    });
    let t = tap.lock().unwrap();
    let peak = t.0[start..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.01 && peak <= 1.0, "peak {peak}");
}

/// The default output device: `PRAXIS_DEVICE_TEST=1 cargo test -p praxis-app --test audio device`.
#[test]
fn device_plays_without_xruns() {
    if std::env::var("PRAXIS_DEVICE_TEST").as_deref() != Ok("1") {
        println!("SKIP device_plays_without_xruns: set PRAXIS_DEVICE_TEST=1 to use the speakers");
        return;
    }
    let score = bell_score();
    let (_, _, gain) = offline(&score);
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let service = AudioService::start(
        cpal_opener(),
        Arc::new(TestFactory::new()),
        Box::new(move |e| ev.lock().unwrap().push(e.clone())),
    );
    let s = service.status();
    assert!(s.event.available, "no device: {:?}", s.event.error);
    service.load(score, Some(gain));
    wait_for("the graph", 30.0, || service.status().loaded);
    service.play();
    std::thread::sleep(Duration::from_secs(4));
    service.stop();
    std::thread::sleep(Duration::from_millis(200));
    let s = service.status();
    let evs = events.lock().unwrap();
    let peak = evs.iter().map(|e| e.peak_db[0]).fold(f32::MIN, f32::max);
    println!(
        "device {:?} {} Hz, requested {:?}, callbacks {}..{} frames x {}, xruns {}, late wakeups {}, errors {}, render p99 {:.3} ms max {:.3} ms, peak {peak:.1} dBFS, {} events",
        s.event.device,
        s.event.sample_rate,
        s.requested_buffer_frames,
        s.event.min_callback_frames,
        s.event.max_callback_frames,
        s.event.callbacks,
        s.event.xruns,
        s.event.late_wakeups,
        s.event.device_errors,
        s.p99_render_ms,
        s.max_render_ms,
        evs.len()
    );
    assert!(s.event.callbacks > 100);
    assert_eq!(s.event.xruns, 0);
    assert!(peak > -60.0, "nothing was heard: peak {peak} dBFS");
}
