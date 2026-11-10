//! SoundFont 2 parser: RIFF `sfbk`, `sdta`/`smpl` (16-bit) and the `pdta` hydra
//! (phdr/pbag/pgen/inst/ibag/igen/shdr), flattened to the same [`Region`] model as SFZ.
//!
//! Generator semantics follow the SoundFont 2.04 spec: an instrument's first zone without a
//! `sampleID` is its global zone (likewise a preset's first zone without `instrument`); preset
//! generators add to instrument generators, and key/velocity ranges intersect. Modulators are
//! ignored (the default modulators' velocity-to-attenuation is covered by `amp_veltrack`).

use super::region::{AmpEnvelope, FilterType, LoopMode, Region, RegionFilter, SampleRef};
use super::sample::SampleData;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("SoundFont: {0}")]
pub struct Sf2Error(pub String);

fn e(msg: impl Into<String>) -> Sf2Error {
    Sf2Error(msg.into())
}

/// One sample header (`shdr`), positions in sample points within `smpl`.
#[derive(Clone, Debug, PartialEq)]
pub struct Sf2Sample {
    pub name: String,
    pub start: u32,
    pub end: u32,
    pub loop_start: u32,
    pub loop_end: u32,
    pub sample_rate: u32,
    pub original_pitch: u8,
    pub pitch_correction: i8,
    pub sample_type: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sf2Preset {
    pub name: String,
    pub bank: u16,
    pub program: u16,
    /// Regions with `SampleRef::Sf2(index into samples)`; offsets and loop points are relative
    /// to that sample's `start`.
    pub regions: Vec<Region>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sf2File {
    pub presets: Vec<Sf2Preset>,
    pub samples: Vec<Sf2Sample>,
    /// Byte range of the 16-bit `smpl` data in the source buffer.
    pub smpl: std::ops::Range<usize>,
}

fn u16le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}
fn name20(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

/// Iterates RIFF sub-chunks of `data` (id, body range in the original buffer).
/// A RIFF chunk: four-character id and the byte range of its body.
type Chunk = ([u8; 4], std::ops::Range<usize>);

fn chunks(data: &[u8], base: usize) -> Result<Vec<Chunk>, Sf2Error> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 <= data.len() {
        let id = [data[i], data[i + 1], data[i + 2], data[i + 3]];
        let size = u32le(data, i + 4) as usize;
        let start = i + 8;
        let end = start
            .checked_add(size)
            .filter(|&end| end <= data.len())
            .ok_or_else(|| {
                e(format!(
                    "chunk `{}` overruns the file",
                    String::from_utf8_lossy(&id)
                ))
            })?;
        out.push((id, base + start..base + end));
        i = end + (size & 1);
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug)]
struct Gen {
    oper: u16,
    amount: u16,
}

impl Gen {
    fn i(&self) -> i32 {
        self.amount as i16 as i32
    }
    fn range(&self) -> (u8, u8) {
        ((self.amount & 0xff) as u8, (self.amount >> 8) as u8)
    }
}

// Generator operators used here (SF2.04 section 8.1.2).
const START_OFS: u16 = 0;
const END_OFS: u16 = 1;
const STARTLOOP_OFS: u16 = 2;
const ENDLOOP_OFS: u16 = 3;
const START_COARSE: u16 = 4;
const FILTER_FC: u16 = 8;
const FILTER_Q: u16 = 9;
const END_COARSE: u16 = 12;
const PAN: u16 = 17;
const DELAY_VOL: u16 = 33;
const ATTACK_VOL: u16 = 34;
const HOLD_VOL: u16 = 35;
const DECAY_VOL: u16 = 36;
const SUSTAIN_VOL: u16 = 37;
const RELEASE_VOL: u16 = 38;
const INSTRUMENT: u16 = 41;
const KEY_RANGE: u16 = 43;
const VEL_RANGE: u16 = 44;
const STARTLOOP_COARSE: u16 = 45;
const KEYNUM: u16 = 46;
const ATTENUATION: u16 = 48;
const ENDLOOP_COARSE: u16 = 50;
const COARSE_TUNE: u16 = 51;
const FINE_TUNE: u16 = 52;
const SAMPLE_ID: u16 = 53;
const SAMPLE_MODES: u16 = 54;
const SCALE_TUNING: u16 = 56;
const ROOT_KEY: u16 = 58;
const NGEN: usize = 61;

/// Generator values for one zone, `None` where unset.
#[derive(Clone, Copy)]
struct GenSet([Option<i32>; NGEN], (u8, u8), (u8, u8));

impl GenSet {
    fn empty() -> GenSet {
        GenSet([None; NGEN], (0, 127), (0, 127))
    }
    fn from(gens: &[Gen], base: Option<&GenSet>) -> GenSet {
        let mut s = base.copied().unwrap_or_else(GenSet::empty);
        for g in gens {
            match g.oper {
                KEY_RANGE => s.1 = g.range(),
                VEL_RANGE => s.2 = g.range(),
                op if (op as usize) < NGEN => s.0[op as usize] = Some(g.i()),
                _ => {}
            }
        }
        s
    }
    fn get(&self, op: u16) -> Option<i32> {
        self.0[op as usize]
    }
}

fn timecents_to_s(tc: i32) -> f32 {
    if tc <= -12000 {
        0.0
    } else {
        2f32.powf(tc as f32 / 1200.0)
    }
}

fn read_gens(b: &[u8]) -> Vec<Gen> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|c| Gen {
            oper: u16le(c, 0),
            amount: u16le(c, 2),
        })
        .collect()
}

