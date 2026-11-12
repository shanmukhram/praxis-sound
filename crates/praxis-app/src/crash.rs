//! Local crash reports for the desktop app (`praxis_cli::ops::crash`): the panic hook writes to
//! `<home>/logs`, and on the next start the window shows reports it has not shown yet, with where
//! the file is and how to send it. Nothing is uploaded.

use crate::shell::AppState;
use praxis_cli::ops::crash::{self, CrashReports};
use std::path::PathBuf;

/// Installs the crash hook, writing under the app's home folder.
pub fn install(home: PathBuf) {
    crash::install_in("praxis-app", home);
}

/// The app's crash reports, newest first; `seen: true` marks them all as shown.
#[tauri::command]
pub fn crash_reports(state: tauri::State<'_, AppState>, seen: Option<bool>) -> CrashReports {
    if seen == Some(true) {
        let _ = crash::mark_seen(&state.home);
    }
    crash::reports(&state.home)
}
