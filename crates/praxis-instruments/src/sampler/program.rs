//! A loaded sampler program: regions plus their decoded samples, read through [`Packs`].
//!
//! Decoded samples are shared: a process-wide cache maps the blake3 of each encoded file to a
//! weak reference of its decoded [`SampleData`], so two instruments on the same pack (for example
//! `felt_piano` and `upright_piano`, or two layers) decode and hold the audio once. The cache is
//! only touched while building instruments, never on the audio thread.

use super::region::{Region, SampleRef};
use super::sample::{self, SampleData};
use super::{sf2, sfz};
use crate::packs::Packs;
use praxis_engine::InstrumentError;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

/// Regions and the samples they play.
#[derive(Clone, Debug)]
pub struct Program {
    pub regions: Vec<Region>,
    /// `sample_of[i]` indexes `samples` for `regions[i]`.
    pub sample_of: Vec<usize>,
    pub samples: Vec<Arc<SampleData>>,
    /// Parser warnings (unsupported opcodes...).
    pub warnings: Vec<String>,
    /// Where the program came from, for messages.
    pub source: String,
}

impl Program {
    /// Builds a program from regions whose samples are given in the same order as
    /// `SampleRef::File` paths appear in `files` (for in-memory construction and tests).
    pub fn from_parts(
        regions: Vec<Region>,
        sample_of: Vec<usize>,
        samples: Vec<Arc<SampleData>>,
        source: &str,
    ) -> Result<Program, String> {
        if regions.len() != sample_of.len() {
            return Err("regions and sample_of differ in length".into());
        }
        if let Some(&bad) = sample_of.iter().find(|&&s| s >= samples.len()) {
            return Err(format!("sample index {bad} out of range"));
        }
        Ok(Program {
            regions,
            sample_of,
            samples,
            warnings: Vec::new(),
            source: source.to_string(),
        })
    }

    /// Loads an SFZ (`.sfz`) or SoundFont (`.sf2`, first preset) by pack-relative path.
    pub fn load(packs: &Packs, path: &str) -> Result<Program, InstrumentError> {
        if path.to_ascii_lowercase().ends_with(".sf2") {
            Program::load_sf2(packs, path, None)
        } else {
            Program::load_sfz(packs, path)
        }
    }

    pub fn load_sfz(packs: &Packs, path: &str) -> Result<Program, InstrumentError> {
        let mut read = |p: &str| -> Result<String, String> {
            let b = packs.read(p)?;
            String::from_utf8(b).map_err(|_| format!("{p}: not UTF-8 text"))
        };
        let inst =
            sfz::parse_file(path, &mut read).map_err(|e| InstrumentError::Load(e.to_string()))?;
        let mut index: BTreeMap<String, usize> = BTreeMap::new();
        let mut samples = Vec::new();
        let mut sample_of = Vec::with_capacity(inst.regions.len());
        for r in &inst.regions {
            let SampleRef::File(f) = &r.sample else {
                return Err(InstrumentError::Load(format!(
                    "{}: SFZ region without a sample file",
                    r.origin
                )));
            };
            let i = match index.get(f) {
                Some(&i) => i,
                None => {
                    let bytes = packs
                        .read(f)
                        .map_err(|e| InstrumentError::Load(format!("{}: {e}", r.origin)))?;
                    let data = cached_decode(&bytes, f)
                        .map_err(|e| InstrumentError::Load(format!("{}: {e}", r.origin)))?;
                    samples.push(data);
                    index.insert(f.clone(), samples.len() - 1);
                    samples.len() - 1
                }
            };
            sample_of.push(i);
        }
        Ok(Program {
            regions: inst.regions,
            sample_of,
            samples,
            warnings: inst.warnings,
            source: path.to_string(),
        })
    }

    /// Loads one preset of a SoundFont: `preset` = (bank, program), or the first preset.
    pub fn load_sf2(
        packs: &Packs,
        path: &str,
        preset: Option<(u16, u16)>,
    ) -> Result<Program, InstrumentError> {
        let bytes = packs.read(path).map_err(InstrumentError::Load)?;
        Program::from_sf2_bytes(&bytes, path, preset)
    }

    pub fn from_sf2_bytes(
        bytes: &[u8],
        path: &str,
        preset: Option<(u16, u16)>,
    ) -> Result<Program, InstrumentError> {
        let load = |m: String| InstrumentError::Load(format!("{path}: {m}"));
        let file = sf2::parse(bytes).map_err(|e| load(e.to_string()))?;
        let p = match preset {
            Some((bank, prog)) => file
                .presets
                .iter()
                .find(|p| p.bank == bank && p.program == prog)
                .ok_or_else(|| load(format!("no preset {bank}:{prog}")))?,
            None => file
                .presets
                .first()
                .ok_or_else(|| load("no presets".into()))?,
        };
        let file_hash = blake3::hash(bytes);
        let mut index: BTreeMap<usize, usize> = BTreeMap::new();
        let mut samples = Vec::new();
        let mut sample_of = Vec::with_capacity(p.regions.len());
        for r in &p.regions {
            let SampleRef::Sf2(si) = r.sample else {
                return Err(load("SoundFont region without a sample".into()));
            };
            let i = match index.get(&si) {
                Some(&i) => i,
                None => {
                    let mut key = blake3::Hasher::new();
                    key.update(file_hash.as_bytes());
                    key.update(&(si as u64).to_le_bytes());
                    let key = *key.finalize().as_bytes();
                    let data = cached(key, || {
                        sf2::sample_data(&file, bytes, si).map_err(|e| e.to_string())
                    })
                    .map_err(load)?;
                    samples.push(data);
                    index.insert(si, samples.len() - 1);
                    samples.len() - 1
                }
            };
            sample_of.push(i);
        }
        Ok(Program {
            regions: p.regions.clone(),
            sample_of,
            samples,
            warnings: Vec::new(),
            source: format!("{path} ({})", p.name),
        })
    }

    /// Lowest and highest key any attack region covers.
    pub fn key_range(&self) -> Option<(u8, u8)> {
        let attack = self
            .regions
            .iter()
            .filter(|r| r.trigger != super::region::Trigger::Release);
        let lo = attack.clone().map(|r| r.lokey).min()?;
        let hi = attack.map(|r| r.hikey).max()?;
        Some((lo, hi))
    }

    /// Decoded audio held by this program, in bytes.
    pub fn sample_bytes(&self) -> usize {
        self.samples.iter().map(|s| s.bytes()).sum()
    }
}

type Cache = Mutex<BTreeMap<[u8; 32], Weak<SampleData>>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn cached(
    key: [u8; 32],
    decode: impl FnOnce() -> Result<SampleData, String>,
) -> Result<Arc<SampleData>, String> {
    if let Ok(map) = cache().lock()
        && let Some(hit) = map.get(&key).and_then(Weak::upgrade)
    {
        return Ok(hit);
    }
    let data = Arc::new(decode()?);
    if let Ok(mut map) = cache().lock() {
        map.retain(|_, w| w.strong_count() > 0);
        map.insert(key, Arc::downgrade(&data));
    }
    Ok(data)
}

/// Decodes a WAV/FLAC file, sharing the result with any other program that loaded the same bytes.
pub fn cached_decode(bytes: &[u8], name: &str) -> Result<Arc<SampleData>, String> {
    cached(*blake3::hash(bytes).as_bytes(), || {
        sample::decode(bytes, name)
    })
}