/// Zone bag ranges: for record `i`, generators `[bag[i].gen, bag[i+1].gen)`.
fn bag_gen_index(b: &[u8]) -> Vec<usize> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|c| u16le(c, 0) as usize)
        .collect()
}

/// Parses a SoundFont 2 file.
pub fn parse(data: &[u8]) -> Result<Sf2File, Sf2Error> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"sfbk" {
        return Err(e("not a RIFF sfbk file"));
    }
    let riff_len = (u32le(data, 4) as usize + 8).min(data.len());
    let top = chunks(&data[12..riff_len], 12)?;
    let mut smpl: Option<std::ops::Range<usize>> = None;
    let mut pdta: Vec<Chunk> = Vec::new();
    for (id, r) in top {
        if &id != b"LIST" || r.len() < 4 {
            continue;
        }
        let kind = &data[r.start..r.start + 4];
        let subs = chunks(&data[r.start + 4..r.end], r.start + 4)?;
        if kind == b"sdta" {
            for (sid, sr) in subs {
                if &sid == b"smpl" {
                    smpl = Some(sr);
                }
            }
        } else if kind == b"pdta" {
            pdta = subs;
        }
    }
    let smpl = smpl.ok_or_else(|| e("missing sdta/smpl chunk (only 16-bit samples supported)"))?;
    let get = |name: &[u8; 4], rec: usize| -> Result<&[u8], Sf2Error> {
        let r = pdta
            .iter()
            .find(|(id, _)| id == name)
            .map(|(_, r)| r.clone())
            .ok_or_else(|| e(format!("missing pdta/{}", String::from_utf8_lossy(name))))?;
        let b = &data[r];
        if !b.len().is_multiple_of(rec) || b.len() < 2 * rec {
            return Err(e(format!(
                "pdta/{} has a bad size",
                String::from_utf8_lossy(name)
            )));
        }
        Ok(b)
    };
    let phdr = get(b"phdr", 38)?;
    let pbag = bag_gen_index(get(b"pbag", 4)?);
    let pgen = read_gens(get(b"pgen", 4)?);
    let inst = get(b"inst", 22)?;
    let ibag = bag_gen_index(get(b"ibag", 4)?);
    let igen = read_gens(get(b"igen", 4)?);
    let shdr = get(b"shdr", 46)?;

    let n_points = (smpl.len() / 2) as u32;
    let samples: Vec<Sf2Sample> = shdr
        .as_chunks::<46>()
        .0
        .iter()
        .take(shdr.len() / 46 - 1) // last record is the terminal "EOS"
        .map(|c| Sf2Sample {
            name: name20(&c[0..20]),
            start: u32le(c, 20),
            end: u32le(c, 24),
            loop_start: u32le(c, 28),
            loop_end: u32le(c, 32),
            sample_rate: u32le(c, 36),
            original_pitch: c[40],
            pitch_correction: c[41] as i8,
            sample_type: u16le(c, 44),
        })
        .collect();
    for s in &samples {
        if s.start > s.end || s.end > n_points {
            return Err(e(format!("sample `{}` lies outside smpl", s.name)));
        }
        if s.sample_type & 0x8000 != 0 {
            return Err(e(format!("ROM sample `{}` is not supported", s.name)));
        }
    }

    let zone_gens = |bags: &[usize], gens: &[Gen], first: usize, last: usize| {
        (first..last)
            .map(|z| {
                let (a, b) = (bags[z], bags[z + 1]);
                if a > b || b > gens.len() {
                    Err(e("zone generator index out of range"))
                } else {
                    Ok(gens[a..b].to_vec())
                }
            })
            .collect::<Result<Vec<_>, _>>()
    };

    // Instruments: list of (key range, vel range, generator set, sample id) per zone.
    let n_inst = inst.len() / 22 - 1;
    let mut instruments: Vec<Vec<(GenSet, usize)>> = Vec::with_capacity(n_inst);
    for i in 0..n_inst {
        let first = u16le(inst, i * 22 + 20) as usize;
        let last = u16le(inst, (i + 1) * 22 + 20) as usize;
        if first > last || last >= ibag.len() {
            return Err(e("instrument bag index out of range"));
        }
        let zones = zone_gens(&ibag, &igen, first, last)?;
        let mut global: Option<GenSet> = None;
        let mut out = Vec::new();
        for (zi, z) in zones.iter().enumerate() {
            match z.iter().rev().find(|g| g.oper == SAMPLE_ID) {
                Some(g) => {
                    let sid = g.amount as usize;
                    if sid >= samples.len() {
                        return Err(e(format!("sampleID {sid} out of range")));
                    }
                    out.push((GenSet::from(z, global.as_ref()), sid));
                }
                None if zi == 0 => global = Some(GenSet::from(z, None)),
                None => {}
            }
        }
        instruments.push(out);
    }

    let n_presets = phdr.len() / 38 - 1;
    let mut presets = Vec::with_capacity(n_presets);
    for p in 0..n_presets {
        let c = &phdr[p * 38..p * 38 + 38];
        let first = u16le(c, 24) as usize;
        let last = u16le(phdr, (p + 1) * 38 + 24) as usize;
        if first > last || last >= pbag.len() {
            return Err(e("preset bag index out of range"));
        }
        let zones = zone_gens(&pbag, &pgen, first, last)?;
        let mut global: Option<GenSet> = None;
        let mut regions = Vec::new();
        for (zi, z) in zones.iter().enumerate() {
            match z.iter().rev().find(|g| g.oper == INSTRUMENT) {
                Some(g) => {
                    let ii = g.amount as usize;
                    let izones = instruments
                        .get(ii)
                        .ok_or_else(|| e(format!("instrument {ii} out of range")))?;
                    let pset = GenSet::from(z, global.as_ref());
                    for (iset, sid) in izones {
                        if let Some(r) = make_region(pset, *iset, *sid, &samples[*sid]) {
                            regions.push(r);
                        }
                    }
                }
                None if zi == 0 => global = Some(GenSet::from(z, None)),
                None => {}
            }
        }
        presets.push(Sf2Preset {
            name: name20(&c[0..20]),
            program: u16le(c, 20),
            bank: u16le(c, 22),
            regions,
        });
    }
    presets.sort_by_key(|p| (p.bank, p.program));
    Ok(Sf2File {
        presets,
        samples,
        smpl,
    })
}

