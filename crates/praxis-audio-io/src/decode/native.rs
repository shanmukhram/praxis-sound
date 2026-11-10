//! In-process decoders: WAV (hound, plus a fallback reader), FLAC (claxon), Ogg Vorbis (lewton),
//! MP3 (nanomp3).

use super::{DecodeError, Decoded, Format, SourceKind, from_interleaved};
use std::io::Cursor;

pub(crate) enum NativeError {
    /// The container is native but the codec inside is not (e.g. ADPCM in WAV).
    NeedsFfmpeg,
    Failed(DecodeError),
}

impl NativeError {
    pub(crate) fn into_decode_error(self) -> DecodeError {
        match self {
            NativeError::NeedsFfmpeg => DecodeError::UnknownFormat,
            NativeError::Failed(e) => e,
        }
    }
}

impl From<DecodeError> for NativeError {
    fn from(e: DecodeError) -> Self {
        NativeError::Failed(e)
    }
}

pub(crate) fn decode(format: Format, bytes: &[u8]) -> Result<Decoded, NativeError> {
    match format {
        Format::Wav => decode_wav(bytes),
        Format::Flac => Ok(decode_flac(strip_id3v2(bytes))?),
        Format::OggVorbis => Ok(decode_vorbis(bytes)?),
        Format::Mp3 => Ok(decode_mp3(bytes)?),
        _ => Err(NativeError::NeedsFfmpeg),
    }
}

fn strip_id3v2(bytes: &[u8]) -> &[u8] {
    let n = nanomp3::id3v2_len(bytes);
    if n > 0 && n <= bytes.len() {
        &bytes[n..]
    } else {
        bytes
    }
}

// ---- WAV ----

fn decode_wav(bytes: &[u8]) -> Result<Decoded, NativeError> {
    match decode_wav_hound(bytes) {
        Ok(d) => Ok(d),
        Err(hound_err) => match wav_fallback::decode(bytes) {
            Ok(d) => Ok(d),
            Err(wav_fallback::Error::UnsupportedCodec) => Err(NativeError::NeedsFfmpeg),
            Err(wav_fallback::Error::Invalid(why)) => Err(NativeError::Failed(DecodeError::Wav(
                format!("{hound_err} ({why})"),
            ))),
        },
    }
}

fn decode_wav_hound(bytes: &[u8]) -> Result<Decoded, hound::Error> {
    let mut r = hound::WavReader::new(Cursor::new(bytes))?;
    let spec = r.spec();
    let data: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            if spec.bits_per_sample == 0 || spec.bits_per_sample > 32 {
                return Err(hound::Error::Unsupported);
            }
            let full = (1u64 << (spec.bits_per_sample - 1)) as f64;
            r.samples::<i32>()
                .map(|s| s.map(|v| (v as f64 / full) as f32))
                .collect::<Result<_, _>>()?
        }
    };
    if spec.channels == 0 {
        return Err(hound::Error::FormatError("zero channels"));
    }
    Ok(from_interleaved(
        &data,
        spec.channels,
        spec.sample_rate,
        SourceKind::Audio,
    ))
}

/// A permissive RIFF/WAVE reader for layouts hound rejects: 64-bit float, unusual `fmt ` chunk
/// sizes, odd container widths, truncated `data` chunks (streamed WAVs with a 0 or
/// 0xFFFFFFFF size).
mod wav_fallback {
    use super::super::{Decoded, SourceKind, from_interleaved};

    pub(crate) enum Error {
        UnsupportedCodec,
        Invalid(&'static str),
    }

    const PCM: u16 = 1;
    const FLOAT: u16 = 3;
    const EXTENSIBLE: u16 = 0xFFFE;

    struct Fmt {
        tag: u16,
        channels: u16,
        sample_rate: u32,
        block_align: u16,
        bits: u16,
    }

    fn u16le(b: &[u8], at: usize) -> Option<u16> {
        Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
    }

    fn u32le(b: &[u8], at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
    }

