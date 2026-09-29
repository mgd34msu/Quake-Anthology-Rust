//! Runtime memory helpers: guest argument decoding and checked access.
//!
//! Donor: `src/guest/runtime/common/memory.ts`. Argument helpers decode
//! [`GuestCallValue`] lanes; the read/write helpers operate on checked guest
//! bytes without touching host memory layout.

use crate::core::contracts::{GuestAddress, GuestAllocationOptions, GuestCallValue, GuestPermissions};
use crate::core::memory::SparseGuestMemory;
use crate::error::GuestError;

/// Maximum guest byte count accepted by runtime routines.
pub const RUNTIME_ALLOCATION_LIMIT: usize = 0x1000_0000;
/// Maximum guest string length scanned by runtime routines.
pub const RUNTIME_STRING_LIMIT: usize = 1024 * 1024;

/// Borrow call argument `index`.
pub fn argument(args: &[GuestCallValue], index: usize) -> Result<&GuestCallValue, GuestError> {
    args.get(index).ok_or_else(|| {
        GuestError::invalid(format!("Missing guest argument {index}"))
    })
}

/// Decode an integer call argument as a signed 128-bit value (covers the
/// full `u64` lane).
pub fn integer(args: &[GuestCallValue], index: usize) -> Result<i128, GuestError> {
    match argument(args, index)? {
        GuestCallValue::Int32(value) => Ok(i128::from(*value)),
        GuestCallValue::Uint32(value) => Ok(i128::from(*value)),
        GuestCallValue::Int64(value) => Ok(i128::from(*value)),
        GuestCallValue::Uint64(value) => Ok(i128::from(*value)),
        _ => Err(GuestError::invalid("Guest integer required")),
    }
}

/// Decode a byte-count call argument, bounded by the runtime limit.
pub fn count(args: &[GuestCallValue], index: usize) -> Result<usize, GuestError> {
    let value = integer(args, index)?;
    if value < 0 || value > RUNTIME_ALLOCATION_LIMIT as i128 {
        return Err(GuestError::invalid(
            "Guest byte count exceeds runtime allocation limit",
        ));
    }
    Ok(value as usize)
}

/// Decode a pointer call argument (nullable).
pub fn pointer(
    args: &[GuestCallValue],
    index: usize,
) -> Result<Option<GuestAddress>, GuestError> {
    match argument(args, index)? {
        GuestCallValue::Pointer(value) => Ok(*value),
        _ => Err(GuestError::invalid("Guest pointer required")),
    }
}

/// Decode a non-null pointer call argument.
pub fn required_pointer(
    args: &[GuestCallValue],
    index: usize,
) -> Result<GuestAddress, GuestError> {
    pointer(args, index)?.ok_or_else(|| GuestError::invalid("Nonnull guest pointer required"))
}

/// Read an unsigned `width`-byte little-endian value (`width` in 1, 2, 4, 8).
pub fn read_unsigned(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    width: usize,
) -> Result<u64, GuestError> {
    match width {
        1 => Ok(u64::from(memory.read_u8(address)?)),
        2 => Ok(u64::from(memory.read_u16(address)?)),
        4 => Ok(u64::from(memory.read_u32(address)?)),
        8 => memory.read_u64(address),
        _ => Err(GuestError::invalid(format!(
            "Unsupported guest integer width {width}"
        ))),
    }
}

/// Write `value` wrapped to `width` bytes little-endian.
pub fn write_unsigned(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    width: usize,
    value: i128,
) -> Result<(), GuestError> {
    let mask = if width >= 16 {
        u128::MAX
    } else {
        (1u128 << (width * 8)) - 1
    };
    let wrapped = (value as u128) & mask;
    match width {
        1 => memory.write_u8(address, wrapped as u8),
        2 => memory.write_u16(address, wrapped as u16),
        4 => memory.write_u32(address, wrapped as u32),
        8 => memory.write_u64(address, wrapped as u64),
        _ => Err(GuestError::invalid(format!(
            "Unsupported guest integer width {width}"
        ))),
    }
}

/// Read a nullable guest pointer.
pub fn read_pointer(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
) -> Result<Option<GuestAddress>, GuestError> {
    memory.read_pointer(address)
}

