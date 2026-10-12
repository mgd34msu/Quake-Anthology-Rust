//! Owned OS file access. VFS and format readers retain these handles and keep
//! their own naming, archive and decode policies outside the platform.
use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

pub struct ReadFile {
    file: File,
}
impl ReadFile {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            file: File::open(path)?,
        })
    }

    pub fn length(&self) -> io::Result<u64> {
        Ok(self.file.metadata()?.len())
    }

    #[cfg(unix)]
    pub fn read_at(&self, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
        use std::os::unix::fs::FileExt;
        self.file.read_at(destination, offset)
    }

    #[cfg(windows)]
    pub fn read_at(&self, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
        use std::os::windows::fs::FileExt;
        self.file.seek_read(destination, offset)
    }

    pub fn read_exact_at(&self, mut offset: u64, mut destination: &mut [u8]) -> io::Result<()> {
        while !destination.is_empty() {
            let count = match self.read_at(offset, destination) {
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            if count == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            offset += count as u64;
            destination = &mut destination[count..];
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    Other,
}

pub struct DirectoryEntry {
    pub path: PathBuf,
    pub name: OsString,
    pub kind: EntryKind,
}

pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

/// Native filename order matches VFS mount admission. Entry kinds do not
/// follow symlinks, and querying them stays behind this OS boundary.
pub fn directory_entries(path: impl AsRef<Path>) -> io::Result<Vec<DirectoryEntry>> {
    let mut entries = std::fs::read_dir(path)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_unstable_by_key(|entry| entry.file_name());
    entries
        .into_iter()
        .map(|entry| {
            let kind = entry.file_type()?;
            Ok(DirectoryEntry {
                path: entry.path(),
                name: entry.file_name(),
                kind: if kind.is_dir() {
                    EntryKind::Directory
                } else if kind.is_file() {
                    EntryKind::File
                } else {
                    EntryKind::Other
                },
            })
        })
        .collect()
}

/// Cold developer/profile reads use the same private owned handle.
pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let mut file = ReadFile::open(path)?;
    let mut bytes = Vec::new();
    file.file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    std::fs::write(path, bytes)
}

pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    std::fs::create_dir_all(path)
}

pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    std::fs::remove_file(path)
}

pub fn remove_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    std::fs::remove_dir_all(path)
}
