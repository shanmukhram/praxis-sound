//! The Tauri wiring: one `#[tauri::command]` per front-end call, each a thin wrapper over
//! [`crate::commands`] run off the UI thread, plus the take watcher's event, and the transport
//! commands over the real-time [`AudioService`].

use crate::audio::{
    AudioService, CompareLevel, FakeOutput, LayerLive, Opener, TRANSPORT_EVENT, TransportStatus,
    cpal_opener,
};
use crate::clips::{self, Clip};
use crate::commands::{self, AGENT_EVENT, AgentProgress, CmdError, CmdResult, ExportArgs};
use crate::hand::{self, MakeTakeArgs, NewSound, NewSoundArgs};
use crate::sounds::{self, SoundInfo};
use crate::watch::{WatchHandle, read_revision, watch};
use praxis_cli::agent::{AgentEvent, Progress};
use praxis_cli::ops;
use praxis_score::BarRange;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager};

/// Event sent when the open sound's `takes.json` gets a new revision (a take by anyone).
pub const TAKES_EVENT: &str = "praxis://takes";

/// How often the watcher looks at `takes.json`.
const WATCH_INTERVAL: Duration = Duration::from_millis(400);

pub struct AppState {
    pub home: PathBuf,
    pub root: PathBuf,
    watching: Mutex<Option<WatchHandle>>,
}

