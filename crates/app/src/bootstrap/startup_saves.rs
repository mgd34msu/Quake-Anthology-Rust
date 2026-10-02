//! Saved-game browser list over the save directory.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/startup-saves.ts`
//! (`StartupSaveRow`, `StartupSaveList`, `StartupSaves`). Files are inspected
//! on browser open/refresh; drawing only reads the retained list. Sync port
//! using `std::fs` (the donor mixes `node:fs` and `Bun.file`). Save decoding
//! reuses [`crate::persistence::saved_game`]; catalog product resolution and
//! shared-settings validation (unported siblings) arrive through
//! [`StartupSaveCatalog`].

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use crate::persistence::image::UnifiedSaveImage;
use crate::persistence::native_weapon::NativeWeaponBehaviorDeclaration;
use crate::persistence::native_weapon::WeaponBehaviorDefinition;
use crate::persistence::q1::source_text::Q1SaveData;
use crate::persistence::saved_game::read_saved_game;
use crate::persistence::saved_game::SavedGame;

/// One saved-game row.
#[derive(Debug, Clone, PartialEq)]
pub struct StartupSaveRow {
    /// Relative id under the directory.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Map name.
    pub map: String,
    /// Game product text.
    pub game: String,
    /// Modification time in milliseconds.
    pub saved_at_milliseconds: f64,
    /// Unavailability reason, when the slot cannot load.
    pub unavailable: Option<String>,
}

/// Retained browser list.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StartupSaveList {
    /// Rows newest first.
    pub rows: Vec<StartupSaveRow>,
    /// Scan-level error, when some files could not be read.
    pub error: Option<String>,
}

/// Resolved save product (donor catalog `product` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupSaveProduct {
    /// Product title.
    pub title: String,
    /// Product edition.
    pub edition: String,
    /// Whether the content is installed.
    pub installed: bool,
}

impl StartupSaveProduct {
    /// Donor `game` text (`"{title} ({edition})"`).
    #[must_use]
    pub fn game_text(&self) -> String {
        format!("{} ({})", self.title, self.edition)
    }
}

/// Catalog product resolution plus shared-settings validation.
pub trait StartupSaveCatalog {
    /// Resolve the product for shared-save content.
    fn shared_product(&self, content: &str) -> Result<StartupSaveProduct, String>;
    /// Select the product for a Q1 source save.
    fn q1_product(&self, data: &Q1SaveData, path: &str) -> Result<StartupSaveProduct, String>;
    /// Validate shared-save simulation settings (donor `savedSimulationSettings`).
    fn check_shared_settings(&self, image: &UnifiedSaveImage) -> Result<(), String>;
}

fn save_label(file_stem: &str) -> String {
    if file_stem.to_lowercase() == "autosave" {
        return "Autosave".to_string();
    }
    if file_stem.to_lowercase() == "quicksave" {
        return "Quicksave".to_string();
    }
    file_stem.replace('_', " ")
}

fn shared_map_name(requested_path: &str) -> String {
    requested_path
        .strip_prefix("maps/")
        .unwrap_or(requested_path)
        .strip_suffix(".bsp")
        .unwrap_or(requested_path.strip_prefix("maps/").unwrap_or(requested_path))
        .to_string()
}

fn valid_slot_label(label: &str) -> bool {
    let count = label.chars().count();
    (1..=48).contains(&count)
        && label
            .chars()
            .all(|char| char.is_alphabetic() || char.is_numeric() || matches!(char, ' ' | '_' | '-'))
}

fn scan_saves(directory: &Path, files: &mut Vec<PathBuf>, error: &mut Option<String>) {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(io_error) if io_error.kind() == std::io::ErrorKind::NotFound => return,
        Err(_) => {
            *error = Some("Some saved games could not be read. Check access and refresh.".to_string());
            return;
        }
    };
    for entry in entries {
        let Ok(entry) = entry else {
            *error = Some("Some saved games could not be read. Check access and refresh.".to_string());
            continue;
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => {
                *error = Some("Some saved games could not be read. Check access and refresh.".to_string());
                continue;
            }
        };
        if file_type.is_dir() {
            scan_saves(&path, files, error);
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.to_string_lossy().to_lowercase() == "sav")
        {
            files.push(path);
        }
    }
}

/// Saved-game browser over a directory.
#[derive(Debug)]
pub struct StartupSaves<Catalog> {
    catalog: Catalog,
    /// Save directory.
    pub directory: PathBuf,
    current: StartupSaveList,
    paths: HashMap<String, PathBuf>,
}

impl<Catalog: StartupSaveCatalog> StartupSaves<Catalog> {
    /// Open the browser on a directory.
    #[must_use]
    pub fn new(catalog: Catalog, directory: PathBuf) -> Self {
        Self {
            catalog,
            directory,
            current: StartupSaveList::default(),
            paths: HashMap::new(),
        }
    }

