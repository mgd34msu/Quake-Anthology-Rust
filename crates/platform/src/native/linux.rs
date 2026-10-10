use super::{
    NativeAbi, NativeCall, NativeEntry, NativeError, NativeImage, NativeRegion, NativeScalar,
    PAGE_BYTES as PAGE,
};
use std::{
    fs::File,
    io::{self, Read, Write},
    mem::ManuallyDrop,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{net::UnixStream, process::ExitStatusExt},
    },
    process::{Child, Command, Stdio},
    ptr::NonNull,
    sync::atomic::{Ordering, compiler_fence},
    time::{Duration, Instant},
};

const LIMIT: usize = 512 * 1024 * 1024;
const STACK: usize = 8 * 1024 * 1024;
#[path = "x64.rs"]
mod x64;
const START: u8 = 1;
const READY: u8 = 2;
const INVOKE: u8 = 3;
const IMPORT: u8 = 4;
const RETURN: u8 = 5;
const REPLY: u8 = 6;
const MAP: u8 = 7;
const BIND: u8 = 8;
#[path = "runtime/child.rs"]
mod runtime;
const PACKET_BYTES: usize = 200;
const SIGSTOP: i32 = 19;
const SIGCONT: i32 = 18;

pub(super) fn executable_offset(regions: &[NativeRegion], offset: usize) -> bool {
    regions
        .partition_point(|r| r.offset <= offset)
        .checked_sub(1)
        .and_then(|index| regions.get(index))
        .is_some_and(|r| r.permissions & 4 != 0 && offset - r.offset < r.length)
}

unsafe extern "C" {
    fn ppoll(
        fds: *mut PollFd,
        count: usize,
        timeout: *const Timespec,
        mask: *const std::ffi::c_void,
    ) -> i32;
    fn recv(fd: i32, bytes: *mut u8, length: usize, flags: i32) -> isize;
    fn send(fd: i32, bytes: *const u8, length: usize, flags: i32) -> isize;
    fn memfd_create(name: *const std::ffi::c_char, flags: u32) -> i32;
    fn mmap(
        address: *mut std::ffi::c_void,
        length: usize,
        rights: i32,
        flags: i32,
        fd: i32,
        offset: i64,
    ) -> *mut std::ffi::c_void;
    fn munmap(address: *mut std::ffi::c_void, length: usize) -> i32;
    fn mprotect(address: *mut std::ffi::c_void, length: usize, rights: i32) -> i32;
    fn fcntl(fd: i32, operation: i32, ...) -> i32;
    fn kill(pid: i32, signal: i32) -> i32;
    fn getpid() -> i32;
    fn signal(number: i32, handler: usize) -> usize;
    fn waitpid(pid: i32, status: *mut i32, flags: i32) -> i32;
    fn prctl(operation: i32, ...) -> i32;
    fn syscall(number: std::ffi::c_long, ...) -> std::ffi::c_long;
}

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    returned: i16,
}
#[repr(C)]
struct Timespec {
    seconds: i64,
    nanos: i64,
}

struct Mapping {
    address: NonNull<u8>,
    length: usize,
}
impl Mapping {
    fn map(file: &File, length: usize, base: Option<u64>) -> Result<Self, NativeError> {
        let address = base.map_or(std::ptr::null_mut(), |b| b as usize as *mut _);
        // SAFETY: the owned memfd is sealed against resizing and covers length.
        // FIXED_NOREPLACE never replaces the child's controller mappings.
        let result = unsafe {
            mmap(
                address,
                length,
                3,
                1 | if base.is_some() { 0x100000 } else { 0 },
                file.as_raw_fd(),
                0,
            )
        };
        if result as isize == -1 {
            return Err(io::Error::last_os_error().into());
        }
        if base.is_some() && result != address {
            // SAFETY: release a mapping from a kernel lacking NOREPLACE.
            unsafe {
                munmap(result, length);
            }
            return Err(NativeError::Extent);
        }
        let Some(address) = NonNull::new(result.cast()) else {
            // SAFETY: mmap succeeded; release the rejected null mapping.
            unsafe {
                munmap(result, length);
            }
            return Err(NativeError::Extent);
        };
        Ok(Self { address, length })
    }
    fn bytes(&self) -> &[u8] {
        // SAFETY: exposed only before spawn, at an enforced kernel child stop,
        // or after reaping. invoke requires exclusive ownership of this view.
        unsafe { std::slice::from_raw_parts(self.address.as_ptr(), self.length) }
    }
    fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: the same stopped-child invariant holds, and &mut excludes
        // simultaneous controller borrows. The child cannot export the memfd.
        unsafe { std::slice::from_raw_parts_mut(self.address.as_ptr(), self.length) }
    }
}
impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: exactly one mapping owner; its child was reaped before drop.
        unsafe {
            munmap(self.address.as_ptr().cast(), self.length);
        }
    }
}

