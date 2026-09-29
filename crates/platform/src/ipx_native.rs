//! Native AF_IPX datagram sockets.
//!
//! Port of donor `src/platform/ipx-native.ts` onto `std` plus direct libc
//! symbols (`#[link(name = "c")]`, always present on Linux glibc) and the
//! system Winsock library on Windows. Address layouts follow glibc
//! `netipx/ipx.h` and Winsock `wsipx.h`/`wsnwlink.h`.

use std::time::Instant;

use crate::error::{Error, Result};
use crate::ipx::{ipx_address, IpxAddress, NativeIpxBindOptions, ReceiveEvent};

/// Maximum portable IPX payload: 576-byte MTU minus the 30-byte header.
pub const LINUX_MAX_DATAGRAM_BYTES: usize = 546;

/// A bound native IPX socket.
///
/// Sockets are shared with the readability watcher thread, so every method
/// takes `&self` and implementations must be thread-safe (the underlying
/// datagram syscalls are atomic with respect to each other).
pub trait NativeSocket: Send + Sync {
    /// Local bound address.
    fn address(&self) -> IpxAddress;
    /// Maximum payload bytes per datagram.
    fn max_datagram_bytes(&self) -> usize;
    /// Send one datagram; `Ok(false)` means the backend is transiently busy.
    fn send(&self, to: &IpxAddress, payload: &[u8]) -> Result<bool>;
    /// Receive one pending datagram, or `None` when idle.
    fn receive(&self) -> Result<Option<ReceiveEvent>>;
    /// True when a datagram (or a fatal condition) is pending.
    fn readable(&self) -> Result<bool>;
    /// Close the socket, releasing its descriptor. Idempotent.
    fn close(&self);
}

/// Validate bind options like the donor: broadcast is mandatory; the port and
/// packet type ranges are enforced by their integer types.
pub fn checked_options(options: &NativeIpxBindOptions) -> Result<()> {
    if !options.broadcast {
        return Err(Error::OutOfRange("invalid native IPX socket options".to_string()));
    }
    Ok(())
}

/// Encode a Linux (16-byte) or Windows (14-byte) `sockaddr_ipx`.
pub fn sockaddr_bytes(windows: bool, port: u16, packet_type: u8, address: Option<&IpxAddress>) -> Vec<u8> {
    let mut bytes = vec![0u8; if windows { 14 } else { 16 }];
    bytes[0..2].copy_from_slice(&(if windows { 6u16 } else { 4u16 }).to_le_bytes());
    if windows {
        bytes[12..14].copy_from_slice(&port.to_be_bytes());
        bytes[2..6].copy_from_slice(&address.map_or(0, |a| a.network).to_be_bytes());
        if let Some(a) = address {
            bytes[6..12].copy_from_slice(&a.node);
        }
    } else {
        bytes[2..4].copy_from_slice(&port.to_be_bytes());
        bytes[4..8].copy_from_slice(&address.map_or(0, |a| a.network).to_be_bytes());
        if let Some(a) = address {
            bytes[8..14].copy_from_slice(&a.node);
        }
        bytes[14] = packet_type;
    }
    bytes
}

/// Decode an address returned by the kernel, rejecting truncated or foreign families.
pub fn address_from_bytes(bytes: &[u8], length: usize, windows: bool) -> Result<IpxAddress> {
    let needed = if windows { 14 } else { 16 };
    if length < needed || bytes.len() < needed {
        return Err(Error::native(
            "getsockname",
            "native IPX returned a truncated address".to_string(),
        ));
    }
    let family = u16::from_le_bytes([bytes[0], bytes[1]]);
    if family != if windows { 6 } else { 4 } {
        return Err(Error::native(
            "getsockname",
            "native IPX returned another address family".to_string(),
        ));
    }
    let (network, start, port) = if windows {
        (
            u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
            6,
            u16::from_be_bytes([bytes[12], bytes[13]]),
        )
    } else {
        (
            u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            8,
            u16::from_be_bytes([bytes[2], bytes[3]]),
        )
    };
    let mut node = [0u8; 6];
    node.copy_from_slice(&bytes[start..start + 6]);
    ipx_address(network, node, port)
}

/// True when `code` means the OS has no installed IPX datagram provider.
pub fn is_family_missing(code: i32, windows: bool) -> bool {
    if windows {
        code == 10047 || code == 10043 || code == 10044
    } else {
        code == 97 || code == 93 || code == 94
    }
}

