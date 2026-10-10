//! Owned native children and memory published at stopped call boundaries.
use std::{io, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NativeAbi {
    SystemV = 0,
    Microsoft = 1,
}

#[derive(Clone, Copy, Debug)]
pub struct NativeRegion {
    pub offset: usize,
    pub length: usize,
    /// Native page rights: read=1, write=2, execute=4.
    pub permissions: u8,
}

pub struct NativeImage<'a> {
    pub base: u64,
    pub pointer_bytes: u8,
    pub bytes: &'a [u8],
    pub regions: &'a [NativeRegion],
    pub timeout: Duration,
}

#[derive(Clone, Copy, Debug)]
pub struct NativeCall {
    pub number: u32,
    pub arguments: [u64; 13],
}

#[derive(Debug)]
pub enum NativeError {
    Unsupported,
    Extent,
    Protocol,
    Timeout,
    Io(io::Error),
    Exited(std::process::ExitStatus),
    Callback,
}
impl From<io::Error> for NativeError {
    fn from(error: io::Error) -> Self {
        if matches!(
            error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ) {
            Self::Timeout
        } else {
            Self::Io(error)
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "native/linux.rs"]
mod implementation;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use implementation::{NativeProcess, native_child_bootstrap};

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub fn native_child_bootstrap() -> Option<i32> {
    std::env::args()
        .nth(1)
        .filter(|a| a == "--qa-native-child")
        .map(|_| 125)
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
#[path = "../tests/native/process.rs"]
mod tests;
