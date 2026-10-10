//! C library prototypes share the numbered CallTable and module memory.
use super::{CallError, Entry, Invocation};
use qa_platform::native::NativeScalar;

pub const FIRST: u32 = 256;
pub(crate) struct Function {
    pub name: &'static [u8],
    pub number: u32,
    pub parameters: &'static [NativeScalar],
    pub result: NativeScalar,
    pub(super) entry: Entry,
}
use NativeScalar::{I32, Word};
pub(crate) const FUNCTIONS: &[Function] = &[
    Function {
        name: b"memcpy",
        number: FIRST,
        parameters: &[Word, Word, Word],
        result: Word,
        entry: super::memcpy,
    },
    Function {
        name: b"memmove",
        number: FIRST,
        parameters: &[Word, Word, Word],
        result: Word,
        entry: super::memcpy,
    },
    Function {
        name: b"memset",
        number: FIRST + 1,
        parameters: &[Word, I32, Word],
        result: Word,
        entry: super::memset,
    },
    Function {
        name: b"strncpy",
        number: FIRST + 2,
        parameters: &[Word, Word, Word],
        result: Word,
        entry: super::strncpy,
    },
    Function {
        name: b"strlen",
        number: FIRST + 3,
        parameters: &[Word],
        result: Word,
        entry: length,
    },
    Function {
        name: b"strcmp",
        number: FIRST + 4,
        parameters: &[Word, Word],
        result: I32,
        entry: compare_string,
    },
    Function {
        name: b"memcmp",
        number: FIRST + 5,
        parameters: &[Word, Word, Word],
        result: I32,
        entry: compare_memory,
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
