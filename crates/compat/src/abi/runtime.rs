//! C library prototypes share the numbered CallTable and module memory.
use super::{CallError, Entry, Invocation};
use qa_platform::native::NativeScalar;

pub const FIRST: u32 = 256;
pub(crate) struct Function {
    pub name: &'static [u8],
    pub number: u32,
    pub provider: &'static [u8],
    pub versions: &'static [&'static [u8]],
    pub parameters: &'static [NativeScalar],
    pub result: NativeScalar,
    pub(super) entry: Entry,
}
use NativeScalar::{Double, I32, Word};
const LIBC: &[u8] = b"libc.so.6";
const LIBM: &[u8] = b"libm.so.6";
const BASE_VERSION: &[&[u8]] = &[b"GLIBC_2.2.5"];
pub(crate) const FUNCTIONS: &[Function] = &[
    Function {
        name: b"memcpy",
        provider: LIBC,
        versions: &[b"GLIBC_2.2.5", b"GLIBC_2.14"],
        number: FIRST,
        parameters: &[Word, Word, Word],
        result: Word,
        entry: super::memcpy,
    },
    Function {
        name: b"memmove",
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST,
        parameters: &[Word, Word, Word],
        result: Word,
        entry: super::memcpy,
    },
    Function {
        name: b"memset",
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST + 1,
        parameters: &[Word, I32, Word],
        result: Word,
        entry: super::memset,
    },
    Function {
        name: b"strncpy",
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST + 2,
        parameters: &[Word, Word, Word],
        result: Word,
        entry: super::strncpy,
    },
    Function {
        name: b"strlen",
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST + 3,
        parameters: &[Word],
        result: Word,
        entry: length,
    },
    Function {
        name: b"strcmp",
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST + 4,
        parameters: &[Word, Word],
        result: I32,
        entry: compare_string,
    },
    Function {
        name: b"memcmp",
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
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        entry: super::sin::<true>,
    },
    Function {
        name: b"cos",
        number: FIRST + 7,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        entry: super::cos::<true>,
    },
    Function {
        name: b"atan2",
        number: FIRST + 8,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double, Double],
        result: Double,
        entry: super::atan2::<true>,
    },
    Function {
        name: b"sqrt",
        number: FIRST + 9,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        entry: super::sqrt::<true>,
    },
    Function {
        name: b"floor",
        number: FIRST + 10,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        entry: super::floor::<true>,
    },
    Function {
        name: b"ceil",
        number: FIRST + 11,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        entry: super::ceil::<true>,
    },
    Function {
        name: b"acos",
        number: FIRST + 12,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        entry: super::acos::<true>,
    },
    Function {
        name: b"fabs",
        number: FIRST + 13,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        entry: super::absolute::<true>,
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
