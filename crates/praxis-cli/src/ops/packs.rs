//! `packs list` and `packs install <name>`. Only packs listed in `packs/index.json` can be
//! installed; each archive is checked against the index's exact size and blake3 before it is
//! unpacked (praxis_instruments::sampler::install).

use crate::error::{ErrorCode, OpError, io_error};
use praxis_instruments::Packs;
use praxis_instruments::sampler::index::PackIndex;
use praxis_instruments::sampler::install::{InstallError, install_from_archive, install_pack};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PacksInstallRequest {
    /// Pack name from `packs/index.json`, e.g. `upright-piano-kw`.
    pub name: String,
    /// Install from this archive (already downloaded) instead of downloading it. It is verified
    /// against the index exactly like a download.
    #[serde(default)]
    pub archive: Option<PathBuf>,
}

/// The pack directory (`$PRAXIS_PACKS`, else a `packs/` folder with an `index.json` near the
/// current directory or the executable) and its index.
fn store() -> Result<(Packs, PathBuf, PackIndex), OpError> {
    let packs = Packs::discover();
    let root = packs.root.clone().ok_or_else(|| {
        OpError::new(
            ErrorCode::Io,
            "no packs directory found: run from the Praxis Sound folder or set PRAXIS_PACKS",
        )
    })?;
    let index_path = root.join("index.json");
    let text = std::fs::read_to_string(&index_path).map_err(|e| io_error(&index_path, e))?;
    let index = PackIndex::parse(&text).map_err(|e| OpError::new(ErrorCode::BadJson, e))?;
    Ok((packs, root, index))
}

/// `{root, packs[]}`: every pack in the index with `installed` and `installed_version`.
pub fn packs_list() -> Result<Value, OpError> {
    let (packs, root, index) = store()?;
    let list: Vec<Value> = index
        .packs
        .iter()
        .map(|e| {
            let manifest = packs.manifest(&e.name).ok();
            json!({
                "name": e.name,
                "title": e.title,
                "version": e.version,
                "license": e.license,
                "size": e.size,
                "presets": e.presets.keys().collect::<Vec<_>>(),
                "attribution": e.attribution,
                "source_page": e.source_page,
                "installed": manifest.is_some(),
                "installed_version": manifest.map(|m| m.version),
            })
        })
        .collect();
    Ok(json!({ "root": root.display().to_string(), "packs": list }))
}

fn install_error(e: InstallError) -> OpError {
    let code = match &e {
        InstallError::Io { .. } | InstallError::Download { .. } => ErrorCode::Io,
        // The archive is not the one the index names (size or blake3), or unpacked incompletely.
        InstallError::SizeMismatch { .. }
        | InstallError::HashMismatch { .. }
        | InstallError::MissingFile { .. } => ErrorCode::PackVerifyFailed,
        _ => ErrorCode::Error,
    };
    OpError::new(code, e.to_string())
}

/// `{installed: {pack, version, dir, archive, downloaded, already_installed, files,
/// unpacked_bytes}}`. Unknown names fail with `pack_missing`.
pub fn packs_install(req: &PacksInstallRequest) -> Result<Value, OpError> {
    let (_, root, index) = store()?;
    let entry = index.find(&req.name).ok_or_else(|| {
        let names: Vec<&str> = index.packs.iter().map(|p| p.name.as_str()).collect();
        OpError::new(
            ErrorCode::PackMissing,
            format!(
                "`{}` is not in packs/index.json; available: {}",
                req.name,
                names.join(", ")
            ),
        )
    })?;
    let report = match &req.archive {
        Some(a) => install_from_archive(a, entry, &root),
        None => install_pack(entry, &root),
    }
    .map_err(install_error)?;
    Ok(json!({
        "installed": {
            "pack": report.pack,
            "version": entry.version,
            "dir": report.pack_dir.display().to_string(),
            "archive": report.archive.display().to_string(),
            "downloaded": report.downloaded,
            "already_installed": report.already_installed,
            "files": report.files,
            "unpacked_bytes": report.unpacked_bytes,
        },
        "root": display(&root),
    }))
}

fn display(p: &Path) -> String {
    p.display().to_string()
}
