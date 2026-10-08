//! Original wire/progs checksums only. Never use these for content identity.
/* Derived from the RSA Data Security, Inc. MD4 Message-Digest Algorithm.
Copyright (C) 1990-2, RSA Data Security, Inc. All rights reserved.

License to copy and use this software is granted provided that it is identified
as the RSA Data Security, Inc. MD4 Message-Digest Algorithm in all material
mentioning or referencing this software or this function.
License is also granted to make and use derivative works provided that such
works are identified as derived from the RSA Data Security, Inc. MD4
Message-Digest Algorithm in all material mentioning or referencing the derived work.
RSA Data Security, Inc. makes no representations concerning either the
merchantability of this software or the suitability of this software for any
particular purpose. It is provided as is without express or implied warranty
of any kind. These notices must be retained in any copies of any part of this
documentation and/or software. */
mod salts;

struct Md4 {
    state: [u32; 4],
    bytes: u64,
    buffer: [u8; 64],
    used: usize,
}
impl Md4 {
    fn new() -> Self {
        Self {
            state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476],
            bytes: 0,
            buffer: [0; 64],
            used: 0,
        }
    }
    fn update(&mut self, mut data: &[u8]) {
        self.bytes = self.bytes.wrapping_add(data.len() as u64);
        if self.used != 0 {
            let take = data.len().min(64 - self.used);
            self.buffer[self.used..self.used + take].copy_from_slice(&data[..take]);
            self.used += take;
            data = &data[take..];
            if self.used == 64 {
                compress(&mut self.state, &self.buffer);
                self.used = 0;
            }
        }
        for block in data.as_chunks::<64>().0 {
            compress(&mut self.state, block);
        }
        let tail = data.as_chunks::<64>().1;
        if !tail.is_empty() {
            self.buffer[..tail.len()].copy_from_slice(tail);
            self.used = tail.len();
        }
    }
    fn finish(mut self) -> u32 {
        let bits = self.bytes.wrapping_mul(8);
        let mut padding = [0; 64];
        padding[0] = 128;
        let count = if self.used < 56 {
            56 - self.used
        } else {
            120 - self.used
        };
        self.update(&padding[..count]);
        self.update(&bits.to_le_bytes());
        self.state.into_iter().fold(0, |a, b| a ^ b)
    }
}
fn compress(state: &mut [u32; 4], block: &[u8; 64]) {
    let words: [u32; 16] = std::array::from_fn(|i| {
        u32::from_le_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ])
    });
    let [mut a, mut b, mut c, mut d] = *state;
    let shifts = [[3, 7, 11, 19], [3, 5, 9, 13], [3, 9, 11, 15]];
    for (round, rotate) in shifts.into_iter().enumerate() {
        for step in 0..16usize {
            let (mix, index, constant) = match round {
                0 => ((b & c) | (!b & d), step, 0),
                1 => (
                    (b & c) | (b & d) | (c & d),
                    (step % 4) * 4 + step / 4,
                    0x5a827999,
                ),
                _ => (
                    b ^ c ^ d,
                    step.reverse_bits() >> (usize::BITS - 4),
                    0x6ed9eba1,
                ),
            };
            let next = a
                .wrapping_add(mix)
                .wrapping_add(words[index])
                .wrapping_add(constant)
                .rotate_left(rotate[step % 4]);
            a = d;
            d = c;
            c = b;
            b = next;
        }
    }
    for (word, value) in state.iter_mut().zip([a, b, c, d]) {
        *word = word.wrapping_add(value);
    }
}
pub fn block_checksum(data: &[u8]) -> u32 {
    let mut md4 = Md4::new();
    md4.update(data);
    md4.finish()
}
pub fn block_checksum_key(data: &[u8], key: u32) -> u32 {
    let mut md4 = Md4::new();
    md4.update(&key.to_le_bytes());
    md4.update(data);
    md4.finish()
}
const CRC_TABLE: [u16; 256] = {
    let mut table = [0; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = (i as u16) << 8;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};
pub fn crc_block(data: &[u8]) -> u16 {
    data.iter().fold(0xffff, |crc, &byte| {
        (crc << 8) ^ CRC_TABLE[((crc >> 8) ^ u16::from(byte)) as usize]
    })
}
pub fn qw_sequence_crc(data: &[u8], sequence: u32) -> u8 {
    let length = data.len().min(60);
    let at = sequence as usize % 1020;
    let mut block = [0; 64];
    block[..length].copy_from_slice(&data[..length]);
    block[length..length + 4].copy_from_slice(&salts::QW[at..at + 4]);
    block[length] ^= sequence as u8;
    block[length + 2] ^= (sequence >> 8) as u8;
    crc_block(&block[..length + 4]) as u8
}
pub fn q2_sequence_crc(data: &[u8], sequence: u32) -> u8 {
    let length = data.len().min(60);
    let at = sequence as usize % 1020;
    let mut block = [0; 64];
    block[..length].copy_from_slice(&data[..length]);
    block[length..length + 4].copy_from_slice(&salts::Q2[at..at + 4]);
    let sum = block[..length + 4]
        .iter()
        .fold(0u16, |sum, &byte| sum + u16::from(byte));
    (crc_block(&block[..length + 4]) ^ sum) as u8
}