fn intersect(a: (u8, u8), b: (u8, u8)) -> Option<(u8, u8)> {
    let lo = a.0.max(b.0);
    let hi = a.1.min(b.1).min(127);
    (lo <= hi).then_some((lo, hi))
}

fn make_region(p: GenSet, i: GenSet, sid: usize, s: &Sf2Sample) -> Option<Region> {
    let keys = intersect(p.1, i.1)?;
    let vels = intersect(p.2, i.2)?;
    // Instrument value (or default) plus preset offset.
    let sum = |op: u16, default: i32| i.get(op).unwrap_or(default) + p.get(op).unwrap_or(0);
    let mut r = Region::new(SampleRef::Sf2(sid));
    r.origin = format!("sf2 sample `{}`", s.name);
    r.lokey = keys.0;
    r.hikey = keys.1;
    r.lovel = vels.0;
    r.hivel = vels.1;
    let root = i.get(ROOT_KEY).unwrap_or(-1);
    r.pitch_keycenter = if (0..=127).contains(&root) {
        root as u8
    } else {
        s.original_pitch.min(127)
    };
    if let Some(k) = i.get(KEYNUM).filter(|k| (0..=127).contains(k)) {
        // A fixed key number plays every key as that key.
        r.pitch_keytrack = 0.0;
        r.pitch_keycenter = k as u8;
    } else {
        r.pitch_keytrack = sum(SCALE_TUNING, 100) as f32;
    }
    r.transpose = sum(COARSE_TUNE, 0).clamp(-120, 120);
    r.tune = (sum(FINE_TUNE, 0) + s.pitch_correction as i32) as f32;
    r.volume = -(sum(ATTENUATION, 0).clamp(0, 1440) as f32) / 10.0;
    r.pan = (sum(PAN, 0).clamp(-500, 500) as f32) / 5.0;
    r.ampeg = AmpEnvelope {
        delay: timecents_to_s(sum(DELAY_VOL, -12000)),
        attack: timecents_to_s(sum(ATTACK_VOL, -12000)),
        hold: timecents_to_s(sum(HOLD_VOL, -12000)),
        decay: timecents_to_s(sum(DECAY_VOL, -12000)),
        sustain: 10f32.powf(-(sum(SUSTAIN_VOL, 0).clamp(0, 1440) as f32) / 200.0),
        release: timecents_to_s(sum(RELEASE_VOL, -12000)).max(0.001),
    };
    let fc = sum(FILTER_FC, 13500);
    if fc < 13500 {
        let hz = 8.176 * 2f32.powf(fc.clamp(1500, 13500) as f32 / 1200.0);
        r.filter = Some(RegionFilter {
            kind: FilterType::Lpf2p,
            cutoff_hz: hz,
            resonance_db: sum(FILTER_Q, 0).clamp(0, 960) as f32 / 10.0,
        });
    }
    // Address offsets (instrument-level only per spec), relative to the sample start.
    let ofs = |fine: u16, coarse: u16| sum(fine, 0) as i64 + 32768 * sum(coarse, 0) as i64;
    let len = (s.end - s.start) as i64;
    let start = ofs(START_OFS, START_COARSE).clamp(0, len);
    let end = (len + ofs(END_OFS, END_COARSE)).clamp(start, len);
    r.offset = start as u64;
    r.end = Some((end.max(1) - 1) as u64);
    let ls =
        (s.loop_start as i64 - s.start as i64 + ofs(STARTLOOP_OFS, STARTLOOP_COARSE)).clamp(0, len);
    let le = (s.loop_end as i64 - s.start as i64 + ofs(ENDLOOP_OFS, ENDLOOP_COARSE)).clamp(0, len);
    let mode = match sum(SAMPLE_MODES, 0) & 3 {
        1 => LoopMode::Continuous,
        3 => LoopMode::Sustain,
        _ => LoopMode::NoLoop,
    };
    if mode != LoopMode::NoLoop && le > ls + 1 {
        r.loop_mode = Some(mode);
        r.loop_start = Some(ls as u64);
        // SF2 loop end is the first point after the loop; SFZ loop_end is inclusive.
        r.loop_end = Some((le - 1) as u64);
    } else {
        r.loop_mode = Some(LoopMode::NoLoop);
    }
    r.amp_veltrack = 100.0;
    Some(r)
}

/// Decodes one sample header's points (mono, 16-bit) from the file bytes.
pub fn sample_data(file: &Sf2File, data: &[u8], index: usize) -> Result<SampleData, Sf2Error> {
    let s = file
        .samples
        .get(index)
        .ok_or_else(|| e(format!("sample {index} out of range")))?;
    let smpl = data
        .get(file.smpl.clone())
        .ok_or_else(|| e("smpl range outside data"))?;
    let a = s.start as usize * 2;
    let b = s.end as usize * 2;
    let pts = smpl.get(a..b).ok_or_else(|| e("sample outside smpl"))?;
    let left: Vec<f32> = pts
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect();
    if s.sample_rate == 0 {
        return Err(e(format!("sample `{}` has sample rate 0", s.name)));
    }
    Ok(SampleData::new(s.sample_rate as f32, left, None))
}
