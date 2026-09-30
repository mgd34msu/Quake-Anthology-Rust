//! Filesystem conveniences shared by the tools: JSON IO, unique temporary
//! directories, exclusive copies, and cryptographically random hex.

use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::error::ToolsError;
use crate::json::{parse_json, Json};

/// Read a whole file as UTF-8 text.
pub fn read_text(path: &Path) -> Result<String, ToolsError> {
    fs::read_to_string(path).map_err(|error| ToolsError::io(format!("reading {}", path.display()), error))
}

/// Read a whole file as bytes.
pub fn read_bytes(path: &Path) -> Result<Vec<u8>, ToolsError> {
    fs::read(path).map_err(|error| ToolsError::io(format!("reading {}", path.display()), error))
}

/// Read and parse a JSON document.
pub fn read_json(path: &Path) -> Result<Json, ToolsError> {
    parse_json(&read_text(path)?)
}

/// Write bytes to `path`, creating parent directories as needed.
pub fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), ToolsError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| ToolsError::io(format!("creating {}", parent.display()), error))?;
        }
    }
    fs::write(path, bytes).map_err(|error| ToolsError::io(format!("writing {}", path.display()), error))
}

/// Write text plus a trailing newline exactly once.
pub fn write_text(path: &Path, text: &str) -> Result<(), ToolsError> {
    write_bytes(path, text.as_bytes())
}

/// Generate `bytes` random bytes from the operating system.
pub fn random_bytes(count: usize) -> Result<Vec<u8>, ToolsError> {
    let mut file = File::open("/dev/urandom").map_err(|error| ToolsError::io("opening /dev/urandom", error))?;
    let mut bytes = vec![0u8; count];
    file.read_exact(&mut bytes)
        .map_err(|error| ToolsError::io("reading /dev/urandom", error))?;
    Ok(bytes)
}

/// Generate a random version-4 UUID (`crypto.randomUUID` shape).
pub fn random_uuid() -> Result<String, ToolsError> {
    let bytes = random_bytes(16)?;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "{}-{}-4{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

/// Create a uniquely named directory under `parent` starting with `prefix`
/// (donor `mkdtemp`).
pub fn make_temp_dir(parent: &Path, prefix: &str) -> Result<PathBuf, ToolsError> {
    fs::create_dir_all(parent).map_err(|error| ToolsError::io(format!("creating {}", parent.display()), error))?;
    for _ in 0..100 {
        let bytes = random_bytes(8)?;
        let suffix: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let path = parent.join(format!("{prefix}{suffix}"));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(ToolsError::io(format!("creating {}", path.display()), error)),
        }
    }
    Err(ToolsError::invalid(format!("Cannot create a unique directory under {}", parent.display())))
}

/// Stage `text` in a fresh sibling directory of `destination`, then atomically
/// rename it into place (donor staged-write pattern).
pub fn write_atomic_text(destination: &Path, text: &str, file_name: &str, prefix: &str) -> Result<(), ToolsError> {
    let parent = destination.parent().filter(|path| !path.as_os_str().is_empty()).map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let temporary = make_temp_dir(&parent, prefix)?;
    let staged = temporary.join(file_name);
    let result = (|| -> Result<(), ToolsError> {
        fs::write(&staged, text).map_err(|error| ToolsError::io(format!("staging {}", staged.display()), error))?;
        File::open(&staged)
            .and_then(|file| file.sync_all())
            .map_err(|error| ToolsError::io(format!("syncing {}", staged.display()), error))?;
        fs::rename(&staged, destination).map_err(|error| ToolsError::io(format!("publishing {}", destination.display()), error))?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&temporary);
    result
}

/// Copy `source` to `destination` failing when the destination exists
/// (donor `COPYFILE_EXCL` copy).
pub fn copy_exclusive(source: &Path, destination: &Path) -> Result<u64, ToolsError> {
    let mut input = File::open(source).map_err(|error| ToolsError::io(format!("opening {}", source.display()), error))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| ToolsError::io(format!("creating {}", destination.display()), error))?;
    std::io::copy(&mut input, &mut output).map_err(|error| ToolsError::io(format!("copying {}", source.display()), error))?;
    output.sync_all().map_err(|error| ToolsError::io(format!("syncing {}", destination.display()), error))?;
    output.metadata().map(|meta| meta.len()).map_err(|error| ToolsError::io(format!("stating {}", destination.display()), error))
}

