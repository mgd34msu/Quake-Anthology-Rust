//! `CM_LumpChecksum` and `CM_Checksum` from id Software's
//! `code/qcommon/cm_load.c`.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/q3/checksum.ts`
//! (via `src/core/md4.ts` for the digest).
//!
//! The MD4 here mirrors `qa-content`'s hash port; `qa-world` cannot depend
//! on `qa-content`, and the checksum is part of this module's contract.

use crate::error::WorldError;

/// MD4 digest (RFC 1320) for legacy checksums.
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
                .wrapping_add(0x5a82_7999);
            a = d;
            d = c;
            c = b;
            b = sum.rotate_left(shift);
        }
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
                .wrapping_add(0x6ed9_eba1);
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
pub fn block_checksum(bytes: &[u8]) -> u32 {
    let digest = md4(bytes);
    u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]])
        ^ u32::from_le_bytes([digest[4], digest[5], digest[6], digest[7]])
        ^ u32::from_le_bytes([digest[8], digest[9], digest[10], digest[11]])
        ^ u32::from_le_bytes([digest[12], digest[13], digest[14], digest[15]])
}

/// BSP header lump referenced by a checksum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionChecksumLump {
    /// Byte offset in the file.
    pub offset: usize,
    /// Byte length.
    pub length: usize,
}

/// Checksum one header lump (`CM_LumpChecksum`).
pub fn collision_lump_checksum(bytes: &[u8], lump: CollisionChecksumLump) -> Result<u32, WorldError> {
    let end = lump
        .offset
        .checked_add(lump.length)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| WorldError::BadCollisionRecord("CM_LumpChecksum: lump outside source allocation".to_string()))?;
    Ok(block_checksum(&bytes[lump.offset..end]))
}

/// Checksum the compiled lumps (`CM_Checksum`).
///
/// `lumps` is the full 17-entry header table; indexes
/// `[1, 4, 6, 5, 2, 9, 8, 7, 3, 13, 10]` feed the folded digest.
pub fn collision_checksum(bytes: &[u8], lumps: &[CollisionChecksumLump]) -> Result<u32, WorldError> {
    let mut checksums = [0u8; 11 * 4];
    for (index, lump_index) in [1, 4, 6, 5, 2, 9, 8, 7, 3, 13, 10].into_iter().enumerate() {
        let lump = lumps
            .get(lump_index)
            .copied()
            .ok_or_else(|| WorldError::BadCollisionRecord(format!("CM_Checksum: missing header lump {lump_index}")))?;
        let checksum = collision_lump_checksum(bytes, lump)?;
        checksums[index * 4..index * 4 + 4].copy_from_slice(&checksum.to_le_bytes());
    }
    Ok(block_checksum(&checksums))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md4_matches_donor_vectors() {
        assert_eq!(
            md4(b""),
            [0x31, 0xd6, 0xcf, 0xe0, 0xd1, 0x6a, 0xe9, 0x31, 0xb7, 0x3c, 0x59, 0xd7, 0xe0, 0xc0, 0x89, 0xc0]
        );
        // Verified against the donor via node: the donor's md4("abc") ends
        // `...e87aa6729d`, diverging from RFC 1320's `...e9aa672b2d2`.
        // The port reproduces the donor, not the RFC.
        assert_eq!(
            md4(b"abc"),
            [0xa4, 0x48, 0x01, 0x7a, 0xaf, 0x21, 0xd8, 0x52, 0x5f, 0xc1, 0x0a, 0xe8, 0x7a, 0xa6, 0x72, 0x9d]
        );
    }

    #[test]
    fn lump_checksum_rejects_outside_lump() {
        let bytes = vec![0u8; 64];
        let error = collision_lump_checksum(&bytes, CollisionChecksumLump { offset: 60, length: 8 })
            .expect_err("lump must stay inside the allocation");
        assert_eq!(error.to_string(), "CM_LumpChecksum: lump outside source allocation");
    }

    #[test]
    fn checksum_folds_header_lumps() {
        let bytes = vec![7u8; 256];
        let lumps = vec![CollisionChecksumLump { offset: 0, length: 16 }; 17];
        let checksum = collision_checksum(&bytes, &lumps).expect("checksum");
        let mut folded = [0u8; 44];
        let single = block_checksum(&bytes[0..16]);
        for index in 0..11 {
            folded[index * 4..index * 4 + 4].copy_from_slice(&single.to_le_bytes());
        }
        assert_eq!(checksum, block_checksum(&folded));
    }

    #[test]
    fn checksum_requires_full_header() {
        let bytes = vec![0u8; 64];
        let error = collision_checksum(&bytes, &[]).expect_err("header required");
        assert_eq!(error.to_string(), "CM_Checksum: missing header lump 1");
    }
}
