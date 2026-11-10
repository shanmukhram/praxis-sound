//! Pack installation (desktop/CLI only, feature `install`): download, verify, unpack.
//!
//! `install_pack` downloads the archive named by a `packs/index.json` entry into
//! `<packs_root>/_downloads/`, checks its exact size and blake3, unpacks it with `sevenz-rust2`
//! into `<packs_root>/<name>/` and writes `pack.json` last (a pack without `pack.json` is not
//! installed). `install_from_archive` does the same from an archive already on disk; tests and CI
//! use it so they never need the network.
//!
//! Archive contents are untrusted: every entry path is checked to stay inside the pack directory,
//! the total unpacked size is bounded, and nothing from the archive is ever executed.

use super::index::IndexEntry;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

/// Upper bound on unpacked bytes per pack (zip-bomb guard). The owner's limit is 100 MB per
/// archive; FLAC packs unpack to roughly the same size.
pub const MAX_UNPACKED_BYTES: u64 = 2 << 30;

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("{path}: {msg}")]
    Io { path: String, msg: String },
    #[error("download of {url} failed: {msg}")]
    Download { url: String, msg: String },
    #[error("archive {path} has {actual} bytes, index says {expected}")]
    SizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    #[error("archive {path} has blake3 {actual}, index says {expected}")]
    HashMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("cannot unpack {path}: {msg}")]
    Archive { path: String, msg: String },
    #[error("pack `{pack}` is missing `{file}` after unpacking")]
    MissingFile { pack: String, file: String },
    #[error("invalid index entry: {0}")]
    Index(String),
}

fn io_err(path: &Path, e: impl std::fmt::Display) -> InstallError {
    InstallError::Io {
        path: path.display().to_string(),
        msg: e.to_string(),
    }
}

/// What an install did.
#[derive(Clone, Debug, PartialEq)]
pub struct InstallReport {
    pub pack: String,
    pub pack_dir: PathBuf,
    pub archive: PathBuf,
    /// True when the archive was fetched over the network by this call.
    pub downloaded: bool,
    /// True when the same version was already installed and nothing was unpacked.
    pub already_installed: bool,
    pub files: usize,
    pub unpacked_bytes: u64,
}

/// Size and lowercase-hex blake3 of a file.
pub fn hash_file(path: &Path) -> Result<(u64, String), InstallError> {
    let mut f = std::fs::File::open(path).map_err(|e| io_err(path, e))?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| io_err(path, e))?;
        if n == 0 {
            break;
        }
        size += n as u64;
        h.update(&buf[..n]);
    }
    Ok((size, h.finalize().to_hex().to_string()))
}

/// Checks an archive on disk against the index entry's size and blake3.
pub fn verify_archive(path: &Path, entry: &IndexEntry) -> Result<(), InstallError> {
    let (size, hash) = hash_file(path)?;
    if size != entry.size {
        return Err(InstallError::SizeMismatch {
            path: path.display().to_string(),
            expected: entry.size,
            actual: size,
        });
    }
    if !hash.eq_ignore_ascii_case(&entry.blake3) {
        return Err(InstallError::HashMismatch {
            path: path.display().to_string(),
            expected: entry.blake3.clone(),
            actual: hash,
        });
    }
    Ok(())
}

/// Downloads `url` to `dest` (through `dest.part`, renamed on success). Fails if the body is
/// not exactly `expected_size` bytes. Returns the blake3 of what was written; the caller compares
/// it with the index.
pub fn download(url: &str, expected_size: u64, dest: &Path) -> Result<String, InstallError> {
    let dl_err = |msg: String| InstallError::Download {
        url: url.to_string(),
        msg,
    };
    if !url.starts_with("https://") {
        return Err(dl_err("only https URLs are allowed".into()));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    }
    let part = dest.with_extension("part");
    let resp = ureq::get(url)
        .header("User-Agent", "praxis-sound-pack-installer")
        .call()
        .map_err(|e| dl_err(e.to_string()))?;
    let mut reader = resp
        .into_body()
        .into_with_config()
        .limit(expected_size + 1)
        .reader();
    let mut out = std::fs::File::create(&part).map_err(|e| io_err(&part, e))?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut size = 0u64;
    let result = (|| -> Result<(), InstallError> {
        loop {
            let n = reader.read(&mut buf).map_err(|e| dl_err(e.to_string()))?;
            if n == 0 {
                break;
            }
            size += n as u64;
            if size > expected_size {
                return Err(dl_err(format!("body is larger than {expected_size} bytes")));
            }
            h.update(&buf[..n]);
            out.write_all(&buf[..n]).map_err(|e| io_err(&part, e))?;
        }
        out.flush().map_err(|e| io_err(&part, e))?;
        if size != expected_size {
            return Err(InstallError::SizeMismatch {
                path: url.to_string(),
                expected: expected_size,
                actual: size,
            });
        }
        Ok(())
    })();
    drop(out);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, dest).map_err(|e| io_err(dest, e))?;
    Ok(h.finalize().to_hex().to_string())
}

