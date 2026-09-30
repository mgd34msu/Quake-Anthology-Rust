//! Filesystem traps over mounted content and the writable user store.
//!
//! Provenance: `src/compat/qvm/file-syscalls.ts` (Q3
//! `sv_game.c`/`cl_cgame.c`/`cl_ui.c` filesystem traps over the selected
//! mounted content). [`FileMounts`] and [`WritableStore`] are local mirrors
//! of the `MountedContent`/`UserFileStore` surfaces the donor consumes.
//! Donor promises become direct returns; read-handle bytes stay owned by the
//! handle so seeks and restores observe them without re-opening.

use std::collections::BTreeMap;

use super::client_state::{CallKind, HostCall, QvmRole, SyscallMemory};
use super::legacy_bot_abi::{
    CG_FS_FOPENFILE, CG_FS_SEEK, G_FS_FOPEN_FILE, G_FS_GETFILELIST, G_FS_SEEK, UI_FS_FOPENFILE, UI_FS_GETFILELIST,
    UI_FS_SEEK,
};
use crate::error::GuestError;

/// Highest guest file slot; slots run 1 through 63.
pub const QVM_FILE_SLOT_MAX: i32 = 63;

/// Opened read resource: bytes plus archive provenance for seeks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedFile {
    /// File bytes.
    pub bytes: Vec<u8>,
    /// Whether the bytes come from a pk3 archive (ZIP seek rules apply).
    pub pk3: bool,
}

/// Host read-mount surface.
pub trait FileMounts {
    /// Open a file for reading.
    fn open(&mut self, path: &str) -> Option<OpenedFile>;
    /// List files under a path with an extension filter.
    fn list_files(&mut self, path: &str, extension: &str) -> Vec<String>;
}

/// Writable open mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    /// Truncate and write.
    Write,
    /// Append.
    Append,
    /// Append with synchronous flushes.
    AppendSync,
}

/// Seek origin: current, end, or set (Q3 `FS_SEEK_CUR`/`END`/`SET` order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekOrigin {
    /// Relative to the cursor.
    Current,
    /// Relative to the end.
    End,
    /// Absolute.
    Set,
}

/// Host writable-file handle.
pub trait WritableFile {
    /// Write bytes, returning the count written.
    fn write(&mut self, bytes: &[u8]) -> usize;
    /// Seek, returning the donor's status code.
    fn seek(&mut self, offset: i32, origin: SeekOrigin) -> i32;
    /// Current cursor position.
    fn position(&self) -> usize;
    /// File path (for checkpoints).
    fn path(&self) -> &str;
    /// Open mode (for checkpoints).
    fn mode(&self) -> WriteMode;
}

/// Host writable-file store.
pub trait WritableStore {
    /// Open a writable file, or `None` when it cannot be created.
    fn open_file(&mut self, path: &str, mode: WriteMode) -> Option<Box<dyn WritableFile>>;
    /// Resume a checkpointed writable file.
    fn resume_file(&mut self, path: &str, mode: WriteMode, position: usize) -> Box<dyn WritableFile>;
}

enum FileHandle {
    Read { bytes: Vec<u8>, position: usize, pk3: bool },
    Write { file: Box<dyn WritableFile> },
}

/// Checkpointed read handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadHandleCheckpoint {
    /// File bytes.
    pub bytes: Vec<u8>,
    /// Cursor position.
    pub position: usize,
    /// Archive provenance.
    pub pk3: bool,
}

/// Checkpointed write handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteHandleCheckpoint {
    /// File path.
    pub path: String,
    /// Open mode.
    pub mode: WriteMode,
    /// Cursor position.
    pub position: usize,
}

/// Checkpointed handle slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandleCheckpoint {
    /// Read handle.
    Read(ReadHandleCheckpoint),
    /// Write handle.
    Write(WriteHandleCheckpoint),
}

/// Checkpoint of every open handle.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FilesCheckpoint {
    /// Slot/checkpoint pairs.
    pub handles: Vec<(i32, HandleCheckpoint)>,
}

/// Guest file-handle owner for one module lifetime.
pub struct QvmFiles {
    mounts: Box<dyn FileMounts>,
    writable: Option<Box<dyn WritableStore>>,
    print: Box<dyn FnMut(&str)>,
    handles: BTreeMap<i32, FileHandle>,
    closed: bool,
}

