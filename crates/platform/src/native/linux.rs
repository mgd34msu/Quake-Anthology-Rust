use super::{
    NativeAbi, NativeCall, NativeEntry, NativeError, NativeImage, NativeRegion, NativeScalar,
    PAGE_BYTES as PAGE,
};
use crate::Stopwatch;
use std::{
    fs::File,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    process::{Child, Command, Stdio},
    ptr::NonNull,
    sync::atomic::{Ordering, compiler_fence},
    time::Duration,
};

const LIMIT: usize = 512 * 1024 * 1024;
const STACK: usize = 8 * 1024 * 1024;
#[path = "gate.rs"]
mod gate;
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
const RUNTIME_ERROR: u8 = 9;
const CALL_TRAP: u8 = 10;
#[path = "runtime/child.rs"]
mod runtime;
const PACKET_BYTES: usize = 200;
const SIGKILL: i32 = 9;

pub(super) fn executable_offset(regions: &[NativeRegion], offset: usize) -> bool {
    super::region_at(regions, offset, 4).is_some()
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
    fn getpid() -> i32;
    fn gettid() -> i32;
    fn getppid() -> i32;
    fn signal(number: i32, handler: usize) -> usize;
    fn prctl(operation: i32, ...) -> i32;
    fn syscall(number: std::ffi::c_long, ...) -> std::ffi::c_long;
}

