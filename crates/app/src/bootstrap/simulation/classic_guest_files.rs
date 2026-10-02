//! Contained writable files for the classic native guest.
//!
//! Port of donor `src/app/bootstrap/simulation/classic-guest-files.ts`
//! (`classicGuestFiles`).

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::rc::Rc;

use qa_guest::runtime::windows::contracts::{WindowsFile, WindowsFileOpener, WindowsOpenOptions};
use qa_platform::files::contained::{contained_file_parts, open_contained_parent};

const O_NOFOLLOW: i32 = 0o400000;
const O_NONBLOCK: i32 = 0o4000;
/// Linux `ELOOP`; `ErrorKind::FilesystemLoop` is still unstable.
const ELOOP: i32 = 40;

/// Errors that fall through to the mounted-resource lookup, mirroring the
/// donor's `ENOENT`/`EEXIST`/`EACCES`/`ELOOP`/`ENOTDIR` catch.
fn is_fallback_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound
            | std::io::ErrorKind::AlreadyExists
            | std::io::ErrorKind::PermissionDenied
            | std::io::ErrorKind::NotADirectory
    ) || error.raw_os_error() == Some(ELOOP)
}

fn normalize_name(path: &str) -> String {
    let mut name = path.replace('\\', "/");
    while let Some(rest) = name.strip_prefix("./") {
        name = rest.to_string();
    }
    name
}

fn check_range(offset: usize, length: usize) {
    if offset.checked_add(length).is_none() {
        panic!("Invalid native file range");
    }
}

/// DLL CRT files are relative to the selected writable game directory, never
/// the installation.
#[must_use]
pub fn classic_guest_files(root: PathBuf, resources: HashMap<String, Vec<u8>>) -> WindowsFileOpener {
    Rc::new(move |path: &str, options: WindowsOpenOptions| {
        let name = normalize_name(path);
        if let Err(error) = contained_file_parts(&name) {
            panic!("Invalid native file path {name}: {error}");
        }
        if options.creation < 1 || options.creation > 5 {
            panic!("Invalid native file creation disposition");
        }
        if !options.write && options.creation != 3 {
            return None;
        }
        match open_real_file(&root, &name, options) {
            Ok(file) => file,
            Err(error) => {
                if !is_fallback_error(&error) {
                    panic!("Native file open failed for {name}: {error}");
                }
                if options.write || options.creation != 3 {
                    return None;
                }
                let bytes = resources.get(&name)?;
                Some(Box::new(ResourceFile::new(bytes.clone())) as Box<dyn WindowsFile>)
            }
        }
    })
}

fn open_real_file(
    root: &std::path::Path,
    name: &str,
    options: WindowsOpenOptions,
) -> std::io::Result<Option<Box<dyn WindowsFile>>> {
    let parent = match open_contained_parent(
        root,
        name,
        options.write && options.creation != 3 && options.creation != 5,
    ) {
        Ok(parent) => parent,
        Err(qa_platform::error::Error::Io(error)) => return Err(error),
        Err(error) => panic!("Native file containment failed for {name}: {error}"),
    };
    let mut open = OpenOptions::new();
    if options.read {
        open.read(true);
    }
    if options.write {
        open.write(true);
    }
    match options.creation {
        1 => {
            open.create_new(true);
        }
        2 => {
            open.create(true).truncate(true);
        }
        4 => {
            open.create(true);
        }
        5 => {
            open.truncate(true);
        }
        _ => {}
    }
    open.custom_flags(O_NOFOLLOW | O_NONBLOCK).mode(0o600);
    let path = format!("/proc/self/fd/{}/{}", parent.descriptor.as_raw_fd(), parent.leaf);
    let file = open.open(&path)?;
    if !file.metadata()?.is_file() {
        return Ok(None);
    }
    Ok(Some(Box::new(RealFile::new(file, options.read, options.write))))
}

/// Writable file backed by the contained game directory.
struct RealFile {
    file: Option<File>,
    read: bool,
    write: bool,
}

impl RealFile {
    fn new(file: File, read: bool, write: bool) -> Self {
        Self {
            file: Some(file),
            read,
            write,
        }
    }

    fn live(&mut self) -> &File {
        self.file.as_ref().expect("Native file is closed")
    }
}

impl WindowsFile for RealFile {
    fn read(&mut self, offset: usize, length: usize) -> Vec<u8> {
        if !self.read {
            panic!("Native file is not readable");
        }
        check_range(offset, length);
        let mut bytes = vec![0u8; length];
        let mut done = 0usize;
        while done < length {
            match self.live().read_at(&mut bytes[done..], (offset + done) as u64) {
                Ok(0) => break,
                Ok(read) => done += read,
                Err(error) => panic!("Native file read failed: {error}"),
            }
        }
        bytes.truncate(done);
        bytes
    }

