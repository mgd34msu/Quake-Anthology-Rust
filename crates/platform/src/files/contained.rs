use super::WriteFile;
use std::{io, path::Path};

/// Sequential output whose destination stays below a retained directory.
/// Linux publication links the retained file inode without replacing a target.
pub struct StagedFile {
    #[cfg(target_os = "linux")]
    staging: Option<linux::Staging>,
    length: u64,
    maximum_bytes: u64,
}
impl StagedFile {
    pub fn create(
        root: &Path,
        name: &str,
        maximum_bytes: u64,
        buffer_bytes: usize,
    ) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            WriteFile::validate_buffer(buffer_bytes)?;
            let staging = linux::Staging::create(root, name, buffer_bytes)?;
            Ok(Self {
                staging: Some(staging),
                length: 0,
                maximum_bytes,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (root, name, maximum_bytes, buffer_bytes);
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    pub fn len(&self) -> u64 {
        self.length
    }
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            if self
                .staging
                .as_ref()
                .is_none_or(|state| state.file.is_none())
            {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            let next = u64::try_from(bytes.len())
                .ok()
                .and_then(|length| self.length.checked_add(length))
                .filter(|&length| length <= self.maximum_bytes);
            let Some(next) = next else {
                return Err(self.failed(io::ErrorKind::InvalidInput.into()));
            };
            if let Err(error) = self
                .staging
                .as_mut()
                .and_then(|state| state.file.as_mut())
                .ok_or(io::ErrorKind::BrokenPipe)?
                .write_all(bytes)
            {
                return Err(self.failed(error));
            }
            self.length = next;
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = bytes;
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    /// Flush and publish once. An existing file, directory or symlink is kept.
    /// A cleanup error after the link can leave the completed destination.
    pub fn publish(mut self) -> io::Result<u64> {
        #[cfg(target_os = "linux")]
        {
            let result = self
                .staging
                .as_mut()
                .ok_or(io::ErrorKind::BrokenPipe)?
                .publish();
            if let Err(error) = result {
                return Err(self.failed(error));
            }
            self.cleanup()?;
            Ok(self.length)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    pub fn cancel(mut self) -> io::Result<()> {
        self.cleanup()
    }

    fn cleanup(&mut self) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        if let Some(staging) = &mut self.staging {
            staging.cleanup()?;
            self.staging = None;
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn failed(&mut self, error: io::Error) -> io::Error {
        match self.cleanup() {
            Ok(()) => error,
            Err(cleanup) => cleanup,
        }
    }
}
impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::{
        ffi::{CString, c_char, c_int},
        fs::{File, OpenOptions},
        os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    const O_DIRECTORY: c_int = 0o200000;
    const O_NOFOLLOW: c_int = 0o400000;
    const AT_FDCWD: c_int = -100;
    const AT_SYMLINK_FOLLOW: c_int = 0x400;
    static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);
    unsafe extern "C" {
        fn linkat(
            old_directory: c_int,
            old_name: *const c_char,
            new_directory: c_int,
            new_name: *const c_char,
            flags: c_int,
        ) -> c_int;
    }

    pub(super) struct Staging {
        parent: File,
        temporary: PathBuf,
        destination: CString,
        pub(super) file: Option<WriteFile>,
    }
    impl Staging {
        pub(super) fn create(root: &Path, name: &str, buffer_bytes: usize) -> io::Result<Self> {
            let mut parts = name.split('/');
            if name.is_empty()
                || name.contains(['\\', '\0', ':'])
                || parts
                    .clone()
                    .any(|part| part.is_empty() || part == "." || part == "..")
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid contained file path",
                ));
            }
            let destination = CString::new(parts.next_back().ok_or(io::ErrorKind::InvalidInput)?)?;
            let open_directory = |path: &Path| {
                OpenOptions::new()
                    .read(true)
                    .custom_flags(O_DIRECTORY | O_NOFOLLOW)
                    .open(path)
            };
            let mut parent = open_directory(root)?;
            for part in parts {
                let path = descriptor_path(parent.as_raw_fd()).join(part);
                match std::fs::create_dir(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
                parent = open_directory(&path)?;
            }
            for _ in 0..128 {
                let number = NEXT_TEMPORARY
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |number| {
                        number.checked_add(1)
                    })
                    .map_err(|_| io::Error::other("staged file names exhausted"))?;
                let temporary = descriptor_path(parent.as_raw_fd())
                    .join(format!(".qa-download-{}-{number}", std::process::id()));
                match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .custom_flags(O_NOFOLLOW)
                    .open(&temporary)
                {
                    Ok(file) => {
                        return Ok(Self {
                            parent,
                            temporary,
                            destination,
                            file: Some(WriteFile::from_file(file, buffer_bytes)),
                        });
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
            }
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "staged file name collision limit",
            ))
        }

        pub(super) fn publish(&mut self) -> io::Result<()> {
            let writer = self.file.as_mut().ok_or(io::ErrorKind::BrokenPipe)?;
            writer.flush()?;
            let source = CString::new(format!(
                "/proc/self/fd/{}",
                writer.file.get_ref().as_raw_fd()
            ))?;
            // SAFETY: Both descriptors remain owned and open throughout linkat.
            // The procfd source resolves the exact retained regular file inode.
            // The destination is one validated component below the retained
            // parent; linkat does not replace any existing destination.
            let result = unsafe {
                linkat(
                    AT_FDCWD,
                    source.as_ptr(),
                    self.parent.as_raw_fd(),
                    self.destination.as_ptr(),
                    AT_SYMLINK_FOLLOW,
                )
            };
            if result == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }

        pub(super) fn cleanup(&mut self) -> io::Result<()> {
            self.file.take();
            match std::fs::remove_file(&self.temporary) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            }
        }
    }

    fn descriptor_path(descriptor: c_int) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{descriptor}"))
    }
}
