//! Workspace snapshots (donor `tools/verify/snapshot.ts`).
//!
//! Captures a source tree's bytes, modes, and dependency symlinks into a
//! hashed manifest, then materializes exact copies for isolated builds.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::ToolsError;
use crate::fsutil::make_temp_dir;
use crate::json::Json;
use crate::verify::hash::{hash_bytes, hash_json};

/// A captured entry: file bytes or a dependency symlink.
#[derive(Debug, Clone)]
enum SnapshotEntry {
    /// Regular file with bytes.
    File {
        /// Snapshot-relative path with `/` separators.
        path: String,
        /// Permission bits masked to `0o555`.
        mode: u32,
        /// File bytes.
        data: Vec<u8>,
        /// SHA-256 of the bytes.
        sha256: String,
    },
    /// Dependency symlink.
    Link {
        /// Snapshot-relative path with `/` separators.
        path: String,
        /// Link target.
        target: String,
    },
}

impl SnapshotEntry {
    fn path(&self) -> &str {
        match self {
            Self::File { path, .. } | Self::Link { path, .. } => path,
        }
    }

    fn to_json(&self) -> Json {
        match self {
            Self::File {
                path,
                mode,
                data,
                sha256,
            } => Json::object(vec![
                ("kind".to_owned(), Json::string("file")),
                ("path".to_owned(), Json::string(path)),
                ("mode".to_owned(), Json::int(i64::from(*mode))),
                ("bytes".to_owned(), Json::int(data.len() as i64)),
                ("sha256".to_owned(), Json::string(sha256)),
            ]),
            Self::Link { path, target } => Json::object(vec![
                ("kind".to_owned(), Json::string("link")),
                ("path".to_owned(), Json::string(path)),
                ("target".to_owned(), Json::string(target)),
            ]),
        }
    }
}

/// A snapshot manifest: content hash plus file records.
#[derive(Debug, Clone)]
pub struct SnapshotManifest {
    /// SHA-256 over the canonical file list.
    pub sha256: String,
    /// File records.
    pub files: Vec<Json>,
}

impl SnapshotManifest {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("sha256".to_owned(), Json::string(&self.sha256)),
            ("files".to_owned(), Json::array(self.files.clone())),
        ])
    }
}

/// Whether `child` is `parent` or below it (lexical, donor `isWithin`).
#[must_use]
pub fn is_within(parent: &Path, child: &Path) -> bool {
    let relative = relative_path(parent, child);
    relative.is_empty() || (!relative.is_absolute() && relative != Path::new("..") && !relative.starts_with(".."))
}

/// Lexical relative path from `parent` to `child` (donor `path.relative`).
fn relative_path(parent: &Path, child: &Path) -> PathBuf {
    let mut parents: Vec<Component> = parent.components().collect();
    let mut children: Vec<Component> = child.components().collect();
    while !parents.is_empty() && !children.is_empty() && parents[0] == children[0] {
        parents.remove(0);
        children.remove(0);
    }
    let mut out = PathBuf::new();
    for _ in &parents {
        out.push("..");
    }
    for component in children {
        out.push(component.as_os_str());
    }
    out
}

/// Lexically absolutize `path` against the current directory.
fn absolutize(path: &Path) -> PathBuf {
    if path.is_absolute() {
        normalize(path)
    } else {
        match std::env::current_dir() {
            Ok(cwd) => normalize(&cwd.join(path)),
            Err(_) => path.to_path_buf(),
        }
    }
}

/// Lexically normalize `.` and `..` segments.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

fn is_credential_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let stem = lower.split('.').next().unwrap_or("");
    stem == "q3key" || stem == "quake3cdkey"
}

/// A captured workspace tree.
pub struct WorkspaceSnapshot {
    entries: Vec<SnapshotEntry>,
    /// Content manifest.
    pub manifest: SnapshotManifest,
}

impl WorkspaceSnapshot {
    fn new(mut entries: Vec<SnapshotEntry>) -> Result<Self, ToolsError> {
        entries.sort_by(|left, right| left.path().cmp(right.path()));
        let files: Vec<Json> = entries.iter().map(SnapshotEntry::to_json).collect();
        let sha256 = hash_json(&Json::array(files.clone()))?;
        Ok(Self {
            entries,
            manifest: SnapshotManifest { sha256, files },
        })
    }

    /// Capture `directory`, excluding `.git`, `.artifacts`, `dist`, and `exclude`.
    pub fn capture(directory: &Path, exclude: &[String]) -> Result<Self, ToolsError> {
        let root = canonicalize(directory)?;
        let dependency_root = root.join("node_modules");
        let mut exclusions = vec![root.join(".git"), root.join(".artifacts"), root.join("dist")];
        for path in exclude {
            exclusions.push(normalize(&root.join(path)));
        }
        let mut entries = Vec::new();
        visit(&root, &root, String::new(), &exclusions, &dependency_root, &mut entries)?;
        Self::new(entries)
    }

