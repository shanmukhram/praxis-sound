//! Format detection from magic bytes. The file extension is never consulted.

/// Bytes read from the start of a file for detection.
pub(crate) const HEAD_LEN: usize = 64 * 1024;

/// Container or codec family, as recognised from the first bytes of the data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// RIFF/WAVE (decoded natively; compressed WAV codecs fall through to ffmpeg).
    Wav,
    /// Native FLAC stream (`fLaC`), optionally behind an ID3v2 tag.
    Flac,
    /// Ogg with a Vorbis first stream.
    OggVorbis,
    /// MPEG audio (ID3v2 tag or frame sync), decoded natively.
    Mp3,
    /// ISO base media (`ftyp`): MP4, M4A, MOV, 3GP. ffmpeg.
    Mp4,
    /// EBML: WebM and Matroska. ffmpeg.
    Matroska,
    /// Ogg with another codec (Opus, FLAC, Speex). ffmpeg.
    Ogg,
    /// Raw AAC in ADTS frames. ffmpeg.
    Adts,
    /// AIFF / AIFF-C. ffmpeg.
    Aiff,
    /// Core Audio Format. ffmpeg.
    Caf,
    /// RF64 / BW64 (64-bit WAV). ffmpeg.
    Rf64,
    /// AVI (RIFF `AVI `). ffmpeg.
    Avi,
    /// ASF / WMA / WMV. ffmpeg.
    Asf,
    /// Nothing recognised; ffmpeg is tried when available.
    Unknown,
}

impl Format {
    /// Decoded in-process, without ffmpeg.
    pub fn is_native(self) -> bool {
        matches!(
            self,
            Format::Wav | Format::Flac | Format::OggVorbis | Format::Mp3
        )
    }
}

/// Detects the format of `head`, the first bytes of a file (64 KiB is plenty).
pub fn detect_format(head: &[u8]) -> Format {
    let b = head;
    if b.len() >= 12 && &b[0..4] == b"RIFF" {
        return match &b[8..12] {
            b"WAVE" => Format::Wav,
            b"AVI " => Format::Avi,
            _ => Format::Unknown,
        };
    }
    if b.len() >= 12 && (&b[0..4] == b"RF64" || &b[0..4] == b"BW64") && &b[8..12] == b"WAVE" {
        return Format::Rf64;
    }
    if b.starts_with(b"fLaC") {
        return Format::Flac;
    }
    if b.starts_with(b"OggS") {
        return if ogg_first_packet_is_vorbis(b) {
            Format::OggVorbis
        } else {
            Format::Ogg
        };
    }
    if b.len() >= 12 && &b[0..4] == b"FORM" && (&b[8..12] == b"AIFF" || &b[8..12] == b"AIFC") {
        return Format::Aiff;
    }
    if b.starts_with(b"caff") {
        return Format::Caf;
    }
    if b.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Format::Matroska;
    }
    if b.starts_with(&[
        0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE,
        0x6C,
    ]) {
        return Format::Asf;
    }
    if b.len() >= 8 {
        match &b[4..8] {
            b"ftyp" => return Format::Mp4,
            // Old QuickTime files without an ftyp box start with one of these atoms.
            b"moov" | b"mdat" | b"wide" | b"free" | b"skip" | b"pnot"
                if u32::from_be_bytes([b[0], b[1], b[2], b[3]]) >= 8 =>
            {
                return Format::Mp4;
            }
            _ => {}
        }
    }
    if b.starts_with(b"ID3") {
        let tag = nanomp3::id3v2_len(b);
        if tag > 0 && tag < b.len() && b[tag..].starts_with(b"fLaC") {
            return Format::Flac;
        }
        if tag > 0 && tag + 1 < b.len() && is_adts(&b[tag..]) {
            return Format::Adts;
        }
        return Format::Mp3;
    }
    if is_adts(b) {
        return Format::Adts;
    }
    if is_mpeg_audio_sync(b) || nanomp3::detect(b) {
        return Format::Mp3;
    }
    Format::Unknown
}

/// 12-bit sync with layer bits 00: AAC in ADTS (MPEG audio uses layers 1-3).
fn is_adts(b: &[u8]) -> bool {
    b.len() >= 2 && b[0] == 0xFF && (b[1] & 0xF6) == 0xF0
}

/// MPEG audio frame header: 11-bit sync, valid version, layer, bitrate and sample-rate fields,
/// and a second frame header where the first frame says it ends (or nanomp3 agrees).
fn is_mpeg_audio_sync(b: &[u8]) -> bool {
    if b.len() < 4 || b[0] != 0xFF || (b[1] & 0xE0) != 0xE0 {
        return false;
    }
    let version = (b[1] >> 3) & 3; // 1 is reserved
    let layer = (b[1] >> 1) & 3; // 0 is reserved (ADTS uses it)
    let bitrate = b[2] >> 4; // 15 is invalid
    let rate = (b[2] >> 2) & 3; // 3 is reserved
    version != 1 && layer != 0 && bitrate != 15 && rate != 3 && nanomp3::detect(b)
}

/// The first Ogg page carries a packet that starts with `\x01vorbis`.
fn ogg_first_packet_is_vorbis(b: &[u8]) -> bool {
    if b.len() < 27 {
        return false;
    }
    let segments = b[26] as usize;
    let start = 27 + segments;
    b.len() >= start + 7 && &b[start..start + 7] == b"\x01vorbis"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_and_empty_inputs_are_unknown() {
        assert_eq!(detect_format(b""), Format::Unknown);
        assert_eq!(detect_format(b"RI"), Format::Unknown);
        assert_eq!(detect_format(b"hello world, not audio"), Format::Unknown);
    }

    #[test]
    fn signatures() {
        assert_eq!(detect_format(b"RIFF\0\0\0\0WAVEfmt "), Format::Wav);
        assert_eq!(detect_format(b"RIFF\0\0\0\0AVI LIST"), Format::Avi);
        assert_eq!(detect_format(b"fLaC\0\0\0\x22"), Format::Flac);
        assert_eq!(detect_format(b"\0\0\0\x20ftypisom\0\0\x02\0"), Format::Mp4);
        assert_eq!(detect_format(b"\0\0\0\x20ftypM4A \0\0\0\0"), Format::Mp4);
        assert_eq!(
            detect_format(&[0x1A, 0x45, 0xDF, 0xA3, 0x9F]),
            Format::Matroska
        );
        assert_eq!(detect_format(b"FORM\0\0\0\0AIFFCOMM"), Format::Aiff);
        assert_eq!(
            detect_format(&[0xFF, 0xF1, 0x50, 0x80, 0, 0x1F]),
            Format::Adts
        );
        assert_eq!(detect_format(b"ID3\x04\0\0\0\0\0\0"), Format::Mp3);
    }

    #[test]
    fn ogg_codec_is_read_from_the_first_packet() {
        let mut page = b"OggS\0\x02".to_vec();
        page.extend_from_slice(&[0; 20]); // granule, serial, sequence, crc
        page.push(1); // one segment
        page.push(30); // of 30 bytes
        let mut vorbis = page.clone();
        vorbis.extend_from_slice(b"\x01vorbis");
        vorbis.extend_from_slice(&[0; 23]);
        assert_eq!(detect_format(&vorbis), Format::OggVorbis);
        let mut opus = page;
        opus.extend_from_slice(b"OpusHead");
        opus.extend_from_slice(&[0; 22]);
        assert_eq!(detect_format(&opus), Format::Ogg);
    }
}
