//! Quake III map loading (`CM_LoadMap`/`CM_ClearMap`) translated from id
//! Software's `code/qcommon/cm_load.c`.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/q3/map-loader.ts`.

use std::rc::Rc;

use super::allocation::HunkAccountingProfile;
use super::checksum::block_checksum;
use super::counters::CollisionCounters;
use super::map_resource::{CollisionBoxModel, CollisionMapData, CollisionMapResource};
use super::world::{CollisionWorld, CollisionWorldProfile};
use crate::error::WorldError;

/// Retained map file: raw bytes plus the NUL-terminated read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedCollisionFile {
    /// Raw file bytes.
    pub bytes: Vec<u8>,
    /// NUL-terminated bytes handed to the loader.
    pub terminated_bytes: Vec<u8>,
}

/// Synchronous retained file reader behind map loads.
pub trait RetainedFileReader {
    /// Read and retain a file, or `None` when it is missing.
    fn read_file_retained(&self, name: &str) -> Option<RetainedCollisionFile>;
    /// Release a retained file.
    fn free_file(&self, file: RetainedCollisionFile);
}

/// Options behind [`CollisionMapLoader`].
pub struct CollisionMapLoaderOptions {
    /// File-reader factory, consulted per load.
    pub files: Box<dyn Fn() -> Rc<dyn RetainedFileReader>>,
    /// Hunk profile factory, consulted per load.
    pub memory: Box<dyn Fn() -> HunkAccountingProfile>,
    /// Debug/profile wiring shared with the loaded world.
    pub debug: CollisionWorldProfile,
    /// Shared collision counters.
    pub counters: Rc<CollisionCounters>,
    /// Developer log sink.
    pub developer_print: Box<dyn Fn(&str)>,
}

impl std::fmt::Debug for CollisionMapLoaderOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollisionMapLoaderOptions")
            .field("debug", &self.debug)
            .field("counters", &self.counters)
            .finish_non_exhaustive()
    }
}

/// Loaded map: decoded records, world runtime, and file checksum.
#[derive(Debug, Clone)]
pub struct LoadedCollisionMap {
    /// Decoded collision records.
    pub map: Rc<CollisionMapData>,
    /// World runtime over the records.
    pub world: Rc<CollisionWorld>,
    /// Checksum of the raw file bytes.
    pub checksum: i32,
}

/// Loads (and clears) the server collision map.
#[derive(Debug)]
pub struct CollisionMapLoader {
    name: String,
    current: Option<LoadedCollisionMap>,
    box_model: CollisionBoxModel,
    options: CollisionMapLoaderOptions,
}

impl CollisionMapLoader {
    /// Retain loader options with an empty map slot.
    #[must_use]
    pub fn new(options: CollisionMapLoaderOptions) -> Self {
        Self {
            name: String::new(),
            current: None,
            box_model: CollisionBoxModel::new(),
            options,
        }
    }

    /// Load a map, borrowing the current one for same-name client loads.
    pub fn load(&mut self, name: &str, client_load: bool) -> Result<LoadedCollisionMap, WorldError> {
        if name.is_empty() {
            return Err(WorldError::BadCollisionRecord("CM_LoadMap: NULL name".to_string()));
        }
        if let CollisionWorldProfile::Shared { settings, .. } = &self.options.debug {
            settings.register_map()?;
        }
        (self.options.developer_print)(&format!("CM_LoadMap( {name}, {} )\n", i32::from(client_load)));
        if client_load && self.current.is_some() && self.name == name {
            return Ok(self.current.clone().expect("current map"));
        }
        self.clear();
        let files = (self.options.files)();
        let Some(file) = files.read_file_retained(name) else {
            return Err(WorldError::BadCollisionRecord(format!("Couldn't load {name}")));
        };
        let checksum = block_checksum(&file.bytes) as i32;
        let memory = (self.options.memory)();
        let debug = match &self.options.debug {
            CollisionWorldProfile::Shared { owner, .. } => Some(owner.clone()),
            CollisionWorldProfile::Disabled => None,
        };
        let mut resource = CollisionMapResource::new(name, memory, debug, self.box_model.clone());
        resource.load(&file.terminated_bytes)?;
        files.free_file(file);
        resource.initialize_box_hull()?;
        let map = Rc::new(resource.into_data()?);
        let world = Rc::new(CollisionWorld::new(
            map.clone(),
            self.options.debug.clone(),
            self.options.counters.clone(),
        ));
        let loaded = LoadedCollisionMap { map, world, checksum };
        if !client_load {
            // The donor slices 63 UTF-16 units; map names are ASCII, so
            // 63 scalar values keep the same observable prefix.
            self.name = name.chars().take(63).collect();
        }
        self.current = Some(loaded.clone());
        Ok(loaded)
    }

