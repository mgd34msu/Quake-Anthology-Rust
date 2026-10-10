//! Native command field tables and boundary projections, sharing one walker.
use super::{Q2Cmd, Q2RrCmd, Q3Cmd, QwCmd};
use crate::{
    delta::{self, Field, Group, Presence, ScaleRead, Value},
    message::{Error, ErrorKind, Reader, Writer},
};
use qa_core::primitives::Vec3;

const QW: [Group; 1] = [Group {
    presence: Presence::Mask(8),
    fields: &[
        Field::new(0, 16, 1, Value::Angle16),
        Field::new(1, 16, 128, Value::Angle16),
        Field::new(2, 16, 2, Value::Angle16),
        Field::new(3, 16, 4, Value::Signed),
        Field::new(4, 16, 8, Value::Signed),
        Field::new(5, 16, 16, Value::Signed),
        Field::new(6, 8, 32, Value::Unsigned),
        Field::new(7, 8, 64, Value::Unsigned),
        Field::new(8, 8, 0, Value::Unsigned),
    ],
}];
const Q2: [Group; 1] = [Group {
    presence: Presence::Mask(8),
    fields: &[
        Field::new(0, 16, 1, Value::Signed),
        Field::new(1, 16, 2, Value::Signed),
        Field::new(2, 16, 4, Value::Signed),
        Field::new(3, 16, 8, Value::Signed),
        Field::new(4, 16, 16, Value::Signed),
        Field::new(5, 16, 32, Value::Signed),
        Field::new(6, 8, 64, Value::Unsigned),
        Field::new(7, 8, 128, Value::Unsigned),
        Field::new(8, 8, 0, Value::Unsigned),
        Field::new(9, 8, 0, Value::Unsigned),
    ],
}];
// Q2repro 1038 nonbatched commands narrow the float game ABI at the wire.
// Header validation is at the record boundary; the same field walker reads
// the body, including legacy impulse/light bytes absent from the native ABI.
const Q2_REPRO: [Group<true, true>; 1] = [Group {
    presence: Presence::Fixed,
    fields: &[
        Field::new(0, 16, 1, Value::Angle16),
        Field::new(1, 16, 2, Value::Angle16),
        Field::new(2, 16, 4, Value::Angle16),
        Field::new(
            3,
            16,
            8,
            Value::Scaled {
                factor: 1,
                read: ScaleRead::Signed,
            },
        ),
        Field::new(
            4,
            16,
            16,
            Value::Scaled {
                factor: 1,
                read: ScaleRead::Signed,
            },
        ),
        Field::new(6, 8, 64, Value::Unsigned),
        Field::new(7, 8, 128, Value::Unsigned),
        Field::new(8, 8, 0, Value::Unsigned),
        Field::new(9, 8, 0, Value::Unsigned),
    ],
}];
const Q3_TIME: [Group; 1] = [Group {
    presence: Presence::Fixed,
    fields: &[Field::new(10, 32, 0, Value::Time)],
}];
const Q3: [Group; 1] = [Group {
    presence: Presence::Changed {
        aggregate: true,
        keyed: true,
        // qsrc MSG_ReadDeltaKey indexes kbitmask[bits], not [bits-1].
        key_extra: 1,
    },
    fields: &[
        Field::new(0, 16, 0, Value::Unsigned),
        Field::new(1, 16, 0, Value::Unsigned),
        Field::new(2, 16, 0, Value::Unsigned),
        Field::new(3, 8, 0, Value::Signed),
        Field::new(4, 8, 0, Value::Signed),
        Field::new(5, 8, 0, Value::Signed),
        Field::new(6, 16, 0, Value::Unsigned),
        Field::new(7, 8, 0, Value::Unsigned),
    ],
}];
pub fn qw_words(c: QwCmd) -> [u32; 11] {
    let mut v = [0; 11];
    v[..3].copy_from_slice(&c.view_angles.0.map(f32::to_bits));
    v[3..6].copy_from_slice(&c.movement.map(|v| v as u32));
    v[6] = c.buttons.into();
    v[7] = c.impulse.into();
    v[8] = c.msec.into();
    v
}
fn q2_words(c: Q2Cmd) -> [u32; 11] {
    let mut v = [0; 11];
    v[..3].copy_from_slice(&c.angles.map(|v| v as u32));
    v[3..6].copy_from_slice(&c.movement.map(|v| v as u32));
    v[6] = c.buttons.into();
    v[7] = c.impulse.into();
    v[8] = c.msec.into();
    v[9] = c.light_level.into();
    v
}
fn q2_rr_words(c: Q2RrCmd) -> [u32; 11] {
    let mut v = [0; 11];
    v[..3].copy_from_slice(&c.angles.0.map(f32::to_bits));
    v[3..5].copy_from_slice(&c.movement.map(f32::to_bits));
    v[6] = c.buttons.into();
    v[8] = c.msec.into();
    v[10] = c.server_frame;
    v
}
fn q3_words(c: Q3Cmd) -> [u32; 11] {
    let mut v = [0; 11];
    v[..3].copy_from_slice(&c.angles.map(|v| v as u32));
    v[3..6].copy_from_slice(&c.movement.map(|v| v as u32));
    v[6] = c.buttons;
    v[7] = c.weapon.into();
    v[10] = c.server_time as u32;
    v
}
pub fn write_qw(writer: &mut Writer<'_>, from: QwCmd, to: QwCmd) -> Result<(), Error> {
    delta::write(&QW, &qw_words(from), &qw_words(to), 0, writer)
}
pub fn read_qw(reader: &mut Reader<'_>, from: QwCmd) -> Result<QwCmd, Error> {
    let mut v = qw_words(from);
    delta::read(&QW, &mut v, 0, reader)?;
    Ok(qw_from_words(&v))
}
pub fn qw_from_words(v: &[u32; 11]) -> QwCmd {
    QwCmd {
        msec: v[8] as u8,
        view_angles: Vec3(std::array::from_fn(|i| f32::from_bits(v[i]))),
        movement: std::array::from_fn(|i| v[3 + i] as i16),
        buttons: v[6] as u8,
        impulse: v[7] as u8,
    }
}
pub fn write_q2(writer: &mut Writer<'_>, from: Q2Cmd, to: Q2Cmd) -> Result<(), Error> {
    delta::write(&Q2, &q2_words(from), &q2_words(to), 0, writer)
}
pub fn read_q2(reader: &mut Reader<'_>, from: Q2Cmd) -> Result<Q2Cmd, Error> {
    let mut v = q2_words(from);
    delta::read(&Q2, &mut v, 0, reader)?;
    Ok(Q2Cmd {
        msec: v[8] as u8,
        angles: std::array::from_fn(|i| v[i] as i16),
        movement: std::array::from_fn(|i| v[3 + i] as i16),
        buttons: v[6] as u8,
        impulse: v[7] as u8,
        light_level: v[9] as u8,
    })
}
/// Q2repro 1038, not retail KEX 2023. Per-command server_frame has no wire
/// field here; the provider supplies it independently of this delta record.
pub fn write_q2_repro(writer: &mut Writer<'_>, from: Q2RrCmd, to: Q2RrCmd) -> Result<(), Error> {
    let from = q2_rr_words(from);
    let to = q2_rr_words(to);
    let mask = delta::mask::<true, true, false, false>(Q2_REPRO[0].fields, &from, &to, 0, 0, 0);
    writer.write_bits(mask as u32, 8)?;
    delta::write(&Q2_REPRO, &from, &to, mask, writer)
}
pub fn read_q2_repro(reader: &mut Reader<'_>, from: Q2RrCmd) -> Result<Q2RrCmd, Error> {
    let byte = reader.byte_position();
    let mask = reader.read_bits(8)?;
    if mask & 32 != 0 {
        return Err(Error {
            byte,
            kind: ErrorKind::Symbol,
        });
    }
    let mut v = q2_rr_words(from);
    delta::read(&Q2_REPRO, &mut v, u64::from(mask), reader)?;
    Ok(Q2RrCmd {
        angles: Vec3(std::array::from_fn(|i| f32::from_bits(v[i]))),
        movement: std::array::from_fn(|i| f32::from_bits(v[3 + i])),
        buttons: v[6] as u8,
        msec: v[8] as u8,
        server_frame: v[10],
    })
}
#[inline]
pub fn write_q3(writer: &mut Writer<'_>, from: Q3Cmd, to: Q3Cmd, key: u32) -> Result<(), Error> {
    let from = q3_words(from);
    let to = q3_words(to);
    delta::write(&Q3_TIME, &from, &to, 0, writer)?;
    delta::write(&Q3, &from, &to, u64::from(key ^ to[10]), writer)
}
#[inline]
pub fn read_q3(reader: &mut Reader<'_>, from: Q3Cmd, key: u32) -> Result<Q3Cmd, Error> {
    let mut v = q3_words(from);
    delta::read(&Q3_TIME, &mut v, 0, reader)?;
    let key = key ^ v[10];
    delta::read(&Q3, &mut v, u64::from(key), reader)?;
    Ok(Q3Cmd {
        server_time: v[10] as i32,
        angles: std::array::from_fn(|i| v[i] as i32),
        movement: std::array::from_fn(|i| v[3 + i] as i8),
        buttons: v[6],
        weapon: v[7] as u8,
    })
}
