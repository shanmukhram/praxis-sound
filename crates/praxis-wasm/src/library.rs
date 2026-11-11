//! The instruments a browser engine plays: the preset registry over pack files held in memory.
//!
//! A browser has no pack folder. The page fetches a pack's files over HTTP (from the served
//! `packs/` folder) and hands them in one by one ([`Library::insert`]); [`Library::missing`]
//! says which files of a preset are still needed, so the page can fetch exactly those (the
//! manifest first, then the SFZ and its includes, then every sample it names). A sampled preset
//! whose pack is not complete plays a synth preset instead, and the render says so in plain words
//! ([`Library::playable`]).

use praxis_engine::InstrumentFactory;
use praxis_instruments::packs::{PackManifest, Packs};
use praxis_instruments::registry::Registry;
use praxis_instruments::sampler::region::SampleRef;
use praxis_instruments::sampler::sfz;
use praxis_score::{InstrumentKind, InstrumentRef, Score};
use std::collections::{BTreeMap, BTreeSet};

/// Synth presets a sampled preset falls back to when its pack is not loaded, in order of
/// preference (the first one the registry has is used).
pub const FALLBACK_PRESETS: &[&str] = &["soft_keys", "fm_keys", "pluck"];

/// Pack files in memory and the registry built over them.
#[derive(Debug)]
pub struct Library {
    files: BTreeMap<String, Vec<u8>>,
    registry: Registry,
    /// Files were inserted since the registry was last built.
    dirty: bool,
}

impl Default for Library {
    fn default() -> Self {
        Library::new()
    }
}

/// One sampled layer and the pack its preset loads from.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackNeed {
    pub pack: String,
    pub preset: String,
}

impl Library {
    pub fn new() -> Library {
        Library {
            files: BTreeMap::new(),
            registry: Registry::new(Packs::none()),
            dirty: false,
        }
    }

    /// Stores one pack file under its pack-relative path (`"upright-piano-kw/pack.json"`).
    pub fn insert(&mut self, path: &str, bytes: Vec<u8>) {
        self.files.insert(normalize(path), bytes);
        self.dirty = true;
    }

    /// True when a file is held under this pack-relative path.
    pub fn has_file(&self, path: &str) -> bool {
        self.files.contains_key(&normalize(path))
    }

    /// Number of files and their total size in bytes.
    pub fn size(&self) -> (usize, usize) {
        (self.files.len(), self.files.values().map(Vec::len).sum())
    }

    /// The registry over every file inserted so far (rebuilt after inserts).
    pub fn registry(&mut self) -> &Registry {
        if self.dirty {
            let mut packs = Packs::none();
            for (path, bytes) in &self.files {
                packs.insert(path, bytes.clone());
            }
            self.registry = Registry::new(packs);
            self.dirty = false;
        }
        &self.registry
    }

    /// The pack each sampled layer of `score` loads from, deduplicated and sorted.
    pub fn needs(&mut self, score: &Score) -> Vec<PackNeed> {
        let registry = self.registry();
        let mut out = BTreeSet::new();
        for l in &score.layers {
            if let Some(info) = registry.describe(l.instrument.kind, &l.instrument.preset)
                && let Some(pack) = pack_of(&info.source)
            {
                out.insert(PackNeed {
                    pack: pack.to_string(),
                    preset: l.instrument.preset.clone(),
                });
            }
        }
        out.into_iter().collect()
    }

