//! Separate native stat and view-angle services retain their wire ordinals.
use super::{Error, MessageError, Protocol};
use crate::message::{Encoding, ErrorKind, Reader, Writer};
use qa_core::{
    math::{self, AngleByteForm},
    primitives::Vec3,
};

pub const PLAYER_VALUE_SLOTS: usize = 32;

struct Format {
    byte_values: bool,
    angles: AngleByteForm,
}
fn format(protocol: Protocol) -> Result<Format, Error> {
    match protocol {
        Protocol::NetQuake15 => Ok(Format {
            byte_values: false,
            angles: AngleByteForm::Integral,
        }),
        Protocol::QuakeWorld28 => Ok(Format {
            byte_values: true,
            angles: AngleByteForm::Float,
        }),
        _ => Err(Error::Unsupported),
    }
}

/// These protocols publish stats separately from their player snapshot.
pub fn separate_player_values(protocol: Protocol) -> bool {
    format(protocol).is_ok()
}
fn target(output: &mut [u8], length: usize) -> Result<&mut [u8], Error> {
    let capacity = output.len();
    output.get_mut(..length).ok_or(Error::Message(MessageError {
        byte: capacity,
        kind: ErrorKind::Capacity,
    }))
}

/// Native NQ always writes a long; QW selects its byte service for 0..=255.
pub fn player_value(
    protocol: Protocol,
    index: u8,
    value: i32,
    output: &mut [u8],
) -> Result<usize, Error> {
    let format = format(protocol)?;
    if usize::from(index) >= PLAYER_VALUE_SLOTS {
        return Err(Error::Unsupported);
    }
    let byte = format.byte_values && (0..=255).contains(&value);
    let mut writer = Writer::new(target(output, if byte { 3 } else { 6 })?, Encoding::Bytes);
    writer.write_bits(if format.byte_values && !byte { 38 } else { 3 }, 8)?;
    writer.write_bits(u32::from(index), 8)?;
    writer.write_bits(value as u32, if byte { 8 } else { 32 })?;
    Ok(writer.size())
}

pub fn read_player_value(protocol: Protocol, bytes: &[u8]) -> Result<((u8, i32), usize), Error> {
    let format = format(protocol)?;
    let opcode = bytes.first().copied().ok_or(Error::Truncated)?;
    let byte = match opcode {
        3 => format.byte_values,
        38 if format.byte_values => false,
        _ => return Err(Error::Opcode),
    };
    let length = if byte { 3 } else { 6 };
    let mut reader = Reader::new(
        bytes.get(..length).ok_or(Error::Truncated)?,
        Encoding::Bytes,
    );
    reader.read_bits(8)?;
    let index = reader.read_bits(8)? as u8;
    if usize::from(index) >= PLAYER_VALUE_SLOTS {
        return Err(Error::Unsupported);
    }
    let value = if byte {
        reader.read_bits(8)? as i32
    } else {
        reader.read_signed(32)?
    };
    Ok(((index, value), reader.byte_position()))
}

pub fn set_angles(protocol: Protocol, angles: Vec3, output: &mut [u8]) -> Result<usize, Error> {
    let format = format(protocol)?;
    let mut writer = Writer::new(target(output, 4)?, Encoding::Bytes);
    writer.write_bits(10, 8)?;
    for angle in angles.0 {
        writer.write_bits(math::angle_to_byte(angle, format.angles) as u32, 8)?;
    }
    Ok(writer.size())
}

pub fn read_set_angles(protocol: Protocol, bytes: &[u8]) -> Result<(Vec3, usize), Error> {
    format(protocol)?;
    if bytes.first().copied().ok_or(Error::Truncated)? != 10 {
        return Err(Error::Opcode);
    }
    let mut reader = Reader::new(bytes.get(..4).ok_or(Error::Truncated)?, Encoding::Bytes);
    reader.read_bits(8)?;
    let mut angles = Vec3::default();
    for angle in &mut angles.0 {
        *angle = math::byte_to_angle(reader.read_signed(8)?);
    }
    Ok((angles, reader.byte_position()))
}
