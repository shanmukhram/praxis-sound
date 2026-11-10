//! WAV round trips through the frozen `write_wav` and the decoder, plus the WAV layouts the
//! fallback reader handles.

mod common;

use common::*;
use praxis_audio_io::decode::{
    DecodeOptions, FfmpegSetting, Format, SourceKind, decode_bytes, decode_file, decode_file_with,
    detect_format,
};
use praxis_audio_io::{WavFormat, write_wav};
use praxis_dsp::StereoBuffer;

const SR: u32 = 44_100;

fn test_signal() -> StereoBuffer {
    let mut b = stereo_sines(440.0, 660.0, 0.7, SR, SR as usize / 2);
    // Full-scale and out-of-range edges: clamped by the writer.
    b.left[10] = 1.0;
    b.left[11] = -1.0;
    b.right[12] = 1.5;
    b.right[13] = -1.5;
    b
}

fn clamped(b: &StereoBuffer) -> StereoBuffer {
    StereoBuffer {
        left: b.left.iter().map(|v| v.clamp(-1.0, 1.0)).collect(),
        right: b.right.iter().map(|v| v.clamp(-1.0, 1.0)).collect(),
    }
}

fn round_trip(format: WavFormat, tolerance: f32) {
    let dir = temp_dir(&format!("wav_round_trip_{format:?}"));
    let path = dir.join("x.wav");
    let src = test_signal();
    write_wav(&path, &src, SR, format).unwrap();
    let d = decode_file(&path).unwrap();
    assert_eq!(d.sample_rate, SR);
    assert_eq!(d.channels, 2);
    assert_eq!(d.source_kind, SourceKind::Audio);
    assert_eq!(d.audio.len(), src.len());
    assert!((d.duration_s - src.len() as f64 / SR as f64).abs() < 1e-12);
    let want = if format == WavFormat::Float32 {
        src.clone()
    } else {
        clamped(&src)
    };
    let el = max_abs_diff(&d.audio.left, &want.left);
    let er = max_abs_diff(&d.audio.right, &want.right);
    println!("{format:?}: max error L {el:e} R {er:e} (tolerance {tolerance:e})");
    assert!(el <= tolerance && er <= tolerance, "{el} {er}");

    // The same bytes decode identically from memory.
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(detect_format(&bytes), Format::Wav);
    assert_eq!(decode_bytes(&bytes, Some("x.wav")).unwrap(), d);
}

#[test]
fn wav_16_bit_round_trip() {
    // Writer scales by 32767 and rounds, reader divides by 32768: half a step of rounding plus
    // the 1/32768 scale difference at full scale.
    round_trip(WavFormat::Pcm16, 1.5 / 32768.0);
}

#[test]
fn wav_24_bit_round_trip() {
    round_trip(WavFormat::Pcm24, 1.5 / 8_388_608.0);
}

#[test]
fn wav_32_bit_float_round_trip_is_exact() {
    round_trip(WavFormat::Float32, 0.0);
}

fn write_hound(path: &std::path::Path, spec: hound::WavSpec, frames: &[Vec<i32>]) {
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for f in frames {
        for &s in f {
            match spec.bits_per_sample {
                8 => w.write_sample(s as i8).unwrap(),
                16 => w.write_sample(s as i16).unwrap(),
                _ => w.write_sample(s).unwrap(),
            }
        }
    }
    w.finalize().unwrap();
}

#[test]
fn mono_is_duplicated_to_both_sides() {
    let dir = temp_dir("wav_mono");
    let path = dir.join("mono.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 22_050,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let frames: Vec<Vec<i32>> = (0..1000).map(|i| vec![(i * 30) - 15000]).collect();
    write_hound(&path, spec, &frames);
    let d = decode_file(&path).unwrap();
    assert_eq!(d.channels, 1);
    assert_eq!(d.sample_rate, 22_050);
    assert_eq!(d.audio.len(), 1000);
    assert_eq!(d.audio.left, d.audio.right);
    assert_eq!(d.audio.left[0], -15000.0 / 32768.0);
}

#[test]
fn eight_bit_unsigned_pcm() {
    let dir = temp_dir("wav_8bit");
    let path = dir.join("u8.wav");
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 8000,
        bits_per_sample: 8,
        sample_format: hound::SampleFormat::Int,
    };
    write_hound(&path, spec, &[vec![-128, 127], vec![0, 64]]);
    let d = decode_file(&path).unwrap();
    assert_eq!(d.audio.left, vec![-1.0, 0.0]);
    assert_eq!(d.audio.right, vec![127.0 / 128.0, 0.5]);
}

