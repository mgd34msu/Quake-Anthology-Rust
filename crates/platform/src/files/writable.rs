//! Writable user-data files anchored inside one scoped directory.
//!
//! Port of donor `src/platform/files/writable.ts` onto `std::fs` plus
//! positional Unix writes. Unbuffered descriptors already satisfy the source
//! append-sync flush guarantee, so `append-sync` needs no extra work.

use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::FileExt;

use crate::error::{Error, Result};
use crate::files::contained::{contained_file_parts, open_contained_parent};

/// Open mode for a writable binary file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WritableFileMode {
    /// Truncate and write positionally.
    Write,
    /// Append through `O_APPEND`.
    Append,
    /// Append; unbuffered writes already satisfy the flush guarantee.
    AppendSync,
}

impl WritableFileMode {
    fn is_append(self) -> bool {
        !matches!(self, Self::Write)
    }

    fn parse(mode: &str) -> Result<Self> {
        match mode {
            "write" => Ok(Self::Write),
            "append" => Ok(Self::Append),
            "append-sync" => Ok(Self::AppendSync),
            _ => Err(Error::OutOfRange("invalid writable checkpoint mode".to_string())),
        }
    }

    #[must_use]
    fn name(self) -> &'static str {
        match self {
            Self::Write => "write",
            Self::Append => "append",
            Self::AppendSync => "append-sync",
        }
    }
}

/// Seek origin for [`WritableBinaryFile::seek`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WritableSeekOrigin {
    /// Relative to the current cursor.
    Current,
    /// Relative to the end of file.
    End,
    /// Absolute position.
    Set,
}

/// Serializable cursor for [`UserFileStore::resume`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WritableFileCheckpoint {
    /// Contained path within the store root.
    pub path: String,
    /// Mode the file was opened with.
    pub mode: WritableFileMode,
    /// Cursor byte offset.
    pub position: u64,
}

/// An open writable binary file with an explicit cursor.
pub struct WritableBinaryFile {
    file: File,
    mode: WritableFileMode,
    path: String,
    position: u64,
    print: Box<dyn Fn(&str) + Send>,
    ended: bool,
}

impl WritableBinaryFile {
    fn live(&self) -> Result<()> {
        if self.ended {
            return Err(Error::Closed("writable file".to_string()));
        }
        Ok(())
    }

    /// Capture a resumable checkpoint of path, mode, and cursor.
    pub fn capture_checkpoint(&self) -> Result<WritableFileCheckpoint> {
        self.live()?;
        Ok(WritableFileCheckpoint {
            path: self.path.clone(),
            mode: self.mode,
            position: self.position,
        })
    }

    /// File mode.
    #[must_use]
    pub fn mode(&self) -> WritableFileMode {
        self.mode
    }

    /// Write every byte, retrying a single zero-length write like the donor.
    /// Returns the bytes accepted, or 0 after a retried stall.
    pub fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        self.live()?;
        let mut offset = 0;
        let mut retried = false;
        while offset < bytes.len() {
            let written = self.write_some(&bytes[offset..])?;
            if written == 0 {
                if retried {
                    (self.print)("FS_Write: 0 bytes written\n");
                    self.live()?;
                    return Ok(0);
                }
                retried = true;
                continue;
            }
            offset += written;
            self.position = if self.mode == WritableFileMode::Write {
                self.position + written as u64
            } else {
                self.file.metadata().map_err(Error::Io)?.len()
            };
        }
        Ok(bytes.len())
    }

    #[cfg(unix)]
    fn write_some(&self, bytes: &[u8]) -> Result<usize> {
        if self.mode == WritableFileMode::Write {
            // `write_at` issues pwrite; O_APPEND is unset so the offset applies.
            self.file.write_at(bytes, self.position).map_err(Error::Io)
        } else {
            // O_APPEND forces end-of-file atomically; `write_at` ignores the
            // offset on Linux, so every append lands at the end.
            self.file.write_at(bytes, 0).map_err(Error::Io)
        }
    }

    #[cfg(not(unix))]
    fn write_some(&self, bytes: &[u8]) -> Result<usize> {
        use std::io::{Seek, SeekFrom, Write};
        let mut handle = &self.file;
        if self.mode == WritableFileMode::Write {
            handle.seek(SeekFrom::Start(self.position)).map_err(Error::Io)?;
        } else {
            handle.seek(SeekFrom::End(0)).map_err(Error::Io)?;
        }
        handle.write(bytes).map_err(Error::Io)
    }

    /// Move the cursor; returns 0 on success, -1 on invalid input like the source.
    pub fn seek(&mut self, offset: i64, origin: WritableSeekOrigin) -> i64 {
        if self.live().is_err() {
            return -1;
        }
        let length = match self.file.metadata() {
            Ok(metadata) => metadata.len(),
            Err(_) => return -1,
        };
        let base: i64 = match origin {
            WritableSeekOrigin::Current => match i64::try_from(self.position) {
                Ok(position) => position,
                Err(_) => return -1,
            },
            WritableSeekOrigin::End => match i64::try_from(length) {
                Ok(length) => length,
                Err(_) => return -1,
            },
            WritableSeekOrigin::Set => 0,
        };
        let position = base.checked_add(offset);
        match position {
            Some(value) if value >= 0 => match u64::try_from(value) {
                Ok(cursor) => {
                    self.position = cursor;
                    0
                }
                Err(_) => -1,
            },
            _ => -1,
        }
    }

    /// Current cursor.
    pub fn position(&self) -> Result<u64> {
        self.live()?;
        Ok(self.position)
    }

    /// Close the file; idempotent.
    pub fn close(&mut self) {
        self.ended = true;
    }

    /// Whether the file is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.ended
    }
}