    /// Drop the loaded map and clear level patch debug.
    pub fn clear(&mut self) {
        self.name.clear();
        self.current = None;
        if let CollisionWorldProfile::Shared { owner, .. } = &self.options.debug {
            owner.clear_level_patches();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct MemoryFiles {
        files: HashMap<String, Vec<u8>>,
        freed: RefCell<Vec<String>>,
    }

    impl RetainedFileReader for MemoryFiles {
        fn read_file_retained(&self, name: &str) -> Option<RetainedCollisionFile> {
            self.files.get(name).map(|bytes| {
                let mut terminated = bytes.clone();
                terminated.push(0);
                RetainedCollisionFile {
                    bytes: bytes.clone(),
                    terminated_bytes: terminated,
                }
            })
        }

        fn free_file(&self, file: RetainedCollisionFile) {
            let name = self
                .files
                .iter()
                .find(|(_, bytes)| ***bytes == file.bytes)
                .map(|(name, _)| name.clone())
                .unwrap_or_default();
            self.freed.borrow_mut().push(name);
        }
    }

    /// Minimal IBSP46: one shader, plane, node, leaf, and model.
    fn empty_bsp() -> Vec<u8> {
        let mut lumps = vec![Vec::new(); 17];
        lumps[0] = vec![0u8];
        lumps[1] = vec![0u8; 72];
        lumps[2] = vec![0u8; 16];
        lumps[3] = vec![0u8; 36];
        lumps[4] = vec![0u8; 48];
        lumps[7] = vec![0u8; 40];
        let mut blob = vec![0u8; 144];
        blob[0..4].copy_from_slice(&0x5053_4249i32.to_le_bytes());
        blob[4..8].copy_from_slice(&46i32.to_le_bytes());
        let mut cursor = 144;
        for (index, lump) in lumps.iter().enumerate() {
            blob[8 + index * 8..12 + index * 8].copy_from_slice(&(cursor as i32).to_le_bytes());
            blob[12 + index * 8..16 + index * 8].copy_from_slice(&(lump.len() as i32).to_le_bytes());
            cursor += lump.len();
        }
        for lump in &lumps {
            blob.extend_from_slice(lump);
        }
        blob
    }

    fn loader(files: MemoryFiles, log: Rc<RefCell<Vec<String>>>) -> CollisionMapLoader {
        let files = Rc::new(files);
        CollisionMapLoader::new(CollisionMapLoaderOptions {
            files: Box::new(move || files.clone()),
            memory: Box::new(|| HunkAccountingProfile::Unaccounted),
            debug: CollisionWorldProfile::Disabled,
            counters: Rc::new(CollisionCounters::new()),
            developer_print: Box::new(move |text| log.borrow_mut().push(text.to_string())),
        })
    }

    #[test]
    fn map_loader_matches_donor() {
        let bytes = empty_bsp();
        let mut stored = HashMap::new();
        stored.insert("maps/empty.bsp".to_string(), bytes.clone());
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut loader = loader(
            MemoryFiles {
                files: stored,
                freed: RefCell::new(Vec::new()),
            },
            log.clone(),
        );
        let loaded = loader.load("maps/empty.bsp", false).expect("load");
        assert_eq!(loaded.checksum, block_checksum(&bytes) as i32);
        assert_eq!(loaded.map.models.len(), 1);
        assert!(loaded.map.box_hull.is_some());
        assert_eq!(loaded.world.model_count(), 1);
        assert_eq!(log.borrow().as_slice(), ["CM_LoadMap( maps/empty.bsp, 0 )\n"]);
        // Same-name client loads borrow the current world.
        let borrowed = loader.load("maps/empty.bsp", true).expect("borrow");
        assert_eq!(borrowed.checksum, loaded.checksum);
        assert!(Rc::ptr_eq(&borrowed.map, &loaded.map));
        assert!(Rc::ptr_eq(&borrowed.world, &loaded.world));
        assert_eq!(log.borrow().len(), 2);
        // A server load replaces the map and renames the slot.
        let long = format!("maps/{}.bsp", "q".repeat(80));
        let replaced = loader.load(&long, false).expect_err("missing file must fail");
        assert_eq!(replaced.to_string(), format!("Couldn't load {long}"));
        loader.clear();
        let error = loader.load("", false).expect_err("empty name must fail");
        assert_eq!(error.to_string(), "CM_LoadMap: NULL name");
    }

    #[test]
    fn map_loader_truncates_server_names() {
        let bytes = empty_bsp();
        let name = format!("maps/{}.bsp", "q".repeat(80));
        let mut stored = HashMap::new();
        stored.insert(name.clone(), bytes);
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut loader = loader(
            MemoryFiles {
                files: stored,
                freed: RefCell::new(Vec::new()),
            },
            log,
        );
        let first = loader.load(&name, false).expect("load");
        // The 63-unit server name never matches the full client name, so the
        // donor reloads instead of borrowing.
        let second = loader.load(&name, true).expect("reload");
        assert!(!Rc::ptr_eq(&first.world, &second.world));
        assert_eq!(first.checksum, second.checksum);
    }
}
