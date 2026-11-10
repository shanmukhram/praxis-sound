//! Decoded sample audio (f32, shared between voices through `Arc`) and WAV/FLAC decoding.

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
    left: Vec<f32>,
    /// `None` for mono samples.
    right: Option<Vec<f32>>,
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
            left: padded(left),
            right: right.map(padded),
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
    pub fn left_padded(&self) -> &[f32] {
        &self.left
    }

    /// Padded right channel; the left channel for mono samples.
    #[inline]
    pub fn right_padded(&self) -> &[f32] {
        self.right.as_deref().unwrap_or(&self.left)
    }

    /// Frame `i` (unpadded index), for tests and analysis.
    pub fn frame(&self, i: usize) -> (f32, f32) {
        (self.left[i + PAD_FRONT], self.right_padded()[i + PAD_FRONT])
    }

    /// Approximate memory use in bytes.
    pub fn bytes(&self) -> usize {
        (self.left.len() + self.right.as_ref().map_or(0, |r| r.len())) * 4
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
