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

const METHOD_COPY: &[u8] = &[0x00];
const METHOD_DEFLATE: &[u8] = &[0x04, 0x01, 0x08];

/// Unpacks every entry of `archive` under `dest`. Returns (files written, bytes written).
fn unpack(archive: &Path, strip_prefix: &str, dest: &Path) -> Result<(usize, u64), InstallError> {
    let arch_err = |msg: String| InstallError::Archive {
        path: archive.display().to_string(),
        msg,
    };
    let meta = sevenz_rust2::Archive::open(archive).map_err(|e| arch_err(e.to_string()))?;
    if (0..meta.files.len()).any(|i| meta.is_file_encrypted(i)) {
        return Err(arch_err("encrypted archives are not supported".into()));
    }
    let own_decoder = !meta.blocks.is_empty()
        && meta.blocks.iter().all(|b| {
            b.coders.len() == 1
                && matches!(
                    b.coders[0].encoder_method_id(),
                    METHOD_COPY | METHOD_DEFLATE
                )
        });
    if own_decoder {
        unpack_deflate(archive, &meta, strip_prefix, dest)
    } else {
        unpack_sevenz(archive, strip_prefix, dest)
    }
}

/// Writes one regular file, enforcing the unpacked-size bound.
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), InstallError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    }
    std::fs::write(path, bytes).map_err(|e| io_err(path, e))
}

/// Unpacks archives whose blocks are single-coder DEFLATE or COPY: `sevenz-rust2` supplies the
/// archive structure (files, block layout, sizes, CRCs) and [`super::inflate`] decodes the data.
fn unpack_deflate(
    archive: &Path,
    meta: &sevenz_rust2::Archive,
    strip_prefix: &str,
    dest: &Path,
) -> Result<(usize, u64), InstallError> {
    use std::io::Seek;
    let arch_err = |msg: String| InstallError::Archive {
        path: archive.display().to_string(),
        msg,
    };
    let mut files = 0usize;
    let mut total = 0u64;
    for e in meta.files.iter().filter(|e| !e.has_stream()) {
        let Some(path) = entry_path(dest, e.name(), strip_prefix).map_err(arch_err)? else {
            continue;
        };
        if e.is_directory() {
            std::fs::create_dir_all(&path).map_err(|err| io_err(&path, err))?;
        } else if !e.is_anti_item() {
            write_file(&path, &[])?;
            files += 1;
        }
    }
    let file_len = std::fs::metadata(archive)
        .map_err(|e| io_err(archive, e))?
        .len();
    let mut f = std::fs::File::open(archive).map_err(|e| io_err(archive, e))?;
    let stream_map = &meta.stream_map;
    for (bi, block) in meta.blocks.iter().enumerate() {
        let ps = *stream_map
            .block_first_pack_stream_index()
            .get(bi)
            .ok_or_else(|| arch_err(format!("block {bi} has no pack stream")))?;
        let packed = *meta
            .pack_sizes()
            .get(ps)
            .ok_or_else(|| arch_err(format!("pack stream {ps} missing")))?;
        let offset = sevenz_rust2::SIGNATURE_HEADER_SIZE
            + meta.pack_pos()
            + stream_map
                .pack_stream_offsets()
                .get(ps)
                .copied()
                .unwrap_or(0);
        if offset.checked_add(packed).is_none_or(|end| end > file_len) {
            return Err(arch_err(format!("block {bi} lies outside the archive")));
        }
        let unpacked = block.get_unpack_size();
        if total + unpacked > MAX_UNPACKED_BYTES {
            return Err(arch_err(format!(
                "unpacked size exceeds {MAX_UNPACKED_BYTES} bytes"
            )));
        }
        let mut buf = vec![0u8; packed as usize];
        f.seek(std::io::SeekFrom::Start(offset))
            .map_err(|e| io_err(archive, e))?;
        f.read_exact(&mut buf).map_err(|e| io_err(archive, e))?;
        let data = if block.coders[0].encoder_method_id() == METHOD_DEFLATE {
            super::inflate::inflate(&buf, unpacked as usize)
                .map_err(|m| arch_err(format!("block {bi}: {m}")))?
        } else {
            buf
        };
        if data.len() as u64 != unpacked {
            return Err(arch_err(format!("block {bi}: size mismatch")));
        }
        if block.has_crc && super::inflate::crc32(&data) as u64 != block.crc {
            return Err(arch_err(format!("block {bi}: CRC mismatch")));
        }
        let mut cursor = 0usize;
        for (fi, e) in meta.files.iter().enumerate() {
            if stream_map.file_block_index.get(fi).copied().flatten() != Some(bi) {
                continue;
            }
            let n = e.size() as usize;
            let bytes = data
                .get(cursor..cursor + n)
                .ok_or_else(|| arch_err(format!("`{}` runs past its block", e.name())))?;
            cursor += n;
            if e.has_crc && super::inflate::crc32(bytes) as u64 != e.crc {
                return Err(arch_err(format!("`{}`: CRC mismatch", e.name())));
            }
            let Some(path) = entry_path(dest, e.name(), strip_prefix).map_err(arch_err)? else {
                continue;
            };
            write_file(&path, bytes)?;
            files += 1;
            total += n as u64;
        }
        if cursor != data.len() {
            return Err(arch_err(format!("block {bi}: trailing data")));
        }
    }
    Ok((files, total))
}

/// Unpacks archives using `sevenz-rust2`'s own decoders (LZMA, LZMA2, BZIP2, PPMD, COPY).
fn unpack_sevenz(
    archive: &Path,
    strip_prefix: &str,
    dest: &Path,
) -> Result<(usize, u64), InstallError> {
    let arch_err = |msg: String| InstallError::Archive {
        path: archive.display().to_string(),
        msg,
    };
    let mut reader = sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty())
        .map_err(|e| arch_err(e.to_string()))?;
    let mut files = 0usize;
    let mut total = 0u64;
    let mut failure: Option<InstallError> = None;
    let res = reader.for_each_entries(|e, data| {
        let path = match entry_path(dest, e.name(), strip_prefix) {
            Ok(Some(p)) => p,
            Ok(None) => return Ok(true),
            Err(msg) => {
                failure = Some(arch_err(msg));
                return Ok(false);
            }
        };
        if e.is_directory() {
            if let Err(err) = std::fs::create_dir_all(&path) {
                failure = Some(io_err(&path, err));
                return Ok(false);
            }
            return Ok(true);
        }
        total += e.size();
        if total > MAX_UNPACKED_BYTES {
            failure = Some(arch_err(format!(
                "unpacked size exceeds {MAX_UNPACKED_BYTES} bytes"
            )));
            return Ok(false);
        }
        if let Some(parent) = path.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            failure = Some(io_err(parent, err));
            return Ok(false);
        }
        let mut f = match std::fs::File::create(&path) {
            Ok(f) => f,
            Err(err) => {
                failure = Some(io_err(&path, err));
                return Ok(false);
            }
        };
        let mut limited = data.take(e.size());
        if let Err(err) = std::io::copy(&mut limited, &mut f) {
            failure = Some(io_err(&path, err));
            return Ok(false);
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
        assert!(entry_path(d, "Pack\\..\\evil", "Pack").is_err());
    }
}
