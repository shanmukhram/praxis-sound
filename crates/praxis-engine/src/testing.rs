//! Test instruments. **These are not product presets.** They exist so the engine (and other
//! packages) can render and test scores without the real instruments crate
//! (`praxis-instruments`, WP-C1/C2), whose `bell` and `soft_kick` presets will sound different.
//!
//! [`TestFactory`] implements [`InstrumentFactory`] with two presets:
//! - synth `bell` (class motif): a few inharmonic sine partials with exponential decay, velocity-
//!   and brightness-sensitive, releasing on note-off;
//! - drum `soft_kick` (class drums): a pitch-swept sine with a short decay.
//!
//! Both are allocation-free after construction (preallocated voice pools), polyphonic, and
//! deterministic: every value is a function of the note events, their sample positions and the note
//! seeds. Voices are summed in note-on order (not voice-slot order), and voice lifetimes are decided
//! per sample, so the output does not depend on block size, on how the engine splits blocks at
//! events, or on which voice slot a note happened to get.

use crate::instrument::{
    Instrument, InstrumentContext, InstrumentError, InstrumentFactory, NoteOn, PresetInfo,
    PresetParam,
};
use praxis_dsp::rng::Pcg32;
use praxis_score::{GeneratorClass, InstrumentKind, InstrumentRef};

/// Factory for the engine's test instruments. Not a product preset registry.
#[derive(Clone, Copy, Debug, Default)]
pub struct TestFactory;

impl TestFactory {
    pub fn new() -> TestFactory {
        TestFactory
    }
}

const SOURCE: &str = "praxis-engine test instrument (not a product preset)";

fn param(name: &str, min: f64, max: f64, default: f64, unit: &str) -> PresetParam {
    PresetParam {
        name: name.into(),
        min,
        max,
        default,
        unit: unit.into(),
    }
}

/// Description of the test `bell`.
pub fn bell_info() -> PresetInfo {
    PresetInfo {
        kind: InstrumentKind::Synth,
        name: "bell".into(),
        class: GeneratorClass::Motif,
        pitched: true,
        sustaining: true,
        range: (24, 108),
        params: vec![
            param("decay_scale", 0.25, 4.0, 1.0, ""),
            param("release_s", 0.05, 4.0, 0.45, "s"),
        ],
        source: SOURCE.into(),
    }
}

/// Description of the test `soft_kick`.
pub fn soft_kick_info() -> PresetInfo {
    PresetInfo {
        kind: InstrumentKind::Drum,
        name: "soft_kick".into(),
        class: GeneratorClass::Drums,
        pitched: false,
        sustaining: false,
        range: (0, 127),
        params: vec![
            param("decay_s", 0.05, 1.0, 0.16, "s"),
            param("pitch_hz", 30.0, 120.0, 48.0, "Hz"),
        ],
        source: SOURCE.into(),
    }
}

/// Parameter values for `info`, from the spec (clamped) or defaults.
fn resolve_params(spec: &InstrumentRef, info: &PresetInfo) -> Result<Vec<f32>, InstrumentError> {
    for name in spec.params.keys() {
        if !info.params.iter().any(|p| &p.name == name) {
            return Err(InstrumentError::UnknownParam {
                preset: spec.preset.clone(),
                param: name.clone(),
            });
        }
    }
    Ok(info
        .params
        .iter()
        .map(|p| {
            spec.params
                .get(&p.name)
                .copied()
                .filter(|v| v.is_finite())
                .unwrap_or(p.default)
                .clamp(p.min, p.max) as f32
        })
        .collect())
}