#[derive(Clone, Copy)]
struct Packet {
    operation: u8,
    abi: u8,
    sequence: u64,
    address: u64,
    arguments: [u64; 13],
    floats: [u64; 8],
    value: u64,
}
// One packet transfer path handles short IO and EINTR. Native invocation uses
// one deadline across every packet; partial traffic cannot restart its budget.
pub(super) fn transfer(
    stream: &mut UnixStream,
    mut bytes: &mut [u8],
    writing: bool,
    deadline: Option<Instant>,
) -> Result<(), NativeError> {
    while !bytes.is_empty() {
        let io = if let Some(deadline) = deadline {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
                .ok_or(NativeError::Timeout)?;
            let mut fd = PollFd {
                fd: stream.as_raw_fd(),
                events: if writing { 4 } else { 1 },
                returned: 0,
            };
            let timeout = Timespec {
                seconds: remaining
                    .as_secs()
                    .try_into()
                    .map_err(|_| NativeError::Extent)?,
                nanos: remaining.subsec_nanos().into(),
            };
            // SAFETY: one live owned socket and initialized stack records.
            // ppoll uses the remaining absolute budget without changing socket
            // options. Readiness is followed by nonblocking IO, so a short
            // fragment or EINTR cannot start another full blocking timeout.
            let ready = unsafe { ppoll(&mut fd, 1, &timeout, std::ptr::null()) };
            if ready == 0 {
                return Err(NativeError::Timeout);
            }
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error.into());
            }
            // SAFETY: the slice remains live and exclusive for recv; send only
            // reads it. MSG_DONTWAIT never changes the descriptor's flags.
            // MSG_NOSIGNAL preserves UnixStream::write's broken-pipe behavior.
            let count = unsafe {
                if writing {
                    send(fd.fd, bytes.as_ptr(), bytes.len(), 0x40 | 0x4000)
                } else {
                    recv(fd.fd, bytes.as_mut_ptr(), bytes.len(), 0x40)
                }
            };
            if count < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(count as usize)
            }
        } else if writing {
            stream.write(bytes)
        } else {
            stream.read(bytes)
        };
        let count = match io {
            Ok(0) => {
                return Err(io::Error::from(if writing {
                    io::ErrorKind::WriteZero
                } else {
                    io::ErrorKind::UnexpectedEof
                })
                .into());
            }
            Ok(count) => count,
            Err(error)
                if error.kind() == io::ErrorKind::Interrupted
                    || (deadline.is_some() && error.kind() == io::ErrorKind::WouldBlock) =>
            {
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        bytes = &mut bytes[count..];
    }
    Ok(())
}
impl Packet {
    fn new(operation: u8, sequence: u64) -> Self {
        Self {
            operation,
            abi: 0,
            sequence,
            address: 0,
            arguments: [0; 13],
            floats: [0; 8],
            value: 0,
        }
    }
    fn send(&self, stream: &mut UnixStream) -> Result<(), NativeError> {
        self.send_before(stream, None)
    }
    fn send_before(
        &self,
        stream: &mut UnixStream,
        deadline: Option<Instant>,
    ) -> Result<(), NativeError> {
        let mut bytes = [0u8; PACKET_BYTES];
        bytes[..4].copy_from_slice(b"QARN");
        bytes[4] = 2;
        bytes[5] = self.operation;
        bytes[6] = self.abi;
        bytes[8..16].copy_from_slice(&self.sequence.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.address.to_le_bytes());
        for (i, word) in self.arguments.iter().enumerate() {
            bytes[24 + i * 8..32 + i * 8].copy_from_slice(&word.to_le_bytes());
        }
        for (i, word) in self.floats.iter().enumerate() {
            bytes[128 + i * 8..136 + i * 8].copy_from_slice(&word.to_le_bytes());
        }
        bytes[192..200].copy_from_slice(&self.value.to_le_bytes());
        compiler_fence(Ordering::Release);
        transfer(stream, &mut bytes, true, deadline)
    }
    fn receive(stream: &mut UnixStream) -> Result<Self, NativeError> {
        Self::receive_before(stream, None)
    }
    fn receive_before(
        stream: &mut UnixStream,
        deadline: Option<Instant>,
    ) -> Result<Self, NativeError> {
        let mut bytes = [0u8; PACKET_BYTES];
        transfer(stream, &mut bytes, false, deadline)?;
        if &bytes[..4] != b"QARN" || bytes[4] != 2 || bytes[7] != 0 {
            return Err(NativeError::Protocol);
        }
        let word = |offset| {
            let mut b = [0; 8];
            b.copy_from_slice(&bytes[offset..offset + 8]);
            u64::from_le_bytes(b)
        };
        compiler_fence(Ordering::Acquire);
        Ok(Self {
            operation: bytes[5],
            abi: bytes[6],
            sequence: word(8),
            address: word(16),
            arguments: std::array::from_fn(|i| word(24 + i * 8)),
            floats: std::array::from_fn(|i| word(128 + i * 8)),
            value: word(192),
        })
    }
}

