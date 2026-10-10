//! Native command field tables and boundary projections, sharing one walker.
use super::{Q2Cmd, Q3Cmd, QwCmd};
use crate::{
    delta::{self, Field, Group, Presence, Value},
    message::{Error, Reader, Writer},
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
#[inline]
pub fn write_q3(writer: &mut Writer<'_>, from: Q3Cmd, to: Q3Cmd, key: u32) -> Result<(), Error> {
    let from = q3_words(from);
    let to = q3_words(to);
    delta::write(&Q3_TIME, &from, &to, 0, writer)?;
    delta::write(&Q3, &from, &to, key ^ to[10], writer)
}
#[inline]
pub fn read_q3(reader: &mut Reader<'_>, from: Q3Cmd, key: u32) -> Result<Q3Cmd, Error> {
    let mut v = q3_words(from);
    delta::read(&Q3_TIME, &mut v, 0, reader)?;
    let key = key ^ v[10];
    delta::read(&Q3, &mut v, key, reader)?;
    Ok(Q3Cmd {
        server_time: v[10] as i32,
        angles: std::array::from_fn(|i| v[i] as i32),
        movement: std::array::from_fn(|i| v[3 + i] as i8),
        buttons: v[6],
        weapon: v[7] as u8,
    })
}