impl InstrumentFactory for TestFactory {
    fn create(
        &self,
        spec: &InstrumentRef,
        ctx: &InstrumentContext,
    ) -> Result<Box<dyn Instrument>, InstrumentError> {
        match (spec.kind, spec.preset.as_str()) {
            (InstrumentKind::Synth, "bell") => {
                let p = resolve_params(spec, &bell_info())?;
                Ok(Box::new(TestBell::new(ctx.sample_rate, p[0], p[1])))
            }
            (InstrumentKind::Drum, "soft_kick") => {
                let p = resolve_params(spec, &soft_kick_info())?;
                Ok(Box::new(TestKick::new(ctx.sample_rate, p[0], p[1])))
            }
            _ => Err(InstrumentError::UnknownPreset {
                kind: spec.kind,
                preset: spec.preset.clone(),
            }),
        }
    }

    fn describe(&self, kind: InstrumentKind, preset: &str) -> Option<PresetInfo> {
        match (kind, preset) {
            (InstrumentKind::Synth, "bell") => Some(bell_info()),
            (InstrumentKind::Drum, "soft_kick") => Some(soft_kick_info()),
            _ => None,
        }
    }

    fn presets(&self) -> Vec<PresetInfo> {
        vec![bell_info(), soft_kick_info()]
    }
}

/// Voices are freed once their summed partial amplitude falls below this (about -100 dBFS).
const FREE_LEVEL: f64 = 1e-5;
/// ln(peak level / FREE_LEVEL) with headroom: number of time constants a release can last.
const RELEASE_TAUS: f64 = 11.5;

fn midi_hz(pitch: u8) -> f64 {
    440.0 * 2f64.powf((pitch as f64 - 69.0) / 12.0)
}

/// Keeps `order` (active slots in note-on order) without allocating.
fn remove_slot(order: &mut Vec<usize>, slot: usize) {
    if let Some(i) = order.iter().position(|&s| s == slot) {
        order.remove(i);
    }
}

// ---------------------------------------------------------------- bell

/// (frequency ratio, relative amplitude, decay divisor) per partial.
const BELL_PARTIALS: [(f64, f64, f64); 5] = [
    (1.0, 1.0, 1.0),
    (2.0, 0.45, 1.7),
    (2.76, 0.30, 2.4),
    (4.07, 0.16, 3.3),
    (5.40, 0.09, 4.4),
];
const BELL_VOICES: usize = 48;

#[derive(Clone, Copy, Debug, Default)]
struct Partial {
    /// Quadrature oscillator state (rotating phasor), advanced before each output sample.
    re: f64,
    im: f64,
    cos_w: f64,
    sin_w: f64,
    amp: f64,
    decay: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct BellVoice {
    active: bool,
    id: u64,
    parts: [Partial; 5],
    age: u32,
    releasing: bool,
    rel_gain: f64,
    rel_coef: f64,
    level: f64,
}

/// Test bell: inharmonic partials, exponential decay, release on note-off.
pub struct TestBell {
    sr: f64,
    decay_scale: f32,
    release_s: f32,
    attack: u32,
    voices: Vec<BellVoice>,
    order: Vec<usize>,
}

impl TestBell {
    pub fn new(sample_rate: f32, decay_scale: f32, release_s: f32) -> TestBell {
        let sr = sample_rate.max(1.0) as f64;
        TestBell {
            sr,
            decay_scale,
            release_s,
            attack: ((0.0015 * sr).round() as u32).max(1),
            voices: vec![BellVoice::default(); BELL_VOICES],
            order: Vec::with_capacity(BELL_VOICES),
        }
    }

