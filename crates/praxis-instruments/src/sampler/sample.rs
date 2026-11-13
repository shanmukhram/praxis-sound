//! Decoded sample audio (shared between voices through `Arc`) and WAV/FLAC decoding.
//!
//! Storage: a channel whose every value is a whole number of 2^-23 steps in [-1, 1) (all 8-, 16-
//! and 24-bit PCM, as decoded here) is kept as packed 24-bit integers (3 bytes a sample instead
//! of 4) and read back as `v as f32 * 2^-23`, which is exactly the f32 it was decoded to; any
//! other channel (32-bit float WAV, SF2 tests, odd values) stays f32. Reads are bit-identical
//! either way; the felt piano's 24-bit samples take 86 MB instead of 115 MB.

/// Zero frames stored before the first and after the last real frame, so the 4-point
/// interpolator can read one frame before and two after any playable position without
/// bounds branches.
pub const PAD_FRONT: usize = 1;
pub const PAD_BACK: usize = 3;

/// One decoded sample. Channels are stored padded: frame `i` lives at index `i + PAD_FRONT`.
#[derive(Clone, Debug, PartialEq)]
pub struct SampleData {
    pub sample_rate: f32,
    frames: usize,
    left: Store,
    /// `None` for mono samples.
    right: Option<Store>,
    /// Frames of pre-roll before the audible attack, measured at load (see [`measure_onset`]).
    onset: usize,
}

/// Window of the onset envelope (centred RMS), seconds.
pub const ONSET_WINDOW_S: f32 = 0.001;
/// The attack point is where the envelope first reaches this level below the early peak
/// (half amplitude).
pub const ONSET_THRESHOLD_DB: f32 = -6.0;
/// Span after the sample start that defines the early peak, seconds.
pub const ONSET_PEAK_SPAN_S: f32 = 0.08;
/// Playback starts this long before the attack point: the last part of the rise plus the 1 ms
/// click-safe ramp.
pub const ONSET_BACKOFF_S: f32 = 0.002;

/// Where playback of a sample should start so its attack lands on the note time: the attack
/// point minus [`ONSET_BACKOFF_S`], in frames from the sample start.
///
/// Deterministic: a centred 1 ms RMS envelope of the mid signal over the first
/// [`ONSET_PEAK_SPAN_S`]; the attack point is the first frame where it reaches
/// [`ONSET_THRESHOLD_DB`] below the envelope's maximum in that span.
///
/// Why half amplitude and not the first audible sound: in Upright Piano KW the envelope takes
/// 0..10 ms to reach -20 dB and then up to 15 ms more to reach -6 dB. Aligning the -20 dB point
/// still left notes 2..20 ms late (and late by a different amount per sample) to an onset
/// detector, which, like a listener, places the onset of a slow rise near its steep middle.
/// Starting 2 ms before the -6 dB point puts every attack within a few ms of the note time.
/// Measured per sample by the ignored test `sampler_piano::measure_kw_attacks`.
pub fn measure_onset(sample_rate: f32, left: &[f32], right: Option<&[f32]>) -> usize {
    let frames = left.len();
    let w = ((ONSET_WINDOW_S * 0.5 * sample_rate).round() as usize).max(1);
    let span = ((ONSET_PEAK_SPAN_S * sample_rate) as usize).min(frames);
    let n = (span + w).min(frames);
    if span == 0 {
        return 0;
    }
    // Prefix sums of the squared mid signal.
    let mut prefix = Vec::with_capacity(n + 1);
    prefix.push(0f64);
    let mut acc = 0f64;
    for i in 0..n {
        let m = match right {
            Some(r) => 0.5 * (left[i] as f64 + r[i] as f64),
            None => left[i] as f64,
        };
        acc += m * m;
        prefix.push(acc);
    }
    let env = |j: usize| {
        let a = j.saturating_sub(w);
        let b = (j + w).min(n);
        ((prefix[b] - prefix[a]) / (b - a).max(1) as f64).sqrt()
    };
    let peak = (0..span).map(env).fold(0f64, f64::max);
    if peak <= 0.0 {
        return 0;
    }
    let th = peak * 10f64.powf(ONSET_THRESHOLD_DB as f64 / 20.0);
    let onset = (0..span).find(|&j| env(j) >= th).unwrap_or(0);
    onset.saturating_sub((ONSET_BACKOFF_S * sample_rate).round() as usize)
}

/// One padded channel: packed 24-bit (little-endian, 3 bytes a sample) when exact, else f32.
#[derive(Clone, Debug, PartialEq)]
enum Store {
    I24(Vec<u8>),
    F32(Vec<f32>),
}

/// 2^-23: one step of 24-bit PCM scaled to [-1, 1).
const I24_STEP: f32 = 1.0 / 8_388_608.0;

