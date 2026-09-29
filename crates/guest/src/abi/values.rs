//! ABI scalar and aggregate value encoding.
//!
//! Donor: `src/guest/abi/values.ts`. Scalar arguments smaller than 32 bits
//! promote to at least 4 bytes with sign or zero extension; aggregates
//! marshal as exact little-endian record bytes.

use crate::core::contracts::{GuestCallValue, GuestStorage, GuestValueLayout};
use crate::core::memory::SparseGuestMemory;
use crate::error::GuestError;

/// Storage width in bytes at `pointer_bytes`.
#[must_use]
pub const fn storage_bytes(storage: GuestStorage, pointer_bytes: usize) -> usize {
    storage.byte_length(pointer_bytes)
}

/// Marshalled value width: scalar storage or aggregate record length.
#[must_use]
pub fn value_bytes(layout: &GuestValueLayout, pointer_bytes: usize) -> usize {
    match layout {
        GuestValueLayout::Scalar(storage) => storage_bytes(*storage, pointer_bytes),
        GuestValueLayout::Aggregate(layout) => layout.byte_length,
    }
}

/// Stack argument width: scalars promote to at least 4 bytes.
#[must_use]
pub fn argument_bytes(layout: &GuestValueLayout, pointer_bytes: usize) -> usize {
    let bytes = value_bytes(layout, pointer_bytes);
    match layout {
        GuestValueLayout::Scalar(_) => bytes.max(4),
        GuestValueLayout::Aggregate(_) => bytes,
    }
}

/// Argument alignment: aggregate alignment, else scalar width capped at the
/// pointer width.
#[must_use]
pub fn value_alignment(layout: &GuestValueLayout, pointer_bytes: usize) -> usize {
    match layout {
        GuestValueLayout::Aggregate(layout) => layout.alignment,
        GuestValueLayout::Scalar(storage) => storage_bytes(*storage, pointer_bytes).min(pointer_bytes),
    }
}

/// Validate an aggregate layout against the pointer width.
pub fn validate_value_layout(layout: &GuestValueLayout, pointer_bytes: usize) -> Result<(), GuestError> {
    let GuestValueLayout::Aggregate(record) = layout else {
        return Ok(());
    };
    if record.pointer_bytes != pointer_bytes
        || record.byte_length == 0
        || record.alignment == 0
        || record.alignment & (record.alignment - 1) != 0
        || record.alignment > 4096
    {
        return Err(GuestError::abi("Invalid aggregate ABI layout"));
    }
    for field in &record.fields {
        let end = field
            .byte_offset
            .saturating_add(field.count.saturating_mul(storage_bytes(field.storage, pointer_bytes)));
        if end > record.byte_length {
            return Err(GuestError::abi(format!(
                "Aggregate field {} exceeds its record",
                field.name
            )));
        }
    }
    Ok(())
}

/// Infer a value's layout, promoting `float32` to `float64` for variadics.
#[must_use]
pub fn inferred_layout(value: &GuestCallValue, promote: bool) -> GuestValueLayout {
    match value {
        GuestCallValue::Aggregate { layout, .. } => GuestValueLayout::Aggregate(layout.clone()),
        GuestCallValue::Int32(_) => GuestValueLayout::Scalar(GuestStorage::Int32),
        GuestCallValue::Uint32(_) => GuestValueLayout::Scalar(GuestStorage::Uint32),
        GuestCallValue::Int64(_) => GuestValueLayout::Scalar(GuestStorage::Int64),
        GuestCallValue::Uint64(_) => GuestValueLayout::Scalar(GuestStorage::Uint64),
        GuestCallValue::Float32(_) if promote => GuestValueLayout::Scalar(GuestStorage::Float64),
        GuestCallValue::Float32(_) => GuestValueLayout::Scalar(GuestStorage::Float32),
        GuestCallValue::Float64(_) => GuestValueLayout::Scalar(GuestStorage::Float64),
        GuestCallValue::Pointer(_) => GuestValueLayout::Scalar(GuestStorage::Pointer),
    }
}

