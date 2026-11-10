# audio-io test fixtures

Tiny clips generated with ffmpeg's `lavfi` test sources (pure sine tones, amplitude 1/8, and
the `testsrc2` colour-bar pattern). Nothing here is recorded or third-party material.
Generated with ffmpeg 9.0.2 (gyan.dev full build, Windows) on 2026-11-10.

| File | Size | Content | Used by |
| --- | --- | --- | --- |
| `tone440_mono.mp3` | 8.4 KB | 2 s, 440 Hz, mono, 44.1 kHz, MP3 32 kb/s with an ID3v2 tag and LAME/Info tag | native MP3 decoding, ID3v2 skipping |
| `tone660_stereo.ogg` | 8.8 KB | 2 s, 660 Hz, stereo, 44.1 kHz, Vorbis q0 | native Ogg Vorbis decoding |
| `tone550.m4a` | 9.1 KB | 2 s, 550 Hz, stereo, 44.1 kHz, AAC 32 kb/s in M4A | ffmpeg path, `SourceKind::Audio` |
| `clip880.mp4` | 11.8 KB | 2 s, 64x48 H.264 video + 880 Hz AAC | ffmpeg path, `SourceKind::Video` |
| `clip880.mov` | 11.8 KB | same as above in a QuickTime container | ffmpeg path, `SourceKind::Video` |
| `clip880.webm` | 15.6 KB | 2 s, 64x48 VP8 video + 880 Hz Opus (48 kHz) | ffmpeg path, `SourceKind::Video` |
| `video_only.mp4` | 2.1 KB | 1 s, 64x48 H.264, no audio track | `no_audio_stream` |

Commands (run in this directory):

```sh
F="ffmpeg -nostdin -hide_banner -loglevel error -y"
$F -f lavfi -i "sine=frequency=440:duration=2:sample_rate=44100" -ac 1 -c:a libmp3lame -b:a 32k -metadata title="Praxis test tone" tone440_mono.mp3
$F -f lavfi -i "sine=frequency=660:duration=2:sample_rate=44100" -ac 2 -c:a libvorbis -q:a 0 tone660_stereo.ogg
$F -f lavfi -i "sine=frequency=550:duration=2:sample_rate=44100" -ac 2 -c:a aac -b:a 32k tone550.m4a
$F -f lavfi -i "testsrc2=size=64x48:rate=10:duration=2" -f lavfi -i "sine=frequency=880:duration=2:sample_rate=44100" -c:v libx264 -preset veryslow -crf 45 -pix_fmt yuv420p -c:a aac -b:a 32k -shortest clip880.mp4
$F -f lavfi -i "testsrc2=size=64x48:rate=10:duration=2" -f lavfi -i "sine=frequency=880:duration=2:sample_rate=44100" -c:v libx264 -preset veryslow -crf 45 -pix_fmt yuv420p -c:a aac -b:a 32k -shortest -f mov clip880.mov
$F -f lavfi -i "testsrc2=size=64x48:rate=10:duration=2" -f lavfi -i "sine=frequency=880:duration=2:sample_rate=48000" -c:v libvpx -b:v 50k -c:a libopus -b:a 32k -shortest clip880.webm
$F -f lavfi -i "testsrc2=size=64x48:rate=10:duration=1" -c:v libx264 -preset veryslow -crf 45 -pix_fmt yuv420p video_only.mp4
```

`tests/decode_ffmpeg.rs` also generates a video-only MPEG-TS clip at test time (when ffmpeg is
installed) to exercise ffmpeg's own "no stream" report, since MPEG-TS is not probed natively.
The MP3 and Ogg fixtures decode natively, so those tests run without ffmpeg; the M4A, MP4, MOV
and WebM decoding tests print a skip message and pass when ffmpeg is missing.