impl QvmFiles {
    /// Create an owner over `mounts` with an optional writable store.
    pub fn new(
        mounts: Box<dyn FileMounts>,
        writable: Option<Box<dyn WritableStore>>,
        print: impl FnMut(&str) + 'static,
    ) -> Self {
        Self {
            mounts,
            writable,
            print: Box::new(print),
            handles: BTreeMap::new(),
            closed: false,
        }
    }

    /// Reject operations on a closed owner.
    pub fn assert_current(&self) -> Result<(), GuestError> {
        if self.closed {
            return Err(GuestError::runtime("QVM files have been closed"));
        }
        Ok(())
    }

    /// Close every handle and retire the owner.
    pub fn close_all(&mut self) {
        self.closed = true;
        self.handles.clear();
    }

    /// Capture a checkpoint of every open handle.
    pub fn capture_checkpoint(&self) -> Result<FilesCheckpoint, GuestError> {
        self.assert_current()?;
        let mut handles = Vec::new();
        for (slot, handle) in &self.handles {
            let checkpoint = match handle {
                FileHandle::Read { bytes, position, pk3 } => HandleCheckpoint::Read(ReadHandleCheckpoint {
                    bytes: bytes.clone(),
                    position: *position,
                    pk3: *pk3,
                }),
                FileHandle::Write { file } => HandleCheckpoint::Write(WriteHandleCheckpoint {
                    path: file.path().to_string(),
                    mode: file.mode(),
                    position: file.position(),
                }),
            };
            handles.push((*slot, checkpoint));
        }
        Ok(FilesCheckpoint { handles })
    }

    /// Restore into an empty owner, validating slots and archive cursors.
    pub fn restore_checkpoint(&mut self, checkpoint: &FilesCheckpoint) -> Result<(), GuestError> {
        self.assert_current()?;
        if !self.handles.is_empty() {
            return Err(GuestError::runtime("QVM file restore requires an empty owner"));
        }
        let mut seen = std::collections::HashSet::new();
        for (slot, handle) in &checkpoint.handles {
            if *slot < 1 || *slot > QVM_FILE_SLOT_MAX || !seen.insert(slot) {
                return Err(GuestError::invalid("invalid or duplicate file handle slot"));
            }
            if let HandleCheckpoint::Read(read) = handle {
                if read.pk3 && read.position > read.bytes.len() {
                    return Err(GuestError::invalid("ZIP cursor exceeds opened bytes"));
                }
            }
        }
        for (slot, handle) in &checkpoint.handles {
            match handle {
                HandleCheckpoint::Read(read) => {
                    self.handles.insert(
                        *slot,
                        FileHandle::Read {
                            bytes: read.bytes.clone(),
                            position: read.position,
                            pk3: read.pk3,
                        },
                    );
                }
                HandleCheckpoint::Write(write) => {
                    let writable = self
                        .writable
                        .as_mut()
                        .ok_or_else(|| GuestError::runtime("QVM filesystem has no writable owner"))?;
                    let file = writable.resume_file(&write.path, write.mode, write.position);
                    self.handles.insert(*slot, FileHandle::Write { file });
                }
            }
        }
        Ok(())
    }

    fn free_slot(&self) -> Result<i32, GuestError> {
        let mut slot = 1;
        while self.handles.contains_key(&slot) && slot < 64 {
            slot += 1;
        }
        if slot == 64 {
            return Err(GuestError::runtime("FS_HandleForFile: none free"));
        }
        Ok(slot)
    }

    fn handle(&mut self, slot: i32) -> Result<&mut FileHandle, GuestError> {
        self.assert_current()?;
        if !(1..=QVM_FILE_SLOT_MAX).contains(&slot) {
            return Err(GuestError::runtime("FS_FileForHandle: out of range"));
        }
        self.handles
            .get_mut(&slot)
            .ok_or_else(|| GuestError::runtime("FS_FileForHandle: NULL"))
    }

    /// Open a writable file, publishing its slot.
    pub fn open_write(&mut self, path: &str, mode: WriteMode, publish: &mut dyn FnMut(i32)) -> Result<i32, GuestError> {
        self.assert_current()?;
        if self.writable.is_none() {
            return Err(GuestError::runtime("QVM filesystem has no writable owner"));
        }
        let slot = self.free_slot()?;
        let normalized = path.replace('\\', "/");
        let file = self
            .writable
            .as_mut()
            .expect("checked store")
            .open_file(&normalized, mode);
        self.assert_current()?;
        match file {
            None => {
                publish(0);
                Ok(-1)
            }
            Some(file) => {
                self.handles.insert(slot, FileHandle::Write { file });
                publish(slot);
                Ok(0)
            }
        }
    }