/// Write a nullable guest pointer, validating a non-null target first.
pub fn write_pointer(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    value: Option<GuestAddress>,
) -> Result<(), GuestError> {
    if let Some(target) = value {
        memory.offset(target, 0)?;
    }
    memory.write_pointer(address, value)
}

/// Copy `byte_length` bytes, source-first so overlapping ranges move.
pub fn move_bytes(
    memory: &mut SparseGuestMemory,
    destination: GuestAddress,
    source: GuestAddress,
    byte_length: usize,
) -> Result<(), GuestError> {
    memory.move_bytes(destination, source, byte_length)
}

/// Fill `byte_length` bytes with `value`.
pub fn fill_bytes(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    byte_length: usize,
    value: u8,
) -> Result<(), GuestError> {
    memory.fill(address, byte_length, value)
}

/// Length of the NUL-terminated string at `address`.
pub fn string_length(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
) -> Result<usize, GuestError> {
    string_length_bounded(memory, address, RUNTIME_STRING_LIMIT)
}

/// Length of the NUL-terminated string at `address`, bounded by `maximum`.
pub fn string_length_bounded(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    maximum: usize,
) -> Result<usize, GuestError> {
    let length = memory.find_zero(address, maximum)?;
    if length < 0 {
        return Err(GuestError::invalid("Guest string exceeds checked maximum"));
    }
    Ok(length as usize)
}

/// Read a NUL-terminated guest string: latin-1 bytes, or UCS-2 units when
/// `wide`.
pub fn read_string(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    wide: bool,
) -> Result<String, GuestError> {
    read_string_bounded(memory, address, wide, RUNTIME_STRING_LIMIT)
}

/// Read a NUL-terminated guest string bounded by `maximum` units.
pub fn read_string_bounded(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    wide: bool,
    maximum: usize,
) -> Result<String, GuestError> {
    if !wide {
        let length = string_length_bounded(memory, address, maximum)?;
        let bytes = memory.copy(address, length)?;
        return Ok(bytes.iter().map(|byte| *byte as char).collect());
    }
    let mut value = String::new();
    for index in 0..maximum {
        let unit = memory.read_u16(memory.offset(address, (index * 2) as i64)?)?;
        if unit == 0 {
            return Ok(value);
        }
        value.push(char::from_u32(u32::from(unit)).unwrap_or(char::REPLACEMENT_CHARACTER));
    }
    Err(GuestError::invalid("Guest string exceeds checked maximum"))
}

/// Encode `value` as NUL-terminated guest bytes: latin-1, or UCS-2 when
/// `wide`.
pub fn string_bytes(value: &str, wide: bool) -> Vec<u8> {
    if !wide {
        let mut bytes: Vec<u8> = value.encode_utf16().map(|unit| (unit & 0xff) as u8).collect();
        bytes.push(0);
        return bytes;
    }
    let mut bytes = Vec::with_capacity((value.encode_utf16().count() + 1) * 2);
    for unit in value.encode_utf16() {
        bytes.push((unit & 0xff) as u8);
        bytes.push((unit >> 8) as u8);
    }
    bytes.push(0);
    bytes.push(0);
    bytes
}

/// Native allocation granularity: native CRT routines probe page boundaries
/// before vector reads, so every native allocation rounds up to a page.
pub fn native_allocation_bytes(logical_bytes: usize) -> Result<usize, GuestError> {
    if logical_bytes > usize::MAX - 4095 {
        return Err(GuestError::invalid("Invalid native allocation size"));
    }
    Ok((logical_bytes.max(1) + 4095) / 4096 * 4096)
}

/// Allocate page-granular native memory.
pub fn allocate_native_memory(
    memory: &mut SparseGuestMemory,
    logical_bytes: usize,
    label: &str,
) -> Result<GuestAddress, GuestError> {
    memory.allocate(&GuestAllocationOptions {
        byte_length: native_allocation_bytes(logical_bytes)?,
        alignment: 4096,
        permissions: GuestPermissions::ReadWrite,
        label: label.to_string(),
    })
}

/// Allocate `byte_length` bytes with 16-byte alignment.
pub fn allocate_bytes(
    memory: &mut SparseGuestMemory,
    byte_length: usize,
    label: &str,
) -> Result<GuestAddress, GuestError> {
    memory.allocate(&GuestAllocationOptions {
        byte_length: byte_length.max(1),
        alignment: 16,
        permissions: GuestPermissions::ReadWrite,
        label: label.to_string(),
    })
}