/// One explicitly scoped user-data directory.
pub struct UserFileStore {
    root: PathBuf,
}

impl UserFileStore {
    /// Scope a store at `root`. No package or installed-content root is inferred.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Store root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Reopen a checkpointed file, preserving external contents. Append modes
    /// may create the destination on a fresh install.
    pub fn resume(
        &self,
        state: &WritableFileCheckpoint,
        print: impl Fn(&str) + Send + 'static,
    ) -> Result<WritableBinaryFile> {
        contained_file_parts(&state.path)?;
        let append = state.mode.is_append();
        if append {
            fs::create_dir_all(&self.root).map_err(Error::Io)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700)).map_err(Error::Io)?;
            }
        }
        let parent = open_contained_parent(&self.root, &state.path, append)?;
        let leaf_path = PathBuf::from(format!(
            "/proc/self/fd/{}/{}",
            parent.descriptor.as_raw_fd(),
            parent.leaf
        ));
        let file = self.open_leaf(&leaf_path, state.mode, false)?;
        Ok(WritableBinaryFile {
            file,
            mode: state.mode,
            path: state.path.clone(),
            position: state.position,
            print: Box::new(print),
            ended: false,
        })
    }

    /// Open (creating parents) a contained file for writing. Returns `None`
    /// when the platform refuses the open, like the donor.
    pub fn open(
        &self,
        name: &str,
        mode: WritableFileMode,
        print: impl Fn(&str) + Send + 'static,
    ) -> Result<Option<WritableBinaryFile>> {
        fs::create_dir_all(&self.root).map_err(Error::Io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700)).map_err(Error::Io)?;
        }
        let parent = open_contained_parent(self.root(), name, true)?;
        let leaf_path = PathBuf::from(format!(
            "/proc/self/fd/{}/{}",
            parent.descriptor.as_raw_fd(),
            parent.leaf
        ));
        let file = match self.open_leaf(&leaf_path, mode, true) {
            Ok(file) => file,
            Err(Error::Io(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        if mode == WritableFileMode::Write {
            file.set_len(0).map_err(Error::Io)?;
        }
        let position = if mode == WritableFileMode::Write {
            0
        } else {
            file.metadata().map_err(Error::Io)?.len()
        };
        Ok(Some(WritableBinaryFile {
            file,
            mode,
            path: name.to_string(),
            position,
            print: Box::new(print),
            ended: false,
        }))
    }

    fn open_leaf(&self, leaf_path: &Path, mode: WritableFileMode, create: bool) -> Result<File> {
        let mut options = OpenOptions::new();
        options.write(true);
        if create || mode.is_append() {
            options.create(true);
        }
        if mode.is_append() {
            options.append(true);
        }
        #[cfg(target_os = "linux")]
        {
            // O_NOFOLLOW | O_NONBLOCK: 0o400000 | 0o4000.
            options.custom_flags(0o400000 | 0o4000);
            options.mode(0o600);
        }
        let file = options.open(leaf_path).map_err(Error::Io)?;
        let metadata = file.metadata().map_err(Error::Io)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(Error::InvalidInput(
                "writable source file is not a regular file".to_string(),
            ));
        }
        Ok(file)
    }
}

