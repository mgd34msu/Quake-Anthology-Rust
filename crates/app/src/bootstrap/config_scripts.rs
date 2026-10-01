//! Configuration script sources.
//!
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/config-scripts.ts`
//! (`sourceScriptReader`, `consoleConfigRoot`, `seatConsoleConfig`,
//! `readConsoleScript`, `ConsoleScriptFiles`).
//! Async file and mounts access becomes sync reads. The retirement state
//! machine is preserved; sync execution never leaves reads in flight, so
//! `close` retires immediately and `write` runs its operation inline.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use qa_content::catalog::{CatalogError, InstalledCatalog};
use qa_content::contract::{create_mount_plan_id, ContentId, ContractError, MountId};
use qa_content::hash::hex_lower;
use qa_content::mounts::{MountError, MountedContent, OpenedResource, OrderedPlan, OrderedReader};
use qa_content::paths::{find_content_path, normalize_resource_path, PathComparison, PathError};
use qa_content::user_data::default_user_content_root;
use qa_core::cmd_buffer::{CommandContext, CommandOrigin};
use qa_core::identity::SeatId;
use thiserror::Error;

use crate::settings::config::ConfigStore;

/// Configuration script failure.
#[derive(Debug, Error)]
pub enum ConfigScriptError {
    /// Configuration source chain is cyclic.
    #[error("Cyclic configuration source dependency: {0}")]
    CyclicSource(String),
    /// Remote clients cannot read local configuration.
    #[error("Remote clients cannot read local configuration scripts")]
    RemoteRead,
    /// Remote clients cannot list local configuration.
    #[error("Remote clients cannot list local configuration scripts")]
    RemoteList,
    /// Remote clients cannot list mounted files.
    #[error("Remote clients cannot list local mounted files")]
    RemoteListMounted,
    /// Remote clients cannot open mounted files.
    #[error("Remote clients cannot open local mounted files")]
    RemoteOpenMounted,
    /// Reader is retired.
    #[error("Configuration reader is retired")]
    Retired,
    /// Retirement hook failure.
    #[error("Configuration retirement failed: {0}")]
    Retire(String),
    /// Filesystem failure.
    #[error("Configuration script IO failed for {path}: {message}")]
    Io {
        /// Requested path.
        path: String,
        /// Underlying error.
        message: String,
    },
    /// Catalog failure.
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    /// Mount failure.
    #[error(transparent)]
    Mount(#[from] MountError),
    /// Path failure.
    #[error(transparent)]
    Path(#[from] PathError),
    /// Contract failure.
    #[error(transparent)]
    Contract(#[from] ContractError),
}

fn io(path: impl Into<String>, error: std::io::Error) -> ConfigScriptError {
    ConfigScriptError::Io {
        path: path.into(),
        message: error.to_string(),
    }
}

fn caller(origin: &CommandOrigin) -> &CommandOrigin {
    let mut current = origin;
    while let CommandOrigin::Script { caller, .. } = current {
        current = caller;
    }
    current
}

/// Reads one source's configuration with its bases first
/// (`sourceScriptReader`).
pub struct SourceScriptReader<'a> {
    reader: OrderedReader<'a>,
    allowed: HashSet<ContentId>,
}

impl<'a> SourceScriptReader<'a> {
    /// Build a reader for a source and its base chain.
    pub fn new(
        catalog: &InstalledCatalog,
        mounts: &'a MountedContent,
        source: &ContentId,
    ) -> Result<Self, ConfigScriptError> {
        let mut owners: Vec<ContentId> = Vec::new();
        let mut product = catalog.product(source.as_str())?;
        loop {
            if owners.contains(&product.id) {
                return Err(ConfigScriptError::CyclicSource(source.as_str().to_string()));
            }
            owners.push(product.id.clone());
            match &product.expectation.base_product {
                None => break,
                Some(base) => product = catalog.product(base)?,
            }
        }
        let allowed: HashSet<ContentId> = owners.iter().cloned().collect();
        let by_mount: HashMap<&MountId, _> = mounts
            .plan
            .mounts
            .iter()
            .map(|mount| (&mount.identity().id, mount))
            .collect();
        let mut primary: Vec<MountId> = Vec::new();
        for owner in &owners {
            for id in &mounts.plan.default_order {
                if by_mount.get(id).is_some_and(|mount| mount.identity().content == *owner) {
                    primary.push(id.clone());
                }
            }
        }
        let ordered: HashSet<MountId> = primary.iter().cloned().collect();
        let mut default_order = primary;
        default_order.extend(
            mounts
                .plan
                .default_order
                .iter()
                .filter(|id| !ordered.contains(*id))
                .cloned(),
        );
        let revision = format!("{}/{}", mounts.plan.id, source.as_str());
        let reader = mounts.borrow_ordered_reader(&OrderedPlan {
            id: create_mount_plan_id("configuration", &hex_lower(revision.as_bytes()))?,
            default_order,
            prefix_orders: Vec::new(),
        })?;
        Ok(Self { reader, allowed })
    }

    /// Open one script through the source order.
    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>, ConfigScriptError> {
        Ok(self
            .reader
            .open(name, |mount| self.allowed.contains(&mount.identity().content))?
            .map(|found| found.bytes))
    }
}

/// Console configuration root (`consoleConfigRoot`).
#[must_use]
pub fn console_config_root(user_content_root: Option<&Path>) -> PathBuf {
    match user_content_root {
        Some(root) => root.join("console"),
        None => default_user_content_root().join("console"),
    }
}

/// One seat's console configuration store (`seatConsoleConfig`).
#[must_use]
pub fn seat_console_config(root: &Path, seat: &SeatId) -> ConfigStore {
    ConfigStore::new(root.join("settings").join(format!("seat-{}", seat.index())))
}

/// Legacy imported configuration roots (`LegacyConsoleConfigSources`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyConsoleConfigSources {
    /// Shared root.
    pub shared_root: PathBuf,
    /// Per-game roots.
    pub game_roots: Vec<PathBuf>,
}

/// Mounted script sources backing console script reads.
pub trait ConsoleScriptMounts {
    /// Read through the mounted fallback.
    fn read_mounted(&self, _name: &str) -> Result<Option<Vec<u8>>, MountError> {
        Ok(None)
    }
    /// Read through the mounted script override.
    fn read_mounted_script(&self, _name: &str) -> Result<Option<Vec<u8>>, MountError> {
        Ok(None)
    }
    /// List mounted files.
    fn list_mounted(&self, _directory: &str, _extension: &str) -> Result<Vec<String>, MountError> {
        Ok(Vec::new())
    }
    /// Open a mounted resource.
    fn open_mounted(&self, _path: &str) -> Result<Option<OpenedResource>, MountError> {
        Ok(None)
    }
    /// Whether a script override is installed.
    fn has_mounted_script(&self) -> bool {
        false
    }
}

/// Console script read inputs (`readConsoleScript` options).
pub struct ConsoleScriptInputs<M> {
    /// Console root.
    pub console_root: PathBuf,
    /// Product settings store.
    pub settings: ConfigStore,
    /// Mounted sources.
    pub mounts: M,
    /// Legacy imported configuration roots.
    pub legacy_config: Option<LegacyConsoleConfigSources>,
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| *byte as char).collect()
}

fn read_file_latin1(path: &Path) -> Result<String, ConfigScriptError> {
    let bytes = std::fs::read(path).map_err(|error| io(path.to_string_lossy().into_owned(), error))?;
    Ok(latin1(&bytes))
}

fn legacy_config_path(sources: &LegacyConsoleConfigSources) -> Result<Option<PathBuf>, ConfigScriptError> {
    if let Some(shared) = find_content_path(&sources.shared_root, "config.cfg", PathComparison::CaseInsensitive)? {
        if std::fs::metadata(&shared)
            .map_err(|error| io(shared.to_string_lossy().into_owned(), error))?
            .is_file()
        {
            return Ok(Some(shared));
        }
    }
    let mut newest: Option<(PathBuf, std::time::SystemTime)> = None;
    for root in &sources.game_roots {
        let Some(path) = find_content_path(root, "config.cfg", PathComparison::CaseInsensitive)? else {
            continue;
        };
        let entry = std::fs::metadata(&path).map_err(|error| io(path.to_string_lossy().into_owned(), error))?;
        if !entry.is_file() {
            continue;
        }
        let modified = entry
            .modified()
            .map_err(|error| io(path.to_string_lossy().into_owned(), error))?;
        let replace = newest.as_ref().is_none_or(|(_, seen)| modified > *seen);
        if replace {
            newest = Some((path, modified));
        }
    }
    Ok(newest.map(|(path, _)| path))
}

/// Read the invoking seat's exported config, then its product's user files,
/// then mounted content (`readConsoleScript`).
pub fn read_console_script<M: ConsoleScriptMounts>(
    inputs: &ConsoleScriptInputs<M>,
    name: &str,
    source: &CommandContext,
) -> Result<Option<String>, ConfigScriptError> {
    let name = normalize_resource_path(name)?;
    let origin = caller(&source.origin);
    if matches!(origin, CommandOrigin::RemoteClient { .. }) {
        return Err(ConfigScriptError::RemoteRead);
    }
    if let CommandOrigin::LocalSeat { seat, .. } = origin {
        let within = format!("settings/seat-{}/{}", seat.index(), name);
        if let Some(path) = find_content_path(&inputs.console_root, &within, PathComparison::CaseInsensitive)? {
            return Ok(Some(read_file_latin1(&path)?));
        }
    }
    if name.to_lowercase() == "config.cfg" {
        if let Some(sources) = inputs.legacy_config.as_ref() {
            if let Some(legacy) = legacy_config_path(sources)? {
                return Ok(Some(read_file_latin1(&legacy)?));
            }
        }
    }
    if let Some(path) = find_content_path(&inputs.settings.root, &name, PathComparison::CaseInsensitive)? {
        return Ok(Some(read_file_latin1(&path)?));
    }
    let bytes = if inputs.mounts.has_mounted_script() {
        inputs.mounts.read_mounted_script(&name)?
    } else {
        inputs.mounts.read_mounted(&name)?
    };
    Ok(bytes.as_deref().map(latin1))
}

/// One listed console script (`ConsoleScriptEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleScriptEntry {
    /// Script name.
    pub name: String,
    /// Seat or product source.
    pub kind: ConsoleScriptKind,
}

