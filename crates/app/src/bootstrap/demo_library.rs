//! Demo library over user recordings and mounted demos.
//!
//! Donor provenance: `src/app/bootstrap/demo-library.ts` (`DemoLibrary`).
//!
//! Sync port: the donor's async mount calls and `readdir`/`readFile` become
//! the sync [`DemoLibraryMounts`] trait and `std::fs`. Missing files and
//! directories (`ENOENT`/`ENOTDIR`) still fall through exactly as
//! `isMissingFile` does. Sorting uses byte order rather than
//! `localeCompare`; demo ids are ASCII resource paths, where the two agree.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::PathBuf;

use qa_client::ui::library::menu::{LibraryEntry, LibraryMenuService};
use qa_content::paths::{normalize_resource_path, PathError};
use thiserror::Error;

use crate::bootstrap::demo_playback::{demo_family, DemoFamily};

/// Failure of a demo-library operation.
#[derive(Debug, Error)]
pub enum DemoLibraryError {
    /// A mount call failed.
    #[error("demo mount failed: {0}")]
    Mount(String),
    /// A resource path was invalid.
    #[error(transparent)]
    Path(#[from] PathError),
    /// A filesystem operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Mounted-demo source (donor `DemoLibraryMounts`).
pub trait DemoLibraryMounts {
    /// Mount failure type.
    type Error: std::fmt::Display;

    /// List mounted files with an extension under a directory.
    fn list_files(&self, directory: &str, extension: &str) -> Result<Vec<String>, Self::Error>;

    /// Read a mounted file, or [`None`] when absent.
    fn read(&self, path: &str) -> Result<Option<Vec<u8>>, Self::Error>;
}

/// One demo entry (donor `DemoLibraryEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoLibraryEntry {
    /// Normalized resource name.
    pub id: String,
    /// Display label (same as the id).
    pub label: String,
    /// Demo family.
    pub family: DemoFamily,
}

/// User recordings plus mounted demos under one root.
pub struct DemoLibrary<M> {
    /// Recordings root.
    pub root: PathBuf,
    mounts: M,
}

impl<M: DemoLibraryMounts> DemoLibrary<M> {
    /// Create a library over a root and mounts.
    pub fn new(root: PathBuf, mounts: M) -> Self {
        Self { root, mounts }
    }

    /// List every demo, deduplicated case-insensitively and sorted.
    pub fn list(&self) -> Result<Vec<DemoLibraryEntry>, DemoLibraryError> {
        let mut names: HashMap<String, String> = HashMap::new();
        let mut add = |path: &str| -> Result<(), DemoLibraryError> {
            if !is_demo_path(path) {
                return Ok(());
            }
            let normalized = normalize_resource_path(path)?;
            names.insert(normalized.to_lowercase(), normalized);
            Ok(())
        };
        for (directory, extension) in [
            ("", ".dem"),
            ("", ".qwd"),
            ("demos", ".dm2"),
            ("demos", ".mvd"),
            ("demos", ".dm_66"),
            ("demos", ".dm_67"),
            ("demos", ".dm_68"),
        ] {
            let files = self
                .mounts
                .list_files(directory, extension)
                .map_err(|error| DemoLibraryError::Mount(error.to_string()))?;
            for name in &files {
                let path = if directory.is_empty() {
                    name.clone()
                } else {
                    format!("{directory}/{name}")
                };
                add(&path)?;
            }
        }
        self.scan("", &mut add)?;
        let mut ids: Vec<String> = names.into_values().collect();
        ids.sort();
        Ok(ids
            .into_iter()
            .map(|id| DemoLibraryEntry {
                label: id.clone(),
                family: demo_family(&id, DemoFamily::Q1),
                id,
            })
            .collect())
    }

    /// Read a demo, preferring the recordings root over mounts.
    pub fn read(&self, path: &str) -> Result<Option<Vec<u8>>, DemoLibraryError> {
        let normalized = normalize_resource_path(path)?;
        match std::fs::read(self.root.join(&normalized)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == ErrorKind::NotFound || error.kind() == ErrorKind::NotADirectory => self
                .mounts
                .read(&normalized)
                .map_err(|error| DemoLibraryError::Mount(error.to_string())),
            Err(error) => Err(error.into()),
        }
    }

