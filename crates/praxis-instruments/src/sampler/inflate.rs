//! Raw DEFLATE (RFC 1951) decoder and CRC-32, used to unpack 7z archives whose blocks use the
//! DEFLATE coder.
//!
//! `sevenz-rust2` parses the archive structure, but its DEFLATE coder sits behind the crate's
//! `deflate` feature, which the workspace does not enable (see the WP-C1 report for the proposed
//! one-line change). FreePats publishes its packs as DEFLATE-compressed 7z, so this small,
//! canonical-Huffman decoder (after zlib's `puff.c`) fills the gap. Input is untrusted: every
//! read is bounds-checked and the output length is capped by the caller's expected size.

const MAXBITS: usize = 15;
const MAXLCODES: usize = 286;
const MAXDCODES: usize = 30;
const FIXLCODES: usize = 288;

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bitbuf: u64,
    bitcnt: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader {
            data,
            pos: 0,
            bitbuf: 0,
            bitcnt: 0,
        }
    }

    #[inline]
    fn refill(&mut self) {
        while self.bitcnt <= 56 {
            let b = if self.pos < self.data.len() {
                self.data[self.pos]
            } else if self.pos < self.data.len() + 8 {
                // Allow a few bytes of zero padding past the end so the last symbol can be
                // decoded; reading further is an error caught by `overrun`.
                0
            } else {
                return;
            };
            self.pos += 1;
            self.bitbuf |= (b as u64) << self.bitcnt;
            self.bitcnt += 8;
        }
    }

    #[inline]
    fn bits(&mut self, need: u32) -> Result<u32, String> {
        if need == 0 {
            return Ok(0);
        }
        if self.bitcnt < need {
            self.refill();
            if self.bitcnt < need {
                return Err("deflate: unexpected end of input".into());
            }
        }
        let v = (self.bitbuf & ((1u64 << need) - 1)) as u32;
        self.bitbuf >>= need;
        self.bitcnt -= need;
        Ok(v)
    }

    fn overrun(&self) -> bool {
        // Bytes consumed beyond the real input, excluding bits still buffered.
        let consumed = self.pos as i64 - (self.bitcnt / 8) as i64;
        consumed > self.data.len() as i64
    }

    /// Drops buffered bits up to the next byte boundary and returns the byte position.
    fn align_byte(&mut self) -> usize {
        let drop = self.bitcnt % 8;
        self.bitbuf >>= drop;
        self.bitcnt -= drop;
        // Return buffered whole bytes to the stream.
        let back = (self.bitcnt / 8) as usize;
        self.pos -= back;
        self.bitbuf = 0;
        self.bitcnt = 0;
        self.pos
    }
}

struct Huffman {
    count: [u16; MAXBITS + 1],
    symbol: Vec<u16>,
}