/// One controller owns the child, channel and authoritative backing. Engine
/// calls never execute a foreign pointer in the controller's process.
pub struct NativeProcess {
    child: Option<Child>,
    stream: UnixStream,
    memory: Mapping,
    base: u64,
    regions: Box<[NativeRegion]>,
    parked: bool,
    sequence: u64,
    callbacks: [u64; 4],
    imports: Box<[(u32, NativeEntry)]>,
    reaped: Option<std::process::ExitStatus>,
    timeout: Duration,
}
impl NativeProcess {
    pub fn load(image: NativeImage<'_>) -> Result<Self, NativeError> {
        let mut command = Command::new(std::env::current_exe()?);
        command.arg("--qa-native-child");
        Self::launch(command, image)
    }
    pub(super) fn launch(
        mut command: Command,
        image: NativeImage<'_>,
    ) -> Result<Self, NativeError> {
        if image.pointer_bytes != 8 {
            return Err(NativeError::Unsupported);
        }
        if image.bytes.is_empty()
            || image.bytes.len() > LIMIT
            || image.base < PAGE as u64
            || image.base % PAGE as u64 != 0
            || image.timeout.is_zero()
            || image.regions.len() + image.imports.len() > 65536
            || image.imports.len() > 4096
        {
            return Err(NativeError::Extent);
        }
        let thunk_offset =
            (Self::import_address(image.base, image.bytes.len(), 0)? - image.base) as usize;
        let thunk_bytes = image.imports.len() * 32;
        let image_length = thunk_offset + thunk_bytes.div_ceil(PAGE) * PAGE;
        let length = image_length
            .checked_add(PAGE + STACK)
            .filter(|&n| n <= LIMIT)
            .ok_or(NativeError::Extent)?;
        if image.base.checked_add(length as u64).is_none() {
            return Err(NativeError::Extent);
        }
        let imports = image
            .imports
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                Ok((
                    entry.number,
                    NativeEntry::bind(
                        Self::import_address(image.base, image.bytes.len(), index)?,
                        entry.abi,
                        entry.parameters,
                        entry.result,
                    )?,
                ))
            })
            .collect::<Result<Box<[_]>, NativeError>>()?;
        let mut pages = vec![0u8; image_length / PAGE];
        let mut end = 0;
        for region in image.regions {
            if region.offset < end || region.length == 0 || region.permissions > 7 {
                return Err(NativeError::Extent);
            }
            end = region
                .offset
                .checked_add(region.length)
                .filter(|&n| n <= image.bytes.len())
                .ok_or(NativeError::Extent)?;
            for rights in &mut pages[region.offset / PAGE..end.div_ceil(PAGE)] {
                *rights |= region.permissions;
            }
        }
        pages[thunk_offset / PAGE..].fill(5);
        let mut mapped = Vec::new();
        let mut at = 0;
        while at < pages.len() {
            let begin = at;
            let rights = pages[at];
            while at < pages.len() && pages[at] == rights {
                at += 1;
            }
            if rights != 0 {
                mapped.push(NativeRegion {
                    offset: begin * PAGE,
                    length: (at - begin) * PAGE,
                    permissions: rights,
                });
            }
        }
        if mapped.len() > 65536 {
            return Err(NativeError::Extent);
        }
        // SAFETY: valid terminated name; returns a fresh owned descriptor.
        let fd = unsafe { memfd_create(c"qa-native-memory".as_ptr(), 3) };
        if fd < 0 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: fd is fresh and has no other Rust owner.
        let file = unsafe { File::from_raw_fd(fd) };
        file.set_len(length as u64)?;
        // SAFETY: F_ADD_SEALS fixes the size before any mapped view escapes.
        if unsafe { fcntl(fd, 1033, 1 | 2 | 4) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let mut memory = Mapping::map(&file, length, None)?;
        memory.bytes_mut()[..image.bytes.len()].copy_from_slice(image.bytes);
        let (stream, child_stream) = UnixStream::pair()?;
        stream.set_read_timeout(Some(image.timeout))?;
        stream.set_write_timeout(Some(image.timeout))?;
        command
            .stdin(Stdio::from(OwnedFd::from(child_stream)))
            .stdout(Stdio::null())
            .stderr(Stdio::from(file));
        let mut regions = image.regions.to_vec();
        regions.extend(imports.iter().map(|(_, entry)| NativeRegion {
            offset: (entry.address - image.base) as usize,
            length: 12,
            permissions: 5,
        }));
        let child = command.spawn()?;
        let mut owner = Self {
            child: Some(child),
            stream,
            memory,
            base: image.base,
            regions: regions.into_boxed_slice(),
            parked: false,
            sequence: 0,
            callbacks: [0; 4],
            imports,
            reaped: None,
            timeout: image.timeout,
        };
        if let Err(error) = owner.start(&mapped, image.runtime) {
            return Err(owner.failure(error));
        }
        // Patch each numeric import gateway only at the enforced load stop.
        // The indirect jump preserves AL and all native parameter registers.
        for (index, (_, entry)) in owner.imports.iter().enumerate() {
            let at = (entry.address - owner.base) as usize;
            let bytes = &mut owner.memory.bytes_mut()[at..at + 32];
            bytes.fill(0xcc);
            bytes[..2].copy_from_slice(&[0x41, 0xbb]); // mov r11d,index
            bytes[2..6].copy_from_slice(&(index as u32).to_le_bytes());
            bytes[6..12].copy_from_slice(&[0xff, 0x25, 12, 0, 0, 0]); // jmp [rip+12]
            bytes[24..].copy_from_slice(&owner.callbacks[2 + entry.abi as usize].to_le_bytes());
        }
        Ok(owner)
    }
    fn start(
        &mut self,
        mapped: &[NativeRegion],
        runtime: Option<super::runtime::RuntimeConfig>,
    ) -> Result<(), NativeError> {
        let mut packet = Packet::new(START, 0);
        packet.address = self.base;
        packet.arguments[0] = self.memory.length as u64;
        packet.arguments[1] = mapped.len() as u64;
        packet.arguments[2] = self.imports.len() as u64;
        if let Some(config) = runtime {
            packet.arguments[3] = config.base;
            packet.arguments[4] = config.heap_bytes as u64;
        }
        packet.send(&mut self.stream)?;
        for region in mapped {
            let mut packet = Packet::new(MAP, 0);
            packet.address = region.offset as u64;
            packet.arguments[0] = region.length as u64;
            packet.arguments[1] = region.permissions as u64;
            packet.send(&mut self.stream)?;
        }
        for &(number, entry) in &self.imports {
            let mut packet = Packet::new(BIND, 0);
            packet.address = u64::from(number);
            packet.abi = entry.abi as u8;
            packet.send(&mut self.stream)?;
        }
        let ready = Packet::receive(&mut self.stream)?;
        if ready.operation != READY
            || ready.sequence != 0
            || ready.address != self.base
            || ready.arguments[0] != self.memory.length as u64
            || ready.arguments[1] != 8
            || ready.arguments[2] != self.pid() as u64
        {
            return Err(NativeError::Protocol);
        }
        self.stop_boundary()?;
        self.callbacks.copy_from_slice(&ready.arguments[3..7]);
        Ok(())
    }
    pub fn pid(&self) -> u32 {
        self.child.as_ref().map_or(0, Child::id)
    }
    pub fn base(&self) -> u64 {
        self.base
    }
    pub fn callback(&self, abi: NativeAbi) -> u64 {
        self.callbacks[abi as usize]
    }
    pub fn import_pointer(&self, ordinal: usize) -> Option<u64> {
        self.imports.get(ordinal).map(|(_, entry)| entry.address)
    }
    /// Address reserved by load for a function import, used when binding an
    /// inert image's relocations before any child executes its code.
    pub fn import_address(
        base: u64,
        image_bytes: usize,
        ordinal: usize,
    ) -> Result<u64, NativeError> {
        if image_bytes == 0 || image_bytes > LIMIT || ordinal >= 4096 {
            return Err(NativeError::Extent);
        }
        base.checked_add((image_bytes.div_ceil(PAGE) * PAGE + ordinal * 32) as u64)
            .ok_or(NativeError::Extent)
    }
    pub fn executable(&self, address: u64) -> bool {
        address
            .checked_sub(self.base)
            .and_then(|o| usize::try_from(o).ok())
            .is_some_and(|offset| executable_offset(&self.regions, offset))
    }
    pub fn memory(&self) -> Result<&[u8], NativeError> {
        if !self.parked {
            return Err(NativeError::Protocol);
        }
        Ok(self.memory.bytes())
    }
    pub fn memory_mut(&mut self) -> Result<&mut [u8], NativeError> {
        if !self.parked {
            return Err(NativeError::Protocol);
        }
        Ok(self.memory.bytes_mut())
    }
    fn resume(&mut self) -> Result<(), NativeError> {
        if !self.parked || self.child.is_none() {
            return Err(NativeError::Protocol);
        }
        self.parked = false;
        // SAFETY: signal only the PID retained by this owned, unreaped Child.
        if unsafe { kill(self.pid() as i32, SIGCONT) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }
    fn stop_boundary(&mut self) -> Result<(), NativeError> {
        // The existing channel blocks until a publication or its timeout.
        // Enforce the stop ourselves, rather than race a child's self-stop
        // after publication. Even a forged packet cannot leave a writer running
        // while the controller borrows shared bytes. No extra helper is needed.
        // SAFETY: this unreaped PID belongs to the retained Child. ESRCH can
        // mean it exited after publication; waitpid still collects that status.
        if unsafe { kill(self.pid() as i32, SIGSTOP) } < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(3) {
                return Err(error.into());
            }
        }
        loop {
            let mut status = 0;
            // SAFETY: retained PID. WUNTRACED blocks on the kernel stop/exit
            // notification following the controller's unmaskable SIGSTOP.
            let result = unsafe { waitpid(self.pid() as i32, &mut status, 2) };
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error.into());
            }
            if result != 0 {
                if status & 255 == 127 && (status >> 8) & 255 == SIGSTOP {
                    self.parked = true;
                    compiler_fence(Ordering::Acquire);
                    return Ok(());
                }
                if status & 255 != 127 {
                    self.reaped = Some(std::process::ExitStatus::from_raw(status));
                    return Err(NativeError::Exited(std::process::ExitStatus::from_raw(
                        status,
                    )));
                }
                return Err(NativeError::Protocol);
            }
        }
    }
    pub fn bind(
        &self,
        address: u64,
        abi: NativeAbi,
        parameters: &[NativeScalar],
        result: NativeScalar,
    ) -> Result<NativeEntry, NativeError> {
        if !self.parked || !self.executable(address) {
            return Err(NativeError::Extent);
        }
        NativeEntry::bind(address, abi, parameters, result)
    }
    pub fn invoke(
        &mut self,
        entry: NativeEntry,
        arguments: [u64; 13],
        mut callback: impl FnMut(NativeCall, u64, &mut [u8]) -> Result<u64, NativeError>,
    ) -> Result<u64, NativeError> {
        if !self.parked || self.child.is_none() {
            return Err(NativeError::Protocol);
        }
        if !self.executable(entry.address) {
            return Err(NativeError::Extent);
        }
        let deadline = Instant::now()
            .checked_add(self.timeout)
            .ok_or(NativeError::Extent)?;
        let run = (|| {
            self.sequence = self.sequence.checked_add(1).ok_or(NativeError::Protocol)?;
            let mut packet = Packet::new(INVOKE, self.sequence);
            packet.address = entry.address;
            packet.abi = entry.abi as u8;
            (packet.arguments, packet.floats) = entry.pack(arguments);
            packet.value = entry.control;
            packet.send_before(&mut self.stream, Some(deadline))?;
            self.resume()?;
            loop {
                let packet = Packet::receive_before(&mut self.stream, Some(deadline))?;
                if packet.sequence != self.sequence {
                    return Err(NativeError::Protocol);
                }
                self.stop_boundary()?;
                match packet.operation {
                    RETURN => return Ok(entry.result(packet.value)),
                    IMPORT => {
                        let (number, arguments, result_entry) = match packet.abi {
                            0 => (
                                u32::try_from(packet.address).map_err(|_| NativeError::Protocol)?,
                                packet.arguments,
                                None,
                            ),
                            1 => {
                                let ordinal = usize::try_from(packet.address)
                                    .map_err(|_| NativeError::Protocol)?;
                                let &(number, entry) =
                                    self.imports.get(ordinal).ok_or(NativeError::Protocol)?;
                                (
                                    number,
                                    entry.unpack(packet.arguments, packet.floats),
                                    Some(entry),
                                )
                            }
                            _ => return Err(NativeError::Protocol),
                        };
                        let result = callback(
                            NativeCall {
                                number,
                                arguments,
                                function: packet.abi == 1,
                            },
                            self.base,
                            self.memory.bytes_mut(),
                        )?;
                        let mut reply = Packet::new(REPLY, self.sequence);
                        reply.value = result_entry.map_or(result, |entry| entry.result(result));
                        reply.address = result_entry.map_or(0, |entry| entry.control & 3);
                        reply.send_before(&mut self.stream, Some(deadline))?;
                        self.resume()?;
                    }
                    _ => return Err(NativeError::Protocol),
                }
            }
        })();
        run.map_err(|e| self.failure(e))
    }
    fn failure(&mut self, error: NativeError) -> NativeError {
        let Some(mut child) = self.child.take() else {
            return error;
        };
        if let Some(status) = self.reaped.take() {
            self.parked = true;
            return NativeError::Exited(status);
        }
        // A source cannot close/export the channel through the admitted
        // syscall surface. EOF means the child is exiting; block for the real
        // status rather than poll or race it with our cleanup SIGKILL.
        if matches!(&error, NativeError::Io(e) if e.kind() == io::ErrorKind::UnexpectedEof) {
            if let Ok(status) = child.wait() {
                self.parked = true;
                return NativeError::Exited(status);
            }
        }
        if let Some(status) = child.try_wait().ok().flatten() {
            self.parked = true;
            return NativeError::Exited(status);
        }
        let _ = child.kill();
        let _ = child.wait();
        self.parked = true;
        error
    }
}
impl Drop for NativeProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.parked = true;
    }
}

