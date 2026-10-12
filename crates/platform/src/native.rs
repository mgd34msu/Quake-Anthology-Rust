//! Owned native children and memory published at stopped call boundaries.
use std::{io, time::Duration};

pub use qa_core::primitives::NATIVE_PAGE_BYTES as PAGE_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NativeAbi {
    SystemV = 0,
    Microsoft = 1,
}

mod call;
pub mod runtime;
pub use call::{NativeEntry, NativeScalar};

#[derive(Clone, Copy)]
pub struct NativeListFilter {
    /// Load-sized pointer-list banks, separate from the caller's output.
    pub buffer: u64,
    pub capacity: u32,
    pub depth: u8,
    pub list: u8,
    pub limit: u8,
    pub callback: u8,
    pub data: u8,
    pub keep: u32,
    pub end: u32,
}
impl NativeListFilter {
    pub(crate) fn valid(self, arguments: usize) -> bool {
        let slots = [self.list, self.limit, self.callback, self.data];
        arguments <= 13
            && self.capacity != 0
            && self.capacity <= 65536
            && self.depth != 0
            && self.depth <= 8
            && self.keep & self.end == 0
            && slots.iter().all(|&slot| usize::from(slot) < arguments)
            && (0..slots.len()).all(|i| (i + 1..slots.len()).all(|j| slots[i] != slots[j]))
    }
    pub fn byte_length(self) -> Option<usize> {
        self.byte_length_for_width(8)
    }
    pub fn byte_length_for_width(self, pointer_bytes: u8) -> Option<usize> {
        if !matches!(pointer_bytes, 4 | 8) {
            return None;
        }
        (self.capacity as usize)
            .checked_mul(usize::from(self.depth))?
            .checked_mul(usize::from(pointer_bytes))
    }
}

/// A load-selected bounded format adapter, executed inside the owned child.
/// The fixed descriptor ends with the format pointer; variadic values follow
/// the native calling convention rather than the thirteen-word IPC shape.
#[derive(Clone, Copy, Debug)]
pub struct NativeFormat {
    pub buffer: u64,
    /// Includes the terminating NUL. Classic Q2 uses 2048 for broadcasts,
    /// 1024 for console, client, center and fatal prints.
    pub capacity: u16,
    pub format: u8,
}
impl NativeFormat {
    fn valid(self, parameters: &[NativeScalar]) -> bool {
        self.capacity != 0
            && self.capacity <= 2048
            && parameters.len() == usize::from(self.format) + 1
            && parameters.len() <= 3
            && parameters.last() == Some(&NativeScalar::Word)
            && parameters
                .iter()
                .all(|p| matches!(p, NativeScalar::Word | NativeScalar::I32))
    }
}

#[derive(Clone, Copy)]
pub struct NativeImport<'a> {
    /// Missing imports have no scalar argument descriptor and abort only the
    /// current foreign call. Their names remain in the module's NameTable.
    pub trap: bool,
    pub number: u32,
    pub abi: NativeAbi,
    pub parameters: &'a [NativeScalar],
    pub result: NativeScalar,
    /// Optional module callback processing, executed only inside the child.
    pub filter: Option<NativeListFilter>,
    pub format: Option<NativeFormat>,
}

#[derive(Clone, Copy, Debug)]
pub struct NativeRegion {
    pub offset: usize,
    pub length: usize,
    /// Native byte-range rights: read=1, write=2, execute=4. Platform combines
    /// rights for ranges sharing an OS page; entry checks keep the byte ranges.
    pub permissions: u8,
}

pub(super) fn region_at(regions: &[NativeRegion], offset: usize, rights: u8) -> Option<usize> {
    let index = regions
        .partition_point(|region| region.offset <= offset)
        .checked_sub(1)?;
    let region = &regions[index];
    (region.permissions & rights == rights && offset - region.offset < region.length)
        .then_some(index)
}

pub struct NativeImage<'a> {
    pub base: u64,
    pub pointer_bytes: u8,
    pub bytes: &'a [u8],
    pub regions: &'a [NativeRegion],
    pub imports: &'a [NativeImport<'a>],
    pub timeout: Duration,
    pub runtime: Option<runtime::RuntimeConfig>,
}

#[derive(Clone, Copy, Debug)]
pub struct NativeCall {
    pub number: u32,
    pub arguments: [u64; 13],
    pub function: bool,
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
    RuntimeImport(&'static str),
    ImportTrap { ordinal: usize, address: u64 },
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

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "x86")))]
#[path = "native/linux.rs"]
mod implementation;
#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "x86")))]
pub use implementation::{NativeProcess, native_child_bootstrap};

#[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "x86"))))]
pub fn native_child_bootstrap() -> Option<i32> {
    std::env::args()
        .nth(1)
        .filter(|a| a == "--qa-native-child")
        .map(|_| 125)
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
#[path = "../tests/native/process.rs"]
mod tests;
