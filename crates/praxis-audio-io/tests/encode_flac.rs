//! FLAC export (flacenc) verified bit-exact with claxon, and stem export.

mod common;

use common::*;
use praxis_audio_io::WavFormat;
use praxis_audio_io::decode::{Format, decode_bytes, decode_file, detect_format};
use praxis_audio_io::encode::{StemFormat, flac_bytes, write_flac, write_stems};
use praxis_dsp::StereoBuffer;

const SR: u32 = 48_000;

fn signal() -> StereoBuffer {
    // Longer than one 4096-frame block, not a multiple of it, with clipping edges and silence.
    let mut b = stereo_sines(220.0, 1234.5, 0.9, SR, 10_001);
    b.left[0] = 1.0;
    b.left[1] = -1.0;
    b.right[2] = 2.0;
    b.right[3] = f32::NAN;
    for i in 5000..5600 {
        b.left[i] = 0.0;
        b.right[i] = 0.0;
    }
    b
}

fn quantize(v: f32, bits: u32) -> i32 {
    let full = ((1i64 << (bits - 1)) - 1) as f64;
    let x = if v.is_finite() { v as f64 } else { 0.0 };
    (x.clamp(-1.0, 1.0) * full).round() as i32
}

fn flac_bit_exact(bits: u8) {
    let dir = temp_dir(&format!("flac_{bits}"));
    let path = dir.join("x.flac");
    let src = signal();
    write_flac(&path, &src, SR, bits).unwrap();

    let mut r = claxon::FlacReader::open(&path).unwrap();
    let info = r.streaminfo();
    assert_eq!(info.sample_rate, SR);
    assert_eq!(info.channels, 2);
    assert_eq!(info.bits_per_sample, bits as u32);
    assert_eq!(info.samples, Some(src.len() as u64));
    let got: Vec<i32> = r.samples().collect::<Result<_, _>>().unwrap();
    let want: Vec<i32> = (0..src.len())
        .flat_map(|i| {
            [
                quantize(src.left[i], bits as u32),
                quantize(src.right[i], bits as u32),
            ]
        })
        .collect();
    assert_eq!(got.len(), want.len());
    assert!(
        got == want,
        "FLAC {bits}-bit samples differ from the quantised input"
    );

    // And through the generic decoder.
    let d = decode_file(&path).unwrap();
    assert_eq!(d.sample_rate, SR);
    assert_eq!(d.channels, 2);
    assert_eq!(d.audio.len(), src.len());
    let full = (1u64 << (bits - 1)) as f64;
    for i in 0..src.len() {
        assert_eq!(d.audio.left[i], (want[2 * i] as f64 / full) as f32);
        assert_eq!(d.audio.right[i], (want[2 * i + 1] as f64 / full) as f32);
    }
    let size = std::fs::metadata(&path).unwrap().len();
    println!(
        "{bits}-bit FLAC: {size} bytes for {} frames ({} bytes raw)",
        src.len(),
        src.len() * 2 * bits as usize / 8
    );
}

#[test]
fn flac_16_bit_is_bit_exact() {
    flac_bit_exact(16);
}

#[test]
fn flac_24_bit_is_bit_exact() {
    flac_bit_exact(24);
}

#[test]
fn flac_is_deterministic_and_validates_arguments() {
    let src = signal();
    assert_eq!(
        flac_bytes(&src, SR, 16).unwrap(),
        flac_bytes(&src, SR, 16).unwrap()
    );
    assert_eq!(
        flac_bytes(&src, SR, 20).unwrap_err().code(),
        "unsupported_bits"
    );
    assert_eq!(
        flac_bytes(&src, 0, 16).unwrap_err().code(),
        "unsupported_rate"
    );
    // Empty audio is a valid (empty) stream.
    let empty = flac_bytes(&StereoBuffer::default(), SR, 16).unwrap();
    assert_eq!(detect_format(&empty), Format::Flac);
}

#[test]
fn flac_behind_an_id3v2_tag_is_detected_as_flac() {
    let src = stereo_sines(440.0, 440.0, 0.5, SR, 2000);
    let flac = flac_bytes(&src, SR, 16).unwrap();
    // ID3v2.4 header with a 10-byte body (syncsafe size), then the FLAC stream.
    let mut bytes = b"ID3\x04\x00\x00\x00\x00\x00\x0a".to_vec();
    bytes.extend_from_slice(&[0u8; 10]);
    bytes.extend_from_slice(&flac);
    assert_eq!(detect_format(&bytes), Format::Flac);
    let d = decode_bytes(&bytes, Some("tagged.mp3")).unwrap();
    assert_eq!(d.audio.len(), 2000);
}

#[test]
fn stems_are_written_per_layer() {
    let dir = temp_dir("stems");
    let stems = vec![
        (
            "piano".to_string(),
            stereo_sines(440.0, 440.0, 0.5, SR, 4800),
        ),
        ("kick".to_string(), stereo_sines(60.0, 60.0, 0.5, SR, 4800)),
    ];
    let wavs = write_stems(
        &dir.join("wav"),
        &stems,
        SR,
        StemFormat::Wav(WavFormat::Pcm24),
    )
    .unwrap();
    assert_eq!(
        wavs,
        vec![dir.join("wav/piano.wav"), dir.join("wav/kick.wav")]
    );
    let flacs = write_stems(&dir.join("flac"), &stems, SR, StemFormat::Flac { bits: 24 }).unwrap();
    assert_eq!(
        flacs,
        vec![dir.join("flac/piano.flac"), dir.join("flac/kick.flac")]
    );
    for (path, (_, audio)) in wavs.iter().chain(&flacs).zip(stems.iter().cycle()) {
        let d = decode_file(path).unwrap();
        assert_eq!(d.sample_rate, SR);
        assert_eq!(d.audio.len(), audio.len());
        assert!(max_abs_diff(&d.audio.left, &audio.left) < 2.0 / 8_388_608.0);
    }
}

#[test]
fn stem_names_are_sanitised_and_unique() {
    let dir = temp_dir("stems_names");
    let a = StereoBuffer::silent(10);
    let out = write_stems(
        &dir,
        &[
            ("../evil/name".to_string(), a.clone()),
            (String::new(), a.clone()),
        ],
        SR,
        StemFormat::Wav(WavFormat::Pcm16),
    )
    .unwrap();
    assert_eq!(
        out,
        vec![dir.join("___evil_name.wav"), dir.join("stem.wav")]
    );
    let err = write_stems(
        &dir,
        &[("bass".to_string(), a.clone()), ("Bass".to_string(), a)],
        SR,
        StemFormat::Flac { bits: 16 },
    )
    .unwrap_err();
    assert_eq!(err.code(), "duplicate_stem");
}