    /// Write bytes to a slot.
    pub fn write(&mut self, slot: i32, bytes: &[u8]) -> Result<i32, GuestError> {
        self.assert_current()?;
        if slot == 0 {
            return Ok(0);
        }
        if matches!(self.handle(slot)?, FileHandle::Read { .. }) {
            (self.print)("FS_Write: 0 bytes written\n");
            self.assert_current()?;
            return Ok(0);
        }
        let FileHandle::Write { file } = self.handle(slot)? else {
            unreachable!("checked writable handle");
        };
        let written = file.write(bytes);
        self.assert_current()?;
        Ok(written as i32)
    }

    /// Open a file for reading, publishing its slot (or probing when `publish` is `None`).
    pub fn open(&mut self, path: &str, publish: Option<&mut dyn FnMut(i32)>) -> Result<i32, GuestError> {
        self.assert_current()?;
        let mut path = path.to_string();
        if path.starts_with('/') || path.starts_with('\\') {
            path.remove(0);
        }
        if path.contains("..") || path.contains("::") || path.contains("q3key") {
            if let Some(publish) = publish {
                publish(0);
                return Ok(-1);
            }
            return Ok(0);
        }
        let resource = self.mounts.open(&path);
        self.assert_current()?;
        let Some(publish) = publish else {
            return Ok(i32::from(resource.is_some()));
        };
        match resource {
            None => {
                publish(0);
                Ok(-1)
            }
            Some(resource) => {
                let slot = self.free_slot()?;
                let length = resource.bytes.len() as i32;
                self.handles.insert(
                    slot,
                    FileHandle::Read {
                        bytes: resource.bytes,
                        position: 0,
                        pk3: resource.pk3,
                    },
                );
                publish(slot);
                Ok(length)
            }
        }
    }

    /// Read up to `len` bytes from a slot.
    pub fn read(&mut self, slot: i32, len: usize) -> Result<Vec<u8>, GuestError> {
        self.assert_current()?;
        if slot == 0 {
            return Ok(Vec::new());
        }
        let handle = self.handle(slot)?;
        let FileHandle::Read { bytes, position, .. } = handle else {
            return Err(GuestError::runtime("FS_Read: file is not readable"));
        };
        let count = len.min(bytes.len().saturating_sub(*position));
        let out = bytes[*position..*position + count].to_vec();
        *position += count;
        Ok(out)
    }

    /// Close a slot (slot 0 and missing slots are no-ops).
    pub fn close(&mut self, slot: i32) -> Result<(), GuestError> {
        self.assert_current()?;
        if slot == 0 {
            return Ok(());
        }
        if !(1..=QVM_FILE_SLOT_MAX).contains(&slot) {
            return Err(GuestError::runtime("FS_FileForHandle: out of range"));
        }
        self.handles.remove(&slot);
        Ok(())
    }

    /// Seek a slot.
    pub fn seek(&mut self, slot: i32, offset: i32, origin: i32) -> Result<i32, GuestError> {
        let origin = match origin {
            0 => SeekOrigin::Current,
            1 => SeekOrigin::End,
            2 => SeekOrigin::Set,
            _ => return Err(GuestError::runtime("Bad origin in FS_Seek\n")),
        };
        let pk3 = match self.handle(slot)? {
            FileHandle::Write { file } => return Ok(file.seek(offset, origin)),
            FileHandle::Read { pk3, .. } => *pk3,
        };
        if pk3 {
            if offset >= 65536 {
                return Err(GuestError::runtime("ZIP FILE FSEEK NOT YET IMPLEMENTED\n"));
            }
            if offset < 0 {
                return Err(GuestError::invalid(
                    "Negative ZIP seek exceeds the source scratch buffer",
                ));
            }
            let FileHandle::Read { bytes, position, .. } = self.handle(slot)? else {
                unreachable!("checked read handle");
            };
            *position = (offset as usize).min(bytes.len());
            return Ok(if offset == 0 && origin == SeekOrigin::Set {
                0
            } else {
                *position as i32
            });
        }
        // Streamed read handles perform the seek twice; only SEEK_CUR accumulates.
        let mut result = 0;
        for _ in 0..2 {
            let FileHandle::Read { bytes, position, .. } = self.handle(slot)? else {
                unreachable!("checked read handle");
            };
            let next = match origin {
                SeekOrigin::Current => *position as i64 + i64::from(offset),
                SeekOrigin::End => bytes.len() as i64 + i64::from(offset),
                SeekOrigin::Set => i64::from(offset),
            };
            if next < 0 {
                result = -1;
            } else {
                *position = next as usize;
                result = 0;
            }
        }
        Ok(result)
    }

