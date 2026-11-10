//! Preset registry. FROZEN CONTRACT.

use crate::packs::Packs;
use praxis_engine::{
    Instrument, InstrumentContext, InstrumentError, InstrumentFactory, PresetInfo,
};
use praxis_score::{InstrumentKind, InstrumentRef};
use std::sync::Arc;

/// Builds one instrument instance. Must preallocate all voices and buffers.
pub type BuildFn = fn(
    spec: &InstrumentRef,
    ctx: &InstrumentContext,
    packs: &Packs,
) -> Result<Box<dyn Instrument>, InstrumentError>;

#[derive(Clone)]
pub struct PresetEntry {
    pub info: PresetInfo,
    pub build: BuildFn,
}

impl std::fmt::Debug for PresetEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PresetEntry")
            .field("info", &self.info)
            .finish()
    }
}

/// All presets, plus the pack store sampled presets load from.
#[derive(Clone, Debug)]
pub struct Registry {
    entries: Arc<Vec<PresetEntry>>,
    packs: Arc<Packs>,
}

impl Registry {
    pub fn new(packs: Packs) -> Registry {
        let mut entries = Vec::new();
        entries.extend(crate::sampler::presets());
        entries.extend(crate::synth::presets());
        entries.extend(crate::drums::presets());
        entries.extend(crate::sfx::presets());
        Registry {
            entries: Arc::new(entries),
            packs: Arc::new(packs),
        }
    }

    /// Registry using the default pack directory (see [`Packs::discover`]).
    pub fn with_default_packs() -> Registry {
        Registry::new(Packs::discover())
    }

    pub fn packs(&self) -> &Packs {
        &self.packs
    }

    pub fn entry(&self, kind: InstrumentKind, preset: &str) -> Option<&PresetEntry> {
        self.entries
            .iter()
            .find(|e| e.info.kind == kind && e.info.name == preset)
    }
}

impl InstrumentFactory for Registry {
    fn create(
        &self,
        spec: &InstrumentRef,
        ctx: &InstrumentContext,
    ) -> Result<Box<dyn Instrument>, InstrumentError> {
        let entry =
            self.entry(spec.kind, &spec.preset)
                .ok_or_else(|| InstrumentError::UnknownPreset {
                    kind: spec.kind,
                    preset: spec.preset.clone(),
                })?;
        for name in spec.params.keys() {
            if !entry.info.params.iter().any(|p| &p.name == name) {
                return Err(InstrumentError::UnknownParam {
                    preset: spec.preset.clone(),
                    param: name.clone(),
                });
            }
        }
        (entry.build)(spec, ctx, &self.packs)
    }

    fn describe(&self, kind: InstrumentKind, preset: &str) -> Option<PresetInfo> {
        self.entry(kind, preset).map(|e| e.info.clone())
    }

    fn presets(&self) -> Vec<PresetInfo> {
        self.entries.iter().map(|e| e.info.clone()).collect()
    }
}
