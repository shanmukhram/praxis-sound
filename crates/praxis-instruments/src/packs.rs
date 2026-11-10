//! Instrument pack storage. FROZEN CONTRACT. See docs/contracts/instruments.md.
//!
//! A pack is a directory `<root>/<pack-name>/` holding `pack.json` (name, version, licence,
//! source URL, attribution) and its SFZ/SF2 and sample files. Packs are never committed with
//! unclear licences; every shipped pack is listed in THIRD_PARTY.md.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PackManifest {
    pub name: String,
    pub version: String,
    /// SPDX id, e.g. "CC0-1.0", "CC-BY-3.0".
    pub license: String,
    pub source_url: String,
    #[serde(default)]
    pub attribution: String,
    /// Entry file per preset name, relative to the pack directory, e.g. {"felt_piano": "felt.sfz"}.
    #[serde(default)]
    pub presets: BTreeMap<String, String>,
}

/// Where pack files come from: a directory on disk, or bytes supplied in memory (WebAssembly).
#[derive(Clone, Debug, Default)]
pub struct Packs {
    pub root: Option<PathBuf>,
    mem: BTreeMap<String, Vec<u8>>,
}

impl Packs {
    pub fn none() -> Packs {
        Packs::default()
    }

    pub fn at(root: impl Into<PathBuf>) -> Packs {
        Packs {
            root: Some(root.into()),
            mem: BTreeMap::new(),
        }
    }

    /// `$PRAXIS_PACKS` if set, else `./packs` next to the current directory or the executable.
    pub fn discover() -> Packs {
        if let Ok(p) = std::env::var("PRAXIS_PACKS") {
            return Packs::at(p);
        }
        let mut candidates = vec![PathBuf::from("packs")];
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            candidates.push(dir.join("packs"));
            candidates.push(dir.join("../../packs"));
        }
        for c in candidates {
            if c.is_dir() {
                return Packs::at(c);
            }
        }
        Packs::none()
    }

    /// Registers in-memory file contents under a pack-relative path ("pack/file.flac").
    pub fn insert(&mut self, path: &str, bytes: Vec<u8>) {
        self.mem.insert(normalize(path), bytes);
    }

    /// Reads a file by pack-relative path ("upright-kw/samples/A4.flac").
    pub fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let key = normalize(path);
        if let Some(b) = self.mem.get(&key) {
            return Ok(b.clone());
        }
        match &self.root {
            Some(root) => std::fs::read(root.join(Path::new(&key)))
                .map_err(|e| format!("{}: {e}", root.join(&key).display())),
            None => Err(format!("no pack store configured for `{key}`")),
        }
    }

    pub fn manifest(&self, pack: &str) -> Result<PackManifest, String> {
        let bytes = self.read(&format!("{pack}/pack.json"))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("{pack}/pack.json: {e}"))
    }

    pub fn has_pack(&self, pack: &str) -> bool {
        self.manifest(pack).is_ok()
    }
}

fn normalize(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_string()
}