    /// List files under a path.
    pub fn list(&mut self, path: &str, extension: &str) -> Result<Vec<String>, GuestError> {
        self.assert_current()?;
        if path.to_lowercase() == "$modlist" {
            return Err(GuestError::runtime(
                "QVM $modlist requires an installed-mod catalog owner",
            ));
        }
        let names = self.mounts.list_files(path, extension);
        self.assert_current()?;
        Ok(names)
    }
}

/// Dispatch a filesystem trap. Returns `Ok(None)` when unhandled.
pub fn file_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    files: &mut QvmFiles,
) -> Result<Option<i32>, GuestError> {
    if call.kind != CallKind::Engine {
        return Ok(None);
    }
    let (open, seek, list) = match call.role {
        QvmRole::Ui => (UI_FS_FOPENFILE, UI_FS_SEEK, Some(UI_FS_GETFILELIST)),
        QvmRole::Qagame => (G_FS_FOPEN_FILE, G_FS_SEEK, Some(G_FS_GETFILELIST)),
        QvmRole::Cgame => (CG_FS_FOPENFILE, CG_FS_SEEK, None),
    };
    let trap = call.code;
    if trap != open && trap != open + 1 && trap != open + 2 && trap != open + 3 && trap != seek && Some(trap) != list {
        return Ok(None);
    }
    files.assert_current()?;
    if trap == open {
        let path_word = call.int(1)?;
        let destination = call.int(2)?;
        let mode = call.int(3)?;
        if !(0..=3).contains(&mode) {
            return Err(GuestError::runtime("FSH_FOpenFile: bad mode"));
        }
        let game = call.role == QvmRole::Qagame;
        if destination != 0 {
            memory.span(destination, 4, 0)?;
        }
        if game && destination == 0 && mode != 0 {
            return Err(GuestError::invalid(
                "Writable file open requires a nonnull handle pointer",
            ));
        }
        if path_word == 0 {
            return Err(GuestError::runtime(
                "FS_FOpenFileRead: NULL 'filename' parameter passed\n",
            ));
        }
        if !game && destination != 0 {
            memory.span(destination, 4, 0)?;
        }
        if mode == 0 {
            if destination == 0 {
                return Ok(Some(files.open(&memory.read_string(path_word)?, None)?));
            }
            let path = memory.read_string(path_word)?;
            let base = memory.pointer(destination).expect("checked span");
            return Ok(Some(files.open(
                &path,
                Some(&mut |slot| {
                    memory.write_i32(base, slot).expect("checked span");
                }),
            )?));
        }
        if destination == 0 {
            return Err(GuestError::invalid(
                "Writable file open requires a nonnull handle pointer",
            ));
        }
        let path = memory.read_string(path_word)?;
        let base = memory.pointer(destination).expect("checked span");
        let mode = match mode {
            1 => WriteMode::Write,
            2 => WriteMode::Append,
            _ => WriteMode::AppendSync,
        };
        return Ok(Some(files.open_write(&path, mode, &mut |slot| {
            memory.write_i32(base, slot).expect("checked span");
        })?));
    }
    if trap == open + 1 || trap == open + 2 {
        let word = call.int(1)?;
        let length = call.int(2)?;
        let slot = call.int(3)?;
        if slot == 0 {
            return Ok(Some(0));
        }
        if length < 0 {
            return Err(GuestError::invalid("QVM file buffer has an invalid length"));
        }
        if trap == open + 2 {
            let bytes = if word == 0 && length == 0 {
                Vec::new()
            } else {
                let range = memory.span(word, length as usize, 0)?;
                memory.read_bytes(range.start, length as usize)?.to_vec()
            };
            files.write(slot, &bytes)?;
        } else {
            let bytes = if word == 0 && length == 0 {
                Vec::new()
            } else {
                memory.span(word, length as usize, 0)?;
                files.read(slot, length as usize)?
            };
            if !bytes.is_empty() {
                let range = memory.span(word, bytes.len(), 0)?;
                memory.write_bytes(range.start, &bytes)?;
            }
        }
        return Ok(Some(0));
    }
    if trap == open + 3 {
        files.close(call.int(1)?)?;
        return Ok(Some(0));
    }
    if trap == seek {
        return Ok(Some(files.seek(call.int(1)?, call.int(2)?, call.int(3)?)?));
    }
    let path = memory.read_string(call.int(1)?)?;
    let extension = memory.read_string(call.int(2)?)?;
    let word = call.int(3)?;
    let length = call.int(4)?;
    if length <= 0 {
        return Err(GuestError::invalid("File listing requires a nonempty destination"));
    }
    memory.span(word, length as usize, 0)?;
    let names = files.list(&path, &extension)?;
    files.assert_current()?;
    let base = memory.pointer(word).expect("checked span");
    memory.set(base, 0)?;
    let mut offset = 0usize;
    let mut count = 0i32;
    for name in &names {
        if offset + name.len() + 2 >= length as usize {
            break;
        }
        for byte in name.bytes() {
            memory.set(base + offset, byte)?;
            offset += 1;
        }
        memory.set(base + offset, 0)?;
        offset += 1;
        count += 1;
    }
    Ok(Some(count))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeMounts {
        files: BTreeMap<String, OpenedFile>,
        log: Vec<String>,
    }

    impl FileMounts for FakeMounts {
        fn open(&mut self, path: &str) -> Option<OpenedFile> {
            self.log.push(format!("open {path}"));
            self.files.get(path).cloned()
        }
        fn list_files(&mut self, path: &str, extension: &str) -> Vec<String> {
            self.log.push(format!("list {path} {extension}"));
            vec!["a.dm3".to_string(), "b.dm3".to_string()]
        }
    }

    struct FakeWritable {
        path: String,
        mode: WriteMode,
        bytes: Vec<u8>,
        position: usize,
    }

    impl WritableFile for FakeWritable {
        fn write(&mut self, bytes: &[u8]) -> usize {
            if self.position + bytes.len() > self.bytes.len() {
                self.bytes.resize(self.position + bytes.len(), 0);
            }
            self.bytes[self.position..self.position + bytes.len()].copy_from_slice(bytes);
            self.position += bytes.len();
            bytes.len()
        }
        fn seek(&mut self, offset: i32, origin: SeekOrigin) -> i32 {
            let next = match origin {
                SeekOrigin::Current => self.position as i64 + i64::from(offset),
                SeekOrigin::End => self.bytes.len() as i64 + i64::from(offset),
                SeekOrigin::Set => i64::from(offset),
            };
            if next < 0 {
                return -1;
            }
            self.position = next as usize;
            0
        }
        fn position(&self) -> usize {
            self.position
        }
        fn path(&self) -> &str {
            &self.path
        }
        fn mode(&self) -> WriteMode {
            self.mode
        }
    }

    struct FakeStore {
        log: Vec<String>,
    }

    impl WritableStore for FakeStore {
        fn open_file(&mut self, path: &str, mode: WriteMode) -> Option<Box<dyn WritableFile>> {
            self.log.push(format!("open {path} {mode:?}"));
            if path == "denied" {
                return None;
            }
            Some(Box::new(FakeWritable {
                path: path.to_string(),
                mode,
                bytes: Vec::new(),
                position: 0,
            }))
        }
        fn resume_file(&mut self, path: &str, mode: WriteMode, position: usize) -> Box<dyn WritableFile> {
            self.log.push(format!("resume {path}"));
            Box::new(FakeWritable {
                path: path.to_string(),
                mode,
                bytes: vec![0u8; position],
                position,
            })
        }
    }

    fn files() -> QvmFiles {
        let mounts = FakeMounts {
            files: BTreeMap::from([
                (
                    "maps/a.bsp".to_string(),
                    OpenedFile {
                        bytes: b"bspdata".to_vec(),
                        pk3: false,
                    },
                ),
                (
                    "maps/z.bsp".to_string(),
                    OpenedFile {
                        bytes: b"zip".to_vec(),
                        pk3: true,
                    },
                ),
            ]),
            log: Vec::new(),
        };
        QvmFiles::new(Box::new(mounts), Some(Box::new(FakeStore { log: Vec::new() })), |_| {})
    }

    fn game(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(
            QvmRole::Qagame,
            code,
            args,
            super::super::client_state::AbiProfile::Modern,
        )
    }

    #[test]
    fn open_read_close_round_trip() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        memory.write_string(512, "maps/a.bsp", 11).unwrap();
        let mut files = files();
        assert_eq!(
            file_syscall(&game(10, &[512, 256, 0]), &mut memory, &mut files).unwrap(),
            Some(7)
        );
        assert_eq!(memory.read_i32(256).unwrap(), 1);
        assert_eq!(
            file_syscall(&game(11, &[1024, 4, 1]), &mut memory, &mut files).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_bytes(1024, 4).unwrap(), b"bspd");
        assert_eq!(
            file_syscall(&game(11, &[1024, 4, 1]), &mut memory, &mut files).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_bytes(1024, 3).unwrap(), b"ata");
        assert_eq!(file_syscall(&game(13, &[1]), &mut memory, &mut files).unwrap(), Some(0));
        assert!(file_syscall(&game(11, &[1024, 4, 1]), &mut memory, &mut files).is_err());
    }

    #[test]
    fn open_rejects_and_probes() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        memory.write_string(512, "maps/a.bsp", 11).unwrap();
        memory.write_string(1024, "maps/missing.bsp", 17).unwrap();
        memory.write_string(2048, "../evil", 7).unwrap();
        let mut files = files();
        assert_eq!(
            file_syscall(&game(10, &[1024, 256, 0]), &mut memory, &mut files).unwrap(),
            Some(-1)
        );
        assert_eq!(memory.read_i32(256).unwrap(), 0);
        assert_eq!(
            file_syscall(&game(10, &[512, 0, 0]), &mut memory, &mut files).unwrap(),
            Some(1)
        );
        assert_eq!(
            file_syscall(&game(10, &[1024, 0, 0]), &mut memory, &mut files).unwrap(),
            Some(0)
        );
        assert_eq!(
            file_syscall(&game(10, &[2048, 256, 0]), &mut memory, &mut files).unwrap(),
            Some(-1)
        );
        assert!(file_syscall(&game(10, &[0, 256, 0]), &mut memory, &mut files).is_err());
        assert!(file_syscall(&game(10, &[512, 256, 9]), &mut memory, &mut files).is_err());
    }

    #[test]
    fn write_and_seek_modes() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        memory.write_string(512, "demos\\x.dm3", 11).unwrap();
        memory.write_bytes(1024, b"hello").unwrap();
        let mut files = files();
        assert_eq!(
            file_syscall(&game(10, &[512, 256, 1]), &mut memory, &mut files).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_i32(256).unwrap(), 1);
        assert_eq!(
            file_syscall(&game(12, &[1024, 5, 1]), &mut memory, &mut files).unwrap(),
            Some(0)
        );
        assert_eq!(
            file_syscall(&game(45, &[1, 0, 2]), &mut memory, &mut files).unwrap(),
            Some(0)
        );
        assert_eq!(
            file_syscall(&game(12, &[0, 0, 1]), &mut memory, &mut files).unwrap(),
            Some(0)
        );
        assert_eq!(
            file_syscall(&game(12, &[1024, 5, 0]), &mut memory, &mut files).unwrap(),
            Some(0)
        );
        assert!(file_syscall(&game(10, &[512, 0, 1]), &mut memory, &mut files).is_err());
        memory.write_string(512, "denied", 7).unwrap();
        assert_eq!(
            file_syscall(&game(10, &[512, 256, 2]), &mut memory, &mut files).unwrap(),
            Some(-1)
        );
        assert_eq!(memory.read_i32(256).unwrap(), 0);
    }

    #[test]
    fn write_to_read_handle_prints_zero() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        memory.write_string(512, "maps/a.bsp", 11).unwrap();
        let mut files = files();
        assert_eq!(
            file_syscall(&game(10, &[512, 256, 0]), &mut memory, &mut files).unwrap(),
            Some(7)
        );
        assert_eq!(
            file_syscall(&game(12, &[1024, 4, 1]), &mut memory, &mut files).unwrap(),
            Some(0)
        );
        assert!(files.read(1, 4).is_ok());
    }

    #[test]
    fn streamed_seek_doubles_current() {
        let mut files = files();
        let mut slot = 0;
        assert_eq!(files.open("maps/a.bsp", Some(&mut |value| slot = value)).unwrap(), 7);
        assert_eq!(slot, 1);
        assert_eq!(files.seek(1, 2, 0).unwrap(), 0);
        assert_eq!(files.read(1, 7).unwrap(), b"ata");
        assert_eq!(files.seek(1, 1, 2).unwrap(), 0);
        assert_eq!(files.read(1, 1).unwrap(), b"s");
        assert_eq!(files.seek(1, -100, 2).unwrap(), -1);
        assert!(files.seek(1, 0, 9).is_err());
    }

    #[test]
    fn pk3_seek_rules() {
        let mut files = files();
        let mut slot = 0;
        assert_eq!(files.open("maps/z.bsp", Some(&mut |value| slot = value)).unwrap(), 3);
        assert_eq!(files.seek(slot, 2, 0).unwrap(), 2);
        assert_eq!(files.seek(slot, 0, 2).unwrap(), 0);
        assert!(files.seek(slot, 70000, 0).is_err());
        assert!(files.seek(slot, -1, 0).is_err());
    }

    #[test]
    fn checkpoint_round_trip() {
        let mut source = files();
        let mut slot = 0;
        source.open("maps/a.bsp", Some(&mut |value| slot = value)).unwrap();
        source.read(slot, 2).unwrap();
        let checkpoint = source.capture_checkpoint().unwrap();
        assert_eq!(checkpoint.handles.len(), 1);
        let mut restored = files();
        restored.restore_checkpoint(&checkpoint).unwrap();
        assert_eq!(restored.read(slot, 5).unwrap(), b"pdata");
        assert!(restored.restore_checkpoint(&checkpoint).is_err());
        let bad = FilesCheckpoint {
            handles: vec![
                (
                    1,
                    HandleCheckpoint::Read(ReadHandleCheckpoint {
                        bytes: vec![1],
                        position: 0,
                        pk3: false,
                    }),
                ),
                (
                    1,
                    HandleCheckpoint::Read(ReadHandleCheckpoint {
                        bytes: vec![1],
                        position: 0,
                        pk3: false,
                    }),
                ),
            ],
        };
        let mut empty = files();
        assert!(empty.restore_checkpoint(&bad).is_err());
    }

    #[test]
    fn listing_writes_names() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        memory.write_string(512, "demos", 6).unwrap();
        memory.write_string(1024, "dm3", 4).unwrap();
        let mut files = files();
        assert_eq!(
            file_syscall(&game(38, &[512, 1024, 2048, 64]), &mut memory, &mut files).unwrap(),
            Some(2)
        );
        assert_eq!(memory.read_bytes(2048, 12).unwrap(), b"a.dm3\0b.dm3\0");
        assert_eq!(
            file_syscall(&game(38, &[512, 1024, 2048, 8]), &mut memory, &mut files).unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_bytes(2048, 6).unwrap(), b"a.dm3\0");
        assert!(file_syscall(&game(38, &[512, 1024, 2048, 0]), &mut memory, &mut files).is_err());
        memory.write_string(512, "$modlist", 9).unwrap();
        assert!(file_syscall(&game(38, &[512, 1024, 2048, 64]), &mut memory, &mut files).is_err());
    }

    #[test]
    fn role_bases_and_routing() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        memory.write_string(512, "maps/a.bsp", 11).unwrap();
        let mut files = files();
        let profile = super::super::client_state::AbiProfile::Modern;
        let ui = HostCall::engine(QvmRole::Ui, 13, &[512, 256, 0], profile);
        assert_eq!(file_syscall(&ui, &mut memory, &mut files).unwrap(), Some(7));
        let cg = HostCall::engine(QvmRole::Cgame, 10, &[512, 256, 0], profile);
        assert_eq!(file_syscall(&cg, &mut memory, &mut files).unwrap(), Some(7));
        let cg_list = HostCall::engine(QvmRole::Cgame, 38, &[512, 512, 2048, 64], profile);
        assert_eq!(file_syscall(&cg_list, &mut memory, &mut files).unwrap(), None);
        assert_eq!(file_syscall(&game(99, &[]), &mut memory, &mut files).unwrap(), None);
    }
}