/// Downloads (unless a verified copy is already in `<packs_root>/_downloads/`), verifies and
/// installs one pack.
pub fn install_pack(entry: &IndexEntry, packs_root: &Path) -> Result<InstallReport, InstallError> {
    entry.validate().map_err(InstallError::Index)?;
    let archive = packs_root.join("_downloads").join(&entry.archive);
    let mut downloaded = false;
    if archive.is_file() && verify_archive(&archive, entry).is_ok() {
        // Reuse the verified archive.
    } else {
        let hash = download(&entry.url, entry.size, &archive)?;
        downloaded = true;
        if !hash.eq_ignore_ascii_case(&entry.blake3) {
            let _ = std::fs::remove_file(&archive);
            return Err(InstallError::HashMismatch {
                path: entry.url.clone(),
                expected: entry.blake3.clone(),
                actual: hash,
            });
        }
    }
    let mut report = install_from_archive(&archive, entry, packs_root)?;
    report.downloaded = downloaded;
    Ok(report)
}

/// Verifies `archive` against `entry` and unpacks it into `<packs_root>/<name>/`, replacing an
/// older version. Unpacks into a staging directory first, so a failed install never leaves a
/// half-written pack behind; `pack.json` is written last.
pub fn install_from_archive(
    archive: &Path,
    entry: &IndexEntry,
    packs_root: &Path,
) -> Result<InstallReport, InstallError> {
    entry.validate().map_err(InstallError::Index)?;
    verify_archive(archive, entry)?;
    let final_dir = packs_root.join(&entry.name);
    if let Ok(text) = std::fs::read_to_string(final_dir.join("pack.json"))
        && let Ok(m) = serde_json::from_str::<crate::packs::PackManifest>(&text)
        && m == entry.manifest()
        && required_files_present(&final_dir, entry).is_ok()
    {
        return Ok(InstallReport {
            pack: entry.name.clone(),
            pack_dir: final_dir,
            archive: archive.to_path_buf(),
            downloaded: false,
            already_installed: true,
            files: 0,
            unpacked_bytes: 0,
        });
    }

    let staging = packs_root.join(format!(".{}.partial", entry.name));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|e| io_err(&staging, e))?;
    }
    std::fs::create_dir_all(&staging).map_err(|e| io_err(&staging, e))?;
    let (files, unpacked_bytes) = match unpack(archive, &entry.strip_prefix, &staging) {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    if let Err(e) = required_files_present(&staging, entry) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    let manifest = serde_json::to_string_pretty(&entry.manifest())
        .map_err(|e| io_err(&staging.join("pack.json"), e))?;
    std::fs::write(staging.join("pack.json"), manifest + "\n")
        .map_err(|e| io_err(&staging.join("pack.json"), e))?;

    if final_dir.exists() {
        let old = packs_root.join(format!(".{}.old", entry.name));
        if old.exists() {
            std::fs::remove_dir_all(&old).map_err(|e| io_err(&old, e))?;
        }
        std::fs::rename(&final_dir, &old).map_err(|e| io_err(&final_dir, e))?;
        std::fs::rename(&staging, &final_dir).map_err(|e| io_err(&final_dir, e))?;
        std::fs::remove_dir_all(&old).map_err(|e| io_err(&old, e))?;
    } else {
        std::fs::rename(&staging, &final_dir).map_err(|e| io_err(&final_dir, e))?;
    }
    Ok(InstallReport {
        pack: entry.name.clone(),
        pack_dir: final_dir,
        archive: archive.to_path_buf(),
        downloaded: false,
        already_installed: false,
        files,
        unpacked_bytes,
    })
}

fn required_files_present(dir: &Path, entry: &IndexEntry) -> Result<(), InstallError> {
    for f in std::iter::once(&entry.entry)
        .chain(std::iter::once(&entry.license_file))
        .chain(entry.presets.values())
    {
        if !dir.join(f).is_file() {
            return Err(InstallError::MissingFile {
                pack: entry.name.clone(),
                file: f.clone(),
            });
        }
    }
    Ok(())
}

