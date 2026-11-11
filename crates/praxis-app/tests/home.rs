//! The app must open even when its sounds folder cannot be created (Windows' Controlled Folder
//! Access blocks unknown programs from writing into Documents and reports "file not found").

use praxis_app::writable_home;
use std::path::{Path, PathBuf};

fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("praxis-app-{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::path::absolute(dir).unwrap()
}

#[test]
fn a_writable_home_is_kept_and_its_sounds_folder_made() {
    let dir = scratch("home_kept");
    let home = dir.join("Praxis Sound");
    assert_eq!(
        writable_home(home.clone(), Some(dir.join("fallback"))),
        home
    );
    assert!(home.join("sounds").is_dir());
    assert!(!dir.join("fallback").exists());
}

#[test]
fn an_unwritable_home_falls_back_instead_of_failing() {
    let dir = scratch("home_fallback");
    // A file where the home folder should be: creating `<home>/sounds` fails.
    let blocked = dir.join("blocked");
    std::fs::write(&blocked, b"not a folder").unwrap();
    let fallback = dir.join("local").join("Praxis Sound");
    assert_eq!(
        writable_home(blocked.clone(), Some(fallback.clone())),
        fallback
    );
    assert!(fallback.join("sounds").is_dir());
    // With no usable fallback either, it still returns (the window opens; commands then say why).
    let also_blocked = dir.join("also");
    std::fs::write(&also_blocked, b"x").unwrap();
    assert_eq!(writable_home(blocked.clone(), Some(also_blocked)), blocked);
    assert_eq!(writable_home(blocked.clone(), None), blocked);
}
