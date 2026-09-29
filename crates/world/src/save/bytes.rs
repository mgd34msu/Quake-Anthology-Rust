//! Source byte strings ported from `src/persistence/source-bytes.ts`.
//!
//! Quake save strings preserve all eight bits: each byte maps to the same
//! code point. Reads use the shared [`qa_core::binary`] cursor; writes
//! precompute exact capacities like the donor.

use qa_core::binary::{BinaryReader, BinaryWriter};

use crate::WorldError;

use super::value::save_error;

fn binary_error(error: qa_core::binary::BinaryError) -> WorldError {
    WorldError::BadSave(error.to_string())
}

/// Map bytes to code points one-to-one.
#[must_use]
pub fn byte_text(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| char::from(*byte)).collect()
}

/// Map code points back to bytes (fails above `0xFF`).
pub fn text_bytes(text: &str) -> Result<Vec<u8>, WorldError> {
    text.chars()
        .map(|character| {
            u8::try_from(character as u32).map_err(|_| save_error("text", "source save requires byte characters"))
        })
        .collect()
}

/// Read a NUL-terminated string.
pub fn read_c_string(reader: &mut BinaryReader) -> Result<String, WorldError> {
    let mut bytes = Vec::new();
    loop {
        let byte = reader.u8().map_err(binary_error)?;
        if byte == 0 {
            break;
        }
        bytes.push(byte);
    }
    Ok(byte_text(&bytes))
}

/// Write a NUL-terminated string (rejects interior NUL).
pub fn write_c_string(writer: &mut BinaryWriter, value: &str) -> Result<(), WorldError> {
    if value.contains('\0') {
        return Err(save_error("text", "source string contains an interior NUL"));
    }
    let bytes = text_bytes(value)?;
    writer.bytes(&bytes).map_err(binary_error)?;
    writer.u8(0).map_err(binary_error)?;
    Ok(())
}

/// Read a fixed-width NUL-padded string.
pub fn read_fixed(reader: &mut BinaryReader, width: usize) -> Result<String, WorldError> {
    reader.fixed_byte_string(width).map_err(binary_error)
}

/// Write a fixed-width NUL-padded string (must fit `width - 1` bytes).
pub fn write_fixed(writer: &mut BinaryWriter, value: &str, width: usize) -> Result<(), WorldError> {
    if value.contains('\0') || value.chars().count() >= width {
        return Err(save_error(
            "text",
            &format!("source string must fit {} bytes", width - 1),
        ));
    }
    let mut bytes = vec![0u8; width];
    bytes[..text_bytes(value)?.len()].copy_from_slice(&text_bytes(value)?);
    writer.bytes(&bytes).map_err(binary_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_strings_round_trip() {
        let mut writer = BinaryWriter::new(8);
        write_c_string(&mut writer, "abÿ").unwrap();
        write_c_string(&mut writer, "").unwrap();
        let bytes = writer.finish();
        let mut reader = BinaryReader::new(&bytes, "test");
        assert_eq!(read_c_string(&mut reader).unwrap(), "abÿ");
        assert_eq!(read_c_string(&mut reader).unwrap(), "");
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn fixed_strings_pad_and_reject_overflow() {
        let mut writer = BinaryWriter::new(4);
        write_fixed(&mut writer, "ab", 4).unwrap();
        assert_eq!(writer.finish(), vec![b'a', b'b', 0, 0]);
        let mut reader = BinaryReader::new(&[b'a', b'b', 0, 0], "test");
        assert_eq!(read_fixed(&mut reader, 4).unwrap(), "ab");
        let mut writer = BinaryWriter::new(8);
        assert!(write_fixed(&mut writer, "abcd", 4).is_err());
        assert!(write_fixed(&mut writer, "a\0b", 4).is_err());
        assert!(text_bytes("ÿþĀ").is_err());
    }
}
