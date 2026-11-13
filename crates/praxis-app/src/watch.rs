//! Watches the open sound's `takes.json` so a take an agent makes (through the MCP server, in
//! another process) appears in the app live.
//!
//! The take ops write `takes.json` atomically and bump its `revision`. Polling the file's
//! modification time twice a second and re-reading `revision` only when it moved is cheap,
//! needs no extra dependency and works the same on every file system.
//!
//! Another program may write the file in place, so a poll can find it empty or half-written.
//! Such a file is not a revision: the watcher reports nothing and reads it again on the next
//! poll.

use crate::sounds::TAKES_FILE;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

/// The `revision` of a sound's `takes.json`, 0 when there is none yet.
pub fn read_revision(sound: &Path) -> u64 {
    try_read_revision(sound).unwrap_or(0)
}

/// The `revision` of a sound's `takes.json`: 0 when there is no file yet, `None` when there is
/// a file that cannot be read as takes right now (empty, half-written, no `revision`, or held
/// by its writer).
fn try_read_revision(sound: &Path) -> Option<u64> {
    match std::fs::read_to_string(sound.join(TAKES_FILE)) {
        Ok(text) => serde_json::from_str::<serde_json::Value>(&text)
            .ok()?
            .get("revision")?
            .as_u64(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(0),
        Err(_) => None,
    }
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
            // The file's stamp at the last poll that read a revision. `None` until there has
            // been one, so the first poll reads the file whatever happened to it since `from`.
            let mut read_at: Option<Option<(SystemTime, u64)>> = None;
            let mut last_rev = from;
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(interval);
                let now = stamp(&file);
                if read_at == Some(now) {
                    continue;
                }
                // The stamp is taken before the read, so a write that lands in between is
                // seen again on the next poll.
                let Some(rev) = try_read_revision(&sound) else {
                    continue;
                };
                read_at = Some(now);
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
    use praxis_cli::ops::sound::write_atomic;
    use std::sync::mpsc;

    fn temp_sound(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("praxis-app-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn takes_body(rev: u64) -> String {
        format!(
            r#"{{"format":"praxis-takes","version":1,"revision":{rev},"current":0,"takes":[]}}"#
        )
    }

    /// Writes `takes.json` the way the take ops do: a whole temporary file, renamed over it.
    fn write_takes(sound: &Path, rev: u64) {
        write_atomic(&sound.join(TAKES_FILE), takes_body(rev).as_bytes()).unwrap();
    }

    #[test]
    fn reports_a_new_revision_and_stops_when_dropped() {
        let dir = temp_sound("revision");
        write_takes(&dir, 1);
        assert_eq!(read_revision(&dir), 1);
        let (tx, rx) = mpsc::channel();
        let handle = watch(dir.clone(), 1, Duration::from_millis(20), move |r| {
            let _ = tx.send(r);
        });
        // Same revision rewritten: no event. A new revision: one event.
        std::thread::sleep(Duration::from_millis(60));
        write_takes(&dir, 1);
        std::thread::sleep(Duration::from_millis(60));
        write_takes(&dir, 2);
        let got = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(got, 2);
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        drop(handle);
        std::thread::sleep(Duration::from_millis(60));
        write_takes(&dir, 3);
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_half_written_file_is_not_a_revision() {
        let dir = temp_sound("half-written");
        let file = dir.join(TAKES_FILE);
        assert_eq!(try_read_revision(&dir), Some(0), "no file yet");
        write_takes(&dir, 1);
        assert_eq!(try_read_revision(&dir), Some(1));
        let (tx, rx) = mpsc::channel();
        let _handle = watch(dir.clone(), 1, Duration::from_millis(20), move |r| {
            let _ = tx.send(r);
        });
        // What a poll can find while another program writes the file in place: nothing yet,
        // then the first half. Then two files that are whole but hold no revision.
        let whole = takes_body(2);
        let unreadable = [
            "",
            &whole[..whole.len() / 2],
            r#"{"format":"praxis-takes","version":1,"takes":[]}"#,
            r#"{"revision":"2"}"#,
        ];
        for body in unreadable {
            std::fs::write(&file, body).unwrap();
            assert_eq!(try_read_revision(&dir), None, "{body:?}");
            assert_eq!(read_revision(&dir), 0, "{body:?}");
            assert!(
                rx.recv_timeout(Duration::from_millis(100)).is_err(),
                "{body:?} was reported as a revision"
            );
        }
        // The write completes: that revision is reported, once.
        std::fs::write(&file, &whole).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
