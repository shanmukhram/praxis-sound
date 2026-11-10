//! The typed region model shared by the SFZ and SoundFont 2 parsers.
//!
//! A [`Region`] is one sample mapped to a key and velocity range with its playback settings.
//! Units follow SFZ: times in seconds, levels in dB or percent, tuning in cents.

/// Where a region's audio comes from.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SampleRef {
    /// Pack-relative file path (already resolved against the SFZ file and `default_path`).
    File(String),
    /// Index into the SoundFont's sample headers.
    Sf2(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LoopMode {
    /// Play from offset to end; note-off releases.
    #[default]
    NoLoop,
    /// Play to the end ignoring note-off.
    OneShot,
    /// Loop forever between loop points; note-off releases.
    Continuous,
    /// Loop until note-off, then play on to the end while releasing.
    Sustain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Trigger {
    #[default]
    Attack,
    /// Plays at note-off (release samples), attenuated by `rt_decay` per second held.
    Release,
    /// Plays only when no other note is held.
    First,
    /// Plays only when another note is held.
    Legato,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterType {
    Lpf1p,
    Lpf2p,
    Hpf1p,
    Hpf2p,
    Bpf2p,
}

/// A static per-region filter (`fil_type`, `cutoff`, `resonance`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionFilter {
    pub kind: FilterType,
    pub cutoff_hz: f32,
    pub resonance_db: f32,
}

/// DAHDSR amplitude envelope (`ampeg_*`). Times in seconds, sustain 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmpEnvelope {
    pub delay: f32,
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl Default for AmpEnvelope {
    /// SFZ v1 defaults: everything 0 except sustain 100% and a 1 ms release.
    fn default() -> Self {
        AmpEnvelope {
            delay: 0.0,
            attack: 0.0,
            hold: 0.0,
            decay: 0.0,
            sustain: 1.0,
            release: 0.001,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub sample: SampleRef,
    pub lokey: u8,
    pub hikey: u8,
    pub lovel: u8,
    pub hivel: u8,
    pub pitch_keycenter: u8,
    /// Cents per key (100 = normal keyboard tracking).
    pub pitch_keytrack: f32,
    /// Cents.
    pub tune: f32,
    /// Semitones.
    pub transpose: i32,
    /// dB.
    pub volume: f32,
    /// -100..=100.
    pub pan: f32,
    /// Stereo width in percent, -100..=100 (100 = unchanged).
    pub width: f32,
    /// Percent, -100..=100.
    pub amp_veltrack: f32,
    pub ampeg: AmpEnvelope,
    /// Start offset in sample frames.
    pub offset: u64,
    /// Last frame to play (inclusive), if limited.
    pub end: Option<u64>,
    /// `None` = SFZ default: loop continuously if loop points are given, else no loop.
    pub loop_mode: Option<LoopMode>,
    pub loop_start: Option<u64>,
    /// Inclusive.
    pub loop_end: Option<u64>,
    pub trigger: Trigger,
    /// dB per second the note was held (release triggers only).
    pub rt_decay: f32,
    pub seq_length: u32,
    pub seq_position: u32,
    pub lorand: f32,
    pub hirand: f32,
    pub xfin_lovel: u8,
    pub xfin_hivel: u8,
    pub xfout_lovel: u8,
    pub xfout_hivel: u8,
    pub filter: Option<RegionFilter>,
    /// "file:line" of the `<region>` header, for messages.
    pub origin: String,
}

impl Region {
    pub fn new(sample: SampleRef) -> Region {
        Region {
            sample,
            lokey: 0,
            hikey: 127,
            lovel: 0,
            hivel: 127,
            pitch_keycenter: 60,
            pitch_keytrack: 100.0,
            tune: 0.0,
            transpose: 0,
            volume: 0.0,
            pan: 0.0,
            width: 100.0,
            amp_veltrack: 100.0,
            ampeg: AmpEnvelope::default(),
            offset: 0,
            end: None,
            loop_mode: None,
            loop_start: None,
            loop_end: None,
            trigger: Trigger::Attack,
            rt_decay: 0.0,
            seq_length: 1,
            seq_position: 1,
            lorand: 0.0,
            hirand: 1.0,
            xfin_lovel: 0,
            xfin_hivel: 0,
            xfout_lovel: 127,
            xfout_hivel: 127,
            filter: None,
            origin: String::new(),
        }
    }

    pub fn matches_key(&self, key: u8) -> bool {
        self.lokey <= key && key <= self.hikey
    }

    pub fn matches_vel(&self, vel: u8) -> bool {
        self.lovel <= vel && vel <= self.hivel
    }

    /// True if any `xfin_*`/`xfout_*` opcode changed the defaults.
    pub fn has_explicit_xfade(&self) -> bool {
        self.xfin_hivel > 0 || self.xfout_lovel < 127
    }

    /// The effective loop mode (SFZ default when unspecified).
    pub fn effective_loop_mode(&self) -> LoopMode {
        match self.loop_mode {
            Some(m) => m,
            None => match (self.loop_start, self.loop_end) {
                (Some(s), Some(e)) if e > s => LoopMode::Continuous,
                _ => LoopMode::NoLoop,
            },
        }
    }
}

/// Parses an SFZ key value: a MIDI number or a note name (`c4` = 60, `f#3`, `eb2`, `c-1` = 0).
pub fn parse_key(v: &str) -> Result<u8, String> {
    let v = v.trim();
    if let Ok(n) = v.parse::<i32>() {
        return if (0..=127).contains(&n) {
            Ok(n as u8)
        } else {
            Err(format!("key {n} is outside 0..=127"))
        };
    }
    let lower = v.to_ascii_lowercase();
    let mut chars = lower.chars();
    let pc = match chars.next() {
        Some('c') => 0,
        Some('d') => 2,
        Some('e') => 4,
        Some('f') => 5,
        Some('g') => 7,
        Some('a') => 9,
        Some('b') => 11,
        _ => return Err(format!("`{v}` is not a key number or note name")),
    };
    let rest: String = chars.collect();
    let (acc, oct) = if let Some(r) = rest.strip_prefix('#') {
        (1, r)
    } else if let Some(r) = rest.strip_prefix('b') {
        (-1, r)
    } else {
        (0, rest.as_str())
    };
    let oct: i32 = oct
        .parse()
        .map_err(|_| format!("`{v}` is not a key number or note name"))?;
    let n = (oct + 1) * 12 + pc + acc;
    if (0..=127).contains(&n) {
        Ok(n as u8)
    } else {
        Err(format!("note `{v}` is outside 0..=127"))
    }
}

fn num<T: std::str::FromStr>(v: &str) -> Result<T, String> {
    v.trim()
        .parse::<T>()
        .map_err(|_| format!("`{v}` is not a valid number"))
}

fn f(v: &str) -> Result<f32, String> {
    let x: f32 = num(v)?;
    if x.is_finite() {
        Ok(x)
    } else {
        Err(format!("`{v}` is not finite"))
    }
}

fn vel(v: &str) -> Result<u8, String> {
    let n: i32 = num(v)?;
    if (0..=127).contains(&n) {
        Ok(n as u8)
    } else {
        Err(format!("velocity {n} is outside 0..=127"))
    }
}

fn frames(v: &str) -> Result<u64, String> {
    let n: i64 = num(v)?;
    if n < 0 {
        Err(format!("sample position {n} is negative"))
    } else {
        Ok(n as u64)
    }
}

fn shift_key(k: u8, offset: i32) -> Result<u8, String> {
    let n = k as i32 + offset;
    if (0..=127).contains(&n) {
        Ok(n as u8)
    } else {
        Err(format!(
            "key {k} shifted by note/octave offset {offset} is outside 0..=127"
        ))
    }
}

/// What applying an opcode did.
#[derive(Debug, PartialEq, Eq)]
pub enum Applied {
    Yes,
    /// Opcode not supported; the caller warns and ignores it.
    Unsupported,
}

impl Region {
    /// Applies one SFZ opcode (`sample` must already be resolved to a pack path).
    /// `key_offset` is `note_offset + 12 * octave_offset` from `<control>`.
    pub fn apply_opcode(&mut self, op: &str, v: &str, key_offset: i32) -> Result<Applied, String> {
        let key = |v: &str| parse_key(v).and_then(|k| shift_key(k, key_offset));
        match op {
            "sample" => self.sample = SampleRef::File(v.to_string()),
            "lokey" => self.lokey = key(v)?,
            "hikey" => self.hikey = key(v)?,
            "key" => {
                let k = key(v)?;
                self.lokey = k;
                self.hikey = k;
                self.pitch_keycenter = k;
            }
            "pitch_keycenter" => self.pitch_keycenter = key(v)?,
            "pitch_keytrack" => self.pitch_keytrack = f(v)?.clamp(-1200.0, 1200.0),
            "lovel" => self.lovel = vel(v)?,
            "hivel" => self.hivel = vel(v)?,
            "tune" | "pitch" => self.tune = f(v)?.clamp(-9600.0, 9600.0),
            "transpose" => self.transpose = num::<i32>(v)?.clamp(-127, 127),
            "volume" | "gain" => self.volume = f(v)?.clamp(-144.0, 48.0),
            "pan" => self.pan = f(v)?.clamp(-100.0, 100.0),
            "width" => self.width = f(v)?.clamp(-100.0, 100.0),
            "amp_veltrack" => self.amp_veltrack = f(v)?.clamp(-100.0, 100.0),
            "ampeg_delay" => self.ampeg.delay = f(v)?.clamp(0.0, 100.0),
            "ampeg_attack" => self.ampeg.attack = f(v)?.clamp(0.0, 100.0),
            "ampeg_hold" => self.ampeg.hold = f(v)?.clamp(0.0, 100.0),
            "ampeg_decay" => self.ampeg.decay = f(v)?.clamp(0.0, 100.0),
            "ampeg_sustain" => self.ampeg.sustain = f(v)?.clamp(0.0, 100.0) / 100.0,
            "ampeg_release" => self.ampeg.release = f(v)?.clamp(0.0, 100.0),
            "offset" => self.offset = frames(v)?,
            "end" => self.end = Some(frames(v)?),
            "loop_mode" | "loopmode" => {
                self.loop_mode = Some(match v.trim() {
                    "no_loop" => LoopMode::NoLoop,
                    "one_shot" => LoopMode::OneShot,
                    "loop_continuous" => LoopMode::Continuous,
                    "loop_sustain" => LoopMode::Sustain,
                    other => return Err(format!("unknown loop_mode `{other}`")),
                })
            }
            "loop_start" | "loopstart" => self.loop_start = Some(frames(v)?),
            "loop_end" | "loopend" => self.loop_end = Some(frames(v)?),
            "trigger" => {
                self.trigger = match v.trim() {
                    "attack" => Trigger::Attack,
                    "release" => Trigger::Release,
                    "first" => Trigger::First,
                    "legato" => Trigger::Legato,
                    other => return Err(format!("unknown trigger `{other}`")),
                }
            }
            "rt_decay" => self.rt_decay = f(v)?.clamp(0.0, 200.0),
            "seq_length" => self.seq_length = num::<u32>(v)?.clamp(1, 100),
            "seq_position" => self.seq_position = num::<u32>(v)?.clamp(1, 100),
            "lorand" => self.lorand = f(v)?.clamp(0.0, 1.0),
            "hirand" => self.hirand = f(v)?.clamp(0.0, 1.0),
            "xfin_lovel" => self.xfin_lovel = vel(v)?,
            "xfin_hivel" => self.xfin_hivel = vel(v)?,
            "xfout_lovel" => self.xfout_lovel = vel(v)?,
            "xfout_hivel" => self.xfout_hivel = vel(v)?,
            "fil_type" | "filtype" => {
                let kind = match v.trim() {
                    "lpf_1p" => FilterType::Lpf1p,
                    "lpf_2p" => FilterType::Lpf2p,
                    "hpf_1p" => FilterType::Hpf1p,
                    "hpf_2p" => FilterType::Hpf2p,
                    "bpf_2p" => FilterType::Bpf2p,
                    _ => return Ok(Applied::Unsupported),
                };
                let mut flt = self.filter.unwrap_or(RegionFilter {
                    kind,
                    cutoff_hz: f32::INFINITY,
                    resonance_db: 0.0,
                });
                flt.kind = kind;
                self.filter = Some(flt);
            }
            "cutoff" => {
                let c = f(v)?.clamp(1.0, 100_000.0);
                let mut flt = self.filter.unwrap_or(RegionFilter {
                    kind: FilterType::Lpf2p,
                    cutoff_hz: c,
                    resonance_db: 0.0,
                });
                flt.cutoff_hz = c;
                self.filter = Some(flt);
            }
            "resonance" => {
                let r = f(v)?.clamp(0.0, 40.0);
                if let Some(flt) = self.filter.as_mut() {
                    flt.resonance_db = r;
                } else {
                    self.filter = Some(RegionFilter {
                        kind: FilterType::Lpf2p,
                        cutoff_hz: f32::INFINITY,
                        resonance_db: r,
                    });
                }
            }
            _ => return Ok(Applied::Unsupported),
        }
        Ok(Applied::Yes)
    }

    /// Structural checks after all opcodes are applied.
    pub fn validate(&self) -> Result<(), String> {
        if self.lokey > self.hikey {
            return Err(format!("lokey {} > hikey {}", self.lokey, self.hikey));
        }
        if self.lovel > self.hivel {
            return Err(format!("lovel {} > hivel {}", self.lovel, self.hivel));
        }
        if let (Some(s), Some(e)) = (self.loop_start, self.loop_end)
            && e < s
        {
            return Err(format!("loop_end {e} < loop_start {s}"));
        }
        if self.lorand > self.hirand {
            return Err(format!("lorand {} > hirand {}", self.lorand, self.hirand));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names() {
        assert_eq!(parse_key("c4"), Ok(60));
        assert_eq!(parse_key("C4"), Ok(60));
        assert_eq!(parse_key("a4"), Ok(69));
        assert_eq!(parse_key("f#3"), Ok(54));
        assert_eq!(parse_key("eb2"), Ok(39));
        assert_eq!(parse_key("c-1"), Ok(0));
        assert_eq!(parse_key("g9"), Ok(127));
        assert_eq!(parse_key("108"), Ok(108));
        assert!(parse_key("h3").is_err());
        assert!(parse_key("128").is_err());
        assert!(parse_key("g#9").is_err());
    }
}