impl Huffman {
    /// Builds a canonical code from code lengths. Incomplete codes are allowed only for the
    /// single-code distance case (as zlib does); over-subscribed codes are errors.
    fn new(lengths: &[u8]) -> Result<Huffman, String> {
        let mut count = [0u16; MAXBITS + 1];
        for &l in lengths {
            count[l as usize] += 1;
        }
        if count[0] as usize == lengths.len() {
            return Ok(Huffman {
                count,
                symbol: vec![0; lengths.len()],
            });
        }
        let mut left: i32 = 1;
        for len in 1..=MAXBITS {
            left <<= 1;
            left -= count[len] as i32;
            if left < 0 {
                return Err("deflate: over-subscribed Huffman code".into());
            }
        }
        let mut offs = [0u16; MAXBITS + 1];
        for len in 1..MAXBITS {
            offs[len + 1] = offs[len] + count[len];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        Ok(Huffman { count, symbol })
    }

    #[inline]
    fn decode(&self, br: &mut BitReader) -> Result<usize, String> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..=MAXBITS {
            code |= br.bits(1)? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                return Ok(self.symbol[(index + (code - first)) as usize] as usize);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err("deflate: invalid Huffman code".into())
    }
}

const LBASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEXT: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DBASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DEXT: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

fn codes(
    br: &mut BitReader,
    out: &mut Vec<u8>,
    max_out: usize,
    lencode: &Huffman,
    distcode: &Huffman,
) -> Result<(), String> {
    loop {
        let sym = lencode.decode(br)?;
        if sym < 256 {
            if out.len() >= max_out {
                return Err("deflate: output larger than expected".into());
            }
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let s = sym - 257;
            if s >= 29 {
                return Err("deflate: invalid length symbol".into());
            }
            let len = LBASE[s] as usize + br.bits(LEXT[s] as u32)? as usize;
            let ds = distcode.decode(br)?;
            if ds >= 30 {
                return Err("deflate: invalid distance symbol".into());
            }
            let dist = DBASE[ds] as usize + br.bits(DEXT[ds] as u32)? as usize;
            if dist > out.len() {
                return Err("deflate: distance too far back".into());
            }
            if out.len() + len > max_out {
                return Err("deflate: output larger than expected".into());
            }
            let start = out.len() - dist;
            for i in 0..len {
                let b = out[start + i];
                out.push(b);
            }
        }
    }
}

/// Inflates a raw DEFLATE stream. `expected` is the exact decompressed size (from the archive
/// header); producing more is an error, and the output is preallocated to it.
pub fn inflate(data: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(expected);
    let mut br = BitReader::new(data);
    let fixed = {
        let mut l = [0u8; FIXLCODES];
        l[..144].fill(8);
        l[144..256].fill(9);
        l[256..280].fill(7);
        l[280..].fill(8);
        (Huffman::new(&l)?, Huffman::new(&[5u8; MAXDCODES])?)
    };
    loop {
        let last = br.bits(1)?;
        match br.bits(2)? {
            0 => {
                let mut pos = br.align_byte();
                if pos + 4 > data.len() {
                    return Err("deflate: truncated stored block".into());
                }
                let len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
                let nlen = u16::from_le_bytes([data[pos + 2], data[pos + 3]]) as usize;
                if len != (!nlen & 0xffff) {
                    return Err("deflate: stored block length check failed".into());
                }
                pos += 4;
                if pos + len > data.len() {
                    return Err("deflate: truncated stored block".into());
                }
                if out.len() + len > expected {
                    return Err("deflate: output larger than expected".into());
                }
                out.extend_from_slice(&data[pos..pos + len]);
                br.pos = pos + len;
            }
            1 => codes(&mut br, &mut out, expected, &fixed.0, &fixed.1)?,
            2 => {
                let nlen = br.bits(5)? as usize + 257;
                let ndist = br.bits(5)? as usize + 1;
                let ncode = br.bits(4)? as usize + 4;
                if nlen > MAXLCODES || ndist > MAXDCODES {
                    return Err("deflate: bad dynamic block counts".into());
                }
                const ORDER: [usize; 19] = [
                    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
                ];
                let mut lengths = [0u8; MAXLCODES + MAXDCODES];
                for &o in ORDER.iter().take(ncode) {
                    lengths[o] = br.bits(3)? as u8;
                }
                let lencode = Huffman::new(&lengths[..19])?;
                let mut index = 0;
                while index < nlen + ndist {
                    let sym = lencode.decode(&mut br)?;
                    if sym < 16 {
                        lengths[index] = sym as u8;
                        index += 1;
                    } else {
                        let (val, rep) = match sym {
                            16 => {
                                if index == 0 {
                                    return Err("deflate: repeat with no first length".into());
                                }
                                (lengths[index - 1], 3 + br.bits(2)? as usize)
                            }
                            17 => (0, 3 + br.bits(3)? as usize),
                            _ => (0, 11 + br.bits(7)? as usize),
                        };
                        if index + rep > nlen + ndist {
                            return Err("deflate: too many code lengths".into());
                        }
                        lengths[index..index + rep].fill(val);
                        index += rep;
                    }
                }
                if lengths[256] == 0 {
                    return Err("deflate: no end-of-block code".into());
                }
                let lencode = Huffman::new(&lengths[..nlen])?;
                let distcode = Huffman::new(&lengths[nlen..nlen + ndist])?;
                codes(&mut br, &mut out, expected, &lencode, &distcode)?;
            }
            _ => return Err("deflate: invalid block type".into()),
        }
        if br.overrun() {
            return Err("deflate: unexpected end of input".into());
        }
        if last == 1 {
            break;
        }
    }
    if out.len() != expected {
        return Err(format!(
            "deflate: produced {} bytes, expected {expected}",
            out.len()
        ));
    }
    Ok(out)
}

/// CRC-32 (IEEE 802.3, reflected), as used by 7z.
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *e = c;
        }
        t
    });
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = t[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_known_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn stored_block() {
        // BFINAL=1, BTYPE=00, LEN=5, NLEN=!5, "hello".
        let data = [0x01, 0x05, 0x00, 0xFA, 0xFF, b'h', b'e', b'l', b'l', b'o'];
        assert_eq!(inflate(&data, 5).unwrap(), b"hello");
        assert!(inflate(&data, 4).is_err());
    }

    #[test]
    fn fixed_block_with_back_reference() {
        // "abcabcabc" compressed with zlib (raw deflate, fixed Huffman).
        let data = [0x4b, 0x4c, 0x4a, 0x4e, 0x04, 0x23, 0x00];
        assert_eq!(inflate(&data, 9).unwrap(), b"abcabcabc");
    }

    #[test]
    fn dynamic_block() {
        // 40 SFZ-like lines compressed by zlib level 9 (raw, dynamic Huffman); 2092 bytes,
        // CRC-32 3730610608.
        let hex = "85d53d4e03411883e13ea7c809606dcffe1514745c03a1001181209090b83d42597fd16a0b5753b97a9f99f93abc1ccf1ffb6efffdf8fe793adc5d8eefdbfbeee7e1e6f9f4f8b43f9ddf0ebf77ddfef5f87f72f7755960b3c07a8165212fb85970bde0b2685e68b3d07aa165d17bd1368bb65eb4653178d16f16fd7ad12f8bd18b61b318d68b61594c5e8c9bc5b85e8ccb62f6624a3da66581ce933905993d4135ec5212b83baee191aac0e951edc11406ae8fca0fa53630009400b49407368042803e158219a01c6048916009280a18e3bd310694064cb1933db03c608e9d2c8225825deac47a094a04913ad122787d0d983ad1225822a8d48916c112c1963ad1225822d8a74eb40896080ef189b30896088ea9132d82258253ec64112a119c63278b508950973ac922542284d449f53d940831759245e8fa4528759245a844a8a54eb2089508f5f137b20895080da9932c42254263ea248b5089d0143b59442b119a63278b68d8fd01";
        let data: Vec<u8> = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
            .collect();
        let out = inflate(&data, 2092).unwrap();
        assert_eq!(crc32(&out), 3_730_610_608);
        assert!(out.starts_with(b"region 0 sample=samples/A0vH.flac lokey=0 hikey=2\n"));
        assert!(inflate(&data[..data.len() - 20], 2092).is_err());
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        for seed in 0..200u64 {
            let mut r = praxis_dsp::rng::Pcg32::new(seed);
            let data: Vec<u8> = (0..64).map(|_| r.next_u32() as u8).collect();
            let _ = inflate(&data, 256);
        }
    }
}
