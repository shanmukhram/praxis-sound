//! The Tauri wiring: one `#[tauri::command]` per front-end call, each a thin wrapper over
//! [`crate::commands`] run off the UI thread, plus the take watcher's event.

use crate::commands::{self, CmdError, CmdResult, ExportArgs};
use crate::sounds::{self, SoundInfo};
use crate::watch::{WatchHandle, read_revision, watch};
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;
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
#[tauri::command]
fn watch_sound(app: tauri::AppHandle, state: tauri::State<'_, AppState>, sound: String) -> u64 {
    let path = PathBuf::from(&sound);
    let _ = app.asset_protocol_scope().allow_directory(&path, true);
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
    from
}

#[tauri::command]
async fn live(sound: String) -> CmdResult<Option<commands::Live>> {
    blocking(move || commands::live(&PathBuf::from(sound))).await
}

#[tauri::command]
async fn apply_patch(sound: String, patch: Value) -> CmdResult<commands::Applied> {
    blocking(move || commands::apply_patch(&PathBuf::from(sound), patch)).await
}

#[tauri::command]
async fn undo(sound: String) -> CmdResult<commands::Stepped> {
    blocking(move || commands::undo(&PathBuf::from(sound))).await
}

#[tauri::command]
async fn redo(sound: String) -> CmdResult<commands::Stepped> {
    blocking(move || commands::redo(&PathBuf::from(sound))).await
}

#[tauri::command]
async fn render_live(sound: String) -> CmdResult<Value> {
    blocking(move || commands::render_live(&PathBuf::from(sound))).await
}

#[tauri::command]
async fn export(args: ExportArgs) -> CmdResult<commands::Exported> {
    blocking(move || commands::export(&args)).await
}

/// Starts the desktop app.
pub fn run() {
    let state = AppState::new(sounds::home());
    let root = state.root.clone();
    let result = tauri::Builder::default()
        .manage(state)
        .setup(move |app| {
            std::fs::create_dir_all(&root)?;
            // Takes play from their files through the asset protocol (<audio>, fetch).
            app.asset_protocol_scope().allow_directory(&root, true)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            list_sounds,
            new_sound_path,
            watch_sound,
            live,
            apply_patch,
            undo,
            redo,
            render_live,
            export,
        ])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        eprintln!("praxis-app: {e}");
        std::process::exit(1);
    }
}
