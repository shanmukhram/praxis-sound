//! Light container probing for ffmpeg-decoded formats: does the file carry a video track
//! (so the clip is reported as `SourceKind::Video`), and does it carry any audio track?
//! Only box/element headers are read; no codec data is parsed and nothing is executed.

use super::{Format, SourceKind};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// What probing found out about a container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Probe {
    pub kind: SourceKind,
    /// `Some(false)` when the track list was read and holds no audio track.
    pub has_audio: Option<bool>,
}

#[derive(Default)]
struct Tracks {
    video: bool,
    audio: bool,
    any: bool,
}

/// Largest `moov` box read into memory for probing.
const MAX_MOOV: u64 = 64 << 20;
/// Bytes of a Matroska file scanned for the track list.
const EBML_SCAN: usize = 4 << 20;

pub(crate) fn source_kind_of_file(format: Format, path: &Path) -> Probe {
    let tracks = match format {
        Format::Mp4 => std::fs::File::open(path)
            .ok()
            .and_then(|mut f| mp4_tracks_from_reader(&mut f)),
        Format::Matroska => std::fs::File::open(path).ok().and_then(|f| {
            let mut head = Vec::new();
            f.take(EBML_SCAN as u64).read_to_end(&mut head).ok()?;
            ebml_tracks(&head)
        }),
        _ => None,
    };
    let brand = brand_of_file(format, path);
    probe(format, tracks, brand)
}

pub(crate) fn source_kind_of_bytes(format: Format, bytes: &[u8]) -> Probe {
    let tracks = match format {
        Format::Mp4 => mp4_tracks_from_reader(&mut std::io::Cursor::new(bytes)),
        Format::Matroska => ebml_tracks(&bytes[..bytes.len().min(EBML_SCAN)]),
        _ => None,
    };
    let brand = if format == Format::Mp4 {
        bytes.get(8..12).map(|b| [b[0], b[1], b[2], b[3]])
    } else {
        None
    };
    probe(format, tracks, brand)
}

fn brand_of_file(format: Format, path: &Path) -> Option<[u8; 4]> {
    if format != Format::Mp4 {
        return None;
    }
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = [0u8; 12];
    f.read_exact(&mut h).ok()?;
    Some([h[8], h[9], h[10], h[11]])
}

fn probe(format: Format, tracks: Option<Tracks>, brand: Option<[u8; 4]>) -> Probe {
    match tracks {
        Some(t) if t.any => Probe {
            kind: if t.video {
                SourceKind::Video
            } else {
                SourceKind::Audio
            },
            has_audio: Some(t.audio),
        },
        _ => Probe {
            kind: default_kind(format, brand),
            has_audio: None,
        },
    }
}

/// Best guess when the track list could not be read.
fn default_kind(format: Format, brand: Option<[u8; 4]>) -> SourceKind {
    match format {
        Format::Mp4 => match brand.as_ref().map(|b| &b[..]) {
            Some(b"M4A " | b"M4B " | b"M4P " | b"F4A " | b"F4B ") => SourceKind::Audio,
            _ => SourceKind::Video,
        },
        Format::Matroska | Format::Avi => SourceKind::Video,
        _ => SourceKind::Audio,
    }
}

// ---- ISO base media (MP4, M4A, MOV) ----

/// Walks the top-level boxes with seeks, reads `moov` into memory and collects the handler
/// types of its tracks.
fn mp4_tracks_from_reader<R: Read + Seek>(r: &mut R) -> Option<Tracks> {
    let len = r.seek(SeekFrom::End(0)).ok()?;
    let mut pos = 0u64;
    while pos + 8 <= len {
        r.seek(SeekFrom::Start(pos)).ok()?;
        let mut h = [0u8; 16];
        r.read_exact(&mut h[..8]).ok()?;
        let mut size = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as u64;
        let mut header = 8u64;
        if size == 1 {
            r.read_exact(&mut h[8..16]).ok()?;
            size = u64::from_be_bytes([h[8], h[9], h[10], h[11], h[12], h[13], h[14], h[15]]);
            header = 16;
        } else if size == 0 {
            size = len - pos;
        }
        if size < header {
            return None;
        }
        if &h[4..8] == b"moov" {
            let body = size - header;
            if body > MAX_MOOV {
                return None;
            }
            let mut moov = vec![0u8; body as usize];
            r.read_exact(&mut moov).ok()?;
            let mut t = Tracks::default();
            mp4_walk(&moov, 0, &mut t);
            return Some(t);
        }
        pos = pos.checked_add(size)?;
    }
    None
}