/// `x` as a 24-bit integer when `v as f32 * I24_STEP` gives back exactly `x` (bit for bit).
fn as_i24(x: f32) -> Option<i32> {
    let v = x * 8_388_608.0;
    if !(-8_388_608.0..=8_388_607.0).contains(&v) || v.fract() != 0.0 {
        return None;
    }
    let v = v as i32;
    ((v as f32 * I24_STEP).to_bits() == x.to_bits()).then_some(v)
}

impl Store {
    fn new(v: Vec<f32>) -> Store {
        let packed: Option<Vec<u8>> =
            v.iter()
                .map(|&x| as_i24(x))
                .try_fold(Vec::with_capacity(v.len() * 3), |mut out, i| {
                    let b = i?.to_le_bytes();
                    out.extend_from_slice(&b[..3]);
                    Some(out)
                });
        match packed {
            Some(bytes) => Store::I24(bytes),
            None => Store::F32(v),
        }
    }

    fn bytes(&self) -> usize {
        match self {
            Store::I24(b) => b.len(),
            Store::F32(v) => v.len() * 4,
        }
    }

    fn view(&self) -> Channel<'_> {
        match self {
            Store::I24(b) => Channel::I24(b),
            Store::F32(v) => Channel::F32(v),
        }
    }
}

/// A read-only view of one padded channel ([`SampleData::left_padded`]).
#[derive(Clone, Copy, Debug)]
pub enum Channel<'a> {
    I24(&'a [u8]),
    F32(&'a [f32]),
}

impl Channel<'_> {
    /// Sample `i` of the padded channel, exactly the f32 it was decoded to.
    #[inline(always)]
    pub fn at(self, i: usize) -> f32 {
        match self {
            Channel::I24(b) => {
                let b = &b[3 * i..3 * i + 3];
                // Sign-extend the 24-bit value through the top byte.
                (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 * I24_STEP
            }
            Channel::F32(v) => v[i],
        }
    }

    pub fn len(self) -> usize {
        match self {
            Channel::I24(b) => b.len() / 3,
            Channel::F32(v) => v.len(),
        }
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}

fn padded(mut v: Vec<f32>) -> Vec<f32> {
    let mut out = Vec::with_capacity(v.len() + PAD_FRONT + PAD_BACK);
    out.resize(PAD_FRONT, 0.0);
    out.append(&mut v);
    out.resize(out.len() + PAD_BACK, 0.0);
    out
}

impl SampleData {
    /// Builds from unpadded channel data. `right` must have the same length as `left`.
    pub fn new(sample_rate: f32, left: Vec<f32>, right: Option<Vec<f32>>) -> SampleData {
        let frames = left.len();
        let right = right.map(|mut r| {
            r.resize(frames, 0.0);
            r
        });
        let onset = measure_onset(sample_rate, &left, right.as_deref());
        SampleData {
            sample_rate,
            frames,
            left: Store::new(padded(left)),
            right: right.map(|r| Store::new(padded(r))),
            onset,
        }
    }

    /// Pre-roll frames before the audible attack (measured at load).
    pub fn onset(&self) -> usize {
        self.onset
    }

    pub fn frames(&self) -> usize {
        self.frames
    }

    pub fn is_stereo(&self) -> bool {
        self.right.is_some()
    }

    /// Padded left channel (frame `i` at `i + PAD_FRONT`).
    #[inline]
    pub fn left_padded(&self) -> Channel<'_> {
        self.left.view()
    }

    /// Padded right channel; the left channel for mono samples.
    #[inline]
    pub fn right_padded(&self) -> Channel<'_> {
        self.right.as_ref().unwrap_or(&self.left).view()
    }

    /// Frame `i` (unpadded index), for tests and analysis.
    pub fn frame(&self, i: usize) -> (f32, f32) {
        (
            self.left_padded().at(i + PAD_FRONT),
            self.right_padded().at(i + PAD_FRONT),
        )
    }

    /// Whether the channels are held as packed 24-bit integers (see the module docs).
    pub fn is_packed(&self) -> bool {
        matches!(self.left, Store::I24(_))
    }

    /// Memory use of the audio in bytes.
    pub fn bytes(&self) -> usize {
        self.left.bytes() + self.right.as_ref().map_or(0, Store::bytes)
    }
}

fn split_channels(interleaved: &[f32], channels: usize) -> (Vec<f32>, Option<Vec<f32>>) {
    let n = interleaved.len() / channels.max(1);
    let mut l = Vec::with_capacity(n);
    let mut r = if channels >= 2 {
        Some(Vec::with_capacity(n))
    } else {
        None
    };
    for f in interleaved.chunks_exact(channels.max(1)) {
        l.push(f[0]);
        if let Some(r) = r.as_mut() {
            r.push(f[1]);
        }
    }
    (l, r)
}

