//! Native decoding of the committed MP3 and Ogg Vorbis fixtures (see tests/fixtures/README.md),
//! and detection by content rather than by extension.

mod common;

use common::*;
use praxis_audio_io::decode::{
    DecodeOptions, FfmpegSetting, Format, SourceKind, decode_bytes, decode_bytes_with, decode_file,
    decode_file_with, detect_format,
};
use praxis_audio_io::{WavFormat, write_wav};

fn no_ffmpeg() -> DecodeOptions {
    DecodeOptions {
        ffmpeg: FfmpegSetting::Disabled,
    }
}

#[test]
fn mp3_with_id3v2_tag_decodes_natively() {
    let path = fixture("tone440_mono.mp3");
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"ID3"), "fixture carries an ID3v2 tag");
    // Native: works with ffmpeg disabled.
    let d = decode_file_with(&path, &no_ffmpeg()).unwrap();
    assert_eq!(d.sample_rate, 44_100);
    assert_eq!(d.channels, 1);
    assert_eq!(d.source_kind, SourceKind::Audio);
    assert_eq!(d.audio.left, d.audio.right, "mono duplicated");
    // The LAME/Info tag lets nanomp3 trim encoder delay and padding.
    println!("mp3 duration {:.4} s", d.duration_s);
    assert!((d.duration_s - 2.0).abs() < 0.03, "{}", d.duration_s);
    let f = dominant_hz(&d.audio.left, d.sample_rate);
    println!("mp3 dominant {f:.2} Hz");
    assert!((f - 440.0).abs() < 1.0, "{f}");
    let level = rms(&d.audio.left[4410..d.audio.len() - 4410]);
    // lavfi sine sources have amplitude 1/8: rms 0.088.
    assert!((level - 0.0884).abs() < 0.01, "rms {level}");
}

#[test]
fn ogg_vorbis_decodes_natively() {
    let path = fixture("tone660_stereo.ogg");
    let d = decode_file_with(&path, &no_ffmpeg()).unwrap();
    assert_eq!(d.sample_rate, 44_100);
    assert_eq!(d.channels, 2);
    assert_eq!(d.source_kind, SourceKind::Audio);
    println!("ogg duration {:.4} s", d.duration_s);
    // Vorbis carries the exact sample count in the granule position.
    assert!((d.duration_s - 2.0).abs() < 0.001, "{}", d.duration_s);
    for ch in [&d.audio.left, &d.audio.right] {
        let f = dominant_hz(ch, d.sample_rate);
        println!("ogg dominant {f:.2} Hz");
        assert!((f - 660.0).abs() < 1.0, "{f}");
    }
}

#[test]
fn fixtures_are_detected_by_magic_bytes() {
    let cases = [
        ("tone440_mono.mp3", Format::Mp3),
        ("tone660_stereo.ogg", Format::OggVorbis),
        ("tone550.m4a", Format::Mp4),
        ("clip880.mp4", Format::Mp4),
        ("clip880.mov", Format::Mp4),
        ("clip880.webm", Format::Matroska),
        ("video_only.mp4", Format::Mp4),
    ];
    for (name, want) in cases {
        let bytes = std::fs::read(fixture(name)).unwrap();
        assert_eq!(detect_format(&bytes), want, "{name}");
    }
}

#[test]
fn wrong_extensions_are_ignored() {
    let dir = temp_dir("wrong_extensions");
    // A WAV called .mp3, an MP3 called .wav, an Ogg called .flac.
    let wav = dir.join("really_a_wav.mp3");
    let tone = stereo_sines(330.0, 330.0, 0.5, 32_000, 16_000);
    write_wav(&wav, &tone, 32_000, WavFormat::Pcm16).unwrap();
    let mp3 = dir.join("really_an_mp3.wav");
    std::fs::copy(fixture("tone440_mono.mp3"), &mp3).unwrap();
    let ogg = dir.join("really_an_ogg.flac");
    std::fs::copy(fixture("tone660_stereo.ogg"), &ogg).unwrap();

    let d = decode_file_with(&wav, &no_ffmpeg()).unwrap();
    assert_eq!((d.sample_rate, d.audio.len()), (32_000, 16_000));
    let d = decode_file_with(&mp3, &no_ffmpeg()).unwrap();
    assert_eq!((d.sample_rate, d.channels), (44_100, 1));
    assert_eq!(
        d,
        decode_file(&fixture("tone440_mono.mp3")).unwrap(),
        "same result as the correctly named file"
    );
    let d = decode_file_with(&ogg, &no_ffmpeg()).unwrap();
    assert_eq!((d.sample_rate, d.channels), (44_100, 2));

    // In memory, a misleading hint changes nothing either.
    let bytes = std::fs::read(fixture("tone660_stereo.ogg")).unwrap();
    let d2 = decode_bytes_with(&bytes, Some("song.mp4"), &no_ffmpeg()).unwrap();
    assert_eq!(d, d2);
    let d3 = decode_bytes(&std::fs::read(&mp3).unwrap(), Some("clip.webm")).unwrap();
    assert_eq!(d3.channels, 1);
}

#[test]
fn truncated_compressed_files_fail_cleanly() {
    for name in ["tone440_mono.mp3", "tone660_stereo.ogg"] {
        let bytes = std::fs::read(fixture(name)).unwrap();
        for cut in [16usize, 200, 1000] {
            // Must return Ok (partial audio) or a typed error; never panic.
            match decode_bytes_with(&bytes[..cut.min(bytes.len())], None, &no_ffmpeg()) {
                Ok(d) => assert!(d.duration_s < 2.0),
                Err(e) => println!("{name} cut at {cut}: {} ({e})", e.code()),
            }
        }
    }
}
