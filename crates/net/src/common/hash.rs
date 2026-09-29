//! Self-contained SHA-256 (FIPS 180-4) for digests and salted credentials.
//!
//! The donor uses Node's `crypto` (`createHash`, `scryptSync`). This crate
//! stays dependency-free, so composition identities, download checksums, and
//! account verifiers hash through this module instead. [`password_verifier`]
//! is a salted, iterated SHA-256 construction documented at its definition;
//! it is not scrypt and stored verifiers are not interchangeable with donor
//! saves.

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
    0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
    0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
    0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2,
];

#[derive(Debug, Clone)]
pub struct Sha256 {
    state: [u32; 8],
    length_bytes: u64,
    buffer: [u8; 64],
    buffered: usize,
}

impl Sha256 {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            length_bytes: 0,
            buffer: [0; 64],
            buffered: 0,
        }
    }

    pub fn update(&mut self, mut bytes: &[u8]) {
        self.length_bytes = self.length_bytes.wrapping_add(bytes.len() as u64);
        if self.buffered > 0 {
            let take = (64 - self.buffered).min(bytes.len());
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&bytes[..take]);
            self.buffered += take;
            bytes = &bytes[take..];
            if self.buffered == 64 {
                let block = self.buffer;
                Self::compress(&mut self.state, &block);
                self.buffered = 0;
            }
        }
        while bytes.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&bytes[..64]);
            Self::compress(&mut self.state, &block);
            bytes = &bytes[64..];
        }
        self.buffer[self.buffered..self.buffered + bytes.len()].copy_from_slice(bytes);
        self.buffered += bytes.len();
    }

    #[must_use]
    pub fn finish(mut self) -> [u8; 32] {
        let bit_length = self.length_bytes.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buffered != 56 {
            self.update(&[0x00]);
        }
        self.update(&bit_length.to_be_bytes());
        debug_assert_eq!(self.buffered, 0);
        let mut digest = [0u8; 32];
        for (word, slot) in self.state.iter().zip(digest.as_chunks_mut::<4>().0) {
            slot.copy_from_slice(&word.to_be_bytes());
        }
        digest
    }

    fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (index, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                block[index * 4],
                block[index * 4 + 1],
                block[index * 4 + 2],
                block[index * 4 + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7) ^ w[index - 15].rotate_right(18) ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17) ^ w[index - 2].rotate_right(19) ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

/// SHA-256 of `bytes`, lowercase hex.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_lower(&hasher.finish())
}

/// Lowercase hex encoding.
#[must_use]
pub fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(DIGITS[(byte >> 4) as usize] as char);
        text.push(DIGITS[(byte & 15) as usize] as char);
    }
    text
}