    /// Retained list.
    #[must_use]
    pub fn list(&self) -> &StartupSaveList {
        &self.current
    }

    /// Resolve a listed id to its path.
    pub fn path(&self, id: &str) -> Result<&Path, String> {
        let row = self.current.rows.iter().find(|row| row.id == id);
        let path = self.paths.get(id);
        match (row, path) {
            (Some(row), Some(path)) => {
                if let Some(unavailable) = &row.unavailable {
                    return Err(unavailable.clone());
                }
                Ok(path)
            }
            _ => Err("This saved game is no longer listed. Refresh the saved games.".to_string()),
        }
    }

    /// Resolve a new slot name, creating the directory.
    pub fn named_path(&self, name: &str) -> Result<PathBuf, String> {
        let label = name.trim();
        if !valid_slot_label(label) {
            return Err("Use 1-48 letters, numbers, spaces, - or _.".to_string());
        }
        std::fs::create_dir_all(&self.directory)
            .map_err(|_| "Some saved games could not be read. Check access and refresh.".to_string())?;
        let path = self.directory.join(format!("{label}.sav"));
        if path.exists() {
            return Err("Name exists. Select its slot to overwrite.".to_string());
        }
        Ok(path)
    }

    /// Refresh the retained list from disk.
    pub fn refresh(
        &mut self,
        legacy_native: &dyn Fn(&WeaponBehaviorDefinition) -> Option<NativeWeaponBehaviorDeclaration>,
    ) {
        let mut files = Vec::new();
        let mut error = None;
        scan_saves(&self.directory, &mut files, &mut error);
        let mut rows = Vec::new();
        let mut paths = HashMap::new();
        for path in files {
            let id = path
                .strip_prefix(&self.directory)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let stem = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_default();
            let label = save_label(&stem);
            let mut saved_at_milliseconds = 0.0;
            let mut map = "Unknown map".to_string();
            let mut game = "Unknown game".to_string();
            let mut unavailable = None;
            match self.inspect(&path, legacy_native) {
                Ok((row_map, row_game, mtime, product_installed, settings_error)) => {
                    saved_at_milliseconds = mtime;
                    map = row_map;
                    game = row_game;
                    if let Some(message) = settings_error {
                        unavailable = Some(message);
                    } else if !product_installed {
                        unavailable = Some("Required game content is not installed.".to_string());
                    }
                }
                Err(message) => unavailable = Some(message),
            }
            paths.insert(id.clone(), path);
            rows.push(StartupSaveRow {
                id,
                label,
                map,
                game,
                saved_at_milliseconds,
                unavailable,
            });
        }
        rows.sort_by(|left, right| {
            right
                .saved_at_milliseconds
                .total_cmp(&left.saved_at_milliseconds)
                .then_with(|| left.id.cmp(&right.id))
        });
        self.paths = paths;
        self.current = StartupSaveList { rows, error };
    }