/// Encode an integer or pointer value to its raw little-endian bits.
pub fn encode_integer_value(
    storage: GuestStorage,
    value: &GuestCallValue,
    memory: &SparseGuestMemory,
) -> Result<u64, GuestError> {
    if storage == GuestStorage::Pointer {
        let GuestCallValue::Pointer(address) = value else {
            return Err(GuestError::abi("Pointer ABI argument requires a guest pointer"));
        };
        if let Some(address) = address {
            if address.space != memory.address_space() {
                return Err(GuestError::abi("Pointer ABI argument belongs to another address space"));
            }
            return Ok(address.offset);
        }
        return Ok(0);
    }
    let raw: i64 = match value {
        GuestCallValue::Int32(value) => i64::from(*value),
        GuestCallValue::Uint32(value) => i64::from(*value),
        GuestCallValue::Int64(value) => *value,
        GuestCallValue::Uint64(value) => *value as i64,
        _ => return Err(GuestError::abi("Integer ABI argument requires an integer value")),
    };
    let bits = storage_bytes(storage, memory.pointer_bytes()) * 8;
    Ok((raw as u64) & (if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 }))
}

/// Encode one value to exact layout bytes.
pub fn encode_value(
    layout: &GuestValueLayout,
    value: &GuestCallValue,
    memory: &SparseGuestMemory,
) -> Result<Vec<u8>, GuestError> {
    validate_value_layout(layout, memory.pointer_bytes())?;
    if let GuestValueLayout::Aggregate(record) = layout {
        let GuestCallValue::Aggregate {
            layout: value_layout,
            bytes,
        } = value
        else {
            return Err(GuestError::abi("Aggregate call value differs from its signature"));
        };
        if value_layout.id != record.id
            || bytes.len() != record.byte_length
            || value_layout.byte_length != record.byte_length
            || value_layout.pointer_bytes != record.pointer_bytes
        {
            return Err(GuestError::abi("Aggregate call value differs from its signature"));
        }
        return Ok(bytes.clone());
    }
    let GuestValueLayout::Scalar(storage) = layout else {
        unreachable!("aggregate handled above");
    };
    let mut bytes = vec![0; storage_bytes(*storage, memory.pointer_bytes())];
    match storage {
        GuestStorage::Pointer => {
            let raw = encode_integer_value(*storage, value, memory)?;
            if memory.pointer_bytes() == 4 {
                bytes.copy_from_slice(&(raw as u32).to_le_bytes());
            } else {
                bytes.copy_from_slice(&raw.to_le_bytes());
            }
        }
        GuestStorage::Float32 => {
            let scalar = match value {
                GuestCallValue::Float32(value) => *value,
                GuestCallValue::Float64(value) => *value as f32,
                _ => return Err(GuestError::abi("Floating ABI argument requires a floating value")),
            };
            bytes.copy_from_slice(&scalar.to_le_bytes());
        }
        GuestStorage::Float64 => {
            let scalar = match value {
                GuestCallValue::Float32(value) => f64::from(*value),
                GuestCallValue::Float64(value) => *value,
                _ => return Err(GuestError::abi("Floating ABI argument requires a floating value")),
            };
            bytes.copy_from_slice(&scalar.to_le_bytes());
        }
        _ => {
            let raw = encode_integer_value(*storage, value, memory)?;
            match storage_bytes(*storage, memory.pointer_bytes()) {
                1 => bytes[0] = raw as u8,
                2 => bytes.copy_from_slice(&(raw as u16).to_le_bytes()),
                4 => bytes.copy_from_slice(&(raw as u32).to_le_bytes()),
                _ => bytes.copy_from_slice(&raw.to_le_bytes()),
            }
        }
    }
    Ok(bytes)
}

