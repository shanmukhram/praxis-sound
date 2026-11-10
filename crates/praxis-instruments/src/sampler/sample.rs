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
            padded(r)
        });
        SampleData {
            sample_rate,
            frames,
            left: padded(left),
            right,
        }
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
