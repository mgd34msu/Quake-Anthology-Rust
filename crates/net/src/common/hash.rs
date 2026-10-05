//! Hashes for digests and salted credentials.
//!
//! The donor uses Node's `crypto` (`createHash`, `scryptSync`). The
//! SHA-256/MD4 implementations live in `qa-content` (which cannot depend on
//! this crate) and are re-exported here, so composition identities, download
//! checksums, and account verifiers hash through one copy.
//! [`password_verifier`] is a salted, iterated SHA-256 construction
//! documented at its definition; it is not scrypt and stored verifiers are
//! not interchangeable with donor saves.

pub use qa_content::hash::{hex_lower, md4, md4_block_checksum, md4_block_checksum_key, sha256_hex, Sha256};

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