// The fixed protocol carries call words, never per-entity field caches.
static CHILD_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[repr(C)]
struct ImportResult {
    value: u64,
    kind: u64,
}
extern "C" fn import(words: &[u64; 14], floats: &[[u64; 2]; 8], typed: u64) -> ImportResult {
    // The assembly gate supplies private, fully captured ABI words. No Rust
    // reference into the shared guest stack is retained while the parent borrows.
    let number = words[0];
    let arguments = std::array::from_fn(|i| words[i + 1]);
    if typed == 1 {
        match runtime::invoke(
            number as usize,
            arguments,
            std::array::from_fn(|i| floats[i][0]),
        ) {
            Ok(Some(result)) => return result,
            Ok(None) => {}
            Err(_) => std::process::exit(125),
        }
    }
    let sequence = CHILD_SEQUENCE.load(Ordering::Relaxed);
    // SAFETY: borrowed wrapper for inherited fd 0, never closed here. This
    // single-thread child alone uses the channel; clone/fork are not admitted.
    let mut stream = ManuallyDrop::new(unsafe { UnixStream::from_raw_fd(0) });
    let mut packet = Packet::new(IMPORT, sequence);
    packet.address = number;
    packet.arguments = arguments;
    packet.abi = typed as u8;
    packet.floats = std::array::from_fn(|i| floats[i][0]);
    if packet.send(&mut stream).is_err() {
        std::process::exit(125);
    }
    match Packet::receive(&mut stream) {
        Ok(packet)
            if packet.operation == REPLY && packet.sequence == sequence && packet.address <= 3 =>
        {
            ImportResult {
                value: packet.value,
                kind: packet.address,
            }
        }
        _ => std::process::exit(125),
    }
}

