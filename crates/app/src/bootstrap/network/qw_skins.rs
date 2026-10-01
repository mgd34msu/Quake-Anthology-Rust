//! QuakeWorld player skin selection and caching.
//!
//! Port of `src/app/bootstrap/network/qw-skins.ts` (`QwPlayerSkins`,
//! `Skin_Find`/`Skin_Cache` selection). The donor is async; this sync port
//! resolves reads inline and caches decoded skins instead of promises.
//! Hashing uses [`sha256_hex`](qa_content::hash::sha256_hex) (the donor's
//! `node:crypto createHash('sha256')`), decoding uses
//! [`decode_pcx`](qa_content::images::indexed::decode_pcx), and the cached
//! value is the existing
//! [`IndexedModelSkin`](qa_client::render::scene::models::types::IndexedModelSkin).
//! Every decode failure still resolves to `None`, matching the donor's
//! `try/catch` returning `null`.

use std::collections::HashMap;

use qa_client::render::scene::models::types::IndexedModelSkin;
use qa_content::hash::sha256_hex;
use qa_content::images::indexed::decode_pcx;
use thiserror::Error;

/// Content-file lookup for skin bytes.
pub type SkinReader = Box<dyn FnMut(&str) -> Option<Vec<u8>>>;

/// QuakeWorld skin options (`QwSkinOptions`).
pub struct QwSkinOptions {
    /// Read a content file, returning `None` when missing.
    pub read: SkinReader,
    /// `noskins` cvar value.
    pub noskins: Box<dyn Fn() -> i32>,
    /// `baseskin` cvar value.
    pub baseskin: Box<dyn Fn() -> String>,
    /// `allskins` cvar value.
    pub allskins: Box<dyn Fn() -> String>,
}

/// Error for QuakeWorld player skins.
///
/// The donor swallows every load failure into `null`, so selection is total
/// and this only reserves the module's error domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum QwSkinError {}

/// Maximum cached skins before the cache clears (`128`).
const MAX_CACHED_SKINS: usize = 128;
/// Padded skin stride (`320x200`).
const PADDED_WIDTH: usize = 320;
const PADDED_HEIGHT: usize = 200;
/// Cropped skin size (`296x194`).
const SKIN_WIDTH: usize = 296;
const SKIN_HEIGHT: usize = 194;

/// Sanitize a skin name (donor `skinName`).
pub fn skin_name(value: &str) -> String {
    let valid = !value.is_empty()
        && !value.contains("..")
        && !value.starts_with('.')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '+' || c == '.' || c == '-');
    if !valid {
        return "base".to_string();
    }
    let stem = match value.rfind('.') {
        Some(dot) => &value[..dot],
        None => value,
    };
    let stem: String = stem.chars().take(15).collect();
    if stem.is_empty() {
        "base".to_string()
    } else {
        stem
    }
}

/// QuakeWorld player skins (donor `QwPlayerSkins`).
pub struct QwPlayerSkins {
    options: QwSkinOptions,
    cache: HashMap<String, Option<IndexedModelSkin>>,
}

impl QwPlayerSkins {
    /// Create player skins over the given options.
    pub fn new(options: QwSkinOptions) -> Self {
        Self {
            options,
            cache: HashMap::new(),
        }
    }

    /// Resolve the effective skin name (donor `name`).
    pub fn name(&mut self, userinfo_skin: &str) -> String {
        let all = (self.options.allskins)();
        if !all.is_empty() {
            return skin_name(&all);
        }
        if !userinfo_skin.is_empty() {
            return skin_name(userinfo_skin);
        }
        skin_name(&(self.options.baseskin)())
    }

