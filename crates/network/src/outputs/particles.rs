//! NetQuake's native particle record; other protocols have no matching service.
use super::{Error, MessageError, Protocol, cues};
use crate::message::{Encoding, ErrorKind, Reader, Writer};
use qa_core::primitives::Vec3;

const PARTICLE_BYTES: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub position: Vec3,
    pub direction: Vec3,
    pub color: u8,
    pub count: u8,
}

pub fn particle(protocol: Protocol, particle: Particle, output: &mut [u8]) -> Result<usize, Error> {
    if protocol != Protocol::NetQuake15 {
        return Err(Error::Unsupported);
    }
    let capacity = output.len();
    let target = output
        .get_mut(..PARTICLE_BYTES)
        .ok_or(Error::Message(MessageError {
            byte: capacity,
            kind: ErrorKind::Capacity,
        }))?;
    let mut writer = Writer::new(target, Encoding::Bytes);
    writer.write_bits(18, 8)?;
    cues::write_position(&mut writer, particle.position, false)?;
    for value in particle.direction.0 {
        // SV_StartParticle truncates before clamping its signed byte.
        writer.write_bits(((value * 16.0) as i32).clamp(-128, 127) as u32, 8)?;
    }
    writer.write_bits(u32::from(particle.count), 8)?;
    writer.write_bits(u32::from(particle.color), 8)?;
    Ok(writer.size())
}

pub fn read_particle(protocol: Protocol, bytes: &[u8]) -> Result<(Particle, usize), Error> {
    if protocol != Protocol::NetQuake15 {
        return Err(Error::Unsupported);
    }
    if bytes.first().copied().ok_or(Error::Truncated)? != 18 {
        return Err(Error::Opcode);
    }
    let mut reader = Reader::new(
        bytes.get(..PARTICLE_BYTES).ok_or(Error::Truncated)?,
        Encoding::Bytes,
    );
    reader.read_bits(8)?;
    let position = cues::read_position(&mut reader, false)?;
    let mut direction = Vec3::default();
    for value in &mut direction.0 {
        *value = reader.read_signed(8)? as f32 * (1.0 / 16.0);
    }
    let count = reader.read_bits(8)? as u8;
    let color = reader.read_bits(8)? as u8;
    Ok((
        Particle {
            position,
            direction,
            color,
            count,
        },
        reader.byte_position(),
    ))
}