/// Bootstrap before console, SDL or application initialization.
pub fn native_child_bootstrap() -> Option<i32> {
    (std::env::args().nth(1).as_deref() == Some("--qa-native-child"))
        .then(|| if child_main().is_ok() { 0 } else { 125 })
}

pub(super) fn child_main() -> Result<(), NativeError> {
    // SAFETY: fresh owned child bootstrap; fd 0/2 installed by Command become
    // these wrappers' sole owners. No source code has executed yet.
    let mut stream = unsafe { UnixStream::from_raw_fd(0) };
    let file = unsafe { File::from_raw_fd(2) };
    let packet = Packet::receive(&mut stream)?;
    let length = usize::try_from(packet.arguments[0]).map_err(|_| NativeError::Extent)?;
    let count = usize::try_from(packet.arguments[1]).map_err(|_| NativeError::Extent)?;
    let imports = usize::try_from(packet.arguments[2]).map_err(|_| NativeError::Extent)?;
    if packet.operation != START
        || packet.sequence != 0
        || length <= PAGE + STACK
        || length > LIMIT
        || length % PAGE != 0
        || count > 65536
        || imports > 4096
        || packet.address < PAGE as u64
        || packet.address % PAGE as u64 != 0
        || packet.address.checked_add(length as u64).is_none()
        || file.metadata()?.len() != length as u64
    {
        return Err(NativeError::Extent);
    }
    let image_length = length - PAGE - STACK;
    let memory = Mapping::map(&file, length, Some(packet.address))?;
    drop(file); // No transferable backing descriptor remains in the child.
    // SAFETY: actual child mapping; the controller has no borrowed view yet.
    if unsafe { mprotect(memory.address.as_ptr().cast(), length, 0) } < 0 {
        return Err(io::Error::last_os_error().into());
    }
    let mut end = 0;
    let mut regions = Vec::with_capacity(count);
    for _ in 0..count {
        let region = Packet::receive(&mut stream)?;
        let offset = usize::try_from(region.address).map_err(|_| NativeError::Extent)?;
        let bytes = usize::try_from(region.arguments[0]).map_err(|_| NativeError::Extent)?;
        let rights = region.arguments[1];
        if region.operation != MAP
            || region.sequence != 0
            || offset < end
            || offset % PAGE != 0
            || bytes == 0
            || bytes % PAGE != 0
            || rights > 7
        {
            return Err(NativeError::Extent);
        }
        end = offset
            .checked_add(bytes)
            .filter(|&n| n <= image_length)
            .ok_or(NativeError::Extent)?;
        // SAFETY: page-aligned, checked range of this child's mapping.
        if unsafe {
            mprotect(
                memory.address.as_ptr().add(offset).cast(),
                bytes,
                rights as i32,
            )
        } < 0
        {
            return Err(io::Error::last_os_error().into());
        }
        regions.push(NativeRegion {
            offset,
            length: bytes,
            permissions: rights as u8,
        });
    }
    // SAFETY: final load-sized shared stack follows an inaccessible guard page.
    // Native code uses this storage; controller Rust frames remain private.
    if unsafe {
        mprotect(
            memory.address.as_ptr().add(image_length + PAGE).cast(),
            STACK,
            3,
        )
    } < 0
    {
        return Err(io::Error::last_os_error().into());
    }
    let mut bindings = Vec::with_capacity(imports);
    for _ in 0..imports {
        let binding = Packet::receive(&mut stream)?;
        if binding.operation != BIND || binding.sequence != 0 || binding.abi > 1 {
            return Err(NativeError::Protocol);
        }
        let number = u32::try_from(binding.address).map_err(|_| NativeError::Protocol)?;
        let abi = if binding.abi == 0 {
            NativeAbi::SystemV
        } else {
            NativeAbi::Microsoft
        };
        let entry = super::runtime::function(number)
            .map(|function| {
                NativeEntry::bind(0, abi, function.parameters, function.result)
                    .map(|entry| (function, entry))
            })
            .transpose()?;
        if number >= super::runtime::FIRST && entry.is_none() {
            return Err(NativeError::Unsupported);
        }
        bindings.push(entry);
    }
    let stack = NativeRegion {
        offset: image_length + PAGE,
        length: STACK,
        permissions: 3,
    };
    regions.push(stack);
    runtime::initialize(
        packet.address,
        length,
        regions.clone().into_boxed_slice(),
        bindings.into_boxed_slice(),
        (packet.arguments[3] != 0).then_some(super::runtime::RuntimeConfig {
            base: packet.arguments[3],
            heap_bytes: packet.arguments[4] as usize,
        }),
    )?;
    // SAFETY: native identity and guard are child-local, before foreign code.
    let pid = unsafe { getpid() };
    for number in [4, 5, 7, 8, 11] {
        // SAFETY: reset the Rust runtime's stack-fault handlers in this child
        // before restricting syscalls. A native fault must terminate it rather
        // than repeatedly retry an instruction through a blocked re-raise.
        if unsafe { signal(number, 0) } == usize::MAX {
            return Err(io::Error::last_os_error().into());
        }
    }
    guard()?;
    let mut ready = Packet::new(READY, 0);
    ready.address = packet.address;
    ready.arguments[..7].copy_from_slice(&[
        length as u64,
        8,
        pid as u64,
        x64::system_v_import as *const () as usize as u64,
        x64::microsoft_import as *const () as usize as u64,
        x64::system_v_function as *const () as usize as u64,
        x64::microsoft_function as *const () as usize as u64,
    ]);
    ready.send(&mut stream)?;
    loop {
        let packet = Packet::receive(&mut stream)?;
        let offset = usize::try_from(
            packet
                .address
                .checked_sub(ready.address)
                .ok_or(NativeError::Extent)?,
        )
        .map_err(|_| NativeError::Extent)?;
        if packet.operation != INVOKE
            || packet.sequence == 0
            || packet.abi > 1
            || packet.value & !0xff03 != 0
            || packet.value >> 8 > 8
            || !executable_offset(&regions, offset)
        {
            return Err(NativeError::Protocol);
        }
        CHILD_SEQUENCE.store(packet.sequence, Ordering::Relaxed);
        // SAFETY: only the owned child invokes foreign instructions. The gate
        // uses its bounded shared stack and switches to a private controller
        // stack before Rust handles imports. No Rust reference into the guest
        // mapping spans this call; faults terminate/reap only this child.
        let value = unsafe {
            x64::call(
                packet.address,
                u64::from(packet.abi),
                &packet.arguments,
                ready.address + length as u64,
                &packet.floats,
                packet.value,
            )
        };
        let mut reply = Packet::new(RETURN, packet.sequence);
        reply.value = value;
        reply.send(&mut stream)?;
    }
}

