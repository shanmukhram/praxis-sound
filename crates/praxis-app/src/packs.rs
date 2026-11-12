//! The instrument pack downloader in the window: `packs_list` and `packs_install` over the same
//! command layer as `praxis packs list|install` (`praxis_cli::ops::packs`), so a pack installed
//! here is checked exactly as the CLI checks it (the index's size and blake3, then unpacked
//! through a staging folder).
//!
//! The pack installer has no progress callback, so progress is read from the outside: while it
//! downloads it writes `<packs>/_downloads/<archive>.part`, whose size a watcher thread reports
//! as `praxis://packs` events about five times a second.

use crate::commands::{CmdError, CmdResult};
use praxis_cli::ErrorCode;
use praxis_cli::ops;
use praxis_instruments::Packs;
use praxis_instruments::sampler::index::PackIndex;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::Emitter;

/// Progress of one install.
pub const PACKS_EVENT: &str = "praxis://packs";

/// How often the watcher looks at the partial download.
const POLL: Duration = Duration::from_millis(200);

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PackProgress {
    pub pack: String,
    /// `downloading`, `unpacking`, `done` or `failed`.
    pub phase: &'static str,
    pub bytes: u64,
    pub total: u64,
    /// The error in words, for `failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// `ops::packs_list` (every pack in `packs/index.json`, installed or not).
pub fn list() -> CmdResult<Value> {
    Ok(ops::packs_list()?)
}

/// Where the installer writes the partial download of `name`, and the archive's size.
fn partial_download(name: &str) -> Option<(PathBuf, u64)> {
    let root = Packs::discover().root?;
    let index = PackIndex::parse(&std::fs::read_to_string(root.join("index.json")).ok()?).ok()?;
    let entry = index.find(name)?;
    let archive = root.join("_downloads").join(&entry.archive);
    // `download` writes through `dest.with_extension("part")`.
    Some((archive.with_extension("part"), entry.size))
}

/// Installs `name` like `praxis packs install`, calling `progress` while it works and once at
/// the end (`done` or `failed`).
pub fn install(
    name: &str,
    progress: impl Fn(PackProgress) + Send + Sync + 'static,
) -> CmdResult<Value> {
    let progress = Arc::new(progress);
    let download = partial_download(name);
    let total = download.as_ref().map_or(0, |d| d.1);
    let finished = Arc::new(AtomicBool::new(false));
    // An unknown pack has nothing to watch; the install below says why it failed.
    let watcher = download.and_then(|(part, total)| {
        let (finished, progress, pack) = (finished.clone(), progress.clone(), name.to_string());
        std::thread::Builder::new()
            .name("pack-progress".into())
            .spawn(move || {
                let mut seen_part = false;
                while !finished.load(Ordering::Acquire) {
                    match std::fs::metadata(&part) {
                        Ok(m) => {
                            seen_part = true;
                            progress(PackProgress {
                                pack: pack.clone(),
                                phase: "downloading",
                                bytes: m.len(),
                                total,
                                message: None,
                            });
                        }
                        // No partial file: not started yet, reusing a verified archive, or the
                        // download is complete and the archive is being checked and unpacked.
                        Err(_) if seen_part => progress(PackProgress {
                            pack: pack.clone(),
                            phase: "unpacking",
                            bytes: total,
                            total,
                            message: None,
                        }),
                        Err(_) => {}
                    }
                    std::thread::sleep(POLL);
                }
            })
            .ok()
    });
    let result = ops::packs_install(&ops::PacksInstallRequest {
        name: name.to_string(),
        archive: None,
    });
    finished.store(true, Ordering::Release);
    if let Some(w) = watcher {
        let _ = w.join();
    }
    match result {
        Ok(v) => {
            progress(PackProgress {
                pack: name.to_string(),
                phase: "done",
                bytes: total,
                total,
                message: None,
            });
            Ok(v)
        }
        Err(e) => {
            progress(PackProgress {
                pack: name.to_string(),
                phase: "failed",
                bytes: 0,
                total,
                message: Some(e.message.clone()),
            });
            Err(e.into())
        }
    }
}

#[tauri::command]
pub fn packs_list() -> CmdResult<Value> {
    list()
}

/// Downloads, verifies and unpacks one pack off the UI thread, emitting [`PACKS_EVENT`].
#[tauri::command]
pub async fn packs_install(app: tauri::AppHandle, name: String) -> CmdResult<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        install(&name, move |p| {
            let _ = app.emit(PACKS_EVENT, p);
        })
    })
    .await
    .map_err(|e| CmdError::new(ErrorCode::Error, e.to_string()))?
}