pub(super) fn bind_parent_death(parent: u32) -> io::Result<()> {
    // SAFETY: PR_SET_PDEATHSIG binds this child to its creating parent thread.
    // SIGKILL also terminates a parked child and cannot be blocked by a module.
    if unsafe { prctl(1, SIGKILL, 0, 0, 0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // The parent may have died after fork but before the signal was installed.
    // In that case reject startup before reading IPC or executing foreign code.
    if unsafe { getppid() } as u32 != parent {
        return Err(io::Error::from_raw_os_error(3));
    }
    Ok(())
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
    mapped_length: usize,
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
        Ok(Self {
            address,
            length,
            mapped_length: length,
        })
    }
    fn bytes(&self) -> &[u8] {
        // SAFETY: exposed only before spawn, at a received kernel notification,
        // or after reaping. invoke requires exclusive ownership of this view.
        unsafe { std::slice::from_raw_parts(self.address.as_ptr(), self.length) }
    }
    fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: the same kernel-parked invariant holds, and &mut excludes
        // simultaneous controller borrows. The child cannot export the memfd.
        unsafe { std::slice::from_raw_parts_mut(self.address.as_ptr(), self.length) }
    }
}
impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: exactly one mapping owner; its child was reaped before drop.
        unsafe {
            munmap(self.address.as_ptr().cast(), self.mapped_length);
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
    deadline: Option<(&Stopwatch, Duration)>,
) -> Result<(), NativeError> {
    while !bytes.is_empty() {
        let io = if let Some((started, timeout)) = deadline {
            let remaining = timeout
                .checked_sub(started.elapsed())
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
        deadline: Option<(&Stopwatch, Duration)>,
    ) -> Result<(), NativeError> {
        let mut bytes = self.encode();
        compiler_fence(Ordering::Release);
        transfer(stream, &mut bytes, true, deadline)
    }
    fn encode(&self) -> [u8; PACKET_BYTES] {
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
        bytes
    }
    fn receive(stream: &mut UnixStream) -> Result<Self, NativeError> {
        Self::receive_before(stream, None)
    }
    fn receive_before(
        stream: &mut UnixStream,
        deadline: Option<(&Stopwatch, Duration)>,
    ) -> Result<Self, NativeError> {
        let mut bytes = [0u8; PACKET_BYTES];
        transfer(stream, &mut bytes, false, deadline)?;
        compiler_fence(Ordering::Acquire);
        Self::decode(&bytes)
    }
    fn decode(bytes: &[u8; PACKET_BYTES]) -> Result<Self, NativeError> {
        if &bytes[..4] != b"QARN" || bytes[4] != 2 || bytes[7] != 0 {
            return Err(NativeError::Protocol);
        }
        let word = |offset| {
            let mut b = [0; 8];
            b.copy_from_slice(&bytes[offset..offset + 8]);
            u64::from_le_bytes(b)
        };
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
    listener: Option<gate::Listener>,
    parked: Option<gate::Parked>,
    handoff: u64,
    sequence: u64,
    callbacks: [u64; 4],
    imports: Box<
        [(
            u32,
            NativeEntry,
            bool,
            Option<super::NativeListFilter>,
            Option<super::NativeFormat>,
        )],
    >,
    timeout: Duration,
    runtime: Option<super::runtime::RuntimeConfig>,
    epoch_millis: i64,
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
        gate::Listener::check_sizes()?;
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
        let mapped_length = length.checked_add(PAGE).ok_or(NativeError::Extent)?;
        if image.base.checked_add(mapped_length as u64).is_none() {
            return Err(NativeError::Extent);
        }
        let imports = image
            .imports
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                if entry.trap
                    && (!entry.parameters.is_empty() || entry.result != NativeScalar::Void)
                {
                    return Err(NativeError::Extent);
                }
                if let Some(filter) = entry.filter
                    && (entry.trap
                        || !filter.valid(entry.parameters.len())
                        || !matches!(
                            entry.result,
                            NativeScalar::Word | NativeScalar::I32 | NativeScalar::U32
                        ))
                {
                    return Err(NativeError::Extent);
                }
                if let Some(format) = entry.format
                    && (entry.trap
                        || entry.filter.is_some()
                        || entry.number >= super::runtime::FIRST
                        || entry.result != NativeScalar::Void
                        || !format.valid(entry.parameters))
                {
                    return Err(NativeError::Extent);
                }
                Ok((
                    entry.number,
                    NativeEntry::bind(
                        Self::import_address(image.base, image.bytes.len(), index)?,
                        entry.abi,
                        entry.parameters,
                        entry.result,
                    )?,
                    entry.trap,
                    entry.filter,
                    entry.format,
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
        file.set_len(mapped_length as u64)?;
        // SAFETY: F_ADD_SEALS fixes the size before any mapped view escapes.
        if unsafe { fcntl(fd, 1033, 1 | 2 | 4) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let mut memory = Mapping::map(&file, mapped_length, None)?;
        memory.length = length;
        gate::initialize(&memory);
        memory.bytes_mut()[..image.bytes.len()].copy_from_slice(image.bytes);
        let (stream, child_stream) = UnixStream::pair()?;
        stream.set_read_timeout(Some(image.timeout))?;
        stream.set_write_timeout(Some(image.timeout))?;
        command
            .stdin(Stdio::from(OwnedFd::from(child_stream)))
            .stdout(Stdio::null())
            .stderr(Stdio::from(file));
        let parent = std::process::id();
        // SAFETY: the post-fork closure performs only prctl/getppid and creates
        // raw OS errors. It takes no locks or heap allocations. Install the
        // death signal before exec, including while startup waits for START;
        // this non-setuid executable preserves it, and seccomp later forbids
        // foreign code from clearing it. The PID check closes the fork race.
        unsafe {
            command.pre_exec(move || bind_parent_death(parent));
        }
        let mut regions = image.regions.to_vec();
        regions.extend(imports.iter().map(|(_, entry, _, _, _)| NativeRegion {
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
            listener: None,
            parked: None,
            handoff: 0,
            sequence: 0,
            callbacks: [0; 4],
            imports,
            timeout: image.timeout,
            runtime: image.runtime,
            epoch_millis: crate::clock::event_epoch_millis(),
        };
        if let Err(error) = owner.start(&mapped, image.runtime) {
            return Err(owner.failure(error));
        }
        owner.set_event_time(qa_core::sys_events::EventTime(0))?;
        // Patch each numeric import gateway only at the held load notification.
        // The indirect jump preserves AL and all native parameter registers.
        for (index, (_, entry, _, _, _)) in owner.imports.iter().enumerate() {
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
            packet.arguments[5] = config.teb.unwrap_or(0);
        }
        packet.send(&mut self.stream)?;
        for region in mapped {
            let mut packet = Packet::new(MAP, 0);
            packet.address = region.offset as u64;
            packet.arguments[0] = region.length as u64;
            packet.arguments[1] = region.permissions as u64;
            packet.send(&mut self.stream)?;
        }
        for &(number, entry, trap, filter, format) in &self.imports {
            let mut packet = Packet::new(BIND, 0);
            packet.address = u64::from(number);
            packet.abi = entry.abi as u8;
            packet.value = u64::from(trap);
            if let Some(filter) = filter {
                packet.value = 2;
                packet.arguments[..6].copy_from_slice(&[
                    filter.buffer,
                    u64::from(filter.capacity),
                    u64::from(filter.depth),
                    u64::from(filter.list)
                        | u64::from(filter.limit) << 8
                        | u64::from(filter.callback) << 16
                        | u64::from(filter.data) << 24,
                    u64::from(filter.keep),
                    u64::from(filter.end),
                ]);
                packet.arguments[6] = entry.argument_count() as u64;
            }
            if let Some(format) = format {
                packet.value = 3;
                packet.arguments[..4].copy_from_slice(&[
                    format.buffer,
                    u64::from(format.capacity),
                    u64::from(format.format),
                    entry.argument_count() as u64,
                ]);
            }
            packet.send(&mut self.stream)?;
        }
        let started = Stopwatch::start();
        self.listener = Some(gate::receive_listener(
            self.stream.as_raw_fd(),
            &started,
            self.timeout,
        )?);
        let ready = self.receive_handoff(&started)?;
        if ready.operation != READY
            || ready.sequence != 0
            || ready.address != self.base
            || ready.arguments[0] != self.memory.length as u64
            || ready.arguments[1] != 8
            || ready.arguments[2] != self.pid() as u64
        {
            return Err(NativeError::Protocol);
        }
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
        self.imports
            .get(ordinal)
            .map(|(_, entry, _, _, _)| entry.address)
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
        self.validate_parked()?;
        Ok(self.memory.bytes())
    }
    pub fn memory_mut(&mut self) -> Result<&mut [u8], NativeError> {
        self.validate_parked()?;
        Ok(self.memory.bytes_mut())
    }
    /// Redirect an owned image entry to a load-reserved engine import while the
    /// child is kernel-parked. Neither address names code in the Rust process.
    pub fn redirect_import(&mut self, address: u64, ordinal: usize) -> Result<(), NativeError> {
        let target = self.import_pointer(ordinal).ok_or(NativeError::Extent)?;
        let offset = usize::try_from(address.checked_sub(self.base).ok_or(NativeError::Extent)?)
            .map_err(|_| NativeError::Extent)?;
        let end = offset.checked_add(5).ok_or(NativeError::Extent)?;
        let region = self
            .regions
            .partition_point(|region| region.offset <= offset)
            .checked_sub(1)
            .and_then(|index| self.regions.get(index))
            .ok_or(NativeError::Extent)?;
        if region.permissions & 4 == 0 || end > region.offset + region.length {
            return Err(NativeError::Extent);
        }
        let displacement = i32::try_from(i128::from(target) - i128::from(address) - 5)
            .map_err(|_| NativeError::Extent)?;
        let bytes = self
            .memory_mut()?
            .get_mut(offset..end)
            .ok_or(NativeError::Extent)?;
        bytes[0] = 0xe9;
        bytes[1..].copy_from_slice(&displacement.to_le_bytes());
        Ok(())
    }
    /// Publish queued time only while holding the child's kernel notification.
    pub fn set_event_time(
        &mut self,
        time: qa_core::sys_events::EventTime,
    ) -> Result<(), NativeError> {
        let Some(config) = self.runtime else {
            return Ok(());
        };
        let at = usize::try_from(
            config
                .base
                .checked_sub(self.base)
                .ok_or(NativeError::Extent)?,
        )
        .map_err(|_| NativeError::Extent)?
        .checked_add(super::runtime::TIME_OFFSET)
        .ok_or(NativeError::Extent)?;
        let millis = self
            .epoch_millis
            .checked_add(i64::try_from(time.milliseconds()).map_err(|_| NativeError::Extent)?)
            .ok_or(NativeError::Extent)?;
        let bytes = self
            .memory_mut()?
            .get_mut(at..at.checked_add(16).ok_or(NativeError::Extent)?)
            .ok_or(NativeError::Extent)?;
        bytes[..8].copy_from_slice(&time.0.to_le_bytes());
        bytes[8..].copy_from_slice(&millis.to_le_bytes());
        Ok(())
    }
    fn validate_parked(&self) -> Result<(), NativeError> {
        if self.child.is_none() {
            return Ok(());
        } // Reaped memory has no writer.
        self.listener
            .as_ref()
            .ok_or(NativeError::Protocol)?
            .valid(self.parked.ok_or(NativeError::Protocol)?)
    }
    fn receive_handoff(&mut self, started: &Stopwatch) -> Result<Packet, NativeError> {
        if self.parked.is_some() {
            return Err(NativeError::Protocol);
        }
        let handoff = self.handoff.checked_add(1).ok_or(NativeError::Protocol)?;
        let address = self.base + self.memory.length as u64;
        let parked = self
            .listener
            .as_ref()
            .ok_or(NativeError::Protocol)?
            .receive(self.pid(), address, handoff, started, self.timeout)?;
        // SAFETY: RECV plus ID_VALID have established the owned sole thread's
        // kernel park. WAIT_KILLABLE_RECV prevents any nonfatal resumption.
        let packet = unsafe { gate::read(gate::address(&self.memory), handoff) }?;
        self.parked = Some(parked);
        self.handoff = handoff;
        Ok(packet)
    }
    fn reply_handoff(&mut self, packet: Packet, started: &Stopwatch) -> Result<(), NativeError> {
        if started.elapsed() >= self.timeout {
            return Err(NativeError::Timeout);
        }
        self.validate_parked()?;
        let parked = self.parked.ok_or(NativeError::Protocol)?;
        // SAFETY: public memory excludes this control page. All call borrows
        // have ended; the received kernel id grants the slot's sole ownership.
        unsafe { gate::write(gate::address(&self.memory), packet, parked.handoff) };
        self.parked = None;
        self.listener
            .as_ref()
            .ok_or(NativeError::Protocol)?
            .reply(parked)
    }
    pub fn bind(
        &self,
        address: u64,
        abi: NativeAbi,
        parameters: &[NativeScalar],
        result: NativeScalar,
    ) -> Result<NativeEntry, NativeError> {
        if self.parked.is_none() || !self.executable(address) {
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
        if self.parked.is_none() || self.child.is_none() {
            return Err(NativeError::Protocol);
        }
        if !self.executable(entry.address) {
            return Err(NativeError::Extent);
        }
        let started = Stopwatch::start();
        let run = (|| {
            self.sequence = self.sequence.checked_add(1).ok_or(NativeError::Protocol)?;
            let mut packet = Packet::new(INVOKE, self.sequence);
            packet.address = entry.address;
            packet.abi = entry.abi as u8;
            (packet.arguments, packet.floats) = entry.pack(arguments);
            packet.value = entry.control;
            self.reply_handoff(packet, &started)?;
            loop {
                let packet = self.receive_handoff(&started)?;
                if packet.sequence != self.sequence {
                    return Err(NativeError::Protocol);
                }
                match packet.operation {
                    RETURN => return Ok(entry.result(packet.value)),
                    CALL_TRAP => {
                        let ordinal =
                            usize::try_from(packet.address).map_err(|_| NativeError::Protocol)?;
                        let &(_, entry, trap, _, format) =
                            self.imports.get(ordinal).ok_or(NativeError::Protocol)?;
                        // A bounded child format conversion can reject a
                        // live import before publishing its fixed arguments.
                        if !trap && format.is_none() {
                            return Err(NativeError::Protocol);
                        }
                        // A tail call has no native return PC. Report its
                        // native import gateway instead of a controller PC.
                        let address = if self.executable(packet.value) {
                            packet.value
                        } else {
                            entry.address
                        };
                        return Err(NativeError::ImportTrap { ordinal, address });
                    }
                    RUNTIME_ERROR => {
                        let ordinal =
                            usize::try_from(packet.address).map_err(|_| NativeError::Protocol)?;
                        let &(number, _, _, _, _) =
                            self.imports.get(ordinal).ok_or(NativeError::Protocol)?;
                        let function =
                            super::runtime::function(number).ok_or(NativeError::Protocol)?;
                        let name = std::str::from_utf8(function.name)
                            .map_err(|_| NativeError::Protocol)?;
                        return Err(NativeError::RuntimeImport(name));
                    }
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
                                let &(number, entry, _, _, _) =
                                    self.imports.get(ordinal).ok_or(NativeError::Protocol)?;
                                (
                                    number,
                                    entry.unpack(packet.arguments, packet.floats),
                                    Some(entry),
                                )
                            }
                            _ => return Err(NativeError::Protocol),
                        };
                        self.validate_parked()?;
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
                        self.reply_handoff(reply, &started)?;
                    }
                    _ => return Err(NativeError::Protocol),
                }
            }
        })();
        match run {
            Err(error @ NativeError::ImportTrap { .. }) => Err(error),
            result => result.map_err(|e| self.failure(e)),
        }
    }
    fn failure(&mut self, error: NativeError) -> NativeError {
        let Some(mut child) = self.child.take() else {
            return error;
        };
        // Listener HUP means its sole filtered child has exited. Bootstrap EOF
        // also means startup failed; block for the real
        // status rather than poll or race it with our cleanup SIGKILL.
        if matches!(&error, NativeError::Io(e) if e.kind() == io::ErrorKind::UnexpectedEof) {
            if let Ok(status) = child.wait() {
                self.parked = None;
                return NativeError::Exited(status);
            }
        }
        if let Some(status) = child.try_wait().ok().flatten() {
            self.parked = None;
            return NativeError::Exited(status);
        }
        let _ = child.kill();
        let _ = child.wait();
        self.parked = None;
        error
    }
}
impl Drop for NativeProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.parked = None;
    }
}

// The fixed protocol carries call words, never per-entity field caches.
static CHILD_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CHILD_HANDOFF: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CHILD_SLOT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static CHILD_TRAP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);
static CHILD_TRAP_ADDRESS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn child_trap() -> Option<(usize, u64)> {
    let ordinal = CHILD_TRAP.load(Ordering::Relaxed);
    (ordinal != usize::MAX).then(|| (ordinal, CHILD_TRAP_ADDRESS.load(Ordering::Relaxed)))
}

#[repr(C)]
struct ImportResult {
    value: u64,
    kind: u64,
}
extern "C" fn import(
    words: *const [u64; 14],
    floats: *const [[u64; 2]; 8],
    typed: u64,
    address: u64,
    stack: u64,
    float_count: u64,
) -> ImportResult {
    // The assembly gate supplies private, fully captured ABI words. No Rust
    // reference into the shared guest stack is retained while the parent borrows.
    // SAFETY: assembly captured both arrays in its aligned import state. Copy
    // them now so a child-local callback may reenter without aliasing a Rust
    // reference to that state.
    let words = unsafe { words.read() };
    let floats = unsafe { floats.read() };
    let number = words[0];
    let arguments = std::array::from_fn(|i| words[i + 1]);
    if typed == 1 {
        match runtime::invoke(
            number as usize,
            arguments,
            std::array::from_fn(|i| floats[i][0]),
            runtime::Capture {
                stack,
                floats: float_count as u8,
            },
        ) {
            Ok(Some(result)) => return result,
            Ok(None) => {}
            Err(NativeError::ImportTrap { ordinal, .. }) => {
                if child_trap().is_none() {
                    CHILD_TRAP.store(ordinal, Ordering::Relaxed);
                    CHILD_TRAP_ADDRESS.store(address, Ordering::Relaxed);
                }
                return ImportResult { value: 0, kind: 4 };
            }
            Err(_) => {
                // Publish only the load-declared ordinal. The parent receives
                // the same kernel gate, resolves its own static name and reaps
                // this failed child before exposing any shared memory.
                let sequence = CHILD_SEQUENCE.load(Ordering::Relaxed);
                let mut packet = Packet::new(RUNTIME_ERROR, sequence);
                packet.address = number;
                let _ = child_exchange(packet);
                std::process::exit(125);
            }
        }
    }
    host_import(
        number,
        arguments,
        std::array::from_fn(|i| floats[i][0]),
        typed,
    )
}

fn host_import(number: u64, arguments: [u64; 13], floats: [u64; 8], typed: u64) -> ImportResult {
    let sequence = CHILD_SEQUENCE.load(Ordering::Relaxed);
    let mut packet = Packet::new(IMPORT, sequence);
    packet.address = number;
    packet.arguments = arguments;
    packet.abi = typed as u8;
    packet.floats = floats;
    match child_exchange(packet) {
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

fn child_exchange(packet: Packet) -> Result<Packet, NativeError> {
    let handoff = CHILD_HANDOFF
        .load(Ordering::Relaxed)
        .checked_add(1)
        .ok_or(NativeError::Protocol)?;
    let address = CHILD_SLOT.load(Ordering::Relaxed);
    if address == 0 {
        return Err(NativeError::Protocol);
    }
    // SAFETY: exactly one child thread owns this excluded control page until
    // the intercepted syscall parks it. No shared Rust reference survives.
    unsafe { gate::write(address, packet, handoff) };
    CHILD_HANDOFF.store(handoff, Ordering::Relaxed);
    // SAFETY: the filter intercepts this fixed gate syscall and checks its
    // magic/address. It never executes getpid. A zero emulated return certifies
    // a completed parent reply; EINTR/ENOSYS or any other result fails closed.
    let result = unsafe {
        syscall(
            gate::SYSCALL as std::ffi::c_long,
            gate::MAGIC,
            handoff as u32 as u64,
            handoff >> 32,
            address as u64,
            0u64,
            0u64,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error().into());
    }
    compiler_fence(Ordering::Acquire);
    // SAFETY: the parent ended every borrow before SEND and relinquished the
    // slot; the sole child now owns it again, including nested callbacks.
    unsafe { gate::read(address, handoff) }
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
        || packet
            .address
            .checked_add(length as u64 + PAGE as u64)
            .is_none()
        || file.metadata()?.len() != length as u64 + PAGE as u64
    {
        return Err(NativeError::Extent);
    }
    let image_length = length - PAGE - STACK;
    let mut memory = Mapping::map(&file, length + PAGE, Some(packet.address))?;
    memory.length = length;
    drop(file); // No transferable backing descriptor remains in the child.
    // SAFETY: actual child mapping; the controller has no borrowed view yet.
    if unsafe { mprotect(memory.address.as_ptr().cast(), memory.mapped_length, 0) } < 0 {
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
    // SAFETY: this final page is disjoint from every guest and stack extent and
    // is never exposed by runtime::Memory or NativeProcess::memory.
    if unsafe { mprotect(gate::address(&memory) as *mut _, PAGE, 3) } < 0 {
        return Err(io::Error::last_os_error().into());
    }
    CHILD_SLOT.store(gate::address(&memory), Ordering::Relaxed);
    let mut bindings = Vec::with_capacity(imports);
    for _ in 0..imports {
        let binding = Packet::receive(&mut stream)?;
        if binding.operation != BIND
            || binding.sequence != 0
            || binding.abi > 1
            || binding.value > 3
        {
            return Err(NativeError::Protocol);
        }
        let number = u32::try_from(binding.address).map_err(|_| NativeError::Protocol)?;
        let abi = if binding.abi == 0 {
            NativeAbi::SystemV
        } else {
            NativeAbi::Microsoft
        };
        let entry = if binding.value == 3 {
            let format = super::NativeFormat {
                buffer: binding.arguments[0],
                capacity: binding.arguments[1]
                    .try_into()
                    .map_err(|_| NativeError::Protocol)?,
                format: binding.arguments[2]
                    .try_into()
                    .map_err(|_| NativeError::Protocol)?,
            };
            // The parent already checked the fixed scalar types. Validate the
            // serialized shape independently before touching mapped output.
            if number >= super::runtime::FIRST
                || format.capacity == 0
                || format.capacity > 2048
                || format.format > 2
                || binding.arguments[3] != u64::from(format.format) + 1
            {
                return Err(NativeError::Protocol);
            }
            runtime::ImportBinding::Formatted(abi, format)
        } else if binding.value == 2 {
            let slots = u32::try_from(binding.arguments[3])
                .map_err(|_| NativeError::Protocol)?
                .to_le_bytes();
            let filter = super::NativeListFilter {
                buffer: binding.arguments[0],
                capacity: binding.arguments[1]
                    .try_into()
                    .map_err(|_| NativeError::Protocol)?,
                depth: binding.arguments[2]
                    .try_into()
                    .map_err(|_| NativeError::Protocol)?,
                list: slots[0],
                limit: slots[1],
                callback: slots[2],
                data: slots[3],
                keep: binding.arguments[4]
                    .try_into()
                    .map_err(|_| NativeError::Protocol)?,
                end: binding.arguments[5]
                    .try_into()
                    .map_err(|_| NativeError::Protocol)?,
            };
            if number >= super::runtime::FIRST || !filter.valid(binding.arguments[6] as usize) {
                return Err(NativeError::Protocol);
            }
            runtime::ImportBinding::Engine(Some((abi, filter)))
        } else if binding.value == 1 {
            runtime::ImportBinding::Trap
        } else if let Some(function) = super::runtime::function(number) {
            runtime::ImportBinding::Runtime(
                function,
                NativeEntry::bind(0, abi, function.parameters, function.result)?,
            )
        } else if number >= super::runtime::FIRST {
            return Err(NativeError::Unsupported);
        } else {
            runtime::ImportBinding::Engine(None)
        };
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
            teb: (packet.arguments[5] != 0).then_some(packet.arguments[5]),
        }),
    )?;
    if packet.arguments[5] != 0 {
        // SAFETY: runtime::initialize validated and populated this child-owned
        // TEB. Linux x64 Rust TLS uses FS, so setting GS exposes the Windows
        // ABI only inside this child and never changes the parent's segments.
        if unsafe { syscall(158, 0x1001u64, packet.arguments[5]) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
    }
    // SAFETY: native identity and guard are child-local, before foreign code.
    let pid = unsafe { getpid() };
    if unsafe { gettid() } != pid || std::fs::read_dir("/proc/self/task")?.count() != 1 {
        return Err(NativeError::Unsupported);
    }
    for number in [4, 5, 7, 8, 11] {
        // SAFETY: reset the Rust runtime's stack-fault handlers in this child
        // before restricting syscalls. A native fault must terminate it rather
        // than repeatedly retry an instruction through a blocked re-raise.
        if unsafe { signal(number, 0) } == usize::MAX {
            return Err(io::Error::last_os_error().into());
        }
    }
    let listener = guard(gate::address(&memory) as u64)?;
    gate::send_listener(stream.as_raw_fd(), listener)?;
    drop(stream);
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
    let mut next = child_exchange(ready)?;
    loop {
        let packet = next;
        let offset = usize::try_from(
            packet
                .address
                .checked_sub(ready.address)
                .ok_or(NativeError::Extent)?,
        )
        .map_err(|_| NativeError::Extent)?;
        if packet.operation != INVOKE
            || Some(packet.sequence) != CHILD_SEQUENCE.load(Ordering::Relaxed).checked_add(1)
            || packet.abi > 1
            || packet.value & !0xff03 != 0
            || packet.value >> 8 > 8
            || !executable_offset(&regions, offset)
        {
            return Err(NativeError::Protocol);
        }
        CHILD_SEQUENCE.store(packet.sequence, Ordering::Relaxed);
        CHILD_TRAP.store(usize::MAX, Ordering::Relaxed);
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
        if let Some((ordinal, address)) = child_trap() {
            reply.operation = CALL_TRAP;
            reply.address = ordinal as u64;
            reply.value = address;
        } else {
            reply.value = value;
        }
        next = child_exchange(reply)?;
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
fn guard(slot_address: u64) -> Result<OwnedFd, NativeError> {
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
    let required = |value| Filter {
        code: 0x15,
        yes: 1,
        no: 0,
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
    // The sole child thread has already been checked. Clone/fork and signal
    // handlers are forbidden; a received notification therefore parks every
    // possible guest writer. Bootstrap may transfer its listener over fd 0,
    // then both child-side descriptors close before any foreign instruction.
    let filters = [
        load(4),
        required(gate::ARCH),
        Filter {
            code: 0x06,
            yes: 0,
            no: 0,
            value: 0x80000000,
        },
        load(0),
        equal(gate::SYSCALL as u32, 13),
        load(16),
        required(gate::MAGIC as u32),
        deny(),
        load(20),
        required(0),
        deny(),
        load(40),
        required(slot_address as u32),
        deny(),
        load(44),
        required((slot_address >> 32) as u32),
        deny(),
        Filter {
            code: 0x06,
            yes: 0,
            no: 0,
            value: 0x7fc00000,
        },
        load(0),
        equal(46, 3), // bootstrap sendmsg only
        load(16),
        equal(0, 1),
        allow(),
        load(0),
        equal(3, 1), // close bootstrap descriptors
        allow(),
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
    // copying. NEW_LISTENER returns one owned fd. WAIT_KILLABLE_RECV is
    // mandatory; unsupported kernels fail startup, with no signal fallback.
    if unsafe { prctl(4, 0, 0, 0, 0) } < 0 || unsafe { prctl(38, 1, 0, 0, 0) } < 0 {
        return Err(io::Error::last_os_error().into());
    }
    let fd = unsafe {
        syscall(
            gate::SECCOMP,
            1,
            gate::FLAGS,
            &program as *const FilterProgram,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: successful NEW_LISTENER returned this fresh descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}
