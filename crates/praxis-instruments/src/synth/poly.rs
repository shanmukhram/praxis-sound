//! The shared polyphonic voice framework every synth and drum preset runs on.
//!
//! A preset is a [`Patch`] (its settings, shared by all its voices) and a [`Voice`] type.
//! [`PolyInstrument`] owns one patch and a preallocated pool of voices and implements
//! [`Instrument`]:
//!
//! - **Pool.** `polyphony + SPARE_VOICES` voices are built in `create`; nothing is allocated
//!   afterwards (voices preallocate their own buffers in [`Patch::new_voice`]).
//! - **Stealing.** When `polyphony` voices are playing, the quietest one (by [`Voice::level`],
//!   the oldest on ties) fades out over [`STEAL_FADE_S`] in a spare slot while the new note
//!   starts, so stealing never clicks. If even the spare slots are busy, the quietest fading
//!   voice is reused.
//! - **Choke groups.** [`Patch::choke`] lets a note fade out ringing voices of a group over
//!   [`CHOKE_FADE_S`] (a closed hi-hat chokes an open one).
//! - **Determinism.** Voices are summed in note-on order, never slot order, and every voice
//!   decides its own end per sample, so output does not depend on block size, on how the engine
//!   splits blocks at events, or on which slot a note got. Per-note randomness comes only from
//!   `NoteOn.seed` (see [`Note::rng`]).

use super::common::{MAX_PARAMS, Params, resolve_params};
use crate::packs::Packs;
use crate::registry::PresetEntry;
use praxis_dsp::rng::{Pcg32, derive};
use praxis_dsp::util::sanitize;
use praxis_engine::{Instrument, InstrumentContext, InstrumentError, NoteOn, PresetInfo};
use praxis_score::InstrumentRef;

/// Fade applied to a stolen voice, seconds.
pub const STEAL_FADE_S: f32 = 0.005;
/// Fade applied to a choked voice, seconds.
pub const CHOKE_FADE_S: f32 = 0.008;
/// Extra slots that let stolen and choked voices fade while new notes start.
pub const SPARE_VOICES: usize = 4;

/// A note as a voice sees it.
#[derive(Clone, Copy, Debug)]
pub struct Note {
    pub pitch: u8,
    /// Equal-tempered frequency of `pitch`.
    pub hz: f64,
    pub velocity: f32,
    pub brightness: f32,
    pub seed: u64,
    pub length_samples: u64,
    pub sample_rate: f64,
}

impl Note {
    fn new(on: &NoteOn, sample_rate: f64) -> Note {
        Note {
            pitch: on.pitch.min(127),
            hz: super::common::midi_hz(on.pitch.min(127)),
            velocity: if on.velocity.is_finite() {
                on.velocity.clamp(0.0, 1.0)
            } else {
                0.0
            },
            brightness: if on.brightness.is_finite() {
                on.brightness.clamp(0.0, 1.0)
            } else {
                0.0
            },
            seed: on.seed,
            length_samples: on.length_samples,
            sample_rate,
        }
    }

    /// A random stream for this note, independent for each `salt` (one per use).
    pub fn rng(&self, salt: u64) -> Pcg32 {
        Pcg32::new(derive(self.seed, salt))
    }
}

/// One voice of a preset.
pub trait Voice<P>: Send {
    /// Starts a note (overwrites any previous state).
    fn start(&mut self, patch: &P, note: &Note);
    /// Note-off: enter the release.
    fn release(&mut self, patch: &P);
    /// The next stereo sample.
    fn tick(&mut self, patch: &P) -> (f32, f32);
    /// Current loudness estimate (linear peak), used to pick the voice to steal.
    fn level(&self) -> f32;
    /// The voice has reached exact silence and can be freed.
    fn is_done(&self) -> bool;
}

