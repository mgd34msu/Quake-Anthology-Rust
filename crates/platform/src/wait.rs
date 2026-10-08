use std::{net::UdpSocket, time::Duration};

#[cfg(target_os = "linux")]
pub fn readable(sockets: &[UdpSocket], timeout: Duration) {
    use std::os::fd::AsRawFd;
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }
    unsafe extern "C" {
        fn poll(fds: *mut PollFd, count: usize, timeout: i32) -> i32;
    }
    let mut descriptors = [PollFd {
        fd: -1,
        events: 1,
        revents: 0,
    }; 8];
    for (descriptor, socket) in descriptors.iter_mut().zip(sockets) {
        descriptor.fd = socket.as_raw_fd();
    }
    let milliseconds = timeout.as_nanos().div_ceil(1_000_000).min(i32::MAX as u128) as i32;
    // SAFETY: descriptors is initialized storage for all owned socket fds;
    // EventPump admits at most eight sockets and retains them during the call.
    let _ = unsafe { poll(descriptors.as_mut_ptr(), sockets.len(), milliseconds) };
}

#[cfg(not(target_os = "linux"))]
pub fn readable(_sockets: &[UdpSocket], timeout: Duration) {
    crate::clock::pause(timeout);
}
