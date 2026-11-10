//! `packs/index.json`: the committed list of downloadable instrument packs.
//!
//! Each entry names one archive with its URL, exact size and blake3 hash, so an install can
//! verify the bytes before unpacking them, plus the files the pack must contain after unpacking
//! (entry SFZ, licence text) and the preset -> entry-file map written to `pack.json`.

use crate::packs::PackManifest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The whole `packs/index.json` file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PackIndex {
    /// Format version of this file (1).
    pub format: u32,
    pub packs: Vec<IndexEntry>,
}

/// One downloadable pack.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct IndexEntry {
    /// Directory name under `packs/`, e.g. "upright-piano-kw".
    pub name: String,
    /// Human-readable title.
    pub title: String,
    /// Pack version (the upstream release date for FreePats packs).
    pub version: String,
    /// SPDX licence id, e.g. "CC0-1.0".
    pub license: String,
    /// Where the licence was confirmed: a file inside the archive (after `strip_prefix`).
    pub license_file: String,
    /// Upstream page describing the pack.
    pub source_page: String,
    /// Direct archive download URL.
    pub url: String,
    /// Archive file name used in `packs/_downloads/`.
    pub archive: String,
    /// Archive size in bytes.
    pub size: u64,
    /// blake3 of the archive, lowercase hex.
    pub blake3: String,
    /// Leading directory inside the archive that is removed when unpacking ("" for none).
    #[serde(default)]
    pub strip_prefix: String,
    /// Main SFZ (or SF2) file, relative to the pack directory after unpacking.
    pub entry: String,
    #[serde(default)]
    pub attribution: String,
    /// Preset name -> entry file, copied into `pack.json`.
    pub presets: BTreeMap<String, String>,
}

impl PackIndex {
    pub fn parse(text: &str) -> Result<PackIndex, String> {
        let idx: PackIndex =
            serde_json::from_str(text).map_err(|e| format!("packs/index.json: {e}"))?;
        for p in &idx.packs {
            p.validate()?;
        }
        Ok(idx)
    }

    pub fn find(&self, name: &str) -> Option<&IndexEntry> {
        self.packs.iter().find(|p| p.name == name)
    }
}

impl IndexEntry {
    /// Rejects entries that could not be verified or that would unpack outside `packs/<name>/`.
    pub fn validate(&self) -> Result<(), String> {
        let ok_name = !self.name.is_empty()
            && self
                .name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
            && !self.name.starts_with('_');
        if !ok_name {
            return Err(format!(
                "pack name `{}` must be lowercase ascii, digits, '-' or '_', not starting with '_'",
                self.name
            ));
        }
        if self.blake3.len() != 64 || !self.blake3.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "pack `{}`: blake3 must be 64 hex digits",
                self.name
            ));
        }
        if self.size == 0 {
            return Err(format!("pack `{}`: size must be > 0", self.name));
        }
        if self.archive.contains(['/', '\\']) || self.archive.starts_with('.') {
            return Err(format!(
                "pack `{}`: archive must be a plain file name",
                self.name
            ));
        }
        for f in std::iter::once(&self.entry)
            .chain(std::iter::once(&self.license_file))
            .chain(self.presets.values())
        {
            if f.split(['/', '\\']).any(|c| c == "..") || f.starts_with(['/', '\\']) {
                return Err(format!(
                    "pack `{}`: file `{f}` must stay inside the pack",
                    self.name
                ));
            }
        }
        Ok(())
    }

    /// The `pack.json` written after a successful install.
    pub fn manifest(&self) -> PackManifest {
        PackManifest {
            name: self.name.clone(),
            version: self.version.clone(),
            license: self.license.clone(),
            source_url: self.source_page.clone(),
            attribution: self.attribution.clone(),
            presets: self.presets.clone(),
        }
    }
}
