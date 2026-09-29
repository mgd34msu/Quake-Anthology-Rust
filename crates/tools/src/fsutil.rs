//! Filesystem conveniences shared by the tools: JSON IO, unique temporary
//! directories, exclusive copies, and cryptographically random hex.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
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