/// Decodes a WAV (PCM 8/16/24/32-bit or 32-bit float) into f32.
pub fn decode_wav(bytes: &[u8]) -> Result<SampleData, String> {
    let reader =
        hound::WavReader::new(std::io::Cursor::new(bytes)).map_err(|e| format!("WAV: {e}"))?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.sample_rate == 0 {
        return Err("WAV: no channels or zero sample rate".into());
    }
    let data: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| format!("WAV: {e}"))?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()
                .map_err(|e| format!("WAV: {e}"))?
        }
    };
    let (l, r) = split_channels(&data, spec.channels as usize);
    Ok(SampleData::new(spec.sample_rate as f32, l, r))
}

/// Decodes a FLAC file into f32.
pub fn decode_flac(bytes: &[u8]) -> Result<SampleData, String> {
    let mut reader =
        claxon::FlacReader::new(std::io::Cursor::new(bytes)).map_err(|e| format!("FLAC: {e}"))?;
    let info = reader.streaminfo();
    if info.channels == 0 || info.sample_rate == 0 {
        return Err("FLAC: no channels or zero sample rate".into());
    }
    let ch = info.channels as usize;
    let scale = 1.0 / (1u64 << (info.bits_per_sample.clamp(1, 32) - 1)) as f32;
    let mut l = Vec::with_capacity(info.samples.unwrap_or(0) as usize);
    let mut r = (ch >= 2).then(|| Vec::with_capacity(info.samples.unwrap_or(0) as usize));
    let mut blocks = reader.blocks();
    let mut buf = Vec::new();
    loop {
        match blocks.read_next_or_eof(buf) {
            Ok(Some(block)) => {
                let n = block.duration() as usize;
                let left = block.channel(0);
                l.extend(left.iter().take(n).map(|&v| v as f32 * scale));
                if let Some(r) = r.as_mut() {
                    r.extend(block.channel(1).iter().take(n).map(|&v| v as f32 * scale));
                }
                buf = block.into_buffer();
            }
            Ok(None) => break,
            Err(e) => return Err(format!("FLAC: {e}")),
        }
    }
    Ok(SampleData::new(info.sample_rate as f32, l, r))
}

/// Decodes by content (RIFF/WAVE or fLaC magic).
pub fn decode(bytes: &[u8], name: &str) -> Result<SampleData, String> {
    if bytes.starts_with(b"fLaC") {
        decode_flac(bytes)
    } else if bytes.starts_with(b"RIFF") {
        decode_wav(bytes)
    } else {
        Err(format!(
            "`{name}`: unsupported sample format (WAV and FLAC are supported)"
        ))
    }
}

#[cfg(test)]
mod storage_tests {
    use super::*;

    fn pcm24(n: usize, seed: u32) -> Vec<f32> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let v = (x as i32) >> 8; // any 24-bit value
                v as f32 * (1.0 / (1u64 << 23) as f32)
            })
            .collect()
    }

    #[test]
    fn pcm_is_packed_and_reads_back_bit_for_bit() {
        let mut l = pcm24(5000, 1);
        let r = pcm24(5000, 2);
        l[0] = -1.0;
        l[1] = 8_388_607.0 / 8_388_608.0;
        l[2] = 0.0;
        let s = SampleData::new(44_100.0, l.clone(), Some(r.clone()));
        assert!(s.is_packed());
        assert_eq!(s.bytes(), 2 * (5000 + PAD_FRONT + PAD_BACK) * 3);
        for i in 0..5000 {
            let (a, b) = s.frame(i);
            assert_eq!(a.to_bits(), l[i].to_bits(), "left {i}");
            assert_eq!(b.to_bits(), r[i].to_bits(), "right {i}");
        }
        // The padding reads as zeros.
        assert_eq!(s.left_padded().at(0), 0.0);
        assert_eq!(s.left_padded().at(5000 + PAD_FRONT + PAD_BACK - 1), 0.0);
    }

    #[test]
    fn sixteen_bit_pcm_packs_too() {
        let l: Vec<f32> = (-300i32..300)
            .map(|v| (v * 97) as f32 * (1.0 / 32768.0))
            .collect();
        let s = SampleData::new(48_000.0, l.clone(), None);
        assert!(s.is_packed());
        for (i, x) in l.iter().enumerate() {
            assert_eq!(s.frame(i).0.to_bits(), x.to_bits());
        }
    }

    #[test]
    fn anything_else_stays_float() {
        for odd in [0.1f32, -0.0, 1.0, f32::MIN_POSITIVE, 1e-9] {
            let s = SampleData::new(48_000.0, vec![0.25, odd, -0.5], None);
            assert!(!s.is_packed(), "{odd:e}");
            assert_eq!(s.frame(1).0.to_bits(), odd.to_bits());
        }
    }
}