/// Console script source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleScriptKind {
    /// Invoking seat's exported config.
    Seat,
    /// Product user files.
    Product,
}

fn config_names(root: &Path, prefix: &str) -> Result<Vec<String>, ConfigScriptError> {
    let directory = if prefix.is_empty() {
        root.to_path_buf()
    } else {
        root.join(prefix)
    };
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io(directory.to_string_lossy().into_owned(), error)),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| io(directory.to_string_lossy().into_owned(), error))?;
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let name = if prefix.is_empty() {
            file_name
        } else {
            format!("{prefix}/{file_name}")
        };
        let kind = entry
            .file_type()
            .map_err(|error| io(directory.to_string_lossy().into_owned(), error))?;
        if kind.is_dir() {
            names.extend(config_names(root, &name)?);
        } else if kind.is_file() && name.to_lowercase().ends_with(".cfg") {
            names.push(name);
        }
    }
    Ok(names)
}

/// Retirement hook for mounted script sources.
pub type RetireMounted = Box<dyn FnMut() -> Result<(), String>>;

/// Guarded console script reader (`ConsoleScriptFiles`).
pub struct ConsoleScriptFiles<M> {
    inputs: ConsoleScriptInputs<M>,
    retire_mounted: Option<RetireMounted>,
    retired: bool,
}

