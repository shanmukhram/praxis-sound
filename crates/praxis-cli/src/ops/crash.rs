//! Local crash reports. [`install`] adds a panic hook that writes a plain-text report to
//! `PRAXIS_HOME/logs/crash-<UTC time>-<pid>.txt` (falling back to `%LOCALAPPDATA%\Praxis
//! Sound\logs` when that folder cannot be created) and then runs the default hook, so the
//! message still reaches stderr. Nothing is ever uploaded: the report says where it is and how
//! to send it by hand.
//!
//! A report holds the program and version, OS, thread, panic location, message and a backtrace
//! (release builds keep line tables). Anything shaped like an Anthropic API key is blanked
//! before writing. Only the newest [`KEEP`] reports are kept.
//!
//! The CLI, the MCP server and the desktop app all install it; the app lists reports it has not
//! shown yet with [`reports`] and marks them seen with [`mark_seen`].

use crate::ops::praxis_home;
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Reports kept; older ones are deleted when a new one is written.
pub const KEEP: usize = 20;
/// File in the logs folder naming the newest report the user has been shown.
const SEEN_FILE: &str = "crash-seen.txt";

/// How to send a report, shown in every report and by the app.
pub const HOW_TO_SEND: &str = "This report stays on this computer; Praxis Sound never uploads \
it. To send it, attach this file to an email or an issue for the Praxis Sound team. It holds \
no audio, no sounds and no API key.";

/// `<home>/logs`.
pub fn logs_dir(home: &Path) -> PathBuf {
    home.join("logs")
}

/// The logs folder to write into: `<home>/logs` if it can be created, else the local app-data
/// fallback (Windows' Controlled Folder Access can block the user profile for unsigned apps).
fn writable_logs_dir(home: &Path) -> Option<PathBuf> {
    let preferred = logs_dir(home);
    if std::fs::create_dir_all(&preferred).is_ok() {
        return Some(preferred);
    }
    let fallback = PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
        .join("Praxis Sound")
        .join("logs");
    std::fs::create_dir_all(&fallback).ok()?;
    Some(fallback)
}

/// Installs the crash hook for `program` (e.g. "praxis", "praxis-app"), writing under
/// `PRAXIS_HOME` (see [`praxis_home`]).
pub fn install(program: &'static str) {
    install_in(program, praxis_home());
}

/// [`install`] with an explicit home folder (the app passes the home it settled on).
pub fn install_in(program: &'static str, home: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".to_string());
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "(unknown)".to_string());
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        if let Some(dir) = writable_logs_dir(&home) {
            let crash = Crash {
                program,
                version: env!("CARGO_PKG_VERSION"),
                thread: thread.name().unwrap_or("(unnamed)"),
                location: &location,
                message: &message,
                backtrace: &backtrace,
            };
            if let Ok(path) = write_report(&dir, &crash, SystemTime::now()) {
                eprintln!(
                    "{program}: a crash report was written to {}",
                    path.display()
                );
            }
        }
        previous(info);
    }));
}

/// What a report records.
#[derive(Debug)]
pub struct Crash<'a> {
    pub program: &'a str,
    pub version: &'a str,
    pub thread: &'a str,
    pub location: &'a str,
    pub message: &'a str,
    pub backtrace: &'a str,
}

/// Writes one report into `dir` (created if needed), prunes to [`KEEP`], returns its path.
pub fn write_report(dir: &Path, crash: &Crash<'_>, now: SystemTime) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (stamp, iso) = utc(secs);
    let mut path = dir.join(format!("crash-{stamp}-{}.txt", std::process::id()));
    let mut n = 1;
    while path.exists() {
        n += 1;
        path = dir.join(format!("crash-{stamp}-{}-{n}.txt", std::process::id()));
    }
    let text = format!(
        "Praxis Sound crash report\n\
         program: {} {}\n\
         time: {iso}\n\
         os: {} {}\n\
         thread: {}\n\
         location: {}\n\
         message: {}\n\
         \n\
         {HOW_TO_SEND}\n\
         \n\
         backtrace:\n{}\n",
        crash.program,
        crash.version,
        std::env::consts::OS,
        std::env::consts::ARCH,
        crash.thread,
        crash.location,
        scrub(crash.message),
        scrub(crash.backtrace),
    );
    let mut f = std::fs::File::create(&path)?;
    f.write_all(text.as_bytes())?;
    f.flush()?;
    prune(dir, KEEP);
    Ok(path)
}

/// Blanks anything shaped like an Anthropic API key (`sk-ant-...`).
pub fn scrub(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("sk-ant-") {
        out.push_str(&rest[..i]);
        out.push_str("[key removed]");
        let tail = &rest[i..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(tail.len());
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Report files in `dir`, newest first (by name, which starts with the UTC time).
fn report_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-2") && n.ends_with(".txt"))
        })
        .collect();
    files.sort();
    files.reverse();
    files
}

fn prune(dir: &Path, keep: usize) {
    for old in report_files(dir).into_iter().skip(keep) {
        let _ = std::fs::remove_file(old);
    }
}

/// One crash report as the app lists it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CrashReport {
    pub path: String,
    pub file: String,
    pub program: String,
    pub time: String,
    pub message: String,
    /// Not shown to the user yet.
    pub unseen: bool,
}