/// Decode lowercase or uppercase hex into `out`. Returns bytes written.
pub fn hex_decode(text: &str, out: &mut [u8]) -> Option<usize> {
    if !text.len().is_multiple_of(2) || text.len() / 2 > out.len() {
        return None;
    }
    for (index, slot) in out.iter_mut().take(text.len() / 2).enumerate() {
        let pair = &text[index * 2..index * 2 + 2];
        *slot = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(text.len() / 2)
}

/// Salted, iterated SHA-256 password verifier (`iterations` of
/// `SHA256(salt || previous || credential)`), lowercase hex.
/// This replaces donor scrypt; see the module docs.
#[must_use]
pub fn password_verifier(credential: &str, salt_hex: &str, iterations: u32) -> String {
    let mut salt = [0u8; 16];
    let mut block = [0u8; 32];
    let salt_len = hex_decode(salt_hex, &mut salt).unwrap_or(0);
    for _ in 0..iterations.max(1) {
        let mut hasher = Sha256::new();
        hasher.update(&salt[..salt_len]);
        hasher.update(&block);
        hasher.update(credential.as_bytes());
        block = hasher.finish();
    }
    hex_lower(&block)
}

/// MD4 digest (RFC 1320) for legacy BSP checksums.
#[must_use]
pub fn md4(bytes: &[u8]) -> [u8; 16] {
    fn f(x: u32, y: u32, z: u32) -> u32 {
        (x & y) | (!x & z)
    }
    fn g(x: u32, y: u32, z: u32) -> u32 {
        (x & y) | (x & z) | (y & z)
    }
    fn h(x: u32, y: u32, z: u32) -> u32 {
        x ^ y ^ z
    }
    let mut padded = bytes.to_vec();
    let bit_length = (bytes.len() as u64).wrapping_mul(8);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_le_bytes());
    let mut state = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476];
    for block in padded.as_chunks::<64>().0 {
        let mut x = [0u32; 16];
        for (index, word) in x.iter_mut().enumerate() {
            *word = u32::from_le_bytes([
                block[index * 4],
                block[index * 4 + 1],
                block[index * 4 + 2],
                block[index * 4 + 3],
            ]);
        }
        let (mut a, mut b, mut c, mut d) = (state[0], state[1], state[2], state[3]);
        // Round 1.
        for (index, shift) in [
            (0, 3),
            (1, 7),
            (2, 11),
            (3, 19),
            (4, 3),
            (5, 7),
            (6, 11),
            (7, 19),
            (8, 3),
            (9, 7),
            (10, 11),
            (11, 19),
            (12, 3),
            (13, 7),
            (14, 11),
            (15, 19),
        ] {
            let sum = a.wrapping_add(f(b, c, d)).wrapping_add(x[index]);
            a = d;
            d = c;
            c = b;
            b = sum.rotate_left(shift);
        }
        // Round 2.
        for (index, shift) in [
            (0, 3),
            (4, 5),
            (8, 9),
            (12, 13),
            (1, 3),
            (5, 5),
            (9, 9),
            (13, 13),
            (2, 3),
            (6, 5),
            (10, 9),
            (14, 13),
            (3, 3),
            (7, 5),
            (11, 9),
            (15, 13),
        ] {
            let sum = a
                .wrapping_add(g(b, c, d))
                .wrapping_add(x[index])
                .wrapping_add(0x5a827999);
            a = d;
            d = c;
            c = b;
            b = sum.rotate_left(shift);
        }
        // Round 3.
        for (index, shift) in [
            (0, 3),
            (8, 9),
            (4, 11),
            (12, 15),
            (2, 3),
            (10, 9),
            (6, 11),
            (14, 15),
            (1, 3),
            (9, 9),
            (5, 11),
            (13, 15),
            (3, 3),
            (11, 9),
            (7, 11),
            (15, 15),
        ] {
            let sum = a
                .wrapping_add(h(b, c, d))
                .wrapping_add(x[index])
                .wrapping_add(0x6ed9eba1);
            a = d;
            d = c;
            c = b;
            b = sum.rotate_left(shift);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }
    let mut digest = [0u8; 16];
    for (word, slot) in state.iter().zip(digest.as_chunks_mut::<4>().0) {
        slot.copy_from_slice(&word.to_le_bytes());
    }
    digest
}

/// XOR-folded MD4 block checksum (`blockChecksum`).
#[must_use]
pub fn md4_block_checksum(bytes: &[u8]) -> u32 {
    let digest = md4(bytes);
    u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]])
        ^ u32::from_le_bytes([digest[4], digest[5], digest[6], digest[7]])
        ^ u32::from_le_bytes([digest[8], digest[9], digest[10], digest[11]])
        ^ u32::from_le_bytes([digest[12], digest[13], digest[14], digest[15]])
}

/// Keyed MD4 block checksum (`blockChecksumKey`): the little-endian key
/// prefixes the hashed bytes.
#[must_use]
pub fn md4_block_checksum_key(bytes: &[u8], key: u32) -> u32 {
    let mut keyed = Vec::with_capacity(bytes.len() + 4);
    keyed.extend_from_slice(&key.to_le_bytes());
    keyed.extend_from_slice(bytes);
    md4_block_checksum(&keyed)
}

/// Constant-time equality for credential hashes.
#[must_use]
pub fn timing_safe_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_fips_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn md4_matches_rfc_vectors() {
        assert_eq!(hex_lower(&md4(b"")), "31d6cfe0d16ae931b73c59d7e0c089c0");
        assert_eq!(hex_lower(&md4(b"abc")), "a448017aaf21d8525fc10ae87aa6729d");
        assert_eq!(
            hex_lower(&md4(b"abcdefghijklmnopqrstuvwxyz")),
            "d79e1c308aa5bbcdeea8ed63df412da9"
        );
    }
}
