//! Pack index and installation tests. Archives are generated here with `sevenz-rust2`, so no
//! network is needed; the real Upright Piano KW archive is used when it is already downloaded.
#![cfg(feature = "install")]

mod sampler_common;

use praxis_engine::Instrument;
use praxis_instruments::Packs;
use praxis_instruments::sampler::engine::{SamplerInstrument, Voicing};
use praxis_instruments::sampler::index::{IndexEntry, PackIndex};
use praxis_instruments::sampler::install::{self, InstallError};
use praxis_instruments::sampler::program::Program;
use sampler_common::*;
use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderMethod};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn tmp(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("sampler_install")
        .join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn read_index() -> PackIndex {
    let text = std::fs::read_to_string(packs_dir().join("index.json")).unwrap();
    PackIndex::parse(&text).unwrap()
}

#[test]
fn index_json_lists_upright_piano_kw() {
    let idx = read_index();
    assert_eq!(idx.format, 1);
    let e = idx
        .find("upright-piano-kw")
        .expect("upright-piano-kw entry");
    assert_eq!(e.license, "CC0-1.0");
    assert_eq!(e.license_file, "cc0.txt");
    assert_eq!(e.size, 33_294_287);
    assert_eq!(
        e.blake3,
        "1ae0bd73dc5618682bc7d06c9c9d4dfc72458a76d504657e7125b5365a5cc3d1"
    );
    assert!(e.url.starts_with("https://freepats.zenvoid.org/"));
    assert_eq!(e.entry, "UprightPianoKW-20220221.sfz");
    for preset in ["felt_piano", "upright_piano"] {
        assert_eq!(e.presets.get(preset), Some(&e.entry));
    }
    let m = e.manifest();
    assert_eq!(
        (m.name.as_str(), m.license.as_str()),
        ("upright-piano-kw", "CC0-1.0")
    );
}

#[test]
fn index_validation_rejects_unsafe_entries() {
    let good = read_index().packs[0].clone();
    let mut e = good.clone();
    e.name = "../x".into();
    assert!(e.validate().is_err());
    let mut e = good.clone();
    e.entry = "../../etc/passwd".into();
    assert!(e.validate().is_err());
    let mut e = good.clone();
    e.blake3 = "abc".into();
    assert!(e.validate().is_err());
    let mut e = good.clone();
    e.archive = "../a.7z".into();
    assert!(e.validate().is_err());
    assert!(PackIndex::parse(r#"{"format":1,"packs":[],"extra":1}"#).is_err());
}

/// Writes a 7z with `entries` (name, Some(bytes) for files / None for directories).
fn make_archive(path: &Path, entries: &[(&str, Option<Vec<u8>>)], method: EncoderMethod) {
    let f = std::fs::File::create(path).unwrap();
    let mut w = ArchiveWriter::new(f).unwrap();
    w.set_content_methods(vec![method.into()]);
    for (name, data) in entries {
        match data {
            Some(bytes) => {
                w.push_archive_entry(
                    ArchiveEntry::new_file(name),
                    Some(std::io::Cursor::new(bytes.clone())),
                )
                .unwrap();
            }
            None => {
                w.push_archive_entry::<&[u8]>(ArchiveEntry::new_directory(name), None)
                    .unwrap();
            }
        }
    }
    w.finish().unwrap();
}

const TEST_SFZ: &str = "<region> sample=samples/a.wav pitch_keycenter=57 lokey=40 hikey=80\n";

fn test_entries(prefix: &str) -> Vec<(String, Option<Vec<u8>>)> {
    let wav = wav_bytes(&[struck_tone(220.0, 44_100, 0.5, 4, 0.5)], 44_100);
    vec![
        (prefix.to_string(), None),
        (format!("{prefix}/samples"), None),
        (
            format!("{prefix}/inst.sfz"),
            Some(TEST_SFZ.as_bytes().to_vec()),
        ),
        (format!("{prefix}/LICENSE.txt"), Some(b"CC0 1.0".to_vec())),
        (format!("{prefix}/samples/a.wav"), Some(wav)),
        (format!("{prefix}/empty.txt"), Some(Vec::new())),
    ]
}

fn entry_for(archive: &Path) -> IndexEntry {
    let (size, blake3) = install::hash_file(archive).unwrap();
    IndexEntry {
        name: "test-pack".into(),
        title: "Test pack".into(),
        version: "1".into(),
        license: "CC0-1.0".into(),
        license_file: "LICENSE.txt".into(),
        source_page: "https://example.invalid/test".into(),
        url: "https://example.invalid/test.7z".into(),
        archive: "test.7z".into(),
        size,
        blake3,
        strip_prefix: "TestPack-1".into(),
        entry: "inst.sfz".into(),
        attribution: String::new(),
        presets: [("test".to_string(), "inst.sfz".to_string())].into(),
    }
}

fn install_and_play(method: EncoderMethod, dir: &str) {
    let root = tmp(dir);
    let archive = root.join("_downloads").join("test.7z");
    std::fs::create_dir_all(archive.parent().unwrap()).unwrap();
    let entries = test_entries("TestPack-1");
    let refs: Vec<(&str, Option<Vec<u8>>)> = entries
        .iter()
        .map(|(n, d)| (n.as_str(), d.clone()))
        .collect();
    make_archive(&archive, &refs, method);
    let entry = entry_for(&archive);

    let report = install::install_from_archive(&archive, &entry, &root).unwrap();
    assert!(!report.already_installed);
    assert_eq!(report.files, 4);
    let pack = root.join("test-pack");
    assert!(pack.join("samples").join("a.wav").is_file());
    assert_eq!(std::fs::read(pack.join("empty.txt")).unwrap().len(), 0);
    assert!(!root.join(".test-pack.partial").exists());

    let packs = Packs::at(&root);
    let m = packs.manifest("test-pack").unwrap();
    assert_eq!(m.presets.get("test").map(String::as_str), Some("inst.sfz"));
    let prog = Arc::new(Program::load(&packs, "test-pack/inst.sfz").unwrap());
    let mut s = SamplerInstrument::new(prog, SR, 8, Voicing::default(), None, &[]);
    let (l, r) = render(&mut s, &[note(1, 0, 57, 0.8, 9600)], 19200);
    assert!(rms(&mono(&l, &r)) > 0.01);

    // Installing the same version again is a no-op.
    let again = install::install_from_archive(&archive, &entry, &root).unwrap();
    assert!(again.already_installed);
    // install_pack reuses the verified download without touching the network.
    let via_pack = install::install_pack(&entry, &root).unwrap();
    assert!(!via_pack.downloaded && via_pack.already_installed);
    // A new version replaces the old one.
    let mut v2 = entry.clone();
    v2.version = "2".into();
    let replaced = install::install_from_archive(&archive, &v2, &root).unwrap();
    assert!(!replaced.already_installed);
    assert_eq!(Packs::at(&root).manifest("test-pack").unwrap().version, "2");
    assert!(!root.join(".test-pack.old").exists());
}

#[test]
fn installs_lzma2_archive_via_sevenz_decoders() {
    install_and_play(EncoderMethod::LZMA2, "lzma2");
}

#[test]
fn installs_copy_archive_via_own_block_reader() {
    // COPY blocks take the same path as DEFLATE blocks (block layout and CRCs from sevenz-rust2,
    // data read by our code).
    install_and_play(EncoderMethod::COPY, "copy");
}

#[test]
fn rejects_bad_archives_and_leaves_nothing_behind() {
    let root = tmp("bad");
    let archive = root.join("test.7z");
    let entries = test_entries("TestPack-1");
    let refs: Vec<(&str, Option<Vec<u8>>)> = entries
        .iter()
        .map(|(n, d)| (n.as_str(), d.clone()))
        .collect();
    make_archive(&archive, &refs, EncoderMethod::LZMA2);
    let good = entry_for(&archive);

    let mut e = good.clone();
    e.blake3 = "0".repeat(64);
    assert!(matches!(
        install::install_from_archive(&archive, &e, &root),
        Err(InstallError::HashMismatch { .. })
    ));
    let mut e = good.clone();
    e.size += 1;
    assert!(matches!(
        install::install_from_archive(&archive, &e, &root),
        Err(InstallError::SizeMismatch { .. })
    ));
    let mut e = good.clone();
    e.license_file = "COPYING".into();
    assert!(matches!(
        install::install_from_archive(&archive, &e, &root),
        Err(InstallError::MissingFile { .. })
    ));
    let mut e = good.clone();
    e.strip_prefix = "Other".into();
    assert!(matches!(
        install::install_from_archive(&archive, &e, &root),
        Err(InstallError::Archive { .. })
    ));
    assert!(!root.join("test-pack").exists());
    assert!(!root.join(".test-pack.partial").exists());

    // An entry that climbs out of the pack directory is refused before anything is written.
    for method in [EncoderMethod::LZMA2, EncoderMethod::COPY] {
        let evil = root.join("evil.7z");
        make_archive(
            &evil,
            &[
                ("TestPack-1/inst.sfz", Some(TEST_SFZ.as_bytes().to_vec())),
                ("TestPack-1/../../escaped.txt", Some(b"x".to_vec())),
            ],
            method,
        );
        let e = entry_for(&evil);
        let res = install::install_from_archive(&evil, &e, &root);
        assert!(matches!(res, Err(InstallError::Archive { .. })), "{res:?}");
        assert!(!root.join("escaped.txt").exists());
        assert!(!root.parent().unwrap().join("escaped.txt").exists());
        assert!(!root.join("test-pack").exists());
    }
}

#[test]
fn installs_the_downloaded_upright_piano_kw_archive() {
    let idx = read_index();
    let e = idx.find("upright-piano-kw").unwrap();
    let archive = packs_dir().join("_downloads").join(&e.archive);
    if !archive.is_file() {
        println!(
            "SKIPPED installs_the_downloaded_upright_piano_kw_archive: {} not downloaded",
            archive.display()
        );
        return;
    }
    let root = tmp("kw");
    let report = install::install_from_archive(&archive, e, &root).unwrap();
    println!("{report:?}");
    assert_eq!(report.files, 70);
    let packs = Packs::at(&root);
    assert!(packs.has_pack("upright-piano-kw"));
    let cc0 = std::fs::read_to_string(root.join("upright-piano-kw").join("cc0.txt")).unwrap();
    assert!(cc0.contains("CC0 1.0 Universal"));
    let readme = std::fs::read_to_string(root.join("upright-piano-kw").join("readme.txt")).unwrap();
    assert!(readme.contains("Creative Commons CC0"));
    let prog = Program::load(&packs, "upright-piano-kw/UprightPianoKW-20220221.sfz").unwrap();
    assert_eq!(prog.regions.len(), 66);
    let _ = std::fs::remove_dir_all(&root);
}

/// Downloads (32 MiB) and installs Upright Piano KW into the repository's `packs/`.
/// `cargo test -p praxis-instruments --test sampler_install -- --ignored install_upright_piano_kw_from_network`
#[test]
#[ignore]
fn install_upright_piano_kw_from_network() {
    let idx = read_index();
    let e = idx.find("upright-piano-kw").unwrap();
    let report = install::install_pack(e, &packs_dir()).unwrap();
    println!("{report:?}");
    assert!(Packs::at(packs_dir()).has_pack("upright-piano-kw"));
    let mut s: Box<dyn Instrument> = {
        let prog = Arc::new(
            Program::load(
                &Packs::at(packs_dir()),
                "upright-piano-kw/UprightPianoKW-20220221.sfz",
            )
            .unwrap(),
        );
        Box::new(SamplerInstrument::new(
            prog,
            SR,
            8,
            Voicing::default(),
            None,
            &[],
        ))
    };
    let (l, _) = render(s.as_mut(), &[note(1, 0, 60, 0.7, 4800)], 9600);
    assert!(rms(&l) > 1e-3);
}
