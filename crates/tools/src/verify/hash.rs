//! Fingerprint helpers (donor `tools/verify/hash.ts`).
//!
//! Canonical JSON plus SHA-256 over bytes, values, and files.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::error::ToolsError;
use crate::json::Json;
use crate::sha256::{hash_hex, Hasher};

/// Whether a value is a JSON object.
#[must_use]
pub fn is_object(value: &Json) -> bool {
    matches!(value, Json::Object(_))
}

/// Whether a value is a JSON array.
#[must_use]
pub fn is_array(value: &Json) -> bool {
    matches!(value, Json::Array(_))
}

/// Canonical JSON: sorted keys, compact separators (donor `canonicalJson`).
pub fn canonical_json(value: &Json) -> Result<String, ToolsError> {
    value.render_canonical().map_err(|_| ToolsError::invalid("Fingerprints require finite JSON data"))
}

/// SHA-256 hex digest of bytes (donor `hashBytes`).
#[must_use]
pub fn hash_bytes(bytes: &[u8]) -> String {
    hash_hex(bytes)
}

/// SHA-256 hex digest of text.
#[must_use]
pub fn hash_str(text: &str) -> String {
    hash_hex(text.as_bytes())
}

/// SHA-256 hex digest of a value's canonical JSON (donor `hashJson`).
pub fn hash_json(value: &Json) -> Result<String, ToolsError> {
    Ok(hash_hex(canonical_json(value)?.as_bytes()))
}

/// SHA-256 hex digest of a file's bytes (donor `hashFile`).
pub fn hash_file(path: &Path) -> Result<String, ToolsError> {
    let mut file = File::open(path).map_err(|error| ToolsError::io(format!("reading {}", path.display()), error))?;
    let mut hasher = Hasher::new();
    let mut chunk = [0u8; 65536];
    loop {
        let count = file
            .read(&mut chunk)
            .map_err(|error| ToolsError::io(format!("reading {}", path.display()), error))?;
        if count == 0 {
            break;
        }
        hasher.update(&chunk[..count]);
    }
    Ok(crate::sha256::to_hex(&hasher.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::parse_json;

    #[test]
    fn canonical_form_matches_donor() {
        let value = parse_json("{\"b\":[3,2],\"a\":{\"y\":1,\"x\":true}}").unwrap();
        assert_eq!(canonical_json(&value).unwrap(), "{\"a\":{\"x\":true,\"y\":1},\"b\":[3,2]}");
    }

    #[test]
    fn hash_json_is_stable() {
        let left = parse_json("{\"a\":1,\"b\":2}").unwrap();
        let right = parse_json("{\"b\":2,\"a\":1}").unwrap();
        assert_eq!(hash_json(&left).unwrap(), hash_json(&right).unwrap());
    }

    #[test]
    fn hash_bytes_matches_sha256() {
        assert_eq!(
            hash_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
