//! C library prototypes share the numbered CallTable and module memory.
use super::{CallError, Entry, Invocation};
use qa_platform::native::NativeScalar;

pub const FIRST: u32 = 256;
pub(crate) struct Function {
    pub name: &'static [u8],
    pub number: u32,
    pub heap: bool,
    pub provider: &'static [u8],
    pub versions: &'static [&'static [u8]],
    pub parameters: &'static [NativeScalar],
    pub result: NativeScalar,
    pub(super) entry: Entry,
}
use NativeScalar::{Double, I32, Void, Word};
const LIBC: &[u8] = b"libc.so.6";
const LIBM: &[u8] = b"libm.so.6";
const BASE_VERSION: &[&[u8]] = &[b"GLIBC_2.2.5"];
pub(crate) const FUNCTIONS: &[Function] = &[
    Function {
        name: b"memcpy",
        heap: false,
        provider: LIBC,
        versions: &[b"GLIBC_2.2.5", b"GLIBC_2.14"],
        number: FIRST,
        parameters: &[Word, Word, Word],
        result: Word,
        entry: super::memcpy,
    },
    Function {
        name: b"memmove",
        heap: false,
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST,
        parameters: &[Word, Word, Word],
        result: Word,
        entry: super::memcpy,
    },
    Function {
        name: b"memset",
        heap: false,
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST + 1,
        parameters: &[Word, I32, Word],
        result: Word,
        entry: super::memset,
    },
    Function {
        name: b"strncpy",
        heap: false,
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST + 2,
        parameters: &[Word, Word, Word],
        result: Word,
        entry: super::strncpy,
    },
    Function {
        name: b"strlen",
        heap: false,
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST + 3,
        parameters: &[Word],
        result: Word,
        entry: length,
    },
    Function {
        name: b"strcmp",
        heap: false,
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST + 4,
        parameters: &[Word, Word],
        result: I32,
        entry: compare_string,
    },
    Function {
        name: b"memcmp",
        heap: false,
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST + 5,
        parameters: &[Word, Word, Word],
        result: I32,
        entry: compare_memory,
    },
    Function {
        name: b"sin",
        number: FIRST + 6,
        heap: false,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        entry: super::sin::<true>,
    },
    Function {
        name: b"cos",
        number: FIRST + 7,
        heap: false,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        entry: super::cos::<true>,
    },
    Function {
        name: b"atan2",
        number: FIRST + 8,
        heap: false,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double, Double],
        result: Double,
        entry: super::atan2::<true>,
    },
    Function {
        name: b"sqrt",
        number: FIRST + 9,
        heap: false,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        entry: super::sqrt::<true>,
    },
    Function {
        name: b"floor",
        number: FIRST + 10,
        heap: false,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        entry: super::floor::<true>,
    },
    Function {
        name: b"ceil",
        number: FIRST + 11,
        heap: false,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        entry: super::ceil::<true>,
    },
    Function {
        name: b"acos",
        number: FIRST + 12,
        heap: false,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        entry: super::acos::<true>,
    },
    Function {
        name: b"fabs",
        number: FIRST + 13,
        heap: false,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        entry: super::absolute::<true>,
    },
    Function {
        name: b"malloc",
        number: FIRST + 14,
        heap: true,
        provider: LIBC,
        versions: BASE_VERSION,
        parameters: &[Word],
        result: Word,
        entry: malloc,
    },
    Function {
        name: b"calloc",
        number: FIRST + 15,
        heap: true,
        provider: LIBC,
        versions: BASE_VERSION,
        parameters: &[Word, Word],
        result: Word,
        entry: calloc,
    },
    Function {
        name: b"realloc",
        number: FIRST + 16,
        heap: true,
        provider: LIBC,
        versions: BASE_VERSION,
        parameters: &[Word, Word],
        result: Word,
        entry: realloc,
    },
    Function {
        name: b"free",
        number: FIRST + 17,
        heap: true,
        provider: LIBC,
        versions: BASE_VERSION,
        parameters: &[Word],
        result: Void,
        entry: free,
    },
];
fn length(call: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(call.string(0)?.len() as u64)
}
fn difference(left: &[u8], right: &[u8]) -> u64 {
    let difference = left
        .iter()
        .copied()
        .chain([0])
        .zip(right.iter().copied().chain([0]))
        .find_map(|(a, b)| (a != b).then_some(i32::from(a) - i32::from(b)))
        .unwrap_or(0);
    difference as i64 as u64
}
fn compare_string(call: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(difference(call.string(0)?, call.string(1)?))
}
fn compare_memory(call: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let length = call.length(2)?;
    Ok(difference(
        call.memory.read(call.pointer(0)?, length)?,
        call.memory.read(call.pointer(1)?, length)?,
    ))
}

fn malloc(call: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let bytes = call.length(0)?;
    Ok(call
        .heap
        .as_mut()
        .ok_or(CallError::Memory)?
        .allocate(bytes)
        .unwrap_or(0))
}
fn calloc(call: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let count = call.length(0)?;
    let bytes = call.length(1)?;
    let heap = call.heap.as_mut().ok_or(CallError::Memory)?;
    let Some(bytes) = count.checked_mul(bytes) else {
        return Ok(0);
    };
    let Some(address) = heap.allocate(bytes) else {
        return Ok(0);
    };
    if let Ok(storage) = call.memory.read_mut(address, bytes) {
        storage.fill(0);
        Ok(address)
    } else {
        heap.free(address)?;
        Err(CallError::Memory)
    }
}
fn realloc(call: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let address = call.pointer(0)?;
    let bytes = call.length(1)?;
    Ok(call
        .heap
        .as_mut()
        .ok_or(CallError::Memory)?
        .reallocate(call.memory, address, bytes)?)
}
fn free(call: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let address = call.pointer(0)?;
    call.heap.as_mut().ok_or(CallError::Memory)?.free(address)?;
    Ok(0)
}