impl<M: ConsoleScriptMounts> ConsoleScriptFiles<M> {
    /// Build a guarded reader.
    pub fn new(inputs: ConsoleScriptInputs<M>, retire_mounted: Option<RetireMounted>) -> Self {
        Self {
            inputs,
            retire_mounted,
            retired: false,
        }
    }

    /// Product settings root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.inputs.settings.root
    }

    fn check_live(&self) -> Result<(), ConfigScriptError> {
        if self.retired {
            return Err(ConfigScriptError::Retired);
        }
        Ok(())
    }

    /// List product scripts plus the invoking seat's scripts (`list`).
    pub fn list(&self, source: &CommandContext) -> Result<Vec<ConsoleScriptEntry>, ConfigScriptError> {
        self.check_live()?;
        let origin = caller(&source.origin);
        if matches!(origin, CommandOrigin::RemoteClient { .. }) {
            return Err(ConfigScriptError::RemoteList);
        }
        let mut entries: HashMap<String, ConsoleScriptEntry> = HashMap::new();
        for name in config_names(&self.inputs.settings.root, "")? {
            entries.insert(
                name.to_lowercase(),
                ConsoleScriptEntry {
                    name,
                    kind: ConsoleScriptKind::Product,
                },
            );
        }
        if let CommandOrigin::LocalSeat { seat, .. } = origin {
            let seat_root = seat_console_config(&self.inputs.console_root, seat).root;
            for name in config_names(&seat_root, "")? {
                entries.insert(
                    name.to_lowercase(),
                    ConsoleScriptEntry {
                        name,
                        kind: ConsoleScriptKind::Seat,
                    },
                );
            }
        }
        let mut listed: Vec<ConsoleScriptEntry> = entries.into_values().collect();
        listed.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(listed)
    }

    /// Read one script (`read`).
    pub fn read(&self, name: &str, source: &CommandContext) -> Result<Option<String>, ConfigScriptError> {
        self.check_live()?;
        read_console_script(&self.inputs, name, source)
    }

    /// Read through the mounted script override (`readMountedScript`).
    pub fn read_mounted_script(&self, name: &str) -> Result<Option<Vec<u8>>, ConfigScriptError> {
        self.check_live()?;
        Ok(if self.inputs.mounts.has_mounted_script() {
            self.inputs.mounts.read_mounted_script(name)?
        } else {
            self.inputs.mounts.read_mounted(name)?
        })
    }

    /// Read through the mounted fallback (`readMounted`).
    pub fn read_mounted(&self, name: &str) -> Result<Option<Vec<u8>>, ConfigScriptError> {
        self.check_live()?;
        Ok(self.inputs.mounts.read_mounted(name)?)
    }

    /// List mounted files (`listMounted`).
    pub fn list_mounted(
        &self,
        directory: &str,
        extension: &str,
        source: &CommandContext,
    ) -> Result<Vec<String>, ConfigScriptError> {
        self.check_live()?;
        if matches!(caller(&source.origin), CommandOrigin::RemoteClient { .. }) {
            return Err(ConfigScriptError::RemoteListMounted);
        }
        Ok(self.inputs.mounts.list_mounted(directory, extension)?)
    }

    /// Open a mounted resource (`openMounted`).
    pub fn open_mounted(
        &self,
        path: &str,
        source: &CommandContext,
    ) -> Result<Option<OpenedResource>, ConfigScriptError> {
        self.check_live()?;
        if matches!(caller(&source.origin), CommandOrigin::RemoteClient { .. }) {
            return Err(ConfigScriptError::RemoteOpenMounted);
        }
        Ok(self.inputs.mounts.open_mounted(path)?)
    }

    /// Run one serialized write inline (`write`).
    pub fn write(&self, operation: impl FnOnce() -> Result<(), ConfigScriptError>) -> Result<(), ConfigScriptError> {
        self.check_live()?;
        operation()
    }

    /// Retire the reader and its mounted sources (`close`).
    pub fn close(&mut self) -> Result<(), ConfigScriptError> {
        self.retired = true;
        if let Some(retire) = self.retire_mounted.as_mut() {
            retire().map_err(ConfigScriptError::Retire)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct StubMounts {
        files: HashMap<String, Vec<u8>>,
        script_override: bool,
    }

    impl ConsoleScriptMounts for StubMounts {
        fn read_mounted(&self, name: &str) -> Result<Option<Vec<u8>>, MountError> {
            Ok(self.files.get(name).cloned())
        }

        fn read_mounted_script(&self, name: &str) -> Result<Option<Vec<u8>>, MountError> {
            Ok(self.files.get(&format!("script:{name}")).cloned())
        }

        fn list_mounted(&self, _directory: &str, _extension: &str) -> Result<Vec<String>, MountError> {
            Ok(vec!["mounted.cfg".to_string()])
        }

        fn has_mounted_script(&self) -> bool {
            self.script_override
        }
    }

    fn scratch() -> PathBuf {
        std::env::temp_dir().join(format!(
            "qa-config-scripts-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn seat_context() -> (CommandContext, SeatId, IdentityOwner) {
        let owner = IdentityOwner::create("config-scripts").unwrap();
        let seat = owner.seat(0);
        let client = owner.client(0, 0);
        let context = CommandContext::new(
            owner.session().clone(),
            CommandOrigin::LocalSeat {
                seat: seat.clone(),
                client,
            },
        );
        (context, seat, owner)
    }

    fn inputs(root: &Path, mounts: StubMounts) -> ConsoleScriptInputs<StubMounts> {
        ConsoleScriptInputs {
            console_root: root.join("console"),
            settings: ConfigStore::new(root.join("product")),
            mounts,
            legacy_config: None,
        }
    }

    #[test]
    fn seat_product_and_mounted_reads_follow_precedence() {
        let root = scratch();
        let (source, seat, _) = seat_context();
        let seat_dir = root
            .join("console")
            .join("settings")
            .join(format!("seat-{}", seat.index()));
        std::fs::create_dir_all(&seat_dir).unwrap();
        std::fs::write(seat_dir.join("autoexec.cfg"), b"seat").unwrap();
        std::fs::create_dir_all(root.join("product")).unwrap();
        std::fs::write(root.join("product").join("autoexec.cfg"), b"product").unwrap();
        std::fs::write(root.join("product").join("user.cfg"), b"user").unwrap();
        let mut files = HashMap::new();
        files.insert("mounted.cfg".to_string(), b"mounted".to_vec());
        let reader = ConsoleScriptFiles::new(
            inputs(
                &root,
                StubMounts {
                    files,
                    script_override: false,
                },
            ),
            None,
        );
        assert_eq!(reader.read("autoexec.cfg", &source).unwrap().as_deref(), Some("seat"));
        assert_eq!(reader.read("user.cfg", &source).unwrap().as_deref(), Some("user"));
        assert_eq!(reader.read("mounted.cfg", &source).unwrap().as_deref(), Some("mounted"));
        assert_eq!(reader.read("missing.cfg", &source).unwrap(), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn script_origins_unwrap_and_remote_clients_are_rejected() {
        let root = scratch();
        let (inner, _, owner) = seat_context();
        let scripted = CommandContext::new(
            inner.session.clone(),
            CommandOrigin::Script {
                name: "run".to_string(),
                caller: Box::new(inner.origin.clone()),
            },
        );
        let reader = ConsoleScriptFiles::new(
            inputs(
                &root,
                StubMounts {
                    files: HashMap::new(),
                    script_override: false,
                },
            ),
            None,
        );
        assert_eq!(reader.read("missing.cfg", &scripted).unwrap(), None);
        let remote = CommandContext::new(
            inner.session.clone(),
            CommandOrigin::RemoteClient {
                client: owner.client(1, 0),
            },
        );
        assert!(matches!(
            reader.read("autoexec.cfg", &remote).unwrap_err(),
            ConfigScriptError::RemoteRead
        ));
        assert!(matches!(
            reader.list(&remote).unwrap_err(),
            ConfigScriptError::RemoteList
        ));
        assert!(matches!(
            reader.list_mounted("scripts", ".cfg", &remote).unwrap_err(),
            ConfigScriptError::RemoteListMounted
        ));
        assert!(matches!(
            reader.open_mounted("scripts/x.cfg", &remote).unwrap_err(),
            ConfigScriptError::RemoteOpenMounted
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn list_merges_seat_over_product_sorted() {
        let root = scratch();
        let (source, seat, _) = seat_context();
        let seat_dir = root
            .join("console")
            .join("settings")
            .join(format!("seat-{}", seat.index()));
        std::fs::create_dir_all(&seat_dir).unwrap();
        std::fs::write(seat_dir.join("autoexec.cfg"), b"seat").unwrap();
        std::fs::create_dir_all(root.join("product")).unwrap();
        std::fs::write(root.join("product").join("autoexec.cfg"), b"product").unwrap();
        std::fs::write(root.join("product").join("zebra.cfg"), b"z").unwrap();
        std::fs::write(root.join("product").join("notes.txt"), b"ignored").unwrap();
        let reader = ConsoleScriptFiles::new(
            inputs(
                &root,
                StubMounts {
                    files: HashMap::new(),
                    script_override: false,
                },
            ),
            None,
        );
        assert_eq!(
            reader.list(&source).unwrap(),
            vec![
                ConsoleScriptEntry {
                    name: "autoexec.cfg".to_string(),
                    kind: ConsoleScriptKind::Seat
                },
                ConsoleScriptEntry {
                    name: "zebra.cfg".to_string(),
                    kind: ConsoleScriptKind::Product
                },
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn close_retires_reads_writes_and_lists() {
        let root = scratch();
        let (source, _, _) = seat_context();
        let mut reader = ConsoleScriptFiles::new(
            inputs(
                &root,
                StubMounts {
                    files: HashMap::new(),
                    script_override: false,
                },
            ),
            Some(Box::new(|| Ok(()))),
        );
        reader.write(|| Ok(())).unwrap();
        reader.close().unwrap();
        assert!(matches!(
            reader.read("autoexec.cfg", &source).unwrap_err(),
            ConfigScriptError::Retired
        ));
        assert!(matches!(
            reader.write(|| Ok(())).unwrap_err(),
            ConfigScriptError::Retired
        ));
        assert!(matches!(reader.list(&source).unwrap_err(), ConfigScriptError::Retired));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn legacy_config_cfg_falls_back_to_newest_game_root() {
        let root = scratch();
        let (source, _, _) = seat_context();
        let old_game = root.join("old-game");
        let new_game = root.join("new-game");
        std::fs::create_dir_all(&old_game).unwrap();
        std::fs::create_dir_all(&new_game).unwrap();
        std::fs::write(old_game.join("config.cfg"), b"old").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(new_game.join("config.cfg"), b"new").unwrap();
        let inputs = ConsoleScriptInputs {
            legacy_config: Some(LegacyConsoleConfigSources {
                shared_root: root.join("missing-shared"),
                game_roots: vec![old_game, new_game],
            }),
            ..inputs(
                &root,
                StubMounts {
                    files: HashMap::new(),
                    script_override: false,
                },
            )
        };
        let reader = ConsoleScriptFiles::new(inputs, None);
        assert_eq!(reader.read("config.cfg", &source).unwrap().as_deref(), Some("new"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