#[test]
fn extensible_multichannel_keeps_first_two_and_records_count() {
    let dir = temp_dir("wav_quad");
    let path = dir.join("quad.wav");
    // hound writes WAVE_FORMAT_EXTENSIBLE for more than two channels.
    let spec = hound::WavSpec {
        channels: 4,
        sample_rate: 48_000,
        bits_per_sample: 24,
        sample_format: hound::SampleFormat::Int,
    };
    let frames: Vec<Vec<i32>> = (0..64)
        .map(|i| vec![i * 1000, -i * 1000, 4_000_000, -4_000_000])
        .collect();
    write_hound(&path, spec, &frames);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 0xFFFE);
    let d = decode_file(&path).unwrap();
    assert_eq!(d.channels, 4);
    assert_eq!(d.audio.len(), 64);
    for i in 0..64 {
        assert_eq!(d.audio.left[i], (i as f64 * 1000.0 / 8_388_608.0) as f32);
        assert_eq!(
            d.audio.right[i],
            (-(i as f64) * 1000.0 / 8_388_608.0) as f32
        );
    }
}

/// Builds a minimal RIFF/WAVE file by hand.
fn riff(tag: u16, channels: u16, rate: u32, bits: u16, extra_fmt: &[u8], data: &[u8]) -> Vec<u8> {
    let block = channels * bits.div_ceil(8);
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&tag.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * block as u32).to_le_bytes());
    fmt.extend_from_slice(&block.to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());
    fmt.extend_from_slice(extra_fmt);
    let mut body = b"WAVE".to_vec();
    body.extend_from_slice(b"fmt ");
    body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    body.extend_from_slice(&fmt);
    if fmt.len() % 2 == 1 {
        body.push(0);
    }
    // An unrelated chunk before the data, which readers must skip.
    body.extend_from_slice(b"LIST\x03\0\0\0abc\0");
    body.extend_from_slice(b"data");
    body.extend_from_slice(&(data.len() as u32).to_le_bytes());
    body.extend_from_slice(data);
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

#[test]
fn sixty_four_bit_float_wav() {
    let values = [0.25f64, -0.5, 1.0, -1.0, 0.125, 0.0];
    let data: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    let bytes = riff(3, 2, 96_000, 64, &[], &data);
    let d = decode_bytes(&bytes, None).unwrap();
    assert_eq!(d.sample_rate, 96_000);
    assert_eq!(d.channels, 2);
    assert_eq!(d.audio.left, vec![0.25, 1.0, 0.125]);
    assert_eq!(d.audio.right, vec![-0.5, -1.0, 0.0]);
}

#[test]
fn extensible_float64_and_streamed_data_size() {
    // WAVE_FORMAT_EXTENSIBLE with the IEEE float sub-format, 64-bit, mono, and a data chunk
    // whose size field says 0xFFFFFFFF (as written by streaming encoders).
    let mut ext = Vec::new();
    ext.extend_from_slice(&22u16.to_le_bytes()); // cbSize
    ext.extend_from_slice(&64u16.to_le_bytes()); // valid bits
    ext.extend_from_slice(&4u32.to_le_bytes()); // channel mask
    ext.extend_from_slice(&[
        0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B,
        0x71,
    ]);
    let data: Vec<u8> = [0.5f64, -0.25]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let mut bytes = riff(0xFFFE, 1, 44_100, 64, &ext, &data);
    let n = bytes.len();
    bytes[n - 20..n - 16].copy_from_slice(&u32::MAX.to_le_bytes());
    let d = decode_bytes(&bytes, None).unwrap();
    assert_eq!(d.channels, 1);
    assert_eq!(d.audio.left, vec![0.5, -0.25]);
    assert_eq!(d.audio.right, vec![0.5, -0.25]);
}

#[test]
fn compressed_wav_codec_needs_ffmpeg() {
    // IMA ADPCM (0x11) inside RIFF/WAVE is not decoded natively.
    let bytes = riff(0x11, 1, 8000, 4, &[2, 0, 0xF9, 0x01], &[0u8; 256]);
    let dir = temp_dir("wav_adpcm");
    let path = dir.join("adpcm.wav");
    std::fs::write(&path, &bytes).unwrap();
    let opts = DecodeOptions {
        ffmpeg: FfmpegSetting::Disabled,
    };
    let err = decode_file_with(&path, &opts).unwrap_err();
    assert_eq!(err.code(), "ffmpeg_missing", "{err}");
}

#[test]
fn corrupt_and_empty_files() {
    let err = decode_bytes(b"", None).unwrap_err();
    assert_eq!(err.code(), "unknown_format");
    let err = decode_bytes(b"RIFF\x04\0\0\0WAVE", None).unwrap_err();
    assert_eq!(err.code(), "invalid_wav", "{err}");
    let opts = DecodeOptions {
        ffmpeg: FfmpegSetting::Disabled,
    };
    let err =
        praxis_audio_io::decode::decode_bytes_with(b"just some text", None, &opts).unwrap_err();
    assert_eq!(err.code(), "unknown_format");
    let err = decode_file(std::path::Path::new("definitely/not/here.wav")).unwrap_err();
    assert_eq!(err.code(), "io");
}
