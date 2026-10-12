//! One shared native handoff, owned by the received seccomp notification.
use super::{Mapping, NativeError, PACKET_BYTES, Packet, PollFd, Stopwatch, Timespec};
use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    ptr::NonNull,
    sync::atomic::{AtomicU64, Ordering, compiler_fence},
    time::Duration,
};

pub(super) const MAGIC: u64 = 0x5141_524e;
pub(super) const SYSCALL: i32 = 39; // getpid is intercepted, never continued.
pub(super) const ARCH: u32 = 0xc000_003e;
pub(super) const FLAGS: u32 = (1 << 3) | (1 << 5);
pub(super) const SECCOMP: std::ffi::c_long = 317;
const RECV: u64 = 0xc050_2100;
const SEND: u64 = 0xc018_2101;
const VALID: u64 = 0x4008_2102;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Data {
    number: i32,
    architecture: u32,
    instruction: u64,
    arguments: [u64; 6],
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Notification {
    id: u64,
    pid: u32,
    flags: u32,
    data: Data,
}
#[repr(C)]
struct Response {
    id: u64,
    value: i64,
    error: i32,
    flags: u32,
}
#[repr(C)]
#[derive(Default)]
struct Sizes {
    notification: u16,
    response: u16,
    data: u16,
}

#[repr(C)]
struct Slot {
    handoff: AtomicU64,
    packet: [u8; PACKET_BYTES],
}

#[derive(Clone, Copy)]
pub(super) struct Parked {
    id: u64,
    pub handoff: u64,
}

pub(super) struct Listener(OwnedFd);

unsafe extern "C" {
    fn ioctl(fd: i32, command: u64, ...) -> i32;
    fn sendmsg(fd: i32, message: *const Message, flags: i32) -> isize;
    fn recvmsg(fd: i32, message: *mut Message, flags: i32) -> isize;
}

impl Listener {
    pub fn check_sizes() -> Result<(), NativeError> {
        let mut sizes = Sizes::default();
        // SAFETY: GET_NOTIF_SIZES writes this fixed UAPI structure. No filter
        // is installed in the engine. Unsupported kernels fail closed.
        if unsafe { super::syscall(SECCOMP, 3, 0, &mut sizes as *mut Sizes) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        if usize::from(sizes.notification) != size_of::<Notification>()
            || usize::from(sizes.response) != size_of::<Response>()
            || usize::from(sizes.data) != size_of::<Data>()
        {
            return Err(NativeError::Unsupported);
        }
        Ok(())
    }

    pub fn valid(&self, parked: Parked) -> Result<(), NativeError> {
        // SAFETY: the listener is exclusively owned and this is its received
        // kernel id. WAIT_KILLABLE_RECV prevents nonfatal signal cancellation
        // after RECV. Fatal termination cannot resume a guest writer.
        if unsafe { ioctl(self.0.as_raw_fd(), VALID, &parked.id as *const u64) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }

    pub fn receive(
        &self,
        pid: u32,
        address: u64,
        expected: u64,
        started: &Stopwatch,
        timeout: Duration,
    ) -> Result<Parked, NativeError> {
        let flags = wait(self.0.as_raw_fd(), started, timeout)?;
        if flags & 16 != 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
        }
        if flags & (8 | 32) != 0 || flags & 1 == 0 {
            return Err(io::Error::from_raw_os_error(5).into());
        }
        let mut notification = Notification::default();
        // SAFETY: ppoll identified the sole owner's notification. No competing
        // listener reader exists. A cancelled request returns ENOENT, and an
        // exiting sole child wakes RECV. Neither case retries or borrows bytes.
        if unsafe {
            ioctl(
                self.0.as_raw_fd(),
                RECV,
                &mut notification as *mut Notification,
            )
        } < 0
        {
            return Err(io::Error::last_os_error().into());
        }
        if notification.pid != pid
            || notification.flags != 0
            || notification.data.number != SYSCALL
            || notification.data.architecture != ARCH
            || notification.data.arguments
                != [MAGIC, expected as u32 as u64, expected >> 32, address, 0, 0]
        {
            return Err(NativeError::Protocol);
        }
        let parked = Parked {
            id: notification.id,
            handoff: expected,
        };
        self.valid(parked)?;
        Ok(parked)
    }

    pub fn reply(&self, parked: Parked) -> Result<(), NativeError> {
        self.valid(parked)?;
        let response = Response {
            id: parked.id,
            value: 0,
            error: 0,
            flags: 0,
        };
        // SAFETY: all guest and slot references have ended before SEND. Flags
        // are zero: the intercepted syscall is emulated, never CONTINUE'd.
        if unsafe { ioctl(self.0.as_raw_fd(), SEND, &response as *const Response) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }
}

fn slot(memory: &Mapping) -> NonNull<Slot> {
    // SAFETY: launch reserves the final aligned page exclusively for this
    // control slot; public memory views exclude it. It is never unmapped while
    // either owner lives. No Rust reference is made by locating it.
    unsafe { NonNull::new_unchecked(memory.address.as_ptr().add(memory.length).cast()) }
}

pub(super) fn initialize(memory: &Mapping) {
    // SAFETY: before spawn; this sealed, excluded page has exactly one writer.
    unsafe {
        slot(memory).as_ptr().write(Slot {
            handoff: AtomicU64::new(0),
            packet: [0; PACKET_BYTES],
        })
    };
}

pub(super) fn address(memory: &Mapping) -> usize {
    slot(memory).as_ptr() as usize
}

pub(super) unsafe fn write(address: usize, packet: Packet, handoff: u64) {
    let slot = address as *mut Slot;
    // SAFETY: caller owns the slot either before spawn, in the sole running
    // child, or after RECV+ID_VALID while that child is kernel-parked. Raw
    // access makes no reference that can survive a notification reply.
    unsafe {
        std::ptr::addr_of_mut!((*slot).packet).write(packet.encode());
        (*slot).handoff.store(handoff, Ordering::Release);
    }
}

pub(super) unsafe fn read(address: usize, handoff: u64) -> Result<Packet, NativeError> {
    let slot = address as *const Slot;
    // SAFETY: same exclusive handoff invariant as write; the copied packet
    // contains no pointers or Rust lifetimes and leaves no shared reference.
    let bytes = unsafe {
        if (*slot).handoff.load(Ordering::Acquire) != handoff {
            return Err(NativeError::Protocol);
        }
        std::ptr::addr_of!((*slot).packet).read()
    };
    Packet::decode(&bytes)
}

pub(super) fn wait(fd: i32, started: &Stopwatch, timeout: Duration) -> Result<i16, NativeError> {
    let remaining = timeout
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or(NativeError::Timeout)?;
    let mut descriptor = PollFd {
        fd,
        events: 1,
        returned: 0,
    };
    let time = Timespec {
        seconds: remaining
            .as_secs()
            .try_into()
            .map_err(|_| NativeError::Extent)?,
        nanos: remaining.subsec_nanos().into(),
    };
    // SAFETY: initialized fixed records; this descriptor has one owner. Unlike
    // bootstrap traffic, interruption of an active invocation tears it down.
    let result = unsafe { super::ppoll(&mut descriptor, 1, &time, std::ptr::null()) };
    if result == 0 {
        return Err(NativeError::Timeout);
    }
    if result < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(descriptor.returned)
}

#[repr(C)]
struct Vector {
    data: *mut u8,
    length: usize,
}
#[repr(C)]
struct Message {
    name: *mut std::ffi::c_void,
    name_length: u32,
    vectors: *mut Vector,
    vector_count: usize,
    control: *mut std::ffi::c_void,
    control_length: usize,
    flags: i32,
}
#[repr(C)]
struct Rights {
    length: usize,
    level: i32,
    kind: i32,
    fd: i32,
}

pub(super) fn send_listener(stream: i32, listener: OwnedFd) -> Result<(), NativeError> {
    let mut marker = 1u8;
    let mut vector = Vector {
        data: &mut marker,
        length: 1,
    };
    let mut rights = Rights {
        length: size_of::<usize>() + 12,
        level: 1,
        kind: 1,
        fd: listener.as_raw_fd(),
    };
    let message = Message {
        name: std::ptr::null_mut(),
        name_length: 0,
        vectors: &mut vector,
        vector_count: 1,
        control: (&mut rights as *mut Rights).cast(),
        control_length: size_of::<Rights>(),
        flags: 0,
    };
    // SAFETY: bootstrap-only SCM_RIGHTS transfers one descriptor. Both child
    // descriptors close before foreign execution, so the guest cannot reply to
    // its own notifications or export IPC descriptors.
    if unsafe { sendmsg(stream, &message, 0x4000) } != 1 {
        return Err(io::Error::last_os_error().into());
    }
    drop(listener);
    Ok(())
}

pub(super) fn receive_listener(
    stream: i32,
    started: &Stopwatch,
    timeout: Duration,
) -> Result<Listener, NativeError> {
    loop {
        wait(stream, started, timeout)?;
        let mut marker = 0u8;
        let mut vector = Vector {
            data: &mut marker,
            length: 1,
        };
        let mut rights = Rights {
            length: 0,
            level: 0,
            kind: 0,
            fd: -1,
        };
        let mut message = Message {
            name: std::ptr::null_mut(),
            name_length: 0,
            vectors: &mut vector,
            vector_count: 1,
            control: (&mut rights as *mut Rights).cast(),
            control_length: size_of::<Rights>(),
            flags: 0,
        };
        // SAFETY: initialized ancillary storage accepts exactly one fd and
        // MSG_CMSG_CLOEXEC prevents leaking the listener into future children.
        let count = unsafe { recvmsg(stream, &mut message, 0x40000000 | 0x40) };
        if count < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                continue;
            }
            return Err(error.into());
        }
        if count == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
        }
        // Take any installed descriptor before checking the envelope so a
        // malformed startup reply cannot leak an otherwise valid received fd.
        let listener = if rights.fd >= 0 && rights.level == 1 && rights.kind == 1 {
            // SAFETY: kernel returned this fresh SCM_RIGHTS descriptor.
            Some(unsafe { OwnedFd::from_raw_fd(rights.fd) })
        } else {
            None
        };
        if count != 1
            || marker != 1
            || message.flags & (8 | 32) != 0
            || rights.length != size_of::<usize>() + 12
        {
            return Err(NativeError::Protocol);
        }
        compiler_fence(Ordering::Acquire);
        return listener.map(Listener).ok_or(NativeError::Protocol);
    }
}
