//! BLAKE3 of decoded PCM, for `SourceInfo::pcm_hash` (recognise the same clip again).
//!
//! praxis-analyze has no `blake3` dependency (and must stay WebAssembly-safe), so this is a
//! small, portable implementation of the BLAKE3 hash function following the reference
//! implementation (unkeyed hash, 32-byte output). Tests check it against the published test
//! vectors and against the `blake3` crate (through `praxis-testkit::audio_hash`).
//!
//! [`pcm_hash`] hashes the little-endian bits of every left sample, then every right sample,
//! exactly as `praxis_testkit::audio_hash` does, so the two agree on the same buffer.

const IV: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];
const MSG_PERMUTATION: [usize; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];
const CHUNK_START: u32 = 1;
const CHUNK_END: u32 = 2;
const PARENT: u32 = 4;
const ROOT: u32 = 8;
const BLOCK_LEN: usize = 64;
const CHUNK_LEN: usize = 1024;

#[inline]
fn g(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, mx: u32, my: u32) {
    s[a] = s[a].wrapping_add(s[b]).wrapping_add(mx);
    s[d] = (s[d] ^ s[a]).rotate_right(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_right(12);
    s[a] = s[a].wrapping_add(s[b]).wrapping_add(my);
    s[d] = (s[d] ^ s[a]).rotate_right(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_right(7);
}

fn round(s: &mut [u32; 16], m: &[u32; 16]) {
    g(s, 0, 4, 8, 12, m[0], m[1]);
    g(s, 1, 5, 9, 13, m[2], m[3]);
    g(s, 2, 6, 10, 14, m[4], m[5]);
    g(s, 3, 7, 11, 15, m[6], m[7]);
    g(s, 0, 5, 10, 15, m[8], m[9]);
    g(s, 1, 6, 11, 12, m[10], m[11]);
    g(s, 2, 7, 8, 13, m[12], m[13]);
    g(s, 3, 4, 9, 14, m[14], m[15]);
}

fn compress(
    cv: &[u32; 8],
    block: &[u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
) -> [u32; 16] {
    let mut s = [
        cv[0],
        cv[1],
        cv[2],
        cv[3],
        cv[4],
        cv[5],
        cv[6],
        cv[7],
        IV[0],
        IV[1],
        IV[2],
        IV[3],
        counter as u32,
        (counter >> 32) as u32,
        block_len,
        flags,
    ];
    let mut m = *block;
    for r in 0..7 {
        round(&mut s, &m);
        if r < 6 {
            let mut p = [0u32; 16];
            for (i, src) in MSG_PERMUTATION.iter().enumerate() {
                p[i] = m[*src];
            }
            m = p;
        }
    }
    for i in 0..8 {
        s[i] ^= s[i + 8];
        s[i + 8] ^= cv[i];
    }
    s
}

fn first8(w: &[u32; 16]) -> [u32; 8] {
    let mut o = [0u32; 8];
    o.copy_from_slice(&w[..8]);
    o
}

fn words(block: &[u8; BLOCK_LEN]) -> [u32; 16] {
    let mut w = [0u32; 16];
    for (i, c) in block.as_chunks::<4>().0.iter().enumerate() {
        w[i] = u32::from_le_bytes(*c);
    }
    w
}

struct Output {
    cv: [u32; 8],
    block: [u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
}

impl Output {
    fn chaining_value(&self) -> [u32; 8] {
        first8(&compress(
            &self.cv,
            &self.block,
            self.counter,
            self.block_len,
            self.flags,
        ))
    }

    fn root_bytes(&self) -> [u8; 32] {
        let w = compress(&self.cv, &self.block, 0, self.block_len, self.flags | ROOT);
        let mut out = [0u8; 32];
        for (i, v) in w[..8].iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        out
    }
}

struct ChunkState {
    cv: [u32; 8],
    counter: u64,
    block: [u8; BLOCK_LEN],
    block_len: usize,
    blocks_compressed: usize,
}

impl ChunkState {
    fn new(counter: u64) -> ChunkState {
        ChunkState {
            cv: IV,
            counter,
            block: [0; BLOCK_LEN],
            block_len: 0,
            blocks_compressed: 0,
        }
    }

    fn len(&self) -> usize {
        BLOCK_LEN * self.blocks_compressed + self.block_len
    }

    fn start_flag(&self) -> u32 {
        if self.blocks_compressed == 0 {
            CHUNK_START
        } else {
            0
        }
    }

    fn update(&mut self, mut input: &[u8]) {
        while !input.is_empty() {
            if self.block_len == BLOCK_LEN {
                let w = words(&self.block);
                self.cv = first8(&compress(
                    &self.cv,
                    &w,
                    self.counter,
                    BLOCK_LEN as u32,
                    self.start_flag(),
                ));
                self.blocks_compressed += 1;
                self.block = [0; BLOCK_LEN];
                self.block_len = 0;
            }
            let take = (BLOCK_LEN - self.block_len).min(input.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&input[..take]);
            self.block_len += take;
            input = &input[take..];
        }
    }

    fn output(&self) -> Output {
        Output {
            cv: self.cv,
            block: words(&self.block),
            counter: self.counter,
            block_len: self.block_len as u32,
            flags: self.start_flag() | CHUNK_END,
        }
    }
}

fn parent_output(left: [u32; 8], right: [u32; 8]) -> Output {
    let mut block = [0u32; 16];
    block[..8].copy_from_slice(&left);
    block[8..].copy_from_slice(&right);
    Output {
        cv: IV,
        block,
        counter: 0,
        block_len: BLOCK_LEN as u32,
        flags: PARENT,
    }
}

/// Incremental BLAKE3 hasher (unkeyed, 32-byte output).
pub struct Hasher {
    chunk: ChunkState,
    stack: Vec<[u32; 8]>,
}

impl std::fmt::Debug for Hasher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hasher").finish_non_exhaustive()
    }
}

impl Default for Hasher {
    fn default() -> Self {
        Hasher::new()
    }
}

impl Hasher {
    pub fn new() -> Hasher {
        Hasher {
            chunk: ChunkState::new(0),
            stack: Vec::new(),
        }
    }

    fn add_chunk_cv(&mut self, mut cv: [u32; 8], mut total_chunks: u64) {
        while total_chunks & 1 == 0 {
            let Some(left) = self.stack.pop() else { break };
            cv = parent_output(left, cv).chaining_value();
            total_chunks >>= 1;
        }
        self.stack.push(cv);
    }

    pub fn update(&mut self, mut input: &[u8]) -> &mut Hasher {
        while !input.is_empty() {
            if self.chunk.len() == CHUNK_LEN {
                let cv = self.chunk.output().chaining_value();
                let total = self.chunk.counter + 1;
                self.add_chunk_cv(cv, total);
                self.chunk = ChunkState::new(total);
            }
            let take = (CHUNK_LEN - self.chunk.len()).min(input.len());
            self.chunk.update(&input[..take]);
            input = &input[take..];
        }
        self
    }

    /// The 32-byte digest.
    pub fn finalize(&self) -> [u8; 32] {
        let mut out = self.chunk.output();
        for left in self.stack.iter().rev() {
            out = parent_output(*left, out.chaining_value());
        }
        out.root_bytes()
    }

    /// The digest as 64 lowercase hex characters.
    pub fn finalize_hex(&self) -> String {
        to_hex(&self.finalize())
    }
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 15) as usize] as char);
    }
    s
}

/// BLAKE3 of `bytes`, lowercase hex.
pub fn hash_hex(bytes: &[u8]) -> String {
    Hasher::new().update(bytes).finalize_hex()
}

/// BLAKE3 over the little-endian bits of every left sample, then every right sample.
pub fn pcm_hash(left: &[f32], right: &[f32]) -> String {
    let mut h = Hasher::new();
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
    h.finalize_hex()
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
    fn incremental_equals_one_shot() {
        let data: Vec<u8> = (0..10_000u32).map(|i| (i * 31 % 251) as u8).collect();
        let mut h = Hasher::new();
        for c in data.chunks(77) {
            h.update(c);
        }
        assert_eq!(h.finalize_hex(), hash_hex(&data));
    }
}
