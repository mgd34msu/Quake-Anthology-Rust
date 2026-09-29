//! MD3 level-of-detail slots (`src/formats/q3-model/lod.ts`).
//!
//! Donor provenance: `src/formats/q3-model/lod.ts` (slot probing mirrors
//! the Q3 `R_LoadMD3` LOD probing). The donor loader is async over the
//! VFS; this port takes a synchronous [`Md3ModelReader`].

use qa_core::binary::BinaryReader;

use crate::md3::{parse_md3, Md3Model, MD3_IDENT};

/// MD3 byte source for LOD probing.
pub trait Md3ModelReader {
    /// Read a model file, or return `None` when absent.
    fn read(&mut self, path: &str) -> Option<Vec<u8>>;
}

impl<F> Md3ModelReader for F
where
    F: FnMut(&str) -> Option<Vec<u8>>,
{
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        self(path)
    }
}

/// One LOD slot (`Md3LodSlot`).
#[derive(Debug, Clone, PartialEq)]
pub enum Md3LodSlot {
    /// Slot file is missing.
    Missing {
        /// Slot path.
        path: String,
    },
    /// Slot parsed.
    Loaded {
        /// Slot path.
        path: String,
        /// Parsed model.
        model: Md3Model,
        /// Slot bytes.
        bytes: Vec<u8>,
    },
    /// Slot failed to parse.
    Invalid {
        /// Slot path.
        path: String,
        /// Failure message.
        error: String,
    },
    /// Slot reuses a lower LOD.
    Alias {
        /// Slot path.
        path: String,
        /// Source slot index.
        source_slot: usize,
    },
}

impl Md3LodSlot {
    /// Slot path.
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Self::Missing { path }
            | Self::Loaded { path, .. }
            | Self::Invalid { path, .. }
            | Self::Alias { path, .. } => path,
        }
    }
}

/// Loaded LOD chain (`Md3LodModel`).
#[derive(Debug, Clone, PartialEq)]
pub struct Md3LodModel {
    /// Requested path.
    pub path: String,
    /// Slots in LOD order.
    pub slots: [Md3LodSlot; 3],
    /// Slots probed, in probe order.
    pub load_order: Vec<usize>,
    /// Loaded LOD count, including aliases.
    pub num_lods: usize,
    /// Total loaded bytes.
    pub byte_length: usize,
}

/// Slot paths: the model itself plus `_1`/`.md3` siblings (`md3LodPaths`).
#[must_use]
pub fn md3_lod_paths(path: &str) -> [String; 3] {
    let stem = match path.rfind('.') {
        Some(dot) => &path[..dot],
        None => path,
    };
    [path.to_string(), format!("{stem}_1.md3"), format!("{stem}_2.md3")]
}

/// Missing files keep their independent slots. A failed nonzero LOD
/// aliases only lower slots (`loadMd3Lods`).
pub fn load_md3_lods(path: &str, reader: &mut dyn Md3ModelReader) -> Option<Md3LodModel> {
    let paths = md3_lod_paths(path);
    let mut slots: [Md3LodSlot; 3] = paths.clone().map(|path| Md3LodSlot::Missing { path });
    let mut load_order = Vec::new();
    let mut num_lods = 0;
    let mut byte_length = 0;
    for slot in [2, 1, 0] {
        let filename = paths[slot].clone();
        match probe_slot(&filename, reader) {
            Probe::Missing => {}
            Probe::Loaded { model, bytes } => {
                load_order.push(slot);
                byte_length += bytes.len();
                num_lods += 1;
                slots[slot] = Md3LodSlot::Loaded {
                    path: filename,
                    model,
                    bytes,
                };
            }
            Probe::WrongMagic => return None,
            Probe::Invalid { error } => {
                load_order.push(slot);
                if slot == 0 {
                    return None;
                }
                slots[slot] = Md3LodSlot::Invalid { path: filename, error };
                if num_lods == 0 {
                    return None;
                }
                for lower in (0..slot).rev() {
                    num_lods += 1;
                    let source_slot = lower + 1;
                    let path = paths[lower].clone();
                    slots[lower] = Md3LodSlot::Alias { path, source_slot };
                }
                break;
            }
        }
    }
    if num_lods == 0 {
        return None;
    }
    Some(Md3LodModel {
        path: path.to_string(),
        slots,
        load_order,
        num_lods,
        byte_length,
    })
}