/// A preset's settings and voice factory.
pub trait Patch: Send + Sized + 'static {
    type Voice: Voice<Self>;
    /// Description for the registry.
    fn info() -> PresetInfo;
    /// Builds the patch from resolved parameter values (allocation allowed here).
    fn new(params: &Params, ctx: &InstrumentContext) -> Self;
    /// Voices that may play at once.
    fn polyphony(&self) -> usize;
    /// Builds one voice (allocation allowed here: called only from `create`).
    fn new_voice(&self) -> Self::Voice;
    /// Applies a parameter change (index into [`PresetInfo::params`], value in range).
    fn set_param(&mut self, index: usize, value: f32);
    /// Longest sound after a note-off, in samples (see [`Instrument::tail_samples`]).
    fn tail_samples(&self) -> usize;
    /// Whether this note plays at all (a drum kit ignores unmapped notes).
    fn accepts(&self, _note: &NoteOn) -> bool {
        true
    }
    /// Choke group of the note (0 = none) and a bit mask of groups it silences.
    fn choke(&self, _note: &NoteOn) -> (u8, u32) {
        (0, 0)
    }
}

#[derive(Clone, Debug)]
struct Slot<V> {
    voice: V,
    active: bool,
    released: bool,
    id: u64,
    group: u8,
    fade: f32,
    fade_step: f32,
}

/// A preset instance: one patch, a preallocated voice pool.
pub struct PolyInstrument<P: Patch> {
    patch: P,
    slots: Vec<Slot<P::Voice>>,
    /// Active slots in note-on order.
    order: Vec<usize>,
    polyphony: usize,
    ranges: [(f32, f32); MAX_PARAMS],
    param_count: usize,
    steal_step: f32,
    choke_step: f32,
    sr: f64,
}

impl<P: Patch> PolyInstrument<P> {
    pub fn new(patch: P, sample_rate: f32) -> PolyInstrument<P> {
        let sr = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate as f64
        } else {
            48_000.0
        };
        let info = P::info();
        let mut ranges = [(0.0f32, 0.0f32); MAX_PARAMS];
        for (r, p) in ranges.iter_mut().zip(info.params.iter()) {
            *r = (p.min as f32, p.max as f32);
        }
        let polyphony = patch.polyphony().max(1);
        let total = polyphony + SPARE_VOICES;
        let slots = (0..total)
            .map(|_| Slot {
                voice: patch.new_voice(),
                active: false,
                released: false,
                id: 0,
                group: 0,
                fade: 1.0,
                fade_step: 0.0,
            })
            .collect();
        PolyInstrument {
            patch,
            slots,
            order: Vec::with_capacity(total),
            polyphony,
            ranges,
            param_count: info.params.len().min(MAX_PARAMS),
            steal_step: 1.0 / (STEAL_FADE_S as f64 * sr).max(1.0) as f32,
            choke_step: 1.0 / (CHOKE_FADE_S as f64 * sr).max(1.0) as f32,
            sr,
        }
    }

    /// The patch (for tests and inspection).
    pub fn patch(&self) -> &P {
        &self.patch
    }

    /// Voices currently sounding (including fading ones).
    pub fn active_voices(&self) -> usize {
        self.order.len()
    }

    fn free_slot(&mut self) -> usize {
        if let Some(i) = self.slots.iter().position(|s| !s.active) {
            return i;
        }
        // Every slot is busy (all spares are fading): reuse the quietest fading voice.
        let mut best = self.order.first().copied().unwrap_or(0);
        let mut best_level = f32::INFINITY;
        for &s in &self.order {
            let slot = &self.slots[s];
            let l = slot.voice.level() * slot.fade;
            if slot.fade_step > 0.0 && l < best_level {
                best_level = l;
                best = s;
            }
        }
        if best_level == f32::INFINITY {
            // No fading voice (cannot happen with spares, kept for safety): quietest of all.
            for &s in &self.order {
                let l = self.slots[s].voice.level();
                if l < best_level {
                    best_level = l;
                    best = s;
                }
            }
        }
        if let Some(i) = self.order.iter().position(|&s| s == best) {
            self.order.remove(i);
        }
        best
    }
}