    fn pick_slot(&mut self) -> usize {
        if let Some(i) = self.voices.iter().position(|v| !v.active) {
            return i;
        }
        // Steal the quietest voice.
        let mut best = 0usize;
        for i in 1..self.voices.len() {
            if self.voices[i].level < self.voices[best].level {
                best = i;
            }
        }
        remove_slot(&mut self.order, best);
        best
    }
}

impl Instrument for TestBell {
    fn note_on(&mut self, note: &NoteOn) {
        let slot = self.pick_slot();
        let sr = self.sr;
        let f0 = midi_hz(note.pitch);
        let vel = note.velocity.clamp(0.0, 1.0) as f64;
        let bright = note.brightness.clamp(0.0, 1.0) as f64;
        let base = 0.22 * vel.powf(1.6);
        let tone = ((0.35 + 0.65 * vel) * (1.0 + 0.8 * bright)).min(1.8);
        let tau0 = (2.2 * (261.63 / f0).sqrt()).clamp(0.35, 4.0) * self.decay_scale as f64;
        let mut rng = Pcg32::new(note.seed);
        let mut parts = [Partial::default(); 5];
        let mut level = 0.0;
        for (k, &(ratio, rel, div)) in BELL_PARTIALS.iter().enumerate() {
            let cents = rng.range(-1.5, 1.5) as f64;
            let f = f0 * ratio * 2f64.powf(cents / 1200.0);
            if f >= 0.45 * sr {
                continue;
            }
            let w = std::f64::consts::TAU * f / sr;
            let amp = base * rel * if k == 0 { 1.0 } else { tone };
            parts[k] = Partial {
                re: 1.0,
                im: 0.0,
                cos_w: w.cos(),
                sin_w: w.sin(),
                amp,
                decay: (-1.0 / (tau0 / div * sr)).exp(),
            };
            level += amp;
        }
        self.voices[slot] = BellVoice {
            active: true,
            id: note.id,
            parts,
            age: 0,
            releasing: false,
            rel_gain: 1.0,
            rel_coef: 1.0,
            level,
        };
        self.order.push(slot);
    }

    fn note_off(&mut self, id: u64) {
        let coef = (-1.0 / (self.release_s.max(0.001) as f64 * self.sr)).exp();
        for v in &mut self.voices {
            if v.active && v.id == id && !v.releasing {
                v.releasing = true;
                v.rel_coef = coef;
            }
        }
    }

    fn all_notes_off(&mut self) {
        let coef = (-1.0 / (self.release_s.max(0.001) as f64 * self.sr)).exp();
        for v in &mut self.voices {
            if v.active && !v.releasing {
                v.releasing = true;
                v.rel_coef = coef;
            }
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.decay_scale = value.clamp(0.25, 4.0),
            1 => self.release_s = value.clamp(0.05, 4.0),
            _ => {}
        }
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        left[..n].fill(0.0);
        let attack = self.attack;
        let inv_attack = 1.0 / attack as f64;
        for &slot in &self.order {
            let v = &mut self.voices[slot];
            for out in left[..n].iter_mut() {
                let mut s = 0.0f64;
                let mut lvl = 0.0f64;
                for p in v.parts.iter_mut() {
                    if p.amp == 0.0 {
                        continue;
                    }
                    let re = p.re * p.cos_w - p.im * p.sin_w;
                    let im = p.re * p.sin_w + p.im * p.cos_w;
                    p.re = re;
                    p.im = im;
                    s += p.amp * im;
                    lvl += p.amp;
                    p.amp *= p.decay;
                    if p.amp < 1e-9 {
                        p.amp = 0.0;
                    }
                }
                let att = if v.age < attack {
                    (v.age + 1) as f64 * inv_attack
                } else {
                    1.0
                };
                *out += (s * att * v.rel_gain) as f32;
                if v.releasing {
                    v.rel_gain *= v.rel_coef;
                }
                v.age = v.age.saturating_add(1);
                v.level = lvl * v.rel_gain;
                if v.level < FREE_LEVEL {
                    v.active = false;
                    break;
                }
            }
        }
        let voices = &self.voices;
        self.order.retain(|&s| voices[s].active);
        let (l, r) = (&left[..n], &mut right[..n]);
        r.copy_from_slice(l);
    }

    fn tail_samples(&self) -> usize {
        (RELEASE_TAUS * self.release_s as f64 * self.sr).ceil() as usize + self.attack as usize
    }

