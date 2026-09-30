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
    fn list_files(
        &self,
        directory: &str,
        extension: &str,
    ) -> Result<Vec<String>, Self::Error>;

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
            Err(error)
                if error.kind() == ErrorKind::NotFound
                    || error.kind() == ErrorKind::NotADirectory =>
            {
                self.mounts
                    .read(&normalized)
                    .map_err(|error| DemoLibraryError::Mount(error.to_string()))
            }
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
            Err(error)
                if error.kind() == ErrorKind::NotFound
                    || error.kind() == ErrorKind::NotADirectory =>
            {
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

        fn list_files(
            &self,
            directory: &str,
            extension: &str,
        ) -> Result<Vec<String>, Self::Error> {
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
        let root: PathBuf = std::env::temp_dir().join(format!(
            "qa-demo-library-{}-{name}",
            std::process::id()
        ));
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
        let root: PathBuf = std::env::temp_dir().join(format!(
            "qa-demo-library-{}-absent",
            std::process::id()
        ));
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
}
