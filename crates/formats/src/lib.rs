pub mod archive;
pub mod bsp;
pub mod image;
pub mod model;
mod read;
pub mod sound;
mod text;

pub use bsp::{Bsp, BspFormat};

#[derive(Debug, PartialEq, Eq)]
pub enum FormatError {
    Truncated,
    Unsupported,
    InvalidRange,
    InvalidRecordSize,
    Io(std::io::ErrorKind),
    Compression,
    Checksum,
    InvalidValue,
    InvalidReference(&'static str, usize),
    Cycle,
}

fn word(bytes: &[u8], offset: usize) -> Result<u32, FormatError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(FormatError::Truncated)?;
    Ok(u32::from_le_bytes(value.try_into().unwrap()))
}

fn span(bytes: &[u8], offset: u32, length: u32) -> Result<&[u8], FormatError> {
    if offset > i32::MAX as u32 || length > i32::MAX as u32 {
        return Err(FormatError::InvalidRange);
    }
    let start = offset as usize;
    let end = start
        .checked_add(length as usize)
        .ok_or(FormatError::InvalidRange)?;
    bytes.get(start..end).ok_or(FormatError::InvalidRange)
}