/// `{dir, reports[], unseen, how_to_send}`: every report under `<home>/logs` (and the local
/// app-data fallback), newest first.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CrashReports {
    pub dir: String,
    pub reports: Vec<CrashReport>,
    pub unseen: usize,
    pub how_to_send: &'static str,
}

fn field(text: &str, name: &str) -> String {
    text.lines()
        .find_map(|l| l.strip_prefix(name))
        .map(|v| v.trim().to_string())
        .unwrap_or_default()
}

/// Lists the reports in the logs folder of `home`.
pub fn reports(home: &Path) -> CrashReports {
    let dir = logs_dir(home);
    let seen = std::fs::read_to_string(dir.join(SEEN_FILE))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let reports: Vec<CrashReport> = report_files(&dir)
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            let file = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            CrashReport {
                path: p.display().to_string(),
                unseen: file.as_str() > seen.as_str(),
                file,
                program: field(&text, "program:"),
                time: field(&text, "time:"),
                message: field(&text, "message:"),
            }
        })
        .collect();
    CrashReports {
        dir: dir.display().to_string(),
        unseen: reports.iter().filter(|r| r.unseen).count(),
        reports,
        how_to_send: HOW_TO_SEND,
    }
}

/// Marks every current report as shown.
pub fn mark_seen(home: &Path) -> std::io::Result<()> {
    let dir = logs_dir(home);
    if let Some(newest) = report_files(&dir).first()
        && let Some(name) = newest.file_name()
    {
        std::fs::write(dir.join(SEEN_FILE), name.to_string_lossy().as_bytes())?;
    }
    Ok(())
}

/// `(YYYYMMDD-HHMMSS, YYYY-MM-DDTHH:MM:SSZ)` for Unix seconds (proleptic Gregorian, UTC).
fn utc(secs: u64) -> (String, String) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant's days_from_civil inverse.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    (
        format!("{y:04}{mo:02}{d:02}-{h:02}{m:02}{s:02}"),
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("praxis-crash-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn crash<'a>(message: &'a str) -> Crash<'a> {
        Crash {
            program: "praxis",
            version: "0.1.0",
            thread: "main",
            location: "src/x.rs:1:2",
            message,
            backtrace: "0: here",
        }
    }

    #[test]
    fn utc_dates_are_right() {
        assert_eq!(utc(0).1, "1970-01-01T00:00:00Z");
        assert_eq!(utc(951_782_400).1, "2000-02-29T00:00:00Z");
        assert_eq!(utc(1_794_441_600 + 3661).0, "20261112-010101");
    }

    #[test]
    fn keys_are_blanked() {
        assert_eq!(
            scrub("bad key sk-ant-api03-AbC_d-9 here"),
            "bad key [key removed] here"
        );
        assert_eq!(scrub("nothing"), "nothing");
        assert_eq!(scrub("sk-ant-"), "[key removed]");
    }

    #[test]
    fn a_report_is_written_listed_and_marked_seen() {
        let home = temp("list");
        let dir = logs_dir(&home);
        let t = UNIX_EPOCH + Duration::from_secs(1_794_441_600);
        let p = write_report(&dir, &crash("index out of bounds sk-ant-secret"), t).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("message: index out of bounds [key removed]"));
        assert!(!text.contains("secret"));
        assert!(text.contains("never uploads"));
        let r = reports(&home);
        assert_eq!(r.reports.len(), 1);
        assert_eq!(r.unseen, 1);
        assert_eq!(r.reports[0].program, "praxis 0.1.0");
        assert_eq!(r.reports[0].time, "2026-11-12T00:00:00Z");
        mark_seen(&home).unwrap();
        assert_eq!(reports(&home).unseen, 0);
        // A later crash is unseen again.
        write_report(&dir, &crash("again"), t + Duration::from_secs(60)).unwrap();
        let r = reports(&home);
        assert_eq!((r.reports.len(), r.unseen), (2, 1));
        assert_eq!(r.reports[0].message, "again");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn only_the_newest_reports_are_kept() {
        let home = temp("prune");
        let dir = logs_dir(&home);
        for i in 0..(KEEP as u64 + 5) {
            let t = UNIX_EPOCH + Duration::from_secs(1_794_441_600 + i);
            write_report(&dir, &crash("x"), t).unwrap();
        }
        let files = report_files(&dir);
        assert_eq!(files.len(), KEEP);
        assert!(files[0].to_string_lossy().contains("20261112-000024"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_real_panic_writes_a_report() {
        let home = temp("panic");
        install_in("praxis-test", home.clone());
        let joined = std::thread::Builder::new()
            .name("crasher".into())
            .spawn(|| panic!("deliberate test panic"))
            .unwrap()
            .join();
        // Back to the default hook for the other tests in this binary.
        let _ = std::panic::take_hook();
        assert!(joined.is_err());
        let r = reports(&home);
        let ours = r
            .reports
            .iter()
            .find(|c| c.message == "deliberate test panic")
            .expect("a report for the panic");
        assert_eq!(ours.program, "praxis-test 0.1.0");
        let text = std::fs::read_to_string(&ours.path).unwrap();
        assert!(text.contains("thread: crasher"));
        assert!(
            text.contains("crash.rs"),
            "location names this file: {text}"
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}
