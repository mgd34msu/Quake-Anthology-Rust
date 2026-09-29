//! Bot asset files from `src/bots/behavior/assets.ts`.
//!
//! Text payloads (`botfiles/`, `bots/`, `scripts/`) are prepared from the
//! mounted resources before synchronous source AI setup. Paths are
//! normalized to forward slashes and matched case-insensitively; the
//! listing budget caps directory output at 1024 bytes like the donor.

use std::collections::HashMap;

/// Read/listing view over prepared bot source files.
pub trait BotSourceFiles {
    /// Read a file by path, or `None` when absent.
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    /// List immediate children of `directory` ending in `extension`.
    fn list(&self, directory: &str, extension: &str) -> Vec<String>;
    /// Provenance digest over the prepared payloads, when available.
    fn provenance(&self) -> Option<String> {
        None
    }
}

/// Normalize a bot asset path for case-insensitive lookup.
#[must_use]
pub fn normalize_bot_path(path: &str) -> String {
    path.replace('\\', "/")
        .strip_prefix("./")
        .unwrap_or(path)
        .replace('\\', "/")
        .to_lowercase()
}

/// Prepared bot asset files.
#[derive(Debug, Clone, Default)]
pub struct BotAssetFiles {
    files: HashMap<String, Vec<u8>>,
    paths: Vec<String>,
}

impl BotAssetFiles {
    /// Empty file set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a payload, preserving first-seen presentation order.
    pub fn add(&mut self, path: &str, bytes: &[u8]) {
        let normalized = normalize_bot_path(path);
        if !self.files.contains_key(&normalized) {
            self.paths.push(path.replace('\\', "/"));
        }
        self.files.insert(normalized, bytes.to_vec());
    }

    /// Number of prepared files.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether no files are prepared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

impl BotSourceFiles for BotAssetFiles {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.files.get(&normalize_bot_path(path)).cloned()
    }

    fn provenance(&self) -> Option<String> {
        // FNV-1a over (name length, payload length, name, payload) in
        // presentation order; the donor hashes the same framing with SHA-256.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |byte: u8| {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        };
        for path in &self.paths {
            let bytes = self
                .files
                .get(&normalize_bot_path(path))
                .expect("bot asset provenance lost its mounted payload");
            for b in (path.len() as u32).to_le_bytes() {
                mix(b);
            }
            for b in (bytes.len() as u32).to_le_bytes() {
                mix(b);
            }
            for b in path.bytes() {
                mix(b);
            }
            for b in bytes {
                mix(*b);
            }
        }
        Some(format!("{hash:016x}"))
    }

    fn list(&self, directory: &str, extension: &str) -> Vec<String> {
        let prefix = format!("{}/", normalize_bot_path(directory).trim_end_matches('/').to_owned());
        let suffix = extension.to_lowercase();
        let mut result = Vec::new();
        let mut bytes = 0usize;
        for path in &self.paths {
            let normalized = normalize_bot_path(path);
            if !normalized.starts_with(&prefix) || !normalized.ends_with(&suffix) {
                continue;
            }
            let name = &path[prefix.len()..];
            if name.contains('/') {
                continue;
            }
            if bytes + name.len() + 1 >= 1024 {
                break;
            }
            result.push(name.to_owned());
            bytes += name.len() + 1;
        }
        result
    }
}

/// Whether a mount path is a bot asset candidate.
#[must_use]
pub fn is_bot_asset_candidate(path: &str) -> bool {
    let lower = path.replace('\\', "/").to_lowercase();
    lower.starts_with("botfiles/")
        || lower.starts_with("bots/")
        || (lower.starts_with("scripts/")
            && (lower.ends_with(".txt") || lower.ends_with(".bot") || lower.ends_with(".arena")))
}

/// Load bot assets from an ordered mount listing plus explicit extras.
pub fn load_bot_asset_files(
    ordered_paths: &[String],
    extra_paths: &[String],
    open: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
) -> BotAssetFiles {
    let mut files = BotAssetFiles::new();
    let mut loaded = std::collections::HashSet::new();
    let candidates: Vec<&str> = ordered_paths
        .iter()
        .map(String::as_str)
        .filter(|path| is_bot_asset_candidate(path))
        .chain(extra_paths.iter().map(String::as_str))
        .collect();
    for path in candidates {
        let normalized = normalize_bot_path(path);
        if loaded.contains(&normalized) || normalized.ends_with('/') {
            continue;
        }
        loaded.insert(normalized);
        if let Some(bytes) = open(path) {
            files.add(path, &bytes);
        }
    }
    files
}