#[repr(C)]
struct Filter {
    code: u16,
    yes: u8,
    no: u8,
    value: u32,
}
#[repr(C)]
struct FilterProgram {
    length: u16,
    filters: *const Filter,
}
fn guard() -> Result<(), NativeError> {
    let load = |offset| Filter {
        code: 0x20,
        yes: 0,
        no: 0,
        value: offset,
    };
    let equal = |value, skip| Filter {
        code: 0x15,
        yes: 0,
        no: skip,
        value,
    };
    let allow = || Filter {
        code: 0x06,
        yes: 0,
        no: 0,
        value: 0x7fff0000,
    };
    let deny = || Filter {
        code: 0x06,
        yes: 0,
        no: 0,
        value: 0x00050000 | 1,
    };
    // No native OS provider is enabled yet: admit only channel I/O,
    // signal return and exit. Fork/clone, descriptor export and remapping may
    // not leave a writer running while the engine borrows authoritative bytes.
    let filters = [
        load(4),
        equal(0xc000003e, 1),
        Filter {
            code: 0x05,
            yes: 0,
            no: 0,
            value: 1,
        },
        Filter {
            code: 0x06,
            yes: 0,
            no: 0,
            value: 0x80000000,
        },
        load(0),
        equal(0, 3),
        load(16),
        equal(0, 1),
        allow(),
        load(0),
        equal(1, 3),
        load(16),
        equal(0, 1),
        allow(),
        load(0),
        equal(44, 3),
        load(16),
        equal(0, 1),
        allow(),
        load(0),
        equal(45, 3),
        load(16),
        equal(0, 1),
        allow(),
        load(0),
        equal(60, 1),
        allow(),
        equal(231, 1),
        allow(),
        equal(15, 1),
        allow(),
        deny(),
    ];
    let program = FilterProgram {
        length: filters.len() as u16,
        filters: filters.as_ptr(),
    };
    // SAFETY: native prctl/seccomp ABI; BPF storage lives through kernel
    // copying. TSYNC covers every pre-existing child thread as well. A positive
    // return names a thread that could not synchronize and must fail startup.
    if unsafe { prctl(4, 0, 0, 0, 0) } < 0
        || unsafe { prctl(38, 1, 0, 0, 0) } < 0
        || unsafe { syscall(317, 1, 1, &program as *const FilterProgram) } != 0
    {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
}
