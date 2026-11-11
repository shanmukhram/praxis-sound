//! Watches the open sound's `takes.json` so a take an agent makes (through the MCP server, in
//! another process) appears in the app live.
//!
//! The take ops write `takes.json` atomically and bump its `revision`. Polling the file's
//! modification time twice a second and re-reading `revision` only when it moved is cheap,
//! needs no extra dependency and works the same on every file system.

use crate::sounds::{TAKES_FILE, read_takes_json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

/// The `revision` of a sound's `takes.json`, 0 when there is none yet.
pub fn read_revision(sound: &Path) -> u64 {
    read_takes_json(sound)
        .and_then(|v| v.get("revision").and_then(serde_json::Value::as_u64))
        .unwrap_or(0)
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// Stops its watch thread when dropped.
#[derive(Debug)]
pub struct WatchHandle {
    stop: Arc<AtomicBool>,
    sound: PathBuf,
}

impl WatchHandle {
    pub fn sound(&self) -> &Path {
        &self.sound
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Calls `on_change(revision)` from a background thread whenever `sound/takes.json`'s
/// `revision` differs from the last one seen (starting from `from`).
pub fn watch(
    sound: PathBuf,
    from: u64,
    interval: Duration,
    on_change: impl Fn(u64) + Send + 'static,
) -> WatchHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let handle = WatchHandle {
        stop: stop.clone(),
        sound: sound.clone(),
    };
    let file = sound.join(TAKES_FILE);
    std::thread::Builder::new()
        .name("praxis-takes-watch".into())
        .spawn(move || {
            let mut last_stamp = stamp(&file);
            let mut last_rev = from;
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(interval);
                let now = stamp(&file);
                if now == last_stamp {
                    continue;
                }
                last_stamp = now;
                let rev = read_revision(&sound);
                if rev != last_rev && !stop.load(Ordering::Relaxed) {
                    last_rev = rev;
                    on_change(rev);
                }
            }
        })
        .ok();
    handle
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn reports_a_new_revision_and_stops_when_dropped() {
        let dir = std::env::temp_dir().join(format!("praxis-app-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let write = |rev: u64| {
            let body = format!(
                r#"{{"format":"praxis-takes","version":1,"revision":{rev},"current":0,"takes":[]}}"#
            );
            std::fs::write(dir.join(TAKES_FILE), body).unwrap();
        };
        write(1);
        assert_eq!(read_revision(&dir), 1);
        let (tx, rx) = mpsc::channel();
        let handle = watch(dir.clone(), 1, Duration::from_millis(20), move |r| {
            let _ = tx.send(r);
        });
        // Same revision rewritten: no event. A new revision: one event.
        std::thread::sleep(Duration::from_millis(60));
        write(1);
        std::thread::sleep(Duration::from_millis(60));
        write(2);
        let got = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(got, 2);
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        drop(handle);
        std::thread::sleep(Duration::from_millis(60));
        write(3);
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