impl<P: Patch> Instrument for PolyInstrument<P> {
    fn note_on(&mut self, note: &NoteOn) {
        if !self.patch.accepts(note) {
            return;
        }
        let (group, chokes) = self.patch.choke(note);
        if chokes != 0 {
            for &s in &self.order {
                let slot = &mut self.slots[s];
                if slot.group != 0
                    && chokes & (1u32 << (slot.group as u32 & 31)) != 0
                    && (slot.fade_step == 0.0 || slot.fade_step < self.choke_step)
                {
                    slot.fade_step = self.choke_step;
                }
            }
        }
        let playing = self
            .order
            .iter()
            .filter(|&&s| self.slots[s].fade_step == 0.0)
            .count();
        if playing >= self.polyphony {
            let mut best = None;
            let mut best_level = f32::INFINITY;
            for &s in &self.order {
                let slot = &self.slots[s];
                if slot.fade_step == 0.0 {
                    let l = slot.voice.level();
                    // Strictly quieter: ties keep the oldest (order is note-on order).
                    if l < best_level {
                        best_level = l;
                        best = Some(s);
                    }
                }
            }
            if let Some(s) = best {
                self.slots[s].fade_step = self.steal_step;
            }
        }
        let slot = self.free_slot();
        let n = Note::new(note, self.sr);
        let s = &mut self.slots[slot];
        s.voice.start(&self.patch, &n);
        s.active = true;
        s.released = false;
        s.id = note.id;
        s.group = group;
        s.fade = 1.0;
        s.fade_step = 0.0;
        self.order.push(slot);
    }

    fn note_off(&mut self, id: u64) {
        for &s in &self.order {
            let slot = &mut self.slots[s];
            if slot.id == id && !slot.released {
                slot.released = true;
                slot.voice.release(&self.patch);
            }
        }
    }

    fn all_notes_off(&mut self) {
        for &s in &self.order {
            let slot = &mut self.slots[s];
            if !slot.released {
                slot.released = true;
                slot.voice.release(&self.patch);
            }
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if index >= self.param_count {
            return;
        }
        let (lo, hi) = self.ranges[index];
        let v = if value.is_finite() { value } else { lo };
        self.patch.set_param(index, v.clamp(lo, hi));
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        left.fill(0.0);
        right.fill(0.0);
        let PolyInstrument {
            patch,
            slots,
            order,
            ..
        } = self;
        for &s in order.iter() {
            let slot = &mut slots[s];
            for i in 0..n {
                let (a, b) = slot.voice.tick(patch);
                let g = slot.fade;
                left[i] += a * g;
                right[i] += b * g;
                if slot.fade_step > 0.0 {
                    slot.fade -= slot.fade_step;
                    if slot.fade <= 0.0 {
                        slot.active = false;
                        break;
                    }
                }
                if slot.voice.is_done() {
                    slot.active = false;
                    break;
                }
            }
        }
        order.retain(|&s| slots[s].active);
        for v in left[..n].iter_mut().chain(right[..n].iter_mut()) {
            *v = sanitize(*v);
        }
    }

    fn tail_samples(&self) -> usize {
        self.patch.tail_samples()
    }

    fn reset(&mut self) {
        for s in &mut self.slots {
            s.active = false;
            s.released = false;
            s.fade = 1.0;
            s.fade_step = 0.0;
        }
        self.order.clear();
    }
}

/// Builds a [`PolyInstrument`] for patch type `P` (the registry's build function).
pub fn build<P: Patch>(
    spec: &InstrumentRef,
    ctx: &InstrumentContext,
    _packs: &Packs,
) -> Result<Box<dyn Instrument>, InstrumentError> {
    Ok(Box::new(instrument::<P>(spec, ctx)?))
}

/// The concrete instrument for patch type `P` (tests use this to inspect the pool).
pub fn instrument<P: Patch>(
    spec: &InstrumentRef,
    ctx: &InstrumentContext,
) -> Result<PolyInstrument<P>, InstrumentError> {
    let info = P::info();
    let params = resolve_params(spec, &info)?;
    Ok(PolyInstrument::new(P::new(&params, ctx), ctx.sample_rate))
}

/// Registry entry for patch type `P`.
pub fn entry<P: Patch>() -> PresetEntry {
    PresetEntry {
        info: P::info(),
        build: build::<P>,
    }
}

/// Sample rate from a context, with a safe default.
pub fn context_rate(ctx: &InstrumentContext) -> f64 {
    if ctx.sample_rate.is_finite() && ctx.sample_rate > 0.0 {
        ctx.sample_rate as f64
    } else {
        48_000.0
    }
}