/// Maps an archive entry name to a path under `dest`, removing `strip_prefix`. `None` for the
/// prefix directory itself. Errors on anything that is not a plain relative path.
fn entry_path(dest: &Path, name: &str, strip_prefix: &str) -> Result<Option<PathBuf>, String> {
    let name = name.replace('\\', "/");
    let prefix = strip_prefix.trim_end_matches('/');
    let rel = if prefix.is_empty() {
        name.as_str()
    } else if name == prefix {
        return Ok(None);
    } else if let Some(rest) = name.strip_prefix(&format!("{prefix}/")) {
        rest
    } else {
        return Err(format!("entry `{name}` is outside `{prefix}/`"));
    };
    let mut out = dest.to_path_buf();
    let mut any = false;
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(p) => {
                out.push(p);
                any = true;
            }
            Component::CurDir => {}
            _ => return Err(format!("unsafe entry path `{name}`")),
        }
    }
    Ok(any.then_some(out))
}

/// CRC-32 (IEEE 802.3, reflected), as stored in 7z headers.
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *e = c;
        }
        t
    });
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = t[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// Unpacks every entry of `archive` under `dest` with `sevenz-rust2` (LZMA, LZMA2, BZIP2,
/// PPMD, DEFLATE, COPY). Each file must have exactly its declared size and, when the archive
/// stores one, its CRC-32. Returns (files written, bytes written).
fn unpack(archive: &Path, strip_prefix: &str, dest: &Path) -> Result<(usize, u64), InstallError> {
    let arch_err = |msg: String| InstallError::Archive {
        path: archive.display().to_string(),
        msg,
    };
    let meta = sevenz_rust2::Archive::open(archive).map_err(|e| arch_err(e.to_string()))?;
    if (0..meta.files.len()).any(|i| meta.is_file_encrypted(i)) {
        return Err(arch_err("encrypted archives are not supported".into()));
    }
    // Check every entry path before anything is written.
    for e in &meta.files {
        entry_path(dest, e.name(), strip_prefix).map_err(arch_err)?;
    }
    let declared: u64 = meta.files.iter().map(|e| e.size()).sum();
    if declared > MAX_UNPACKED_BYTES {
        return Err(arch_err(format!(
            "unpacked size exceeds {MAX_UNPACKED_BYTES} bytes"
        )));
    }
    let mut reader = sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty())
        .map_err(|e| arch_err(e.to_string()))?;
    let mut files = 0usize;
    let mut total = 0u64;
    let mut failure: Option<InstallError> = None;
    let res = reader.for_each_entries(|e, data| {
        let mut fail = |err: InstallError| {
            failure = Some(err);
            Ok(false)
        };
        let path = match entry_path(dest, e.name(), strip_prefix) {
            Ok(Some(p)) => p,
            Ok(None) => return Ok(true),
            Err(msg) => return fail(arch_err(msg)),
        };
        if e.is_directory() {
            if let Err(err) = std::fs::create_dir_all(&path) {
                return fail(io_err(&path, err));
            }
            return Ok(true);
        }
        if e.is_anti_item() {
            return Ok(true);
        }
        // Read one byte more than declared to detect overruns.
        let mut bytes = Vec::with_capacity(e.size() as usize);
        if let Err(err) = data.take(e.size() + 1).read_to_end(&mut bytes) {
            return fail(arch_err(format!("`{}`: {err}", e.name())));
        }
        if bytes.len() as u64 != e.size() {
            return fail(arch_err(format!(
                "`{}`: {} bytes, header says {}",
                e.name(),
                bytes.len(),
                e.size()
            )));
        }
        if e.has_crc && crc32(&bytes) as u64 != e.crc {
            return fail(arch_err(format!("`{}`: CRC mismatch", e.name())));
        }
        total += bytes.len() as u64;
        if let Some(parent) = path.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            return fail(io_err(parent, err));
        }
        if let Err(err) = std::fs::write(&path, &bytes) {
            return fail(io_err(&path, err));
        }
        files += 1;
        Ok(true)
    });
    if let Some(f) = failure {
        return Err(f);
    }
    res.map_err(|e| arch_err(e.to_string()))?;
    Ok((files, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_paths_stay_inside() {
        let d = Path::new("root");
        assert_eq!(
            entry_path(d, "Pack/samples/a.flac", "Pack").unwrap(),
            Some(d.join("samples").join("a.flac"))
        );
        assert_eq!(entry_path(d, "Pack", "Pack/").unwrap(), None);
        assert!(entry_path(d, "Other/a.flac", "Pack").is_err());
        assert!(entry_path(d, "Pack/../evil", "Pack").is_err());
        assert!(entry_path(d, "../evil", "").is_err());
        assert!(entry_path(d, "/abs", "").is_err());
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert!(entry_path(d, "Pack\\..\\evil", "Pack").is_err());
    }
}
