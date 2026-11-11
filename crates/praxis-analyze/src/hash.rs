//! BLAKE3 of decoded PCM, for `SourceInfo::pcm_hash` (recognise the same clip again), using the
//! audited `blake3` crate (std only, WebAssembly-safe).
//!
//! [`pcm_hash`] hashes the little-endian bits of every left sample, then every right sample,
//! exactly as `praxis_testkit::audio_hash` does, so the two agree on the same buffer.

/// BLAKE3 of `bytes`, lowercase hex.
pub fn hash_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// BLAKE3 over the little-endian bits of every left sample, then every right sample.
pub fn pcm_hash(left: &[f32], right: &[f32]) -> String {
    let mut h = blake3::Hasher::new();
    let mut buf = Vec::with_capacity(4096);
    for ch in [left, right] {
        for v in ch {
            buf.extend_from_slice(&v.to_bits().to_le_bytes());
            if buf.len() >= 4096 {
                h.update(&buf);
                buf.clear();
            }
        }
    }
    h.update(&buf);
    h.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_vectors() {
        assert_eq!(
            hash_hex(b""),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert_eq!(
            hash_hex(b"abc"),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
        );
    }

    #[test]
    fn pcm_bytes_are_left_then_right_little_endian() {
        let (l, r) = ([0.5f32, -1.0], [0.25f32, 2.0]);
        let mut bytes = Vec::new();
        for v in l.iter().chain(r.iter()) {
            bytes.extend_from_slice(&v.to_bits().to_le_bytes());
        }
        assert_eq!(pcm_hash(&l, &r), hash_hex(&bytes));
    }
}
