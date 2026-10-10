use super::read_areas;
use crate::{
    commands::packet,
    message::{Reader, Writer},
};

/// Native svc_frame prefix. Protocol 34 and KEX retain full frame numbers;
/// protocol 1038 packs a 27-bit frame and a five-bit delta offset. These are
/// wire columns, independent of the engine's timeline and player storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Q2Header {
    pub sequence: u32,
    pub delta: i32,
    pub flags: u8,
    pub player_flags: u8,
}

impl Q2Header {
    /// Body after svc_frame. Areas are borrowed connection-owned scratch;
    /// the supplied limit is the negotiated protocol's native area capacity.
    pub fn read<const PACKED: bool>(
        reader: &mut Reader<'_>,
        areas: &mut [u8],
        area_limit: usize,
    ) -> Result<(Self, usize), packet::Error> {
        let encoded = reader.read_bits(32)?;
        let (sequence, delta) = if PACKED {
            let offset = encoded >> 27;
            let sequence = encoded & 0x07ff_ffff;
            (
                sequence,
                if offset == 31 {
                    -1
                } else {
                    sequence.wrapping_sub(offset) as i32
                },
            )
        } else {
            (encoded, reader.read_bits(32)? as i32)
        };
        let mut flags = reader.read_bits(8)? as u8;
        let player_flags = if PACKED {
            flags &= 0x0f;
            reader.read_bits(8)? as u8
        } else {
            0
        };
        let area_bytes = read_areas(reader, areas, area_limit)?;
        Ok((
            Self {
                sequence,
                delta,
                flags,
                player_flags,
            },
            area_bytes,
        ))
    }

    /// Writes svc_frame and its area mask, leaving the native player/entity
    /// records to the existing scalar field walker.
    pub fn write<const PACKED: bool>(
        self,
        writer: &mut Writer<'_>,
        areas: &[u8],
    ) -> Result<(), packet::Error> {
        if areas.len() > 255 {
            return Err(packet::Error::Count);
        }
        writer.write_bits(20, 8)?;
        if PACKED {
            let offset = if self.delta == -1 {
                31
            } else {
                self.sequence.wrapping_sub(self.delta as u32)
            };
            writer.write_bits((self.sequence & 0x07ff_ffff) | (offset << 27), 32)?;
        } else {
            writer.write_bits(self.sequence, 32)?;
            writer.write_bits(self.delta as u32, 32)?;
        }
        writer.write_bits(u32::from(self.flags), 8)?;
        if PACKED {
            writer.write_bits(u32::from(self.player_flags), 8)?;
        }
        writer.write_bits(areas.len() as u32, 8)?;
        writer.write_data(areas)?;
        Ok(())
    }
}