    fn scan(
        &self,
        directory: &str,
        add: &mut dyn FnMut(&str) -> Result<(), DemoLibraryError>,
    ) -> Result<(), DemoLibraryError> {
        let dir = if directory.is_empty() {
            self.root.clone()
        } else {
            self.root.join(directory)
        };
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound || error.kind() == ErrorKind::NotADirectory => {
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = if directory.is_empty() {
                name
            } else {
                format!("{directory}/{name}")
            };
            if file_type.is_dir() {
                self.scan(&path, add)?;
            } else if file_type.is_file() {
                add(&path)?;
            }
        }
        Ok(())
    }
}

fn is_demo_path(path: &str) -> bool {
    let Some(dot) = path.rfind('.') else {
        return false;
    };
    let extension = path[dot + 1..].to_lowercase();
    match extension.as_str() {
        "dem" | "qwd" | "dm2" | "mvd" => true,
        _ => {
            extension.len() > 3
                && extension.starts_with("dm_")
                && extension[3..].bytes().all(|byte| byte.is_ascii_digit())
        }
    }
}

/// Demo library menu service (donor `demoLibraryMenu`): browse user
/// recordings and mounted demos; play, record, and stop run through
/// injected command sinks. Sync port: refresh lists inline, so no
/// generation guard is needed; sink failures surface as status text. The
/// recording reader reports the in-progress recording path (donor status
/// override while recording).
pub struct DemoMenuService<M, P, R, S, T> {
    library: DemoLibrary<M>,
    entries: Vec<LibraryEntry>,
    status: String,
    play: P,
    record: R,
    stop: S,
    recording: T,
}

impl<M, P, R, S, T> DemoMenuService<M, P, R, S, T>
where
    M: DemoLibraryMounts,
    P: FnMut(&str) -> Result<(), String>,
    R: FnMut(&str) -> Result<(), String>,
    S: FnMut() -> Result<(), String>,
    T: Fn() -> Option<String>,
{
    /// Build the service over a demo library, command sinks, and the
    /// in-progress recording path reader.
    pub fn new(library: DemoLibrary<M>, play: P, record: R, stop: S, recording: T) -> Self {
        Self {
            library,
            entries: Vec::new(),
            status: String::new(),
            play,
            record,
            stop,
            recording,
        }
    }

    /// Family detail tag (donor `family.toUpperCase()`).
    fn family_detail(family: DemoFamily) -> String {
        match family {
            DemoFamily::Q1 => "Q1",
            DemoFamily::Qw => "QW",
            DemoFamily::Q2 => "Q2",
            DemoFamily::Q3 => "Q3",
        }
        .to_string()
    }
}

impl<M, P, R, S, T> LibraryMenuService for DemoMenuService<M, P, R, S, T>
where
    M: DemoLibraryMounts,
    P: FnMut(&str) -> Result<(), String>,
    R: FnMut(&str) -> Result<(), String>,
    S: FnMut() -> Result<(), String>,
    T: Fn() -> Option<String>,
{
    fn entries(&self) -> Vec<LibraryEntry> {
        self.entries.clone()
    }

    fn status(&self) -> String {
        (self.recording)().unwrap_or_else(|| self.status.clone())
    }

    fn refresh(&mut self) {
        match self.library.list() {
            Ok(list) => {
                self.entries = list
                    .into_iter()
                    .map(|entry| LibraryEntry {
                        id: entry.id.clone(),
                        label: entry.label,
                        detail: Some(Self::family_detail(entry.family)),
                        unavailable: None,
                    })
                    .collect();
                self.status = format!("{} demos", self.entries.len());
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn activate(&mut self, id: &str) {
        if !self.entries.iter().any(|entry| entry.id == id) {
            return;
        }
        if let Err(error) = (self.play)(id) {
            self.status = error;
        }
    }

    fn create_label(&self) -> Option<String> {
        Some("Record".to_string())
    }

    fn create_submit(&mut self, name: &str) {
        if let Err(error) = (self.record)(name) {
            self.status = error;
        }
    }

    fn stop_label(&self) -> Option<String> {
        Some("Stop recording".to_string())
    }

    fn stop_activate(&mut self) {
        if let Err(error) = (self.stop)() {
            self.status = error;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FakeMounts {
        files: HashMap<(String, String), Vec<String>>,
        reads: HashMap<String, Vec<u8>>,
    }

    impl DemoLibraryMounts for FakeMounts {
        type Error = String;

        fn list_files(&self, directory: &str, extension: &str) -> Result<Vec<String>, Self::Error> {
            Ok(self
                .files
                .get(&(directory.to_string(), extension.to_string()))
                .cloned()
                .unwrap_or_default())
        }

        fn read(&self, path: &str) -> Result<Option<Vec<u8>>, Self::Error> {
            Ok(self.reads.get(path).cloned())
        }
    }

    fn root(name: &str) -> PathBuf {
        let root: PathBuf = std::env::temp_dir().join(format!("qa-demo-library-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        root
    }

    #[test]
    fn list_merges_mounts_and_root() {
        let root = root("merge");
        std::fs::write(root.join("a.dem"), [1]).unwrap();
        std::fs::write(root.join("sub").join("b.dm2"), [2]).unwrap();
        std::fs::write(root.join("notes.txt"), [3]).unwrap();
        let mut files = HashMap::new();
        files.insert(
            ("".to_string(), ".dem".to_string()),
            vec!["A.DEM".to_string(), "mounted.dem".to_string()],
        );
        files.insert(
            ("demos".to_string(), ".dm_68".to_string()),
            vec!["q3.dm_68".to_string()],
        );
        let library = DemoLibrary::new(
            root,
            FakeMounts {
                files,
                reads: HashMap::new(),
            },
        );
        let entries = library.list().unwrap();
        let ids: Vec<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(ids, ["a.dem", "demos/q3.dm_68", "mounted.dem", "sub/b.dm2"]);
        assert_eq!(entries[0].family, DemoFamily::Q1);
        assert_eq!(entries[1].family, DemoFamily::Q3);
        assert_eq!(entries[3].family, DemoFamily::Q2);
        assert_eq!(entries[0].label, "a.dem");
    }

    #[test]
    fn list_skips_missing_root() {
        let root: PathBuf = std::env::temp_dir().join(format!("qa-demo-library-{}-absent", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let library = DemoLibrary::new(
            root,
            FakeMounts {
                files: HashMap::new(),
                reads: HashMap::new(),
            },
        );
        assert!(library.list().unwrap().is_empty());
    }

    #[test]
    fn read_prefers_root_then_mounts() {
        let root = root("read");
        std::fs::write(root.join("a.dem"), [1]).unwrap();
        let mut reads = HashMap::new();
        reads.insert("a.dem".to_string(), vec![9]);
        reads.insert("mounted.dem".to_string(), vec![7]);
        let library = DemoLibrary::new(
            root,
            FakeMounts {
                files: HashMap::new(),
                reads,
            },
        );
        assert_eq!(library.read("a.dem").unwrap(), Some(vec![1]));
        assert_eq!(library.read("mounted.dem").unwrap(), Some(vec![7]));
        assert_eq!(library.read("absent.dem").unwrap(), None);
        assert!(library.read("../escape.dem").is_err());
    }

    #[test]
    fn demo_suffix_matching() {
        assert!(is_demo_path("x.dm_68"));
        assert!(is_demo_path("x.DM_7"));
        assert!(is_demo_path("x.QWD"));
        assert!(!is_demo_path("x.dm_"));
        assert!(!is_demo_path("x.dm_x"));
        assert!(!is_demo_path("x.txt"));
        assert!(!is_demo_path("nodot"));
    }

    #[test]
    fn menu_service_lists_and_invokes_sinks() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let root = root("service");
        std::fs::write(root.join("a.dem"), [1]).unwrap();
        let mut files = HashMap::new();
        files.insert(
            ("demos".to_string(), ".dm_68".to_string()),
            vec!["q3.dm_68".to_string()],
        );
        let library = DemoLibrary::new(
            root,
            FakeMounts {
                files,
                reads: HashMap::new(),
            },
        );
        let played: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let recorded: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let stopped: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));
        let sink_played = Rc::clone(&played);
        let sink_recorded = Rc::clone(&recorded);
        let sink_stopped = Rc::clone(&stopped);
        let mut service = DemoMenuService::new(
            library,
            move |id: &str| {
                sink_played.borrow_mut().push(id.to_string());
                Ok(())
            },
            move |name: &str| {
                sink_recorded.borrow_mut().push(name.to_string());
                Ok(())
            },
            move || {
                *sink_stopped.borrow_mut() += 1;
                Ok(())
            },
            || None,
        );
        service.refresh();
        let entries = service.entries();
        let ids: Vec<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(ids, ["a.dem", "demos/q3.dm_68"]);
        assert_eq!(entries[1].detail.as_deref(), Some("Q3"));
        assert_eq!(service.status(), "2 demos");
        assert_eq!(service.create_label().as_deref(), Some("Record"));
        assert_eq!(service.stop_label().as_deref(), Some("Stop recording"));
        service.activate("a.dem");
        service.activate("absent.dem");
        service.create_submit("fresh");
        service.stop_activate();
        assert_eq!(*played.borrow(), ["a.dem"]);
        assert_eq!(*recorded.borrow(), ["fresh"]);
        assert_eq!(*stopped.borrow(), 1);
    }

    #[test]
    fn menu_service_reports_sink_failures_as_status() {
        let root = root("service-error");
        let library = DemoLibrary::new(
            root,
            FakeMounts {
                files: HashMap::new(),
                reads: HashMap::new(),
            },
        );
        let mut service = DemoMenuService::new(
            library,
            |_: &str| Err("no playback".to_string()),
            |_: &str| Err("no record".to_string()),
            || Err("no stop".to_string()),
            || None,
        );
        service.refresh();
        assert_eq!(service.status(), "0 demos");
        service.stop_activate();
        assert_eq!(service.status(), "no stop");
        service.create_submit("fresh");
        assert_eq!(service.status(), "no record");
    }

    #[test]
    fn menu_service_prefers_recording_path_as_status() {
        let root = root("service-recording");
        let library = DemoLibrary::new(
            root,
            FakeMounts {
                files: HashMap::new(),
                reads: HashMap::new(),
            },
        );
        let mut service = DemoMenuService::new(
            library,
            |_: &str| Ok(()),
            |_: &str| Ok(()),
            || Ok(()),
            || Some("demos/live.dm_68".to_string()),
        );
        service.refresh();
        assert_eq!(service.status(), "demos/live.dm_68");
    }
}