/// True when `code` means no usable configured IPX interface or route.
pub fn is_no_interface(code: i32, windows: bool) -> bool {
    if windows {
        code == 10049 || code == 10050 || code == 10051
    } else {
        code == 99 || code == 100 || code == 101
    }
}

/// Build the donor's `failure()` error for an AF_IPX operation.
pub fn ipx_failure(operation: &str, code: i32, windows: bool) -> Error {
    let name = if windows {
        format!("WSA error {code}")
    } else {
        format!("{} ({code})", std::io::Error::from_raw_os_error(code))
    };
    let detail = format!(
        "{} AF_IPX {operation}: {name}",
        if windows { "Winsock" } else { "Linux" }
    );
    if is_family_missing(code, windows) {
        return Error::Unsupported(format!(
            "ipx-native: {detail}; the OS has no installed IPX datagram provider"
        ));
    }
    if is_no_interface(code, windows) {
        return Error::native(
            operation,
            format!("{detail}; no usable configured IPX interface or route"),
        );
    }
    Error::native(operation, detail)
}

/// Monotonic receive timestamp in the donor's millisecond shape.
pub fn received_at_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

/// Bind a native IPX socket for `options` on the host ABI.
pub fn bind_native_ipx_socket(options: NativeIpxBindOptions) -> Result<std::sync::Arc<dyn NativeSocket>> {
    checked_options(&options)?;
    #[cfg(target_os = "linux")]
    {
        linux::bind_socket(options)
    }
    #[cfg(windows)]
    {
        windows::bind(options)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = options;
        Err(Error::Unsupported(
            "no AF_IPX socket ABI is implemented for this platform".to_string(),
        ))
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::ffi::{c_int, c_void};
    use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};

    #[link(name = "c")]
    unsafe extern "C" {
        fn socket(domain: c_int, kind: c_int, protocol: c_int) -> c_int;
        fn bind(fd: c_int, addr: *const c_void, len: u32) -> c_int;
        fn getsockname(fd: c_int, addr: *mut c_void, len: *mut u32) -> c_int;
        fn setsockopt(fd: c_int, level: c_int, name: c_int, value: *const c_void, len: u32) -> c_int;
        fn sendto(fd: c_int, buf: *const c_void, len: usize, flags: c_int, dest: *const c_void, addrlen: u32) -> isize;
        fn recvfrom(
            fd: c_int,
            buf: *mut c_void,
            len: usize,
            flags: c_int,
            addr: *mut c_void,
            addrlen: *mut u32,
        ) -> isize;
        fn poll(fds: *mut PollFd, nfds: u64, timeout: c_int) -> c_int;
        fn close(fd: c_int) -> c_int;
        fn __errno_location() -> *mut c_int;
    }

    #[repr(C)]
    struct PollFd {
        fd: c_int,
        events: i16,
        revents: i16,
    }

    const AF_IPX: c_int = 4;
    const SOCK_DGRAM: c_int = 2;
    const SOCK_NONBLOCK: c_int = 0x800;
    const SOCK_CLOEXEC: c_int = 0x80000;
    const SOL_SOCKET: c_int = 1;
    const SO_BROADCAST: c_int = 6;
    const SOL_IPX: c_int = 256;
    const IPX_TYPE: c_int = 1;
    const MSG_NOSIGNAL: c_int = 0x4000;

    fn errno() -> i32 {
        // SAFETY: libc always provides errno storage.
        unsafe { *__errno_location() }
    }

    pub struct LinuxIpxSocket {
        fd: OwnedFd,
        address: IpxAddress,
        packet_type: u8,
        epoch: Instant,
        closed: std::sync::atomic::AtomicBool,
    }

    pub fn bind_socket(options: NativeIpxBindOptions) -> Result<std::sync::Arc<dyn NativeSocket>> {
        // SAFETY: direct libc calls with validated arguments; every result checked.
        unsafe {
            // SOCK_DGRAM | SOCK_NONBLOCK | SOCK_CLOEXEC.
            let fd = socket(AF_IPX, SOCK_DGRAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
            if fd < 0 {
                return Err(ipx_failure("socket", errno(), false));
            }
            let result = bind_inner(fd, &options);
            match result {
                Ok(socket) => Ok(std::sync::Arc::new(socket)),
                Err(error) => {
                    close(fd);
                    Err(error)
                }
            }
        }
    }

    /// # Safety
    ///
    /// `fd` must be a live IPX datagram descriptor owned by the caller; on
    /// success ownership moves into the returned socket.
    unsafe fn bind_inner(fd: c_int, options: &NativeIpxBindOptions) -> Result<LinuxIpxSocket> {
        // SAFETY: caller guarantees a live descriptor; pointers describe live data.
        unsafe {
            let one: c_int = 1;
            let packet: c_int = c_int::from(options.packet_type);
            if setsockopt(fd, SOL_SOCKET, SO_BROADCAST, std::ptr::addr_of!(one).cast(), 4) < 0 {
                return Err(ipx_failure("SO_BROADCAST", errno(), false));
            }
            if setsockopt(fd, SOL_IPX, IPX_TYPE, std::ptr::addr_of!(packet).cast(), 4) < 0 {
                return Err(ipx_failure("IPX_TYPE", errno(), false));
            }
            let mut local = sockaddr_bytes(false, options.port, options.packet_type, None);
            if bind(fd, local.as_ptr().cast(), local.len() as u32) < 0 {
                return Err(ipx_failure("bind", errno(), false));
            }
            let mut length = local.len() as u32;
            if getsockname(fd, local.as_mut_ptr().cast(), &mut length) < 0 {
                return Err(ipx_failure("getsockname", errno(), false));
            }
            let address = address_from_bytes(&local, length as usize, false)?;
            Ok(LinuxIpxSocket {
                fd: OwnedFd::from_raw_fd(fd),
                address,
                packet_type: options.packet_type,
                epoch: Instant::now(),
                closed: std::sync::atomic::AtomicBool::new(false),
            })
        }
    }

    impl NativeSocket for LinuxIpxSocket {
        fn address(&self) -> IpxAddress {
            self.address.clone()
        }

        fn max_datagram_bytes(&self) -> usize {
            LINUX_MAX_DATAGRAM_BYTES
        }

        fn send(&self, to: &IpxAddress, payload: &[u8]) -> Result<bool> {
            self.check_open()?;
            let target = sockaddr_bytes(false, to.port, self.packet_type, Some(to));
            let one = [0u8; 1];
            let buf = if payload.is_empty() { &one[..] } else { payload };
            // SAFETY: pointers and lengths describe live buffers.
            let sent = unsafe {
                sendto(
                    self.fd.as_raw_fd(),
                    buf.as_ptr().cast(),
                    payload.len(),
                    MSG_NOSIGNAL,
                    target.as_ptr().cast(),
                    target.len() as u32,
                )
            };
            if sent < 0 {
                let code = errno();
                if code == 11 || code == 4 || code == 105 {
                    return Ok(false);
                }
                return Err(ipx_failure("sendto", code, false));
            }
            if sent as usize != payload.len() {
                return Err(Error::native("sendto", "Linux IPX sent a partial datagram".to_string()));
            }
            Ok(true)
        }

        fn receive(&self) -> Result<Option<ReceiveEvent>> {
            self.check_open()?;
            let mut buffer = vec![0u8; 65535];
            let mut from = [0u8; 16];
            let mut from_length = from.len() as u32;
            // SAFETY: pointers and lengths describe live buffers.
            let size = unsafe {
                recvfrom(
                    self.fd.as_raw_fd(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    0,
                    from.as_mut_ptr().cast(),
                    &mut from_length,
                )
            };
            if size < 0 {
                let code = errno();
                if code == 11 || code == 4 {
                    return Ok(None);
                }
                return Err(ipx_failure("recvfrom", code, false));
            }
            let sender = address_from_bytes(&from, from_length as usize, false)?;
            let size = size as usize;
            if size > LINUX_MAX_DATAGRAM_BYTES {
                return Ok(Some(ReceiveEvent::Dropped {
                    reason: "oversize".to_string(),
                    from: sender,
                }));
            }
            buffer.truncate(size);
            Ok(Some(ReceiveEvent::Packet {
                from: sender,
                payload: buffer,
                received_at_ms: received_at_ms(self.epoch),
            }))
        }

        fn readable(&self) -> Result<bool> {
            self.check_open()?;
            let mut pollfd = PollFd {
                fd: self.fd.as_raw_fd(),
                events: 1,
                revents: 0,
            };
            // SAFETY: `pollfd` is a valid single-entry array.
            let result = unsafe { poll(&mut pollfd, 1, 0) };
            if result < 0 {
                let code = errno();
                if code == 4 {
                    return Ok(false);
                }
                return Err(ipx_failure("poll", code, false));
            }
            let events = pollfd.revents;
            if events & 32 != 0 {
                return Err(Error::native("poll", "Linux IPX descriptor is invalid".to_string()));
            }
            if events & 24 != 0 {
                return Err(Error::native(
                    "poll",
                    "Linux IPX socket reported a device error or hangup".to_string(),
                ));
            }
            Ok(result > 0 && events & 1 != 0)
        }

        fn close(&self) {
            use std::sync::atomic::Ordering;
            self.closed.store(true, Ordering::SeqCst);
        }
    }

    impl LinuxIpxSocket {
        fn check_open(&self) -> Result<()> {
            use std::sync::atomic::Ordering;
            if self.closed.load(Ordering::SeqCst) {
                return Err(Error::Closed("native IPX socket".to_string()));
            }
            Ok(())
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::ffi::c_void;

    const INVALID_SOCKET: u64 = 0xffff_ffff_ffff_ffff;

    struct Ws2 {
        _library: libloading::Library,
        wsa_startup: unsafe extern "C" fn(u16, *mut u8) -> i32,
        wsa_cleanup: unsafe extern "C" fn() -> i32,
        wsa_get_last_error: unsafe extern "C" fn() -> i32,
        socket: unsafe extern "C" fn(i32, i32, i32) -> u64,
        closesocket: unsafe extern "C" fn(u64) -> i32,
        ioctlsocket: unsafe extern "C" fn(u64, u32, *const u32) -> i32,
        bind: unsafe extern "C" fn(u64, *const u8, i32) -> i32,
        getsockname: unsafe extern "C" fn(u64, *mut u8, *mut i32) -> i32,
        setsockopt: unsafe extern "C" fn(u64, i32, i32, *const i32, i32) -> i32,
        getsockopt: unsafe extern "C" fn(u64, i32, i32, *mut u32, *mut i32) -> i32,
        sendto: unsafe extern "C" fn(u64, *const u8, i32, i32, *const u8, i32) -> i32,
        recvfrom: unsafe extern "C" fn(u64, *mut u8, i32, i32, *mut u8, *mut i32) -> i32,
        select: unsafe extern "C" fn(i32, *mut u8, *const c_void, *const c_void, *mut i32) -> i32,
    }

    impl Ws2 {
        /// # Safety
        ///
        /// Loading maps the system image; each resolved symbol is called with
        /// the documented Winsock ABI below.
        unsafe fn load() -> Result<Self> {
            // SAFETY: mapping the system Winsock image invokes no foreign code.
            let library = unsafe { libloading::Library::new("ws2_32.dll") }
                .map_err(|error| Error::unavailable("ws2_32", error.to_string()))?;
            macro_rules! sym {
                ($name:literal, $ty:ty) => {
                    // SAFETY: the address is only read; the type matches ws2_32.
                    *unsafe { library.get::<$ty>(concat!($name, "\0").as_bytes()) }
                        .map_err(|_| Error::unavailable("ws2_32", format!("symbol `{}` is missing", $name)))?
                };
            }
            // SAFETY: every symbol type matches the ws2_32 export.
            unsafe {
                Ok(Self {
                    wsa_startup: sym!("WSAStartup", unsafe extern "C" fn(u16, *mut u8) -> i32),
                    wsa_cleanup: sym!("WSACleanup", unsafe extern "C" fn() -> i32),
                    wsa_get_last_error: sym!("WSAGetLastError", unsafe extern "C" fn() -> i32),
                    socket: sym!("socket", unsafe extern "C" fn(i32, i32, i32) -> u64),
                    closesocket: sym!("closesocket", unsafe extern "C" fn(u64) -> i32),
                    ioctlsocket: sym!("ioctlsocket", unsafe extern "C" fn(u64, u32, *const u32) -> i32),
                    bind: sym!("bind", unsafe extern "C" fn(u64, *const u8, i32) -> i32),
                    getsockname: sym!("getsockname", unsafe extern "C" fn(u64, *mut u8, *mut i32) -> i32),
                    setsockopt: sym!(
                        "setsockopt",
                        unsafe extern "C" fn(u64, i32, i32, *const i32, i32) -> i32
                    ),
                    getsockopt: sym!(
                        "getsockopt",
                        unsafe extern "C" fn(u64, i32, i32, *mut u32, *mut i32) -> i32
                    ),
                    sendto: sym!(
                        "sendto",
                        unsafe extern "C" fn(u64, *const u8, i32, i32, *const u8, i32) -> i32
                    ),
                    recvfrom: sym!(
                        "recvfrom",
                        unsafe extern "C" fn(u64, *mut u8, i32, i32, *mut u8, *mut i32) -> i32
                    ),
                    select: sym!(
                        "select",
                        unsafe extern "C" fn(i32, *mut u8, *const c_void, *const c_void, *mut i32) -> i32
                    ),
                    _library: library,
                })
            }
        }
    }

    pub struct WindowsIpxSocket {
        ws2: Ws2,
        socket: u64,
        address: IpxAddress,
        max_datagram: usize,
        packet_type: u8,
        epoch: Instant,
        released: std::sync::atomic::AtomicBool,
    }

    // SAFETY: Winsock calls on one datagram socket from multiple threads are
    // mutually atomic; `released` is the only shared flag and it is atomic.
    unsafe impl Send for WindowsIpxSocket {}
    unsafe impl Sync for WindowsIpxSocket {}

    pub fn bind(options: NativeIpxBindOptions) -> Result<std::sync::Arc<dyn NativeSocket>> {
        // SAFETY: Winsock calls below use validated arguments; errors checked.
        unsafe {
            let ws2 = Ws2::load()?;
            let mut data = vec![0u8; 512];
            let startup = (ws2.wsa_startup)(0x0202, data.as_mut_ptr());
            if startup != 0 {
                return Err(ipx_failure("WSAStartup", startup, true));
            }
            let version = u16::from_le_bytes([data[0], data[1]]);
            if version != 0x0202 {
                (ws2.wsa_cleanup)();
                return Err(Error::Unsupported("ipx-native: Winsock 2.2 is unavailable".to_string()));
            }
            let socket = (ws2.socket)(6, 2, 1000);
            if socket == INVALID_SOCKET {
                let code = (ws2.wsa_get_last_error)();
                (ws2.wsa_cleanup)();
                return Err(ipx_failure("socket", code, true));
            }
            let result = bind_inner(&ws2, socket, &options);
            match result {
                Ok(part) => Ok(std::sync::Arc::new(WindowsIpxSocket {
                    ws2,
                    socket,
                    address: part.0,
                    max_datagram: part.1,
                    packet_type: options.packet_type,
                    epoch: Instant::now(),
                    released: std::sync::atomic::AtomicBool::new(false),
                })),
                Err(error) => {
                    (ws2.closesocket)(socket);
                    (ws2.wsa_cleanup)();
                    Err(error)
                }
            }
        }
    }

    /// # Safety
    ///
    /// `socket` must be a live Winsock descriptor; `ws2` must outlive the call.
    unsafe fn bind_inner(ws2: &Ws2, socket: u64, options: &NativeIpxBindOptions) -> Result<(IpxAddress, usize)> {
        // SAFETY: caller guarantees a live socket; pointers describe live data.
        unsafe {
            let check = |result: i32, operation: &str| -> Result<()> {
                if result == -1 {
                    // SAFETY: error read immediately after the failing call.
                    let code = unsafe { (ws2.wsa_get_last_error)() };
                    return Err(ipx_failure(operation, code, true));
                }
                Ok(())
            };
            let one_u32: u32 = 1;
            check((ws2.ioctlsocket)(socket, 0x8004_667e, &one_u32), "FIONBIO")?;
            let one: i32 = 1;
            check((ws2.setsockopt)(socket, 0xffff, 0x20, &one, 4), "SO_BROADCAST")?;
            let packet = i32::from(options.packet_type);
            check((ws2.setsockopt)(socket, 1000, 0x4000, &packet, 4), "IPX_PTYPE")?;
            let mut local = sockaddr_bytes(true, options.port, options.packet_type, None);
            check((ws2.bind)(socket, local.as_ptr(), local.len() as i32), "bind")?;
            let mut address_length = local.len() as i32;
            check(
                (ws2.getsockname)(socket, local.as_mut_ptr(), &mut address_length),
                "getsockname",
            )?;
            let mut maximum: u32 = 0;
            let mut maximum_length: i32 = 4;
            check(
                (ws2.getsockopt)(socket, 1000, 0x4006, &mut maximum, &mut maximum_length),
                "IPX_MAXSIZE",
            )?;
            if maximum_length != 4 || maximum < 1 || maximum > 65505 {
                return Err(Error::native(
                    "getsockopt",
                    "Winsock IPX returned an invalid maximum datagram size".to_string(),
                ));
            }
            let address = address_from_bytes(&local, address_length as usize, true)?;
            Ok((address, maximum as usize))
        }
    }

    impl NativeSocket for WindowsIpxSocket {
        fn address(&self) -> IpxAddress {
            self.address.clone()
        }

        fn max_datagram_bytes(&self) -> usize {
            self.max_datagram
        }

        fn send(&self, to: &IpxAddress, payload: &[u8]) -> Result<bool> {
            self.check_open()?;
            self.send_inner(to, payload)
        }

        fn receive(&self) -> Result<Option<ReceiveEvent>> {
            self.check_open()?;
            self.receive_inner()
        }

        fn readable(&self) -> Result<bool> {
            self.check_open()?;
            self.readable_inner()
        }

        fn close(&self) {
            use std::sync::atomic::Ordering;
            if self.released.swap(true, Ordering::SeqCst) {
                return;
            }
            // SAFETY: the socket and WSA state are live until these calls return.
            unsafe {
                (self.ws2.closesocket)(self.socket);
                (self.ws2.wsa_cleanup)();
            }
        }
    }

    impl WindowsIpxSocket {
        fn check_open(&self) -> Result<()> {
            use std::sync::atomic::Ordering;
            if self.released.load(Ordering::SeqCst) {
                return Err(Error::Closed("native IPX socket".to_string()));
            }
            Ok(())
        }

        fn send_inner(&self, to: &IpxAddress, payload: &[u8]) -> Result<bool> {
            let target = sockaddr_bytes(true, to.port, self.packet_type, Some(to));
            let one = [0u8; 1];
            let buf = if payload.is_empty() { &one[..] } else { payload };
            // SAFETY: pointers and lengths describe live buffers.
            let sent = unsafe {
                (self.ws2.sendto)(
                    self.socket,
                    buf.as_ptr(),
                    payload.len() as i32,
                    0,
                    target.as_ptr(),
                    target.len() as i32,
                )
            };
            if sent == -1 {
                // SAFETY: error read immediately after the failing call.
                let code = unsafe { (self.ws2.wsa_get_last_error)() };
                if code == 10035 || code == 10004 || code == 10055 {
                    return Ok(false);
                }
                return Err(ipx_failure("sendto", code, true));
            }
            if sent as usize != payload.len() {
                return Err(Error::native(
                    "sendto",
                    "Winsock IPX sent a partial datagram".to_string(),
                ));
            }
            Ok(true)
        }

        fn receive_inner(&self) -> Result<Option<ReceiveEvent>> {
            let mut buffer = vec![0u8; 65535];
            let mut from = vec![0u8; 14];
            let mut from_length = from.len() as i32;
            // SAFETY: pointers and lengths describe live buffers.
            let size = unsafe {
                (self.ws2.recvfrom)(
                    self.socket,
                    buffer.as_mut_ptr(),
                    buffer.len() as i32,
                    0,
                    from.as_mut_ptr(),
                    &mut from_length,
                )
            };
            if size == -1 {
                // SAFETY: error read immediately after the failing call.
                let code = unsafe { (self.ws2.wsa_get_last_error)() };
                if code == 10035 || code == 10004 {
                    return Ok(None);
                }
                return Err(ipx_failure("recvfrom", code, true));
            }
            let sender = address_from_bytes(&from, from_length as usize, true)?;
            let size = size as usize;
            if size > self.max_datagram {
                return Ok(Some(ReceiveEvent::Dropped {
                    reason: "oversize".to_string(),
                    from: sender,
                }));
            }
            buffer.truncate(size);
            Ok(Some(ReceiveEvent::Packet {
                from: sender,
                payload: buffer,
                received_at_ms: received_at_ms(self.epoch),
            }))
        }

        fn readable_inner(&self) -> Result<bool> {
            let mut read_set = vec![0u8; 520];
            read_set[0..4].copy_from_slice(&1u32.to_le_bytes());
            read_set[8..16].copy_from_slice(&self.socket.to_le_bytes());
            let mut timeout = [0i32; 2];
            // SAFETY: the fd_set and timeout describe live buffers.
            let result = unsafe {
                (self.ws2.select)(
                    0,
                    read_set.as_mut_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    timeout.as_mut_ptr(),
                )
            };
            if result == -1 {
                // SAFETY: error read immediately after the failing call.
                let code = unsafe { (self.ws2.wsa_get_last_error)() };
                if code == 10004 {
                    return Ok(false);
                }
                return Err(ipx_failure("select", code, true));
            }
            Ok(result > 0)
        }
    }

    impl Drop for WindowsIpxSocket {
        fn drop(&mut self) {
            self.close();
        }
    }
}