    /// Select a skin, returning `None` for `noskins` or any load failure
    /// (donor `select`).
    pub fn select(&mut self, userinfo_skin: &str) -> Option<IndexedModelSkin> {
        if (self.options.noskins)() == 1 {
            return None;
        }
        let base = skin_name(&(self.options.baseskin)());
        let selected = self.name(userinfo_skin);
        let key = format!("{selected}\0{base}");
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }
        if self.cache.len() == MAX_CACHED_SKINS {
            self.clear();
        }
        let loaded = self.load(&selected, &base);
        self.cache.insert(key, loaded.clone());
        loaded
    }

    /// Clear the cache (donor `clear`).
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Read the skin policy cvars (`noskins`, `baseskin`, `allskins`).
    pub fn policy(&self) -> (i32, String, String) {
        (
            (self.options.noskins)(),
            (self.options.baseskin)(),
            (self.options.allskins)(),
        )
    }

    /// Load, pad, and crop a skin (donor `load`).
    fn load(&mut self, selected: &str, base: &str) -> Option<IndexedModelSkin> {
        let mut path = format!("skins/{selected}.pcx");
        let mut bytes = (self.options.read)(&path);
        if bytes.is_none() && selected != base {
            path = format!("skins/{base}.pcx");
            bytes = (self.options.read)(&path);
        }
        let bytes = bytes?;
        if bytes.len() < 128 {
            return None;
        }
        let xmax = u16::from_le_bytes([bytes[8], bytes[9]]);
        let ymax = u16::from_le_bytes([bytes[10], bytes[11]]);
        if xmax >= PADDED_WIDTH as u16 || ymax >= PADDED_HEIGHT as u16 {
            return None;
        }
        let decoded = decode_pcx(&bytes, &path).ok()?;
        let pixel_count = decoded.width.checked_mul(decoded.height)?;
        if decoded.indices.len() < pixel_count {
            return None;
        }
        let mut padded = vec![0u8; PADDED_WIDTH * PADDED_HEIGHT];
        for row in 0..decoded.height {
            let source_start = row.checked_mul(decoded.width)?;
            let source_end = source_start.checked_add(decoded.width)?;
            let dest_start = row.checked_mul(PADDED_WIDTH)?;
            let dest_end = dest_start.checked_add(decoded.width)?;
            let source = decoded.indices.get(source_start..source_end)?;
            let dest = padded.get_mut(dest_start..dest_end)?;
            dest.copy_from_slice(source);
        }
        let mut pixels = vec![0u8; SKIN_WIDTH * SKIN_HEIGHT];
        for row in 0..SKIN_HEIGHT {
            let source = padded.get(row * PADDED_WIDTH..row * PADDED_WIDTH + SKIN_WIDTH)?;
            let dest = pixels.get_mut(row * SKIN_WIDTH..(row + 1) * SKIN_WIDTH)?;
            dest.copy_from_slice(source);
        }
        Some(IndexedModelSkin {
            name: format!("qw-skin:{}:crop:0,0,296,194:stride320", sha256_hex(&bytes)),
            width: SKIN_WIDTH as u32,
            height: SKIN_HEIGHT as u32,
            pixels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn options(
        files: HashMap<String, Vec<u8>>,
        noskins: i32,
        baseskin: &str,
        allskins: &str,
        reads: Rc<RefCell<Vec<String>>>,
    ) -> QwSkinOptions {
        let baseskin = baseskin.to_string();
        let allskins = allskins.to_string();
        QwSkinOptions {
            read: Box::new(move |path: &str| {
                reads.borrow_mut().push(path.to_string());
                files.get(path).cloned()
            }),
            noskins: Box::new(move || noskins),
            baseskin: Box::new({
                let baseskin = baseskin.clone();
                move || baseskin.clone()
            }),
            allskins: Box::new({
                let allskins = allskins.clone();
                move || allskins.clone()
            }),
        }
    }

    /// Minimal 2x1 valid PCX: header + two literal pixels.
    fn pcx_fixture() -> Vec<u8> {
        let mut header = vec![0u8; 128];
        header[0] = 10;
        header[1] = 5;
        header[2] = 1;
        header[3] = 8;
        header[8] = 1;
        header[65] = 1;
        header[66] = 2;
        header.extend_from_slice(&[7, 9]);
        header
    }

    #[test]
    fn skin_name_sanitizes_and_strips_extensions() {
        assert_eq!(skin_name("duke"), "duke");
        assert_eq!(skin_name("duke.pcx"), "duke");
        assert_eq!(skin_name("a.b.c"), "a.b");
        assert_eq!(skin_name("0123456789abcdef"), "0123456789abcde");
        assert_eq!(skin_name(".."), "base");
        assert_eq!(skin_name("a..b"), "base");
        assert_eq!(skin_name(".hidden"), "base");
        assert_eq!(skin_name("a/b"), "base");
        assert_eq!(skin_name("a b"), "base");
        assert_eq!(skin_name(""), "base");
        assert_eq!(skin_name("a+b_c-d.e"), "a+b_c-d");
    }

    #[test]
    fn name_prefers_allskins_then_userinfo_then_base() {
        let reads = Rc::new(RefCell::new(Vec::new()));
        let mut skins = QwPlayerSkins::new(options(HashMap::new(), 0, "base", "forced", Rc::clone(&reads)));
        assert_eq!(skins.name("duke"), "forced");

        let mut skins = QwPlayerSkins::new(options(HashMap::new(), 0, "base", "", Rc::clone(&reads)));
        assert_eq!(skins.name("duke"), "duke");
        assert_eq!(skins.name(""), "base");
    }

    #[test]
    fn noskins_disables_selection_without_reading() {
        let reads = Rc::new(RefCell::new(Vec::new()));
        let mut skins = QwPlayerSkins::new(options(HashMap::new(), 1, "base", "", Rc::clone(&reads)));
        assert!(skins.select("duke").is_none());
        assert!(reads.borrow().is_empty());
    }

    #[test]
    fn select_decodes_pads_and_crops() {
        let bytes = pcx_fixture();
        let mut files = HashMap::new();
        files.insert("skins/duke.pcx".to_string(), bytes);
        let reads = Rc::new(RefCell::new(Vec::new()));
        let mut skins = QwPlayerSkins::new(options(files, 0, "base", "", Rc::clone(&reads)));
        let skin = skins.select("duke").expect("valid skin decodes");
        assert_eq!(skin.width, 296);
        assert_eq!(skin.height, 194);
        assert_eq!(skin.pixels.len(), 296 * 194);
        assert_eq!(&skin.pixels[..4], &[7, 9, 0, 0]);
        assert_eq!(
            skin.name,
            "qw-skin:0c55e46e8102fc383cc50c4e11752273c8d7e5e6831d5e5f64ac376d9f53bd96:crop:0,0,296,194:stride320"
        );
        assert_eq!(*reads.borrow(), vec!["skins/duke.pcx".to_string()]);
    }

    #[test]
    fn select_caches_by_selected_and_base_key() {
        let mut files = HashMap::new();
        files.insert("skins/duke.pcx".to_string(), pcx_fixture());
        let reads = Rc::new(RefCell::new(Vec::new()));
        let mut skins = QwPlayerSkins::new(options(files, 0, "base", "", Rc::clone(&reads)));
        assert!(skins.select("duke").is_some());
        assert!(skins.select("duke").is_some());
        assert_eq!(reads.borrow().len(), 1);
    }

    #[test]
    fn select_falls_back_to_base_skin() {
        let mut files = HashMap::new();
        files.insert("skins/base.pcx".to_string(), pcx_fixture());
        let reads = Rc::new(RefCell::new(Vec::new()));
        let mut skins = QwPlayerSkins::new(options(files, 0, "base", "", Rc::clone(&reads)));
        assert!(skins.select("missing").is_some());
        assert_eq!(
            *reads.borrow(),
            vec!["skins/missing.pcx".to_string(), "skins/base.pcx".to_string()]
        );
    }

    #[test]
    fn select_returns_none_when_everything_is_missing() {
        let reads = Rc::new(RefCell::new(Vec::new()));
        let mut skins = QwPlayerSkins::new(options(HashMap::new(), 0, "base", "", Rc::clone(&reads)));
        assert!(skins.select("missing").is_none());
    }

    #[test]
    fn short_and_oversized_files_resolve_to_none() {
        let mut files = HashMap::new();
        files.insert("skins/short.pcx".to_string(), vec![0u8; 64]);
        let mut oversized = pcx_fixture();
        oversized[8] = 0x40;
        oversized[9] = 0x01;
        files.insert("skins/big.pcx".to_string(), oversized);
        files.insert("skins/junk.pcx".to_string(), vec![0u8; 256]);
        let reads = Rc::new(RefCell::new(Vec::new()));
        let mut skins = QwPlayerSkins::new(options(files, 0, "absent", "", Rc::clone(&reads)));
        assert!(skins.select("short").is_none());
        assert!(skins.select("big").is_none());
        assert!(skins.select("junk").is_none());
    }

    #[test]
    fn clear_empties_the_cache() {
        let mut files = HashMap::new();
        files.insert("skins/duke.pcx".to_string(), pcx_fixture());
        let reads = Rc::new(RefCell::new(Vec::new()));
        let mut skins = QwPlayerSkins::new(options(files, 0, "base", "", Rc::clone(&reads)));
        assert!(skins.select("duke").is_some());
        skins.clear();
        assert!(skins.select("duke").is_some());
        assert_eq!(reads.borrow().len(), 2);
    }
}