    fn parse_fmt(c: &[u8]) -> Result<Fmt, Error> {
        let short = || Error::Invalid("short fmt chunk");
        let mut tag = u16le(c, 0).ok_or_else(short)?;
        let channels = u16le(c, 2).ok_or_else(short)?;
        let sample_rate = u32le(c, 4).ok_or_else(short)?;
        let block_align = u16le(c, 12).ok_or_else(short)?;
        let mut bits = u16le(c, 14).ok_or_else(short)?;
        if tag == EXTENSIBLE {
            // cbSize(2) validBits(2) channelMask(4) subFormat GUID(16), whose first two bytes
            // are the format tag. Samples with fewer valid bits are left-justified in the
            // container, so reading them at container width gives the right value.
            tag = u16le(c, 24).ok_or(Error::Invalid("short WAVE_FORMAT_EXTENSIBLE chunk"))?;
        }
        if bits == 0 {
            bits = block_align.checked_div(channels).unwrap_or(0) * 8;
        }
        Ok(Fmt {
            tag,
            channels,
            sample_rate,
            block_align,
            bits,
        })
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Decoded, Error> {
        if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err(Error::Invalid("not RIFF/WAVE"));
        }
        let mut pos = 12usize;
        let mut fmt: Option<Fmt> = None;
        let mut data: Option<&[u8]> = None;
        while pos + 8 <= bytes.len() {
            let id = &bytes[pos..pos + 4];
            let size = u32le(bytes, pos + 4).unwrap_or(0) as usize;
            let body_start = pos + 8;
            let body_end = body_start.saturating_add(size).min(bytes.len());
            match id {
                b"fmt " => fmt = Some(parse_fmt(&bytes[body_start..body_end])?),
                b"data" => {
                    // Streamed files write 0 or 0xFFFFFFFF; take everything to the end then.
                    let end = if size == 0 || size == u32::MAX as usize {
                        bytes.len()
                    } else {
                        body_end
                    };
                    data = Some(&bytes[body_start..end]);
                    break;
                }
                _ => {}
            }
            pos = body_start.saturating_add(size).saturating_add(size & 1);
        }
        let fmt = fmt.ok_or(Error::Invalid("no fmt chunk"))?;
        let data = data.ok_or(Error::Invalid("no data chunk"))?;
        if fmt.channels == 0 {
            return Err(Error::Invalid("zero channels"));
        }
        let width = (fmt.bits as usize).div_ceil(8);
        let frame = (fmt.block_align as usize).max(width * fmt.channels as usize);
        if width == 0 || frame == 0 {
            return Err(Error::Invalid("zero sample width"));
        }
        let ch = fmt.channels as usize;
        let frames = data.len() / frame;
        let mut out = Vec::with_capacity(frames * ch);
        for f in 0..frames {
            for c in 0..ch {
                let at = f * frame + c * width;
                let s = &data[at..at + width];
                let v = match (fmt.tag, width) {
                    (PCM, 1) => (s[0] as f64 - 128.0) / 128.0,
                    (PCM, 2) => i16::from_le_bytes([s[0], s[1]]) as f64 / 32768.0,
                    (PCM, 3) => {
                        (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f64 / 8_388_608.0
                    }
                    (PCM, 4) => {
                        i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64 / 2_147_483_648.0
                    }
                    (FLOAT, 4) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64,
                    (FLOAT, 8) => {
                        f64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])
                    }
                    (PCM | FLOAT, _) => return Err(Error::Invalid("unsupported sample width")),
                    _ => return Err(Error::UnsupportedCodec),
                };
                out.push(if v.is_finite() { v as f32 } else { 0.0 });
            }
        }
        Ok(from_interleaved(
            &out,
            fmt.channels,
            fmt.sample_rate,
            SourceKind::Audio,
        ))
    }
}

// ---- FLAC ----

fn decode_flac(bytes: &[u8]) -> Result<Decoded, DecodeError> {
    let err = |e: claxon::Error| DecodeError::Flac(e.to_string());
    let mut r = claxon::FlacReader::new(Cursor::new(bytes)).map_err(err)?;
    let info = r.streaminfo();
    if info.channels == 0 || info.bits_per_sample == 0 || info.bits_per_sample > 32 {
        return Err(DecodeError::Flac("invalid stream info".into()));
    }
    let full = (1u64 << (info.bits_per_sample - 1)) as f64;
    let data: Vec<f32> = r
        .samples()
        .map(|s| s.map(|v| (v as f64 / full) as f32))
        .collect::<Result<_, _>>()
        .map_err(err)?;
    Ok(from_interleaved(
        &data,
        info.channels as u16,
        info.sample_rate,
        SourceKind::Audio,
    ))
}

// ---- Ogg Vorbis ----

fn decode_vorbis(bytes: &[u8]) -> Result<Decoded, DecodeError> {
    use lewton::inside_ogg::OggStreamReader;
    use lewton::samples::InterleavedSamples;
    let err = |e: lewton::VorbisError| DecodeError::Vorbis(e.to_string());
    let mut r = OggStreamReader::new(Cursor::new(bytes)).map_err(err)?;
    let channels = r.ident_hdr.audio_channels as u16;
    let sample_rate = r.ident_hdr.audio_sample_rate;
    if channels == 0 {
        return Err(DecodeError::Vorbis("zero channels".into()));
    }
    let mut data = Vec::new();
    while let Some(pck) = r
        .read_dec_packet_generic::<InterleavedSamples<f32>>()
        .map_err(err)?
    {
        // A chained stream may change the channel count; keep the first layout.
        if pck.channel_count == channels as usize {
            data.extend_from_slice(&pck.samples);
        }
    }
    Ok(from_interleaved(
        &data,
        channels,
        sample_rate,
        SourceKind::Audio,
    ))
}

// ---- MP3 ----

fn decode_mp3(bytes: &[u8]) -> Result<Decoded, DecodeError> {
    let d = nanomp3::decode_all::<f32>(bytes);
    let channels = match d.channels {
        Some(c) => c.num() as u16,
        None if d.samples.is_empty() => {
            return Err(DecodeError::Mp3("no MPEG audio frames found".into()));
        }
        // Switched between mono and stereo mid-stream; nanomp3 then stops at the switch.
        None => return Err(DecodeError::Mp3("channel count changes mid-stream".into())),
    };
    if d.sample_rate == 0 {
        return Err(DecodeError::Mp3("no MPEG audio frames found".into()));
    }
    Ok(from_interleaved(
        &d.samples,
        channels,
        d.sample_rate,
        SourceKind::Audio,
    ))
}
