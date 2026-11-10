//! M4A/MP4/MOV/WebM decoding through the external ffmpeg executable. Tests that need ffmpeg
//! print a message and pass when it is not installed; the error paths run everywhere.

mod common;

use common::*;
use praxis_audio_io::decode::{
    DecodeOptions, FfmpegSetting, SourceKind, decode_bytes, decode_bytes_with, decode_file,
    decode_file_with,
};

fn check_clip(name: &str, kind: SourceKind, freq: f64) {
    let Some(_) = ffmpeg_or_skip(name) else {
        return;
    };
    let d = decode_file(&fixture(name)).unwrap();
    assert_eq!(d.sample_rate, 48_000, "{name}: ffmpeg output rate");
    assert_eq!(d.channels, 2);
    assert_eq!(d.source_kind, kind, "{name}");
    println!("{name}: {:.4} s", d.duration_s);
    // AAC and Opus add encoder priming/padding of a few tens of milliseconds at most.
    assert!((d.duration_s - 2.0).abs() < 0.1, "{name}: {}", d.duration_s);
    let f = dominant_hz(&d.audio.left, d.sample_rate);
    println!("{name}: dominant {f:.2} Hz");
    assert!((f - freq).abs() < 1.5, "{name}: {f}");
    // lavfi sine sources have amplitude 1/8 (rms 0.088).
    assert!(rms(&d.audio.left) > 0.06, "{name}: audible");
}

#[test]
fn m4a_audio() {
    check_clip("tone550.m4a", SourceKind::Audio, 550.0);
}

#[test]
fn mp4_video_audio_track() {
    check_clip("clip880.mp4", SourceKind::Video, 880.0);
}

#[test]
fn mov_video_audio_track() {
    check_clip("clip880.mov", SourceKind::Video, 880.0);
}

#[test]
fn webm_video_audio_track() {
    check_clip("clip880.webm", SourceKind::Video, 880.0);
}

#[test]
fn in_memory_video_goes_through_a_temp_file() {
    let Some(_) = ffmpeg_or_skip("in_memory_video_goes_through_a_temp_file") else {
        return;
    };
    let bytes = std::fs::read(fixture("clip880.webm")).unwrap();
    // The hint is wrong on purpose; detection uses the content.
    let d = decode_bytes(&bytes, Some("dropped.mp3")).unwrap();
    assert_eq!(d.source_kind, SourceKind::Video);
    assert_eq!(d, decode_file(&fixture("clip880.webm")).unwrap());
}

#[test]
fn video_without_audio_reports_no_audio_stream() {
    // Detected from the container's track list, so this holds with or without ffmpeg.
    for opts in [
        DecodeOptions::default(),
        DecodeOptions {
            ffmpeg: FfmpegSetting::Disabled,
        },
    ] {
        let err = decode_file_with(&fixture("video_only.mp4"), &opts).unwrap_err();
        assert_eq!(err.code(), "no_audio_stream", "{err}");
    }
}

#[test]
fn ffmpeg_no_audio_message_is_recognised() {
    // MPEG-TS is not probed for tracks, so here ffmpeg itself must report the missing audio.
    // Generated at test time (a few KB) with ffmpeg's lavfi test source.
    let Some(ffmpeg) = ffmpeg_or_skip("ffmpeg_no_audio_message_is_recognised") else {
        return;
    };
    let dir = temp_dir("ffmpeg_no_audio");
    let ts = dir.join("video_only.ts");
    let status = std::process::Command::new(ffmpeg)
        .args(["-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=32x32:rate=5:duration=1")
        .args(["-c:v", "mpeg2video", "-f", "mpegts"])
        .arg(&ts)
        .status()
        .unwrap();
    assert!(status.success());
    let err = decode_file(&ts).unwrap_err();
    assert_eq!(err.code(), "no_audio_stream", "{err}");
}

#[test]
fn missing_ffmpeg_is_a_clear_error() {
    let disabled = DecodeOptions {
        ffmpeg: FfmpegSetting::Disabled,
    };
    let nowhere = DecodeOptions {
        ffmpeg: FfmpegSetting::Path(fixture("no-such-ffmpeg-executable")),
    };
    for opts in [&disabled, &nowhere] {
        for name in ["tone550.m4a", "clip880.mp4", "clip880.mov", "clip880.webm"] {
            let err = decode_file_with(&fixture(name), opts).unwrap_err();
            assert_eq!(err.code(), "ffmpeg_missing", "{name}: {err}");
            assert!(err.to_string().contains("ffmpeg"));
        }
    }
    // Unrecognised data without ffmpeg is "unknown format", not "install ffmpeg".
    let err = decode_bytes_with(b"plain text, no media here", None, &disabled).unwrap_err();
    assert_eq!(err.code(), "unknown_format");
}

#[test]
fn ffmpeg_rejects_non_media() {
    let Some(_) = ffmpeg_or_skip("ffmpeg_rejects_non_media") else {
        return;
    };
    let err = decode_bytes(
        b"plain text, no media here, long enough to probe",
        Some("x.m4a"),
    )
    .unwrap_err();
    assert_eq!(err.code(), "unknown_format", "{err}");
}

#[test]
fn user_files_are_never_executed() {
    // A file that is itself a script or executable is just bytes to the decoder: it is
    // detected as unknown and, at most, handed to ffmpeg as input data.
    let dir = temp_dir("never_execute");
    let marker = dir.join("ran.txt");
    let script = dir.join("ffmpeg.cmd");
    std::fs::write(&script, format!("echo pwned > \"{}\"\n", marker.display())).unwrap();
    let sh = dir.join("ffmpeg");
    std::fs::write(
        &sh,
        format!("#!/bin/sh\necho pwned > '{}'\n", marker.display()),
    )
    .unwrap();
    for f in [&script, &sh] {
        let _ = decode_file(f);
    }
    assert!(!marker.exists(), "a user file was executed");
}