    fn reset(&mut self) {
        for v in &mut self.voices {
            v.active = false;
        }
        self.order.clear();
    }
}

// ---------------------------------------------------------------- kick

const KICK_VOICES: usize = 4;
const KICK_START_HZ: f64 = 140.0;
const KICK_SWEEP_S: f64 = 0.035;

#[derive(Clone, Copy, Debug, Default)]
struct KickVoice {
    active: bool,
    phase: f64,
    sweep: f64,
    amp: f64,
    age: u32,
}

/// Test soft kick: sine swept from 140 Hz down to `pitch_hz`, short exponential decay.
pub struct TestKick {
    sr: f64,
    decay_s: f32,
    end_hz: f32,
    attack: u32,
    voices: Vec<KickVoice>,
    order: Vec<usize>,
}

impl TestKick {
    pub fn new(sample_rate: f32, decay_s: f32, end_hz: f32) -> TestKick {
        let sr = sample_rate.max(1.0) as f64;
        TestKick {
            sr,
            decay_s,
            end_hz,
            attack: ((0.001 * sr).round() as u32).max(1),
            voices: vec![KickVoice::default(); KICK_VOICES],
            order: Vec::with_capacity(KICK_VOICES),
        }
    }
}

impl Instrument for TestKick {
    fn note_on(&mut self, note: &NoteOn) {
        let slot = match self.voices.iter().position(|v| !v.active) {
            Some(i) => i,
            None => {
                let mut best = 0usize;
                for i in 1..self.voices.len() {
                    if self.voices[i].amp < self.voices[best].amp {
                        best = i;
                    }
                }
                remove_slot(&mut self.order, best);
                best
            }
        };
        let vel = note.velocity.clamp(0.0, 1.0) as f64;
        self.voices[slot] = KickVoice {
            active: true,
            phase: 0.0,
            sweep: 1.0,
            amp: 0.5 * vel.powf(1.4),
            age: 0,
        };
        self.order.push(slot);
    }

    fn note_off(&mut self, _id: u64) {
        // A drum hit decays on its own.
    }