/// Decode exact layout bytes to one value.
pub fn decode_value(
    layout: &GuestValueLayout,
    bytes: &[u8],
    memory: &SparseGuestMemory,
) -> Result<GuestCallValue, GuestError> {
    if bytes.len() != value_bytes(layout, memory.pointer_bytes()) {
        return Err(GuestError::abi("ABI value byte length differs from its layout"));
    }
    if let GuestValueLayout::Aggregate(record) = layout {
        return Ok(GuestCallValue::Aggregate {
            layout: record.clone(),
            bytes: bytes.to_vec(),
        });
    }
    let GuestValueLayout::Scalar(storage) = layout else {
        unreachable!("aggregate handled above");
    };
    let word = |index: usize| -> Result<[u8; 8], GuestError> {
        let mut word = [0u8; 8];
        let width = storage_bytes(*storage, memory.pointer_bytes());
        word[..width].copy_from_slice(&bytes[index..index + width]);
        Ok(word)
    };
    match storage {
        GuestStorage::Int8 => Ok(GuestCallValue::Int32(i32::from(bytes[0] as i8))),
        GuestStorage::Uint8 => Ok(GuestCallValue::Uint32(u32::from(bytes[0]))),
        GuestStorage::Int16 => Ok(GuestCallValue::Int32(i32::from(i16::from_le_bytes([
            bytes[0], bytes[1],
        ])))),
        GuestStorage::Uint16 => Ok(GuestCallValue::Uint32(u32::from(u16::from_le_bytes([
            bytes[0], bytes[1],
        ])))),
        GuestStorage::Int32 => Ok(GuestCallValue::Int32(i32::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
        ]))),
        GuestStorage::Uint32 => Ok(GuestCallValue::Uint32(u32::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
        ]))),
        GuestStorage::Int64 => {
            let word = word(0)?;
            Ok(GuestCallValue::Int64(i64::from_le_bytes(word)))
        }
        GuestStorage::Uint64 => {
            let word = word(0)?;
            Ok(GuestCallValue::Uint64(u64::from_le_bytes(word)))
        }
        GuestStorage::Float32 => Ok(GuestCallValue::Float32(f32::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
        ]))),
        GuestStorage::Float64 => {
            let word = word(0)?;
            Ok(GuestCallValue::Float64(f64::from_le_bytes(word)))
        }
        GuestStorage::Pointer => {
            let raw = if memory.pointer_bytes() == 4 {
                u64::from(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            } else {
                let word = word(0)?;
                u64::from_le_bytes(word)
            };
            Ok(GuestCallValue::Pointer(memory.pointer(raw)?))
        }
    }
}

/// Encode one stack argument with 32-bit promotion and sign extension.
pub fn encode_argument_value(
    layout: &GuestValueLayout,
    value: &GuestCallValue,
    memory: &SparseGuestMemory,
) -> Result<Vec<u8>, GuestError> {
    let bytes = encode_value(layout, value, memory)?;
    let size = argument_bytes(layout, memory.pointer_bytes());
    if bytes.len() == size {
        return Ok(bytes);
    }
    let signed = matches!(
        layout,
        GuestValueLayout::Scalar(GuestStorage::Int8 | GuestStorage::Int16 | GuestStorage::Int32 | GuestStorage::Int64)
    );
    let fill = if signed && bytes.last().is_some_and(|last| last & 0x80 != 0) {
        0xff
    } else {
        0x00
    };
    let mut extended = vec![fill; size];
    extended[..bytes.len()].copy_from_slice(&bytes);
    Ok(extended)
}

/// Round `value` up to `alignment`.
#[must_use]
pub const fn align_up(value: usize, alignment: usize) -> usize {
    value.next_multiple_of(alignment)
}

/// Round `value` down to `alignment`.
#[must_use]
pub const fn align_down(value: u64, alignment: u64) -> u64 {
    value - value % alignment
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;

    use crate::core::contracts::{ContentDigest, ModuleIdentity};

    fn test_module() -> ModuleIdentity {
        ModuleIdentity::new(
            ProviderId::new("test", "abi-values"),
            "test.so",
            ContentDigest::new("sha256", "abc"),
            "r1",
        )
    }

    #[test]
    fn scalar_promotion_sign_extends() {
        let memory = SparseGuestMemory::new(test_module(), 8, 0x10000).unwrap();
        let layout = GuestValueLayout::Scalar(GuestStorage::Int8);
        let bytes = encode_argument_value(&layout, &GuestCallValue::Int32(-1), &memory).unwrap();
        assert_eq!(bytes, vec![0xff, 0xff, 0xff, 0xff]);
        let layout = GuestValueLayout::Scalar(GuestStorage::Uint8);
        let bytes = encode_argument_value(&layout, &GuestCallValue::Uint32(0x80), &memory).unwrap();
        assert_eq!(bytes, vec![0x80, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn pointer_round_trip_decodes_null() {
        let memory = SparseGuestMemory::new(test_module(), 4, 0x10000).unwrap();
        let layout = GuestValueLayout::Scalar(GuestStorage::Pointer);
        let bytes = encode_value(&layout, &GuestCallValue::Pointer(None), &memory).unwrap();
        assert_eq!(bytes, vec![0, 0, 0, 0]);
        assert_eq!(
            decode_value(&layout, &bytes, &memory).unwrap(),
            GuestCallValue::Pointer(None)
        );
    }
}