    fn write(&mut self, offset: usize, bytes: &[u8]) -> usize {
        if !self.write {
            panic!("Native file is not writable");
        }
        check_range(offset, bytes.len());
        self.live()
            .write_at(bytes, offset as u64)
            .unwrap_or_else(|error| panic!("Native file write failed: {error}"))
    }

    fn size(&mut self) -> usize {
        self.live()
            .metadata()
            .unwrap_or_else(|error| panic!("Native file stat failed: {error}"))
            .len() as usize
    }

    fn truncate(&mut self, length: usize) {
        if !self.write {
            panic!("Native file is not writable");
        }
        check_range(length, 0);
        self.live()
            .set_len(length as u64)
            .unwrap_or_else(|error| panic!("Native file truncate failed: {error}"));
    }

    fn flush(&mut self) {
        self.live()
            .sync_all()
            .unwrap_or_else(|error| panic!("Native file flush failed: {error}"));
    }

    fn close(&mut self) {
        self.file = None;
    }
}

/// Read-only mounted resource fallback.
struct ResourceFile {
    bytes: Vec<u8>,
    closed: bool,
}

impl ResourceFile {
    fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, closed: false }
    }

    fn check(&self) {
        if self.closed {
            panic!("Native file is closed");
        }
    }
}

impl WindowsFile for ResourceFile {
    fn read(&mut self, offset: usize, length: usize) -> Vec<u8> {
        self.check();
        check_range(offset, length);
        let start = offset.min(self.bytes.len());
        let end = offset.saturating_add(length).min(self.bytes.len());
        self.bytes[start..end].to_vec()
    }

    fn write(&mut self, _offset: usize, _bytes: &[u8]) -> usize {
        panic!("Native mounted file is read-only");
    }

    fn size(&mut self) -> usize {
        self.check();
        self.bytes.len()
    }

    fn truncate(&mut self, _length: usize) {
        panic!("Native mounted file is read-only");
    }

    fn flush(&mut self) {
        self.check();
    }

    fn close(&mut self) {
        self.closed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(case: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("qa-sim-guest-files-{}-{}", std::process::id(), case));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp root");
        root
    }

    fn options(read: bool, write: bool, creation: u32) -> WindowsOpenOptions {
        WindowsOpenOptions { read, write, creation }
    }

    #[test]
    fn creates_writes_and_reads_contained_file() {
        let root = temp_root("rw");
        let open = classic_guest_files(root.clone(), HashMap::new());
        let mut file = open("saves/game.sav", options(true, true, 2)).expect("file");
        assert_eq!(file.write(0, b"hello"), 5);
        file.flush();
        assert_eq!(file.size(), 5);
        assert_eq!(file.read(0, 5), b"hello");
        file.truncate(3);
        assert_eq!(file.read(0, 9), b"hel");
        file.close();
        assert!(root.join("saves/game.sav").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_only_open_misses_disk_but_hits_resources() {
        let root = temp_root("res");
        let mut resources = HashMap::new();
        resources.insert("pak/data.bin".to_string(), vec![1, 2, 3, 4]);
        let open = classic_guest_files(root.clone(), resources);
        assert!(open("pak/missing.bin", options(true, false, 3)).is_none());
        let mut file = open("pak/data.bin", options(true, false, 3)).expect("resource");
        assert_eq!(file.size(), 4);
        assert_eq!(file.read(1, 9), vec![2, 3, 4]);
        file.close();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn creation_dispositions_match_source() {
        let root = temp_root("disp");
        let open = classic_guest_files(root.clone(), HashMap::new());
        let mut first = open("a.txt", options(true, true, 1)).expect("create-new");
        first.close();
        assert!(open("a.txt", options(true, true, 1)).is_none());
        let mut trunc = open("a.txt", options(false, true, 5)).expect("truncate");
        trunc.close();
        assert!(open("other.txt", options(false, true, 5)).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[should_panic(expected = "Invalid native file creation disposition")]
    fn rejects_bad_disposition() {
        let open = classic_guest_files(PathBuf::from("/tmp"), HashMap::new());
        let _ = open("a.txt", options(true, true, 9));
    }

    #[test]
    #[should_panic(expected = "Invalid native file path")]
    fn rejects_escape() {
        let open = classic_guest_files(PathBuf::from("/tmp"), HashMap::new());
        let _ = open("../evil.txt", options(true, true, 2));
    }

    #[test]
    #[should_panic(expected = "Native mounted file is read-only")]
    fn resource_files_reject_writes() {
        let root = temp_root("ro");
        let mut resources = HashMap::new();
        resources.insert("r.bin".to_string(), vec![9]);
        let open = classic_guest_files(root.clone(), resources);
        let mut file = open("r.bin", options(true, false, 3)).expect("resource");
        let _ = file.write(0, b"x");
    }
}
