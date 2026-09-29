//! Contained relative paths for user-data storage.
//!
//! Port of donor `src/platform/files/contained.ts`. Paths must be relative
//! and stay inside their storage directory: no drive letters, no backslashes,
//! no NULs, no empty segments, no `.` / `..`.

use std::fs;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Split `name` into path parts, rejecting anything that could escape the root.
pub fn contained_file_parts(name: &str) -> Result<Vec<String>> {
    if name.is_empty() || name.contains('\\') || name.contains('\0') || name.contains(':') {
        return Err(Error::OutOfRange(
            "file path must be relative and stay inside its storage directory".to_string(),
        ));
    }
    let parts: Vec<String> = name.split('/').map(str::to_string).collect();
    if parts.iter().any(|part| part.is_empty() || part == "." || part == "..") {
        return Err(Error::OutOfRange(
            "file path must be relative and stay inside its storage directory".to_string(),
        ));
    }
    Ok(parts)
}

/// An anchored parent directory: descriptor plus final leaf name.
pub struct ContainedParent {
    /// Directory descriptor the leaf resolves under.
    pub descriptor: OwnedFd,
    /// Final path component.
    pub leaf: String,
}

/// Open the chain of parent directories under `root`, creating them when
/// `create` is set. Directory descriptors keep child operations anchored
/// despite concurrent path renames (Linux only, like the donor).
pub fn open_contained_parent(root: &Path, name: &str, create: bool) -> Result<ContainedParent> {
    if !cfg!(target_os = "linux") {
        return Err(Error::Unsupported(
            "contained file storage currently requires Linux directory descriptors".to_string(),
        ));
    }
    let parts = contained_file_parts(name)?;
    let leaf = parts
        .last()
        .cloned()
        .ok_or_else(|| Error::OutOfRange("file path has no filename".to_string()))?;
    let mut descriptor = open_dir_no_follow(root)?;
    for part in &parts[..parts.len() - 1] {
        if create {
            let path = PathBuf::from(format!("/proc/self/fd/{}/{}", descriptor.as_raw_fd(), part));
            match fs::create_dir(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(Error::Io(error)),
            }
        }
        let path = PathBuf::from(format!("/proc/self/fd/{}/{}", descriptor.as_raw_fd(), part));
        let next = open_dir_no_follow(&path)?;
        descriptor = next;
    }
    Ok(ContainedParent { descriptor, leaf })
}

fn open_dir_no_follow(path: &Path) -> Result<OwnedFd> {
    use std::fs::OpenOptions;
    // O_RDONLY | O_DIRECTORY | O_NOFOLLOW: 0o0 | 0o200000 | 0o400000.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc_o_directory() | libc_o_nofollow())
        .open(path)
        .map_err(Error::Io)?;
    Ok(OwnedFd::from(file))
}

#[cfg(target_os = "linux")]
fn libc_o_directory() -> i32 {
    0o200000
}

#[cfg(target_os = "linux")]
fn libc_o_nofollow() -> i32 {
    0o400000
}

#[cfg(not(target_os = "linux"))]
fn libc_o_directory() -> i32 {
    0
}

#[cfg(not(target_os = "linux"))]
fn libc_o_nofollow() -> i32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_relative_paths() {
        assert_eq!(
            contained_file_parts("q3/baseq3/autoexec.cfg").unwrap(),
            vec!["q3", "baseq3", "autoexec.cfg"]
        );
        assert_eq!(contained_file_parts("single.dat").unwrap(), vec!["single.dat"]);
    }

    #[test]
    fn rejects_escapes() {
        for bad in [
            "",
            "/absolute",
            "a//b",
            "a/./b",
            "../escape",
            "a/../../escape",
            "..",
            ".",
            "back\\slash",
            "nul\0byte",
            "C:/drive",
            "trailing/",
        ] {
            assert!(contained_file_parts(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn anchors_parents_on_linux() {
        if !cfg!(target_os = "linux") {
            return;
        }
        let root = std::env::temp_dir().join(format!("qa-platform-contained-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let parent = open_contained_parent(&root, "sub/dir/file.bin", true).unwrap();
        assert_eq!(parent.leaf, "file.bin");
        assert!(root.join("sub/dir").is_dir());
        // Without create, a missing chain fails instead of materializing.
        assert!(open_contained_parent(&root, "absent/file.bin", false).is_err());
        fs::remove_dir_all(&root).ok();
    }
}