    /// Materialize the snapshot into a fresh directory under `parent`.
    pub fn materialize(&self, parent: &Path, prefix: &str) -> Result<PathBuf, ToolsError> {
        if prefix.is_empty()
            || !prefix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        {
            return Err(ToolsError::invalid("Snapshot prefix must be a plain directory prefix"));
        }
        let root = make_temp_dir(parent, prefix)?;
        for entry in &self.entries {
            let path = root.join(entry.path());
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| ToolsError::io(format!("creating {}", parent.display()), error))?;
            }
            match entry {
                SnapshotEntry::Link { target, .. } => {
                    symlink(Path::new(target), &path)?;
                }
                SnapshotEntry::File { data, mode, .. } => {
                    fs::write(&path, data)
                        .map_err(|error| ToolsError::io(format!("writing {}", path.display()), error))?;
                    set_mode(&path, *mode)?;
                }
            }
        }
        Ok(root)
    }
}

fn canonicalize(path: &Path) -> Result<PathBuf, ToolsError> {
    fs::canonicalize(path).map_err(|error| ToolsError::io(format!("resolving {}", path.display()), error))
}

#[cfg(unix)]
fn symlink(target: &Path, path: &Path) -> Result<(), ToolsError> {
    std::os::unix::fs::symlink(target, path)
        .map_err(|error| ToolsError::io(format!("linking {}", path.display()), error))
}

#[cfg(not(unix))]
fn symlink(_target: &Path, path: &Path) -> Result<(), ToolsError> {
    Err(ToolsError::invalid(format!(
        "Snapshots cannot materialize symlinks on this platform: {}",
        path.display()
    )))
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), ToolsError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| ToolsError::io(format!("setting mode on {}", path.display()), error))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), ToolsError> {
    Ok(())
}

#[cfg(unix)]
fn file_mode(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o555
}

#[cfg(not(unix))]
fn file_mode(_meta: &fs::Metadata) -> u32 {
    0o444
}

fn visit(
    root: &Path,
    directory: &Path,
    prefix: String,
    exclusions: &[PathBuf],
    dependency_root: &Path,
    entries: &mut Vec<SnapshotEntry>,
) -> Result<(), ToolsError> {
    let mut names: Vec<fs::DirEntry> = fs::read_dir(directory)
        .map_err(|error| ToolsError::io(format!("reading {}", directory.display()), error))?
        .collect::<Result<_, _>>()
        .map_err(|error| ToolsError::io(format!("reading {}", directory.display()), error))?;
    names.sort_by_key(|entry| entry.file_name());
    for child in names {
        let name = child.file_name().to_string_lossy().into_owned();
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let absolute = directory.join(child.file_name());
        if exclusions.iter().any(|exclusion| is_within(exclusion, &absolute)) {
            continue;
        }
        if is_credential_filename(&name) {
            return Err(ToolsError::invalid(format!(
                "Snapshot refuses a credential filename: {path}"
            )));
        }
        let kind = child
            .file_type()
            .map_err(|error| ToolsError::io(format!("stating {}", absolute.display()), error))?;
        if kind.is_dir() {
            visit(root, &absolute, path, exclusions, dependency_root, entries)?;
        } else if kind.is_symlink() {
            let target = fs::read_link(&absolute)
                .map_err(|error| ToolsError::io(format!("reading {}", absolute.display()), error))?;
            let target_text = target.to_string_lossy().into_owned();
            let resolved = normalize(&absolute.parent().unwrap_or(root).join(&target));
            let canonical = canonicalize(&absolute)?;
            if !path.starts_with("node_modules/")
                || target.is_absolute()
                || !is_within(dependency_root, &resolved)
                || !is_within(dependency_root, &canonical)
            {
                return Err(ToolsError::invalid(format!(
                    "Snapshot refuses a source or external dependency symlink: {path}"
                )));
            }
            entries.push(SnapshotEntry::Link {
                path,
                target: target_text,
            });
        } else if kind.is_file() {
            let stat = fs::symlink_metadata(&absolute)
                .map_err(|error| ToolsError::io(format!("stating {}", absolute.display()), error))?;
            if !stat.is_file() {
                return Err(ToolsError::invalid(format!("Snapshot input changed file kind: {path}")));
            }
            let data = fs::read(&absolute)
                .map_err(|error| ToolsError::io(format!("reading {}", absolute.display()), error))?;
            let sha256 = hash_bytes(&data);
            entries.push(SnapshotEntry::File {
                path,
                mode: file_mode(&stat),
                data,
                sha256,
            });
        } else {
            return Err(ToolsError::invalid(format!(
                "Snapshot refuses a non-regular input: {path}"
            )));
        }
    }
    Ok(())
}

/// Absolute lexical resolution (donor `path.resolve`).
pub fn resolve(path: &Path) -> PathBuf {
    absolutize(path)
}