/// Iterates the boxes of `buf`, descending into the containers on the path
/// moov/trak/mdia/hdlr.
fn mp4_walk(buf: &[u8], depth: u32, t: &mut Tracks) {
    if depth > 8 {
        return;
    }
    let mut pos = 0usize;
    while pos + 8 <= buf.len() {
        let mut size =
            u32::from_be_bytes([buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]]) as usize;
        let kind = &buf[pos + 4..pos + 8];
        let mut header = 8usize;
        if size == 1 {
            let Some(b) = buf.get(pos + 8..pos + 16) else {
                return;
            };
            let large = u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
            size = usize::try_from(large).unwrap_or(usize::MAX);
            header = 16;
        } else if size == 0 {
            size = buf.len() - pos;
        }
        if size < header || pos.saturating_add(size) > buf.len() {
            return;
        }
        let body = &buf[pos + header..pos + size];
        match kind {
            b"trak" | b"mdia" => mp4_walk(body, depth + 1, t),
            b"hdlr" => {
                // version+flags(4), pre_defined(4), handler_type(4)
                if let Some(h) = body.get(8..12) {
                    t.any = true;
                    match h {
                        b"vide" => t.video = true,
                        b"soun" => t.audio = true,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        pos += size;
    }
}

// ---- EBML (WebM, Matroska) ----

const ID_SEGMENT: u32 = 0x1853_8067;
const ID_TRACKS: u32 = 0x1654_AE6B;
const ID_TRACK_ENTRY: u32 = 0xAE;
const ID_TRACK_TYPE: u32 = 0x83;
const ID_CLUSTER: u32 = 0x1F43_B675;

/// Reads an EBML element ID (marker bits kept). Returns (id, length).
fn ebml_id(b: &[u8]) -> Option<(u32, usize)> {
    let first = *b.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len > 4 || b.len() < len {
        return None;
    }
    let mut v = 0u32;
    for &x in &b[..len] {
        v = (v << 8) | x as u32;
    }
    Some((v, len))
}

/// Reads an EBML data size. Returns (size or None when "unknown", length).
fn ebml_size(b: &[u8]) -> Option<(Option<u64>, usize)> {
    let first = *b.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || b.len() < len {
        return None;
    }
    let mask = if len == 8 { 0 } else { 0xFFu8 >> len };
    let mut v = (first & mask) as u64;
    let mut all_ones = (first & mask) == mask;
    for &x in &b[1..len] {
        v = (v << 8) | x as u64;
        all_ones &= x == 0xFF;
    }
    Some((if all_ones { None } else { Some(v) }, len))
}

/// Finds the TrackType of every TrackEntry in the first Tracks element.
fn ebml_tracks(buf: &[u8]) -> Option<Tracks> {
    // Top level: the EBML header, then the Segment.
    let mut pos = 0usize;
    while pos < buf.len() {
        let (id, il) = ebml_id(&buf[pos..])?;
        let (size, sl) = ebml_size(&buf[pos + il..])?;
        let start = pos + il + sl;
        if id == ID_SEGMENT {
            let end = size.map_or(buf.len(), |s| {
                start.saturating_add(s as usize).min(buf.len())
            });
            return ebml_segment(&buf[start..end]);
        }
        pos = start.checked_add(size? as usize)?;
    }
    None
}

fn ebml_segment(seg: &[u8]) -> Option<Tracks> {
    let mut pos = 0usize;
    while pos < seg.len() {
        let (id, il) = ebml_id(&seg[pos..])?;
        let (size, sl) = ebml_size(&seg[pos + il..])?;
        let start = pos + il + sl;
        match id {
            ID_TRACKS => {
                let end = size.map_or(seg.len(), |s| {
                    start.saturating_add(s as usize).min(seg.len())
                });
                return Some(ebml_track_list(&seg[start..end]));
            }
            ID_CLUSTER => return None, // media data before any track list
            _ => {}
        }
        pos = start.checked_add(size? as usize)?;
    }
    None
}

fn ebml_track_list(tracks: &[u8]) -> Tracks {
    let mut t = Tracks::default();
    for_each_child(tracks, |id, body| {
        if id == ID_TRACK_ENTRY {
            for_each_child(body, |id, value| {
                if id == ID_TRACK_TYPE {
                    let v = value.iter().fold(0u64, |a, &x| (a << 8) | x as u64);
                    t.any = true;
                    match v {
                        1 => t.video = true,
                        2 => t.audio = true,
                        _ => {}
                    }
                }
            });
        }
    });
    t
}

fn for_each_child(buf: &[u8], mut f: impl FnMut(u32, &[u8])) {
    let mut pos = 0usize;
    while pos < buf.len() {
        let Some((id, il)) = ebml_id(&buf[pos..]) else {
            return;
        };
        let Some((Some(size), sl)) = buf.get(pos + il..).and_then(ebml_size) else {
            return;
        };
        let start = pos + il + sl;
        let Some(end) = start.checked_add(size as usize).filter(|&e| e <= buf.len()) else {
            return;
        };
        f(id, &buf[start..end]);
        pos = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    fn hdlr(handler: &[u8; 4]) -> Vec<u8> {
        let mut body = vec![0u8; 8];
        body.extend_from_slice(handler);
        body.extend_from_slice(&[0u8; 13]);
        mp4_box(b"hdlr", &body)
    }

    fn mp4_with(handlers: &[&[u8; 4]]) -> Vec<u8> {
        let mut traks = Vec::new();
        for h in handlers {
            traks.extend(mp4_box(b"trak", &mp4_box(b"mdia", &hdlr(h))));
        }
        let mut file = mp4_box(b"ftyp", b"isom\0\0\0\0isom");
        file.extend(mp4_box(b"mdat", &[0u8; 32]));
        file.extend(mp4_box(b"moov", &traks));
        file
    }

    #[test]
    fn mp4_video_and_audio_tracks() {
        let p = source_kind_of_bytes(Format::Mp4, &mp4_with(&[b"vide", b"soun"]));
        assert_eq!(p.kind, SourceKind::Video);
        assert_eq!(p.has_audio, Some(true));
        let p = source_kind_of_bytes(Format::Mp4, &mp4_with(&[b"soun"]));
        assert_eq!(p.kind, SourceKind::Audio);
        let p = source_kind_of_bytes(Format::Mp4, &mp4_with(&[b"vide"]));
        assert_eq!(p.has_audio, Some(false));
    }

    #[test]
    fn mp4_truncated_falls_back_to_brand() {
        let mut f = mp4_box(b"ftyp", b"M4A \0\0\0\0M4A ");
        f.extend_from_slice(&[0, 0, 0x10, 0, b'm', b'o', b'o', b'v']);
        let p = source_kind_of_bytes(Format::Mp4, &f);
        assert_eq!(p.kind, SourceKind::Audio);
        assert_eq!(p.has_audio, None);
    }

    fn el(id: &[u8], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        assert!(body.len() < 127);
        v.push(0x80 | body.len() as u8);
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn matroska_track_types() {
        let entry = |t: u8| el(&[0xAE], &el(&[0x83], &[t]));
        let mut tracks = entry(1);
        tracks.extend(entry(2));
        let mut seg = el(&[0x15, 0x49, 0xA9, 0x66], &[0u8; 4]); // Info
        seg.extend(el(&[0x16, 0x54, 0xAE, 0x6B], &tracks));
        let mut file = el(&[0x1A, 0x45, 0xDF, 0xA3], &el(&[0x42, 0x82], b"webm"));
        // Segment with unknown size.
        file.extend_from_slice(&[
            0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        ]);
        file.extend(seg);
        let p = source_kind_of_bytes(Format::Matroska, &file);
        assert_eq!(p.kind, SourceKind::Video);
        assert_eq!(p.has_audio, Some(true));
    }

    #[test]
    fn garbage_never_panics() {
        let mut x = 0x1234_5678u32;
        for n in 0..512usize {
            let mut b = Vec::with_capacity(n);
            for _ in 0..n {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                b.push(x as u8);
            }
            let _ = source_kind_of_bytes(Format::Mp4, &b);
            let _ = source_kind_of_bytes(Format::Matroska, &b);
            let _ = super::super::detect_format(&b);
        }
    }
}