    fn all_notes_off(&mut self) {}

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.decay_s = value.clamp(0.05, 1.0),
            1 => self.end_hz = value.clamp(30.0, 120.0),
            _ => {}
        }
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        left[..n].fill(0.0);
        let sr = self.sr;
        let decay = (-1.0 / (self.decay_s as f64 * sr)).exp();
        let sweep_coef = (-1.0 / (KICK_SWEEP_S * sr)).exp();
        let end = self.end_hz as f64;
        let attack = self.attack;
        for &slot in &self.order {
            let v = &mut self.voices[slot];
            for out in left[..n].iter_mut() {
                let f = end + (KICK_START_HZ - end) * v.sweep;
                v.phase += f / sr;
                if v.phase >= 1.0 {
                    v.phase -= 1.0;
                }
                let att = if v.age < attack {
                    (v.age + 1) as f64 / attack as f64
                } else {
                    1.0
                };
                *out += ((std::f64::consts::TAU * v.phase).sin() * v.amp * att) as f32;
                v.sweep *= sweep_coef;
                v.amp *= decay;
                v.age = v.age.saturating_add(1);
                if v.amp < FREE_LEVEL {
                    v.active = false;
                    break;
                }
            }
        }
        let voices = &self.voices;
        self.order.retain(|&s| voices[s].active);
        let (l, r) = (&left[..n], &mut right[..n]);
        r.copy_from_slice(l);
    }

    fn tail_samples(&self) -> usize {
        (RELEASE_TAUS * self.decay_s as f64 * self.sr).ceil() as usize
    }

    fn reset(&mut self) {
        for v in &mut self.voices {
            v.active = false;
        }
        self.order.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use praxis_score::{Key, Mode, PitchClass};

    fn ctx() -> InstrumentContext {
        InstrumentContext {
            sample_rate: 48_000.0,
            max_block: 128,
            key: Key {
                tonic: PitchClass::A,
                mode: Mode::Minor,
            },
            bpm: 120.0,
            seed: 1,
        }
    }

    fn spec(kind: InstrumentKind, preset: &str) -> InstrumentRef {
        InstrumentRef {
            kind,
            preset: preset.into(),
            params: Default::default(),
        }
    }

    fn note(id: u64, pitch: u8, velocity: f32, brightness: f32) -> NoteOn {
        NoteOn {
            id,
            pitch,
            velocity,
            brightness,
            seed: 42,
            length_samples: 4800,
        }
    }

    fn render(inst: &mut dyn Instrument, n: usize, block: usize) -> Vec<f32> {
        let mut out = vec![0.0; n];
        let mut r = vec![0.0; block];
        let mut pos = 0;
        while pos < n {
            let m = block.min(n - pos);
            inst.process(&mut out[pos..pos + m], &mut r[..m]);
            pos += m;
        }
        out
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn bell_rings_releases_and_is_velocity_sensitive() {
        let f = TestFactory::new();
        let mut a = f
            .create(&spec(InstrumentKind::Synth, "bell"), &ctx())
            .unwrap();
        a.note_on(&note(1, 69, 0.9, 0.0));
        let loud = render(a.as_mut(), 4800, 128);
        a.note_off(1);
        let tail_len = a.tail_samples() + 48_000;
        let tail = render(a.as_mut(), tail_len, 128);
        assert!(rms(&loud) > 0.01);
        assert!(
            tail[..4800].iter().any(|v| v.abs() > 1e-3),
            "release rings on"
        );
        assert!(
            tail[tail.len() - 48_000..].iter().all(|v| *v == 0.0),
            "voice freed within tail_samples"
        );
        let mut b = f
            .create(&spec(InstrumentKind::Synth, "bell"), &ctx())
            .unwrap();
        b.note_on(&note(1, 69, 0.3, 0.0));
        assert!(rms(&render(b.as_mut(), 4800, 128)) < rms(&loud) * 0.5);
        // Brightness opens the upper partials: more high-frequency energy (first difference).
        let hf = |x: &[f32]| rms(&x.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>());
        let mut c = f
            .create(&spec(InstrumentKind::Synth, "bell"), &ctx())
            .unwrap();
        c.note_on(&note(1, 69, 0.9, 1.0));
        assert!(hf(&render(c.as_mut(), 4800, 128)) > hf(&loud) * 1.1);
    }

    #[test]
    fn output_is_independent_of_block_size() {
        let f = TestFactory::new();
        for (kind, preset) in [
            (InstrumentKind::Synth, "bell"),
            (InstrumentKind::Drum, "soft_kick"),
        ] {
            let mut x = f.create(&spec(kind, preset), &ctx()).unwrap();
            let mut y = f.create(&spec(kind, preset), &ctx()).unwrap();
            x.note_on(&note(1, 60, 0.8, 0.0));
            y.note_on(&note(1, 60, 0.8, 0.0));
            let a = render(x.as_mut(), 20_000, 128);
            let b = render(y.as_mut(), 20_000, 37);
            assert!(a.iter().zip(&b).all(|(p, q)| p.to_bits() == q.to_bits()));
            assert!(a[0] != 0.0, "{preset} sounds on its first sample");
        }
    }

    #[test]
    fn kick_decays_and_frees() {
        let f = TestFactory::new();
        let mut k = f
            .create(&spec(InstrumentKind::Drum, "soft_kick"), &ctx())
            .unwrap();
        k.note_on(&note(1, 36, 0.8, 0.0));
        let len = k.tail_samples() + 4800;
        let out = render(k.as_mut(), len, 128);
        assert!(rms(&out[..4800]) > 0.05);
        assert!(out[out.len() - 4800..].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn unknown_presets_and_params_are_errors() {
        let f = TestFactory::new();
        assert!(matches!(
            f.create(&spec(InstrumentKind::Synth, "nope"), &ctx()),
            Err(InstrumentError::UnknownPreset { .. })
        ));
        let mut s = spec(InstrumentKind::Synth, "bell");
        s.params.insert("bogus".into(), 1.0);
        assert!(matches!(
            f.create(&s, &ctx()),
            Err(InstrumentError::UnknownParam { .. })
        ));
        assert_eq!(f.presets().len(), 2);
    }
}