impl AppState {
    pub fn new(home: PathBuf) -> AppState {
        let root = sounds::sounds_root(&home);
        AppState {
            home,
            root,
            watching: Mutex::new(None),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct TakesChanged {
    sound: String,
    revision: u64,
}

#[derive(Clone, Debug, Serialize)]
struct AppInfo {
    version: &'static str,
    home: String,
    sounds_root: String,
    packs: Option<String>,
}

/// Runs `f` on the blocking pool: renders take seconds and must not stall the WebView.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> CmdResult<T> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| CmdError::new(praxis_cli::ErrorCode::Error, e.to_string()))?
}

#[tauri::command]
fn app_info(state: tauri::State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        home: state.home.display().to_string(),
        sounds_root: state.root.display().to_string(),
        packs: std::env::var("PRAXIS_PACKS").ok(),
    }
}

#[tauri::command]
async fn list_sounds(state: tauri::State<'_, AppState>) -> CmdResult<Vec<SoundInfo>> {
    let root = state.root.clone();
    blocking(move || Ok(commands::sounds(&root))).await
}

/// The folder a new sound started by `text` will live in (not created until its first take).
#[tauri::command]
fn new_sound_path(state: tauri::State<'_, AppState>, text: String) -> String {
    sounds::new_sound_path(&state.root, &text)
        .display()
        .to_string()
}

/// Watches `sound` (and stops watching the previous one); returns its current revision.
/// Refuses a folder that is not a sound; the WebView may then load only the sound's audio
/// subfolders (`commands::audio_dirs`), never the folder itself or anything above it.
#[tauri::command]
fn watch_sound(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    sound: String,
) -> CmdResult<u64> {
    let dirs = commands::audio_dirs(&sound)?;
    let scope = app.asset_protocol_scope();
    for d in &dirs {
        scope
            .allow_directory(d, true)
            .map_err(|e| CmdError::new(praxis_cli::ErrorCode::Error, e.to_string()))?;
    }
    let path = PathBuf::from(&sound);
    let from = read_revision(&path);
    let emitter = app.clone();
    let name = sound.clone();
    let handle = watch(path, from, WATCH_INTERVAL, move |revision| {
        let _ = emitter.emit(
            TAKES_EVENT,
            TakesChanged {
                sound: name.clone(),
                revision,
            },
        );
    });
    if let Ok(mut w) = state.watching.lock() {
        *w = Some(handle);
    }
    Ok(from)
}

// ---------------------------------------------------------------- takes (praxis_cli::ops)

/// A request on a sound. While the sound assistant works, each of its steps goes to the window
/// as an [`AGENT_EVENT`] (`{sound, event: "step", n, tool, words}`, ...), so the thread shows
/// "Reading the sound", "Rendering", "Checking: 8 of 8 passed" under the request.
#[tauri::command]
async fn ask(app: tauri::AppHandle, req: ops::AskRequest) -> CmdResult<Value> {
    let progress = agent_progress(app, req.sound.clone());
    blocking(move || commands::ask_with(req, Some(progress), None)).await
}

/// The progress callback that sends each step to the window. Emitting only queues the message,
/// so it never blocks the turn.
fn agent_progress(app: tauri::AppHandle, sound: String) -> Progress {
    Arc::new(move |e: &AgentEvent| {
        let _ = app.emit(
            AGENT_EVENT,
            AgentProgress {
                sound: sound.clone(),
                event: e.clone(),
            },
        );
    })
}

#[tauri::command]
async fn takes(sound: String) -> CmdResult<Value> {
    blocking(move || commands::takes(&sound)).await
}

#[tauri::command]
async fn restore(req: ops::RestoreRequest) -> CmdResult<Value> {
    blocking(move || commands::restore(&req)).await
}

#[tauri::command]
async fn edit(req: ops::EditRequest) -> CmdResult<Value> {
    blocking(move || commands::edit(&req)).await
}

#[tauri::command]
async fn variations(req: ops::VariationsRequest) -> CmdResult<Value> {
    blocking(move || commands::variations(&req)).await
}

#[tauri::command]
async fn keep_variation(req: ops::KeepVariationRequest) -> CmdResult<Value> {
    blocking(move || commands::keep_variation(req)).await
}

#[tauri::command]
async fn open_view(req: ops::OpenViewRequest) -> CmdResult<Value> {
    blocking(move || commands::open_view(&req)).await
}

// ---------------------------------------------------------------- building by hand

/// A blank sound (key, tempo or length, metre, bars, loop or one-shot) with no take yet, in a new
/// folder under the sounds root.
#[tauri::command]
async fn new_sound(state: tauri::State<'_, AppState>, args: NewSoundArgs) -> CmdResult<NewSound> {
    let root = state.root.clone();
    blocking(move || hand::new_sound(&root, &args)).await
}

/// Renders, checks and records the live score as one take (the hand edits since the last take).
#[tauri::command]
async fn make_take(req: MakeTakeArgs) -> CmdResult<Value> {
    blocking(move || hand::make_take(&req)).await
}

/// The instrument presets a layer can use, and whether each renders here.
#[tauri::command]
async fn presets() -> CmdResult<Value> {
    blocking(hand::presets).await
}

// ---------------------------------------------------------------- reference clips

/// A dropped clip: its bytes as the request body, its file name (percent-encoded) in the
/// `x-clip-name` header. Kept under the home folder (for the assistant) and measured: the
/// clip's path and its Sound Profile.
#[tauri::command]
async fn clip_analyze(
    state: tauri::State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> CmdResult<Clip> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(CmdError::new(
            praxis_cli::ErrorCode::BadJson,
            "send the clip's bytes as the request body",
        ));
    };
    let name = request
        .headers()
        .get(clips::NAME_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(clips::percent_decode)
        .unwrap_or_default();
    let bytes = bytes.clone();
    let home = state.home.clone();
    blocking(move || clips::analyze_clip(&home, &name, &bytes)).await
}

// ---------------------------------------------------------------- the project (no take)

#[tauri::command]
async fn apply_patch(sound: String, patch: Value) -> CmdResult<commands::Applied> {
    blocking(move || commands::apply_patch(&sound, patch)).await
}

#[tauri::command]
async fn undo(sound: String) -> CmdResult<commands::Stepped> {
    blocking(move || commands::undo(&sound)).await
}

#[tauri::command]
async fn redo(sound: String) -> CmdResult<commands::Stepped> {
    blocking(move || commands::redo(&sound)).await
}

#[tauri::command]
async fn render_live(sound: String) -> CmdResult<Value> {
    blocking(move || commands::render_live(&sound)).await
}

#[tauri::command]
async fn export(args: ExportArgs) -> CmdResult<commands::Exported> {
    blocking(move || commands::export(&args)).await
}

// ---------------------------------------------------------------- transport (real time)

/// The live score of `sound` changed (or `None`: no sound is open). Plays it from now on: a
/// mute / solo / gain change goes in live, anything else is rebuilt and swapped in.
#[tauri::command]
async fn transport_sync(
    audio: tauri::State<'_, AudioService>,
    sound: Option<String>,
) -> CmdResult<TransportStatus> {
    match sound {
        None => audio.clear(),
        Some(sound) => {
            let (score, gain) = blocking(move || commands::playback_score(&sound)).await?;
            audio.load(score, gain);
        }
    }
    Ok(audio.status())
}

#[tauri::command]
fn transport_play(audio: tauri::State<'_, AudioService>) -> TransportStatus {
    audio.play();
    audio.status()
}

#[tauri::command]
fn transport_stop(audio: tauri::State<'_, AudioService>) {
    audio.stop();
}

#[tauri::command]
fn transport_seek(audio: tauri::State<'_, AudioService>, seconds: f64) {
    audio.seek(seconds);
}

#[tauri::command]
fn transport_set_loop(audio: tauri::State<'_, AudioService>, range: Option<BarRange>) {
    audio.set_loop(range);
}

/// A live layer override (trim dB, mute, solo) that is heard at once and not saved.
#[tauri::command]
fn transport_layer(audio: tauri::State<'_, AudioService>, layer: String, live: LayerLive) {
    audio.set_layer(&layer, live);
}

/// The take's side of a loudness-matched A/B with a reference clip: a master trim (or
/// silence) while comparing; `null` plays the take at its own level again. Not saved.
#[tauri::command]
fn transport_compare(audio: tauri::State<'_, AudioService>, level: Option<CompareLevel>) {
    audio.set_compare(level);
}

#[tauri::command]
fn transport_status(audio: tauri::State<'_, AudioService>) -> TransportStatus {
    audio.status()
}

/// Environment switch for a silent run: `PRAXIS_AUDIO_OUTPUT=fake` plays through the fake output
/// (the same real-time engine and command path, paced by the clock, 48 kHz in 480-frame
/// callbacks like the Windows device) instead of the speakers.
const OUTPUT_ENV: &str = "PRAXIS_AUDIO_OUTPUT";

/// The output the audio service opens: the default device, or the fake output when asked.
fn output_opener() -> Opener {
    match std::env::var(OUTPUT_ENV) {
        Ok(v) if v.trim().eq_ignore_ascii_case("fake") => FakeOutput {
            sample_rate: 48_000,
            callback_frames: 480,
            speed: 1.0,
            tap: None,
            xruns: None,
        }
        .opener(),
        _ => cpal_opener(),
    }
}

/// The home folder the app can write its sounds into: `preferred` when its `sounds` folder can be
/// created, else `fallback` (the local app-data folder, which Windows never protects), else
/// `preferred` anyway. Never fails: a folder that cannot be created must not stop the window
/// from opening (Controlled Folder Access, for one, reports a blocked write as "file not
/// found"); sound commands then report the error in words.
pub fn writable_home(preferred: PathBuf, fallback: Option<PathBuf>) -> PathBuf {
    match std::fs::create_dir_all(sounds::sounds_root(&preferred)) {
        Ok(()) => return preferred,
        Err(e) => eprintln!(
            "praxis-app: cannot create {}: {e}",
            sounds::sounds_root(&preferred).display()
        ),
    }
    if let Some(f) = fallback {
        match std::fs::create_dir_all(sounds::sounds_root(&f)) {
            Ok(()) => {
                eprintln!("praxis-app: keeping sounds in {} instead", f.display());
                return f;
            }
            Err(e) => eprintln!(
                "praxis-app: cannot create {} either: {e}",
                sounds::sounds_root(&f).display()
            ),
        }
    }
    preferred
}

/// Starts the desktop app.
pub fn run() {
    let fallback = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Praxis Sound"));
    let state = AppState::new(writable_home(sounds::home(), fallback));
    let result = tauri::Builder::default()
        .manage(state)
        .setup(move |app| {
            // The real-time engine on the default output, with the instruments the renders use.
            let emitter = app.handle().clone();
            let audio = AudioService::start(
                output_opener(),
                Arc::new(praxis_instruments::Registry::with_default_packs()),
                Box::new(move |ev| {
                    let _ = emitter.emit(TRANSPORT_EVENT, ev);
                }),
            );
            app.manage(audio);
            // The asset protocol starts with an empty scope: `watch_sound` grants each opened
            // sound's audio folders, and nothing else, so takes play in <audio> and fetch.
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            list_sounds,
            new_sound_path,
            watch_sound,
            ask,
            takes,
            restore,
            edit,
            variations,
            keep_variation,
            open_view,
            new_sound,
            make_take,
            presets,
            apply_patch,
            undo,
            redo,
            render_live,
            export,
            transport_sync,
            transport_play,
            transport_stop,
            transport_seek,
            transport_set_loop,
            transport_layer,
            transport_status,
            transport_compare,
            clip_analyze,
        ])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        eprintln!("praxis-app: {e}");
        std::process::exit(1);
    }
}
