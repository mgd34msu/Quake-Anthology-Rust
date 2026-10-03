//! Content path handling ported from `src/content/mounts/paths.ts`:
//! validated relative resource paths, root containment, and
//! case-insensitive resolution on case-sensitive hosts.

use std::path::{Path, PathBuf};

use thiserror::Error;

/// Error for invalid or escaping resource paths.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PathError {
    /// The path is not a valid relative resource path.
    #[error("Invalid relative resource path: {0}")]
    Invalid(String),
    /// The path escapes its content root.
    #[error("Resource escapes content root: {0}")]
    EscapesRoot(String),
    /// A symlink escapes its content root.
    #[error("Content symlink escapes root: {0}")]
    SymlinkEscapesRoot(String),
    /// Two directory entries differ only by case.
    #[error("Ambiguous content path: {0}")]
    Ambiguous(String),
    /// Filesystem access failed.
    #[error("Content path error for {path}: {message}")]
    Io {
        /// Requested path.
        path: String,
        /// Underlying error.
        message: String,
    },
}

/// Normalize to forward slashes and reject empty parts, `.`/`..` segments,
/// drive prefixes, and NUL bytes.
pub fn normalize_resource_path(path: &str) -> Result<String, PathError> {
    let normalized = path.replace('\\', "/");
    if normalized.is_empty()
        || normalized.contains('\0')
        || normalized.len() >= 2 && normalized.as_bytes()[0].is_ascii_alphabetic() && normalized.as_bytes()[1] == b':'
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(PathError::Invalid(path.to_string()));
    }
    Ok(normalized)
}

/// Join a resource path onto a root, rejecting escapes.
pub fn path_within_root(root: &Path, path: &str) -> Result<PathBuf, PathError> {
    let normalized = normalize_resource_path(path)?;
    let root = root.to_path_buf();
    let mut destination = root.clone();
    destination.extend(normalized.split('/'));
    if destination != root && !destination.starts_with(&root) {
        return Err(PathError::EscapesRoot(path.to_string()));
    }
    Ok(destination)
}

/// How directory entries compare during resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathComparison {
    /// Byte-exact match.
    Exact,
    /// Case-insensitive match with ambiguity rejection.
    CaseInsensitive,
}

/// Resolve a resource path under a root, returning `None` when missing.
/// Case-insensitive resolution prefers exact spellings and rejects
/// ambiguous collisions; symlinks that escape the root fail.
pub fn find_content_path(root: &Path, path: &str, comparison: PathComparison) -> Result<Option<PathBuf>, PathError> {
    let parts: Vec<String> = normalize_resource_path(path)?.split('/').map(str::to_string).collect();
    let canonical_root = match std::fs::canonicalize(root) {
        Ok(canonical) => canonical,
        Err(error) if is_missing(&error) => return Ok(None),
        Err(error) => {
            return Err(PathError::Io {
                path: path.to_string(),
                message: error.to_string(),
            });
        }
    };
    let mut current = root.to_path_buf();
    for part in &parts {
        if comparison == PathComparison::Exact {
            current.push(part);
        } else {
            let entries = match std::fs::read_dir(&current) {
                Ok(entries) => entries,
                Err(error) if is_missing(&error) => return Ok(None),
                Err(error) => {
                    return Err(PathError::Io {
                        path: path.to_string(),
                        message: error.to_string(),
                    });
                }
            };
            let mut names: Vec<String> = Vec::new();
            for entry in entries {
                let entry = entry.map_err(|error| PathError::Io {
                    path: path.to_string(),
                    message: error.to_string(),
                })?;
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
            if names.iter().any(|entry| entry == part) {
                current.push(part);
            } else {
                let matches: Vec<&String> = names
                    .iter()
                    .filter(|entry| entry.to_lowercase() == part.to_lowercase())
                    .collect();
                if matches.len() > 1 {
                    return Err(PathError::Ambiguous(path.to_string()));
                }
                let Some(matched) = matches.first() else {
                    return Ok(None);
                };
                current.push(matched);
            }
        }
    }
    let canonical = match std::fs::canonicalize(&current) {
        Ok(canonical) => canonical,
        Err(error) if is_missing(&error) => return Ok(None),
        Err(error) => {
            return Err(PathError::Io {
                path: path.to_string(),
                message: error.to_string(),
            });
        }
    };
    if canonical != canonical_root && !canonical.starts_with(&canonical_root) {
        return Err(PathError::SymlinkEscapesRoot(path.to_string()));
    }
    match std::fs::metadata(&canonical) {
        Ok(_) => Ok(Some(current)),
        Err(error) if is_missing(&error) => Ok(None),
        Err(error) => Err(PathError::Io {
            path: path.to_string(),
            message: error.to_string(),
        }),
    }
}

fn is_missing(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_paths_validate_and_contain() {
        assert_eq!(normalize_resource_path("maps\\base1.bsp").unwrap(), "maps/base1.bsp");
        assert!(normalize_resource_path("").is_err());
        assert!(normalize_resource_path("a/../b").is_err());
        assert!(normalize_resource_path("a/./b").is_err());
        assert!(normalize_resource_path("c:/autoexec.cfg").is_err());
        assert!(normalize_resource_path("a\0b").is_err());
        let root = Path::new("/data/quake");
        assert_eq!(
            path_within_root(root, "maps/base1.bsp").unwrap(),
            PathBuf::from("/data/quake/maps/base1.bsp")
        );
        assert!(path_within_root(root, "../escape").is_err());
    }

    #[test]
    fn content_lookup_resolves_case_and_missing() {
        let root = std::env::temp_dir().join(format!("quake-anthology-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Maps")).unwrap();
        std::fs::write(root.join("Maps").join("Base1.bsp"), b"bsp").unwrap();
        let found = find_content_path(&root, "maps/base1.bsp", PathComparison::CaseInsensitive)
            .unwrap()
            .unwrap();
        assert!(found.ends_with("Maps/Base1.bsp"));
        assert!(
            find_content_path(&root, "maps/missing.bsp", PathComparison::CaseInsensitive)
                .unwrap()
                .is_none()
        );
        assert!(find_content_path(&root, "maps/base1.bsp", PathComparison::Exact)
            .unwrap()
            .is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
