//! Mounted-content font wiring contracts.
//!
//! Donor provenance: `src/text/mounted.ts`
//! (`createMountedTextFonts`, `MountedFontReader`). The `qa-content`
//! mount plan is not ported yet, so this module defines the sync
//! reader traits plus an in-memory mount for fixtures; behavior
//! follows the donor (mount-plan reads, retained DAT lifetimes).

use std::collections::BTreeMap;

use super::atlas::{FontImageServices, TextFontRegistry};
use super::draw2d::{FontFileReader, RetainedFontFile};

/// Mounted-content reads (`MountedContent` open subset, sync).
pub trait MountedContentReads {
    /// Open a path.
    fn open(&mut self, path: &str) -> Option<Vec<u8>>;
}

/// Scene image queue (`SceneImageRegistry` subset, sync).
pub trait SceneImageQueue {
    /// Register an image; returns its handle.
    fn register(&mut self, name: &str, width: u32, height: u32, rgba: Vec<u8>) -> u32;
    /// Release an image.
    fn release(&mut self, image: u32);
}

struct MountedServices<'a> {
    content: &'a mut dyn MountedContentReads,
    images: &'a mut dyn SceneImageQueue,
}

impl FontImageServices for MountedServices<'_> {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        self.content.open(path)
    }

    fn register_image(&mut self, name: &str, width: u32, height: u32, rgba: Vec<u8>) -> u32 {
        self.images.register(name, width, height, rgba)
    }

    fn release_image(&mut self, image: u32) {
        self.images.release(image);
    }
}

/// Create mounted text fonts (`createMountedTextFonts`).
///
/// The returned registry borrows the services adapter; keep it alive
/// for the registry lifetime.
pub fn create_mounted_text_fonts<'a>(services: &'a mut dyn FontImageServices) -> TextFontRegistry<'a> {
    TextFontRegistry::new(services)
}

/// Adapt mounted content plus the image queue into font services.
pub fn mounted_font_services<'a>(
    content: &'a mut dyn MountedContentReads,
    images: &'a mut dyn SceneImageQueue,
) -> impl FontImageServices + 'a {
    MountedServices { content, images }
}

/// A retained-source font reader (`MountedFontReader`).
#[derive(Default)]
pub struct MountedFontReader<'a> {
    content: Option<&'a mut dyn MountedContentReads>,
    cache: BTreeMap<String, Option<Vec<u8>>>,
    retained: Vec<RetainedFontFile>,
}

impl std::fmt::Debug for MountedFontReader<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MountedFontReader")
            .field("content", &self.content.is_some())
            .field("cache", &self.cache)
            .field("retained", &self.retained)
            .finish()
    }
}

impl<'a> MountedFontReader<'a> {
    /// New reader.
    #[must_use]
    pub fn new(content: &'a mut dyn MountedContentReads) -> Self {
        Self {
            content: Some(content),
            cache: BTreeMap::new(),
            retained: Vec::new(),
        }
    }

    /// Close the reader.
    pub fn close(&mut self) {
        self.retained.clear();
        self.cache.clear();
    }
}

impl FontFileReader for MountedFontReader<'_> {
    fn read_file_length(&mut self, path: &str) -> i64 {
        if !self.cache.contains_key(path) {
            let bytes = self.content.as_mut().and_then(|content| content.open(path));
            self.cache.insert(path.to_string(), bytes);
        }
        self.cache
            .get(path)
            .and_then(|bytes| bytes.as_ref())
            .map_or(-1, |bytes| bytes.len() as i64)
    }

    fn read_file_retained(&mut self, path: &str) -> Option<RetainedFontFile> {
        if !self.cache.contains_key(path) {
            let bytes = self.content.as_mut().and_then(|content| content.open(path));
            self.cache.insert(path.to_string(), bytes);
        }
        let bytes = self.cache.get(path).and_then(|bytes| bytes.clone())?;
        let file = RetainedFontFile {
            bytes: bytes.clone(),
            length: bytes.len(),
        };
        self.retained.push(file.clone());
        Some(file)
    }

    fn free_file(&mut self, file: &RetainedFontFile) {
        self.retained.retain(|retained| retained != file);
    }
}

/// An in-memory mount for fixtures.
#[derive(Debug, Default)]
pub struct MemoryMount {
    files: BTreeMap<String, Vec<u8>>,
}

impl MemoryMount {
    /// New mount.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a file.
    pub fn insert(&mut self, path: &str, bytes: Vec<u8>) {
        self.files.insert(path.to_string(), bytes);
    }
}

impl MountedContentReads for MemoryMount {
    fn open(&mut self, path: &str) -> Option<Vec<u8>> {
        self.files.get(path).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_reads_share_cache() {
        let mut mount = MemoryMount::new();
        mount.insert("fonts/a.dat", vec![1, 2, 3]);
        let mut reader = MountedFontReader::new(&mut mount);
        assert_eq!(reader.read_file_length("fonts/a.dat"), 3);
        assert_eq!(reader.read_file_length("fonts/missing.dat"), -1);
        let file = reader.read_file_retained("fonts/a.dat").unwrap();
        assert_eq!(file.length, 3);
        reader.free_file(&file);
        reader.close();
    }
}