/// Parse a serialized checkpoint mode name.
pub fn parse_writable_mode(mode: &str) -> Result<WritableFileMode> {
    WritableFileMode::parse(mode)
}

/// Name a checkpoint mode for serialization.
#[must_use]
pub fn writable_mode_name(mode: WritableFileMode) -> &'static str {
    mode.name()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("qa-platform-writable-{}-{name}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        root
    }

    #[test]
    fn write_truncates_and_seeks() {
        if !cfg!(target_os = "linux") {
            return;
        }
        let root = scratch("write");
        let store = UserFileStore::new(root.clone());
        let mut file = store
            .open("sub/save.bin", WritableFileMode::Write, |_| {})
            .unwrap()
            .unwrap();
        assert_eq!(file.write(b"hello").unwrap(), 5);
        assert_eq!(file.seek(0, WritableSeekOrigin::Set), 0);
        assert_eq!(file.write(b"HEL").unwrap(), 3);
        let checkpoint = file.capture_checkpoint().unwrap();
        assert_eq!(checkpoint.position, 3);
        file.close();
        assert!(file.is_closed());
        assert_eq!(fs::read(root.join("sub/save.bin")).unwrap(), b"HELlo");
        // Reopening with Write truncates.
        let mut again = store
            .open("sub/save.bin", WritableFileMode::Write, |_| {})
            .unwrap()
            .unwrap();
        assert_eq!(again.write(b"xy").unwrap(), 2);
        again.close();
        assert_eq!(fs::read(root.join("sub/save.bin")).unwrap(), b"xy");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn append_preserves_and_resume_continues() {
        if !cfg!(target_os = "linux") {
            return;
        }
        let root = scratch("append");
        let store = UserFileStore::new(root.clone());
        let mut file = store.open("log.bin", WritableFileMode::Write, |_| {}).unwrap().unwrap();
        file.write(b"ab").unwrap();
        let checkpoint = WritableFileCheckpoint {
            path: "log.bin".to_string(),
            mode: WritableFileMode::Append,
            position: 2,
        };
        file.close();
        let mut resumed = store.resume(&checkpoint, |_| {}).unwrap();
        assert_eq!(resumed.write(b"cd").unwrap(), 2);
        resumed.close();
        assert_eq!(fs::read(root.join("log.bin")).unwrap(), b"abcd");
        assert_eq!(
            parse_writable_mode("append-sync").unwrap(),
            WritableFileMode::AppendSync
        );
        assert_eq!(writable_mode_name(WritableFileMode::Write), "write");
        assert!(parse_writable_mode("bogus").is_err());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn seek_rejects_bad_positions() {
        if !cfg!(target_os = "linux") {
            return;
        }
        let root = scratch("seek");
        let store = UserFileStore::new(root.clone());
        let mut file = store.open("s.bin", WritableFileMode::Write, |_| {}).unwrap().unwrap();
        file.write(b"1234").unwrap();
        assert_eq!(file.seek(-99, WritableSeekOrigin::Set), -1);
        assert_eq!(file.seek(0, WritableSeekOrigin::End), 0);
        assert_eq!(file.position().unwrap(), 4);
        assert_eq!(file.seek(-2, WritableSeekOrigin::Current), 0);
        assert_eq!(file.position().unwrap(), 2);
        file.close();
        assert_eq!(file.seek(0, WritableSeekOrigin::Set), -1);
        assert!(file.write(b"x").is_err());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rejects_escape_and_non_linux() {
        let root = scratch("escape");
        let store = UserFileStore::new(root.clone());
        if !cfg!(target_os = "linux") {
            assert!(store.open("a.bin", WritableFileMode::Write, |_| {}).is_err());
            return;
        }
        assert!(store.open("../escape.bin", WritableFileMode::Write, |_| {}).is_err());
        fs::remove_dir_all(&root).ok();
    }
}