    fn inspect(
        &self,
        path: &Path,
        legacy_native: &dyn Fn(&WeaponBehaviorDefinition) -> Option<NativeWeaponBehaviorDeclaration>,
    ) -> Result<(String, String, f64, bool, Option<String>), String> {
        let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0);
        let text = path.to_string_lossy().to_string();
        let save = read_saved_game(&text, legacy_native).map_err(|error| error.to_string())?;
        match save {
            SavedGame::Shared(image) => {
                let product = self.catalog.shared_product(&image.recipe.map.entities.content)?;
                if let Err(message) = self.catalog.check_shared_settings(&image) {
                    return Ok((String::new(), String::new(), mtime, true, Some(message)));
                }
                Ok((
                    shared_map_name(&image.recipe.map.geometry.requested_path),
                    product.game_text(),
                    mtime,
                    product.installed,
                    None,
                ))
            }
            SavedGame::Q1Source(data) => {
                let product = self.catalog.q1_product(&data, &text)?;
                Ok((data.map.clone(), product.game_text(), mtime, product.installed, None))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::persistence::q1::source_text::encode_q1_save;
    use crate::persistence::q1::source_text::Q1SaveFormat;
    use crate::persistence::q1::source_text::QcTextPair;

    use super::*;

    struct FakeCatalog {
        installed: bool,
    }

    impl StartupSaveCatalog for FakeCatalog {
        fn shared_product(&self, content: &str) -> Result<StartupSaveProduct, String> {
            Ok(StartupSaveProduct {
                title: content.to_string(),
                edition: "shared".to_string(),
                installed: self.installed,
            })
        }

        fn q1_product(&self, _data: &Q1SaveData, _path: &str) -> Result<StartupSaveProduct, String> {
            Ok(StartupSaveProduct {
                title: "Quake".to_string(),
                edition: "id1".to_string(),
                installed: self.installed,
            })
        }

        fn check_shared_settings(&self, _image: &UnifiedSaveImage) -> Result<(), String> {
            Ok(())
        }
    }

    fn save_data(map: &str) -> Q1SaveData {
        Q1SaveData {
            format: Q1SaveFormat::V5,
            comment: "test".to_string(),
            spawn_parameters: vec![0.0; 16],
            skill: 1,
            map: map.to_string(),
            time: 12.5,
            light_styles: vec![String::new(); 64],
            globals: Vec::new(),
            entities: vec![
                vec![QcTextPair {
                    key: "classname".to_string(),
                    value: "worldspawn".to_string(),
                }],
                vec![QcTextPair {
                    key: "classname".to_string(),
                    value: "info_player_start".to_string(),
                }],
            ],
            extension_text: String::new(),
        }
    }

    fn directory(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("qa-startup-saves-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn no_native(_: &WeaponBehaviorDefinition) -> Option<NativeWeaponBehaviorDeclaration> {
        None
    }

    #[test]
    fn refresh_lists_saves_newest_first() {
        let root = directory("list");
        std::fs::write(root.join("quicksave.sav"), encode_q1_save(&save_data("e1m1")).unwrap()).unwrap();
        std::fs::write(root.join("my_slot.sav"), encode_q1_save(&save_data("e1m2")).unwrap()).unwrap();
        std::fs::write(root.join("broken.sav"), b"not a save").unwrap();
        std::fs::write(root.join("notes.txt"), b"ignored").unwrap();
        let nested = root.join("sub");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("deep.SAV"), encode_q1_save(&save_data("e1m3")).unwrap()).unwrap();

        let mut saves = StartupSaves::new(FakeCatalog { installed: true }, root.clone());
        saves.refresh(&no_native);
        let list = saves.list();
        assert!(list.error.is_none());
        assert_eq!(list.rows.len(), 4);
        let by_id: HashMap<&str, &StartupSaveRow> = list.rows.iter().map(|row| (row.id.as_str(), row)).collect();
        assert_eq!(by_id["quicksave.sav"].label, "Quicksave");
        assert_eq!(by_id["quicksave.sav"].map, "e1m1");
        assert_eq!(by_id["quicksave.sav"].game, "Quake (id1)");
        assert_eq!(by_id["my_slot.sav"].label, "my slot");
        assert!(by_id["broken.sav"].unavailable.is_some());
        assert_eq!(by_id["sub/deep.SAV"].map, "e1m3");
        for window in list.rows.windows(2) {
            assert!(window[0].saved_at_milliseconds >= window[1].saved_at_milliseconds);
        }
        assert!(saves.path("quicksave.sav").is_ok());
        assert_eq!(
            saves.path("broken.sav"),
            Err(by_id["broken.sav"].unavailable.clone().unwrap())
        );
        assert_eq!(
            saves.path("missing.sav"),
            Err("This saved game is no longer listed. Refresh the saved games.".to_string())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn refresh_marks_missing_content() {
        let root = directory("content");
        std::fs::write(root.join("slot.sav"), encode_q1_save(&save_data("e1m1")).unwrap()).unwrap();
        let mut saves = StartupSaves::new(FakeCatalog { installed: false }, root.clone());
        saves.refresh(&no_native);
        assert_eq!(
            saves.list().rows[0].unavailable.as_deref(),
            Some("Required game content is not installed.")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn refresh_tolerates_missing_directory() {
        let root = std::env::temp_dir().join(format!("qa-startup-saves-absent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut saves = StartupSaves::new(FakeCatalog { installed: true }, root);
        saves.refresh(&no_native);
        assert!(saves.list().rows.is_empty());
        assert!(saves.list().error.is_none());
    }

    #[test]
    fn named_path_validates_and_detects_collisions() {
        let root = directory("named");
        let saves = StartupSaves::new(FakeCatalog { installed: true }, root.clone());
        let path = saves.named_path("  My Slot-1 ").unwrap();
        assert_eq!(path, root.join("My Slot-1.sav"));
        assert_eq!(
            saves.named_path("bad/name"),
            Err("Use 1-48 letters, numbers, spaces, - or _.".to_string())
        );
        assert_eq!(
            saves.named_path("   "),
            Err("Use 1-48 letters, numbers, spaces, - or _.".to_string())
        );
        std::fs::write(root.join("Taken.sav"), b"x").unwrap();
        assert_eq!(
            saves.named_path("Taken"),
            Err("Name exists. Select its slot to overwrite.".to_string())
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