    /// Pack-relative paths still needed before `preset` of `pack` can load: the manifest, then
    /// the entry file and its includes, then every sample the entry names. Empty when complete.
    /// `Err` when the manifest has no entry for the preset or the entry cannot be parsed.
    pub fn missing(&self, pack: &str, preset: &str) -> Result<Vec<String>, String> {
        let manifest_path = format!("{pack}/pack.json");
        let Some(bytes) = self.files.get(&manifest_path) else {
            return Ok(vec![manifest_path]);
        };
        let manifest: PackManifest =
            serde_json::from_slice(bytes).map_err(|e| format!("{manifest_path}: {e}"))?;
        let entry = manifest
            .presets
            .get(preset)
            .ok_or_else(|| format!("pack `{pack}` has no entry for preset `{preset}`"))?;
        let entry = normalize(&format!("{pack}/{entry}"));
        if !self.files.contains_key(&entry) {
            return Ok(vec![entry]);
        }
        if entry.to_ascii_lowercase().ends_with(".sf2") {
            // A SoundFont holds its samples.
            return Ok(Vec::new());
        }
        let mut wanted_text = BTreeSet::new();
        let mut read = |p: &str| -> Result<String, String> {
            match self.files.get(&normalize(p)) {
                Some(b) => String::from_utf8(b.clone()).map_err(|_| format!("{p}: not UTF-8")),
                None => {
                    wanted_text.insert(normalize(p));
                    Err(format!("{p}: not loaded yet"))
                }
            }
        };
        let parsed = sfz::parse_file(&entry, &mut read);
        if !wanted_text.is_empty() {
            return Ok(wanted_text.into_iter().collect());
        }
        let inst = parsed.map_err(|e| e.to_string())?;
        let samples: BTreeSet<String> = inst
            .regions
            .iter()
            .filter_map(|r| match &r.sample {
                SampleRef::File(f) => Some(normalize(f)),
                SampleRef::Sf2(_) => None,
            })
            .filter(|f| !self.files.contains_key(f))
            .collect();
        Ok(samples.into_iter().collect())
    }

    /// True when every file `preset` of `pack` needs is loaded.
    pub fn complete(&self, pack: &str, preset: &str) -> bool {
        self.missing(pack, preset).is_ok_and(|m| m.is_empty())
    }

    /// `score` as this library can play it: a sampled layer whose pack is not loaded plays a synth
    /// preset instead. Returns the score to render and one plain sentence per substitution.
    pub fn playable(&mut self, score: &Score) -> (Score, Vec<String>) {
        let mut out = score.clone();
        let mut notes = Vec::new();
        let needs = self.needs(score);
        let incomplete: Vec<PackNeed> = needs
            .into_iter()
            .filter(|n| !self.complete(&n.pack, &n.preset))
            .collect();
        if incomplete.is_empty() {
            return (out, notes);
        }
        let registry = self.registry();
        let fallback = FALLBACK_PRESETS.iter().find_map(|name| {
            registry
                .presets()
                .into_iter()
                .find(|p| p.name == *name && p.kind == InstrumentKind::Synth)
        });
        for layer in &mut out.layers {
            let Some(need) = incomplete
                .iter()
                .find(|n| n.preset == layer.instrument.preset)
            else {
                continue;
            };
            if registry
                .describe(layer.instrument.kind, &layer.instrument.preset)
                .and_then(|i| pack_of(&i.source).map(str::to_string))
                .as_deref()
                != Some(need.pack.as_str())
            {
                continue;
            }
            let name = if layer.name.is_empty() {
                layer.id.clone()
            } else {
                layer.name.clone()
            };
            match &fallback {
                Some(fb) => {
                    notes.push(format!(
                        "The {} samples (pack `{}`) are not on this server, so {name} plays the \
                         {} synth instead. Put the pack in the served packs/ folder to hear the \
                         real instrument.",
                        words(&layer.instrument.preset),
                        need.pack,
                        words(&fb.name),
                    ));
                    layer.instrument = InstrumentRef {
                        kind: fb.kind,
                        preset: fb.name.clone(),
                        params: BTreeMap::new(),
                    };
                }
                None => notes.push(format!(
                    "The {} samples (pack `{}`) are not on this server and no synth preset can \
                     stand in, so {name} cannot play.",
                    words(&layer.instrument.preset),
                    need.pack,
                )),
            }
        }
        (out, notes)
    }
}

/// `felt_piano` -> `felt piano`.
fn words(preset: &str) -> String {
    preset.replace('_', " ")
}

/// The pack a sampled preset loads from, from its source note ("..., pack `name`").
pub fn pack_of(source: &str) -> Option<&str> {
    let rest = &source[source.find("pack `")? + 6..];
    Some(&rest[..rest.find('`')?])
}

fn normalize(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_string()
}