/// Remove a file or directory tree, ignoring missing paths.
pub fn remove_forced(path: &Path) {
    if path.is_dir() && !path.is_symlink() {
        let _ = fs::remove_dir_all(path);
    } else {
        let _ = fs::remove_file(path);
    }
}

/// Resolve `path` against `base` lexically, without touching the filesystem
/// (donor `path.resolve` semantics: no symlink resolution).
#[must_use]
pub fn lexical_absolute(base: &Path, path: &str) -> PathBuf {
    let joined = if Path::new(path).is_absolute() { PathBuf::from(path) } else { base.join(path) };
    let mut parts: Vec<String> = Vec::new();
    let mut rooted = false;
    for component in joined.components() {
        match component {
            std::path::Component::Prefix(prefix) => parts.push(prefix.as_os_str().to_string_lossy().into_owned()),
            std::path::Component::RootDir => {
                rooted = true;
                parts.clear();
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
        }
    }
    let mut absolute = if rooted { PathBuf::from("/") } else { PathBuf::new() };
    for part in parts {
        absolute.push(part);
    }
    absolute
}

/// Relative path from `base` to `path` with `/` separators (donor
/// `path.relative(...).replaceAll("\\", "/")`).
#[must_use]
pub fn posix_relative(base: &Path, path: &Path) -> String {
    let absolute_base = if base.is_absolute() { base.to_path_buf() } else { lexical_absolute(&PathBuf::from("/"), &base.to_string_lossy()) };
    let absolute_path = if path.is_absolute() { path.to_path_buf() } else { lexical_absolute(&PathBuf::from("/"), &path.to_string_lossy()) };
    let base_parts: Vec<String> = absolute_base.components().filter_map(|component| match component {
        std::path::Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
        _ => None,
    }).collect();
    let path_parts: Vec<String> = absolute_path.components().filter_map(|component| match component {
        std::path::Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
        _ => None,
    }).collect();
    let common = base_parts.iter().zip(path_parts.iter()).take_while(|(left, right)| left == right).count();
    let mut parts = vec![".."; base_parts.len() - common];
    parts.extend(path_parts[common..].iter().map(String::as_str));
    if parts.is_empty() { String::new() } else { parts.join("/") }
}

/// Create a symlink `link` pointing at `original`.
#[cfg(unix)]
pub fn create_symlink(original: &Path, link: &Path) -> Result<(), ToolsError> {
    std::os::unix::fs::symlink(original, link)
        .map_err(|error| ToolsError::io(format!("linking {} to {}", link.display(), original.display()), error))
}

/// Non-Unix fallback: symlinks are unsupported.
#[cfg(not(unix))]
pub fn create_symlink(original: &Path, link: &Path) -> Result<(), ToolsError> {
    let _ = (original, link);
    Err(ToolsError::invalid("Symbolic links are unsupported on this platform"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_lexically() {
        assert_eq!(lexical_absolute(Path::new("/a/b"), "c"), PathBuf::from("/a/b/c"));
        assert_eq!(lexical_absolute(Path::new("/a/b"), "../c"), PathBuf::from("/a/c"));
        assert_eq!(lexical_absolute(Path::new("/a/b"), "/x/./y"), PathBuf::from("/x/y"));
        assert_eq!(lexical_absolute(Path::new("/a/b"), "../../.."), PathBuf::from("/"));
    }

    #[test]
    fn relativizes_posix() {
        assert_eq!(posix_relative(Path::new("/a/b"), Path::new("/a/b/c")), "c");
        assert_eq!(posix_relative(Path::new("/a/b/c"), Path::new("/a/d")), "../../d");
        assert_eq!(posix_relative(Path::new("/a"), Path::new("/a")), "");
    }
}