enum Probe {
    Missing,
    Loaded { model: Md3Model, bytes: Vec<u8> },
    WrongMagic,
    Invalid { error: String },
}

fn probe_slot(filename: &str, reader: &mut dyn Md3ModelReader) -> Probe {
    let bytes = match reader.read(filename) {
        Some(bytes) => bytes,
        None => return Probe::Missing,
    };
    let mut probe = BinaryReader::new(&bytes, filename);
    match probe.u32() {
        Ok(ident) if ident != MD3_IDENT => return Probe::WrongMagic,
        Err(error) => return Probe::Invalid { error: error.message },
        _ => {}
    }
    match parse_md3(&bytes, filename) {
        Ok(decoded) => Probe::Loaded {
            model: decoded.model,
            bytes: decoded.bytes,
        },
        Err(error) => Probe::Invalid { error: error.message },
    }
}

/// Resolve a slot through aliases (`md3AtLod`).
///
/// # Panics
///
/// Panics when the LOD is outside `0..=2`.
#[must_use]
pub fn md3_at_lod(model: &Md3LodModel, lod: i32) -> Option<&Md3Model> {
    if !(0..=2).contains(&lod) {
        panic!("MD3 LOD must be 0..2");
    }
    match &model.slots[lod as usize] {
        Md3LodSlot::Loaded { model, .. } => Some(model),
        Md3LodSlot::Alias { source_slot, .. } => md3_at_lod(model, *source_slot as i32),
        Md3LodSlot::Missing { .. } | Md3LodSlot::Invalid { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn model_bytes(name: &str) -> Vec<u8> {
        // Minimal model: 1 frame, 0 tags, 0 surfaces.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MD3_IDENT.to_le_bytes());
        bytes.extend_from_slice(&15i32.to_le_bytes());
        let mut stored = [0u8; 64];
        stored[..name.len().min(64)].copy_from_slice(&name.as_bytes()[..name.len().min(64)]);
        bytes.extend_from_slice(&stored);
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&108i32.to_le_bytes());
        bytes.extend_from_slice(&164i32.to_le_bytes());
        bytes.extend_from_slice(&164i32.to_le_bytes());
        bytes.extend_from_slice(&164i32.to_le_bytes());
        for _ in 0..9 {
            bytes.extend_from_slice(&0f32.to_le_bytes());
        }
        bytes.extend_from_slice(&1f32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 16]);
        bytes
    }

    #[test]
    fn lod_paths() {
        assert_eq!(
            md3_lod_paths("models/player/head.md3"),
            [
                "models/player/head.md3".to_string(),
                "models/player/head_1.md3".to_string(),
                "models/player/head_2.md3".to_string(),
            ]
        );
        assert_eq!(md3_lod_paths("head")[2], "head_2.md3".to_string());
    }

    #[test]
    fn lod_chain() {
        let mut files = HashMap::new();
        files.insert("models/player/head_2.md3".to_string(), model_bytes("lod2"));
        files.insert("models/player/head.md3".to_string(), model_bytes("lod0"));
        let mut reader = |path: &str| files.get(path).cloned();
        let model = load_md3_lods("models/player/head.md3", &mut reader).unwrap();
        // Only found files enter the load order.
        assert_eq!(model.load_order, vec![2, 0]);
        assert_eq!(model.num_lods, 2);
        assert_eq!(md3_at_lod(&model, 2).unwrap().name, "lod2");
        assert!(md3_at_lod(&model, 1).is_none());
        assert_eq!(md3_at_lod(&model, 0).unwrap().name, "lod0");
        // Nothing on disk resolves no model.
        let mut empty = |_path: &str| None;
        assert!(load_md3_lods("models/player/head.md3", &mut empty).is_none());
    }

    #[test]
    fn lod_aliases_after_invalid() {
        let mut files = HashMap::new();
        files.insert("models/player/head_2.md3".to_string(), model_bytes("lod2"));
        files.insert("models/player/head_1.md3".to_string(), b"IDP3".to_vec());
        let mut reader = |path: &str| files.get(path).cloned();
        let model = load_md3_lods("models/player/head.md3", &mut reader).unwrap();
        assert!(matches!(model.slots[1], Md3LodSlot::Invalid { .. }));
        assert!(matches!(model.slots[0], Md3LodSlot::Alias { source_slot: 1, .. }));
        // Aliasing an invalid slot resolves nothing.
        assert!(md3_at_lod(&model, 0).is_none());
        assert_eq!(model.num_lods, 2);
    }
}
