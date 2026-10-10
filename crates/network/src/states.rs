//! Native state projections in protocol table order, not engine entity storage.
use crate::{
    delta::{self, Field, Group, Presence, ScaleRead, Value},
    message::{Error, Reader, Writer},
};

pub const ENTITY_LAYOUT: [(&str, i8); 51] = [
    ("pos.trTime", 32),
    ("pos.trBase[0]", 0),
    ("pos.trBase[1]", 0),
    ("pos.trDelta[0]", 0),
    ("pos.trDelta[1]", 0),
    ("pos.trBase[2]", 0),
    ("apos.trBase[1]", 0),
    ("pos.trDelta[2]", 0),
    ("apos.trBase[0]", 0),
    ("event", 10),
    ("angles2[1]", 0),
    ("eType", 8),
    ("torsoAnim", 8),
    ("eventParm", 8),
    ("legsAnim", 8),
    ("groundEntityNum", 10),
    ("pos.trType", 8),
    ("eFlags", 19),
    ("otherEntityNum", 10),
    ("weapon", 8),
    ("clientNum", 8),
    ("angles[1]", 0),
    ("pos.trDuration", 32),
    ("apos.trType", 8),
    ("origin[0]", 0),
    ("origin[1]", 0),
    ("origin[2]", 0),
    ("solid", 24),
    ("powerups", 16),
    ("modelindex", 8),
    ("otherEntityNum2", 10),
    ("loopSound", 8),
    ("generic1", 8),
    ("origin2[2]", 0),
    ("origin2[0]", 0),
    ("origin2[1]", 0),
    ("modelindex2", 8),
    ("angles[0]", 0),
    ("time", 32),
    ("apos.trTime", 32),
    ("apos.trDuration", 32),
    ("apos.trBase[2]", 0),
    ("apos.trDelta[0]", 0),
    ("apos.trDelta[1]", 0),
    ("apos.trDelta[2]", 0),
    ("time2", 32),
    ("angles[2]", 0),
    ("angles2[0]", 0),
    ("angles2[2]", 0),
    ("constantLight", 32),
    ("frame", 16),
];

pub const PLAYER_LAYOUT: [(&str, i8); 48] = [
    ("commandTime", 32),
    ("origin[0]", 0),
    ("origin[1]", 0),
    ("bobCycle", 8),
    ("velocity[0]", 0),
    ("velocity[1]", 0),
    ("viewangles[1]", 0),
    ("viewangles[0]", 0),
    ("weaponTime", -16),
    ("origin[2]", 0),
    ("velocity[2]", 0),
    ("legsTimer", 8),
    ("pm_time", -16),
    ("eventSequence", 16),
    ("torsoAnim", 8),
    ("movementDir", 4),
    ("events[0]", 8),
    ("legsAnim", 8),
    ("events[1]", 8),
    ("pm_flags", 16),
    ("groundEntityNum", 10),
    ("weaponstate", 4),
    ("eFlags", 16),
    ("externalEvent", 10),
    ("gravity", 16),
    ("speed", 16),
    ("delta_angles[1]", 16),
    ("externalEventParm", 8),
    ("viewheight", -8),
    ("damageEvent", 8),
    ("damageYaw", 8),
    ("damagePitch", 8),
    ("damageCount", 8),
    ("generic1", 8),
    ("pm_type", 8),
    ("delta_angles[0]", 16),
    ("delta_angles[2]", 16),
    ("torsoTimer", 12),
    ("eventParms[0]", 8),
    ("eventParms[1]", 8),
    ("clientNum", 8),
    ("weapon", 5),
    ("viewangles[2]", 0),
    ("grapplePoint[0]", 0),
    ("grapplePoint[1]", 0),
    ("grapplePoint[2]", 0),
    ("jumppad_ent", 10),
    ("loopSound", 16),
];

pub const ENTITY_WORDS: usize = ENTITY_LAYOUT.len();
pub const PLAYER_WORDS: usize = PLAYER_LAYOUT.len() + 64;

const fn fields<const N: usize>(layout: &[(&str, i8); N], entity: bool) -> [Field; N] {
    let mut result = [Field::new(0, 0, 0, Value::Unsigned); N];
    let mut i = 0;
    while i < N {
        let width = layout[i].1;
        result[i] = Field::new(
            i,
            width.unsigned_abs(),
            0,
            if width == 0 {
                Value::Float { zero: entity }
            } else if width < 0 {
                Value::Signed
            } else if entity {
                Value::ZeroUnsigned
            } else {
                Value::Unsigned
            },
        );
        i += 1;
    }
    result
}
static ENTITY_FIELDS: [Field; ENTITY_WORDS] = fields(&ENTITY_LAYOUT, true);
static PLAYER_FIELDS: [Field; 48] = fields(&PLAYER_LAYOUT, false);
static ENTITY_GROUP: [Group<true>; 1] = [Group {
    fields: &ENTITY_FIELDS,
    presence: Presence::LastChanged(8),
}];
static PLAYER_GROUP: [Group<true>; 1] = [Group {
    fields: &PLAYER_FIELDS,
    presence: Presence::LastChanged(8),
}];
const fn mask_fields<const N: usize>(start: usize, bits: u8, value: Value) -> [Field; N] {
    let mut fields = [Field::new(0, bits, 0, value); N];
    let mut i = 0;
    while i < fields.len() {
        fields[i].word = start + i;
        fields[i].flag = 1 << i;
        i += 1;
    }
    fields
}
static STATS: [Field; 16] = mask_fields(48, 16, Value::Signed);
static PERSISTANT: [Field; 16] = mask_fields(64, 16, Value::Signed);
static AMMO: [Field; 16] = mask_fields(80, 16, Value::Signed);
static POWERUPS: [Field; 16] = mask_fields(96, 32, Value::Unsigned);
static PLAYER_ARRAYS: [Group<true>; 4] = [
    Group {
        fields: &STATS,
        presence: Presence::OptionalMask(16),
    },
    Group {
        fields: &PERSISTANT,
        presence: Presence::OptionalMask(16),
    },
    Group {
        fields: &AMMO,
        presence: Presence::OptionalMask(16),
    },
    Group {
        fields: &POWERUPS,
        presence: Presence::OptionalMask(16),
    },
];

/// Protocol-68 entity delta. `number` is a native entity number supplied by the
/// connection boundary, never an internal entity handle. Unrepresentable numbers
/// are omitted; the caller selects a fallback for capabilities outside this ABI.
/// Words follow ENTITY_LAYOUT; a missing target removes the native entity.
pub fn write_q3_entity(
    writer: &mut Writer<'_>,
    number: u32,
    from: &[u32; ENTITY_WORDS],
    to: Option<&[u32; ENTITY_WORDS]>,
    force: bool,
) -> Result<bool, Error> {
    if number >= 1024 {
        return Ok(false);
    }
    let count = to.map_or(0, |to| {
        delta::changed::<true, false>(&ENTITY_FIELDS, from, to)
    });
    if to.is_some() && count == 0 && !force {
        return Ok(false);
    }
    writer.write_bits(number, 10)?;
    writer.write_bits(u32::from(to.is_none()), 1)?;
    if let Some(to) = to {
        writer.write_bits(u32::from(count != 0), 1)?;
        if count != 0 {
            delta::write(&ENTITY_GROUP, from, to, 0, writer)?;
        }
    }
    Ok(true)
}

#[derive(Debug, PartialEq, Eq)]
pub struct EntityDelta<const N: usize> {
    pub number: u16,
    pub words: Option<[u32; N]>,
}
pub type Q3EntityDelta = EntityDelta<ENTITY_WORDS>;
pub fn read_q3_entity(
    reader: &mut Reader<'_>,
    from: &[u32; ENTITY_WORDS],
) -> Result<Q3EntityDelta, Error> {
    let number = reader.read_bits(10)? as u16;
    if reader.read_bits(1)? != 0 {
        return Ok(Q3EntityDelta {
            number,
            words: None,
        });
    }
    let mut words = *from;
    if reader.read_bits(1)? != 0 {
        delta::read(&ENTITY_GROUP, &mut words, 0, reader)?;
    }
    Ok(Q3EntityDelta {
        number,
        words: Some(words),
    })
}

/// Scalar fields in PLAYER_LAYOUT followed by the native stats, persistant,
/// ammo and powerups arrays (16 words each). These temporary wire projections do
/// not store another engine PlayerState. The caller supplies its connection's ABI.
pub fn write_q3_player(
    writer: &mut Writer<'_>,
    from: &[u32; PLAYER_WORDS],
    to: &[u32; PLAYER_WORDS],
) -> Result<(), Error> {
    delta::write(&PLAYER_GROUP, from, to, 0, writer)?;
    let arrays = PLAYER_ARRAYS
        .iter()
        .any(|group| delta::changed::<true, false>(group.fields, from, to) != 0);
    writer.write_bits(u32::from(arrays), 1)?;
    if arrays {
        delta::write(&PLAYER_ARRAYS, from, to, 0, writer)?;
    }
    Ok(())
}
#[inline(always)]
pub fn read_q3_player(
    reader: &mut Reader<'_>,
    from: &[u32; PLAYER_WORDS],
) -> Result<[u32; PLAYER_WORDS], Error> {
    let mut words = *from;
    delta::read(&PLAYER_GROUP, &mut words, 0, reader)?;
    if reader.read_bits(1)? != 0 {
        delta::read(&PLAYER_ARRAYS, &mut words, 0, reader)?;
    }
    Ok(words)
}

/// Original Q2 player wire order; signed widths describe decoded scalars.
pub const Q2_PLAYER_LAYOUT: [(&str, i8); 36] = [
    ("pmove.pm_type", 8),
    ("pmove.origin[0]", -16),
    ("pmove.origin[1]", -16),
    ("pmove.origin[2]", -16),
    ("pmove.velocity[0]", -16),
    ("pmove.velocity[1]", -16),
    ("pmove.velocity[2]", -16),
    ("pmove.pm_time", 8),
    ("pmove.pm_flags", 8),
    ("pmove.gravity", -16),
    ("pmove.delta_angles[0]", -16),
    ("pmove.delta_angles[1]", -16),
    ("pmove.delta_angles[2]", -16),
    ("viewoffset[0]", -8),
    ("viewoffset[1]", -8),
    ("viewoffset[2]", -8),
    ("viewangles[0]", 16),
    ("viewangles[1]", 16),
    ("viewangles[2]", 16),
    ("kick_angles[0]", -8),
    ("kick_angles[1]", -8),
    ("kick_angles[2]", -8),
    ("gunindex", 8),
    ("gunframe", 8),
    ("gunoffset[0]", -8),
    ("gunoffset[1]", -8),
    ("gunoffset[2]", -8),
    ("gunangles[0]", -8),
    ("gunangles[1]", -8),
    ("gunangles[2]", -8),
    ("blend[0]", 8),
    ("blend[1]", 8),
    ("blend[2]", 8),
    ("blend[3]", 8),
    ("fov", 8),
    ("rdflags", 8),
];
pub const Q2_PLAYER_WORDS: usize = Q2_PLAYER_LAYOUT.len() + 32;
const fn q2_player_fields() -> [Field; 36] {
    let mut fields = [Field::new(0, 8, 0, Value::Unsigned); 36];
    let mut i = 0;
    while i < fields.len() {
        let (bit, value) = match i {
            0 => (0, Value::Unsigned),
            1..=3 => (1, Value::Signed),
            4..=6 => (2, Value::Signed),
            7 => (3, Value::Unsigned),
            8 => (4, Value::Unsigned),
            9 => (5, Value::Signed),
            10..=12 => (6, Value::Signed),
            13..=15 => (
                7,
                Value::Scaled {
                    factor: 4,
                    read: ScaleRead::Signed,
                },
            ),
            16..=18 => (8, Value::Angle16),
            19..=21 => (
                9,
                Value::Scaled {
                    factor: 4,
                    read: ScaleRead::Signed,
                },
            ),
            22 => (12, Value::Unsigned),
            23 => (13, Value::Unsigned),
            24..=29 => (
                13,
                Value::Scaled {
                    factor: 4,
                    read: ScaleRead::Signed,
                },
            ),
            30..=33 => (
                10,
                Value::Scaled {
                    factor: 255,
                    read: ScaleRead::UnsignedDivide,
                },
            ),
            34 => (
                11,
                Value::Scaled {
                    factor: 1,
                    read: ScaleRead::Unsigned,
                },
            ),
            _ => (14, Value::Unsigned),
        };
        fields[i] = Field::new(i, Q2_PLAYER_LAYOUT[i].1.unsigned_abs(), 1 << bit, value);
        i += 1;
    }
    fields
}
static Q2_PLAYER_FIELDS: [Field; 36] = q2_player_fields();
static Q2_STATS: [Field; 32] = mask_fields(36, 16, Value::Signed);
static Q2_PLAYER_GROUPS: [Group<true>; 2] = [
    Group {
        fields: &Q2_PLAYER_FIELDS,
        // Offsets/angles travel with gunframe, never trigger its flag themselves.
        presence: Presence::MaskPreset {
            bits: 16,
            always: 1 << 12,
            dependent_start: 24,
            dependent_count: 6,
        },
    },
    Group {
        fields: &Q2_STATS,
        presence: Presence::Mask(32),
    },
];

/// Includes native svc_playerinfo. Pmove integer words are already narrowed at
/// the connection ABI boundary; float columns retain their native bit patterns.
pub fn write_q2_player(
    writer: &mut Writer<'_>,
    from: &[u32; Q2_PLAYER_WORDS],
    to: &[u32; Q2_PLAYER_WORDS],
) -> Result<(), Error> {
    writer.write_bits(17, 8)?;
    delta::write(&Q2_PLAYER_GROUPS, from, to, 0, writer)
}
pub fn read_q2_player(
    reader: &mut Reader<'_>,
    from: &[u32; Q2_PLAYER_WORDS],
) -> Result<[u32; Q2_PLAYER_WORDS], Error> {
    if reader.read_bits(8)? != 17 {
        return Err(Error {
            byte: reader.byte_position(),
            kind: crate::message::ErrorKind::Symbol,
        });
    }
    let mut words = *from;
    delta::read(&Q2_PLAYER_GROUPS, &mut words, 0, reader)?;
    Ok(words)
}

pub const QW_ENTITY_LAYOUT: [(&str, i8); 12] = [
    ("modelindex", 8),
    ("frame", 8),
    ("colormap", 8),
    ("skinnum", 8),
    ("effects", 8),
    ("origin[0]", -16),
    ("angles[0]", -8),
    ("origin[1]", -16),
    ("angles[1]", -8),
    ("origin[2]", -16),
    ("angles[2]", -8),
    ("flags", 16),
];
pub const QW_ENTITY_WORDS: usize = QW_ENTITY_LAYOUT.len();
pub type QwEntityDelta = EntityDelta<QW_ENTITY_WORDS>;
static QW_ENTITY_FIELDS: [Field; 11] = [
    Field::new(0, 8, 1 << 2, Value::Unsigned),
    Field::new(1, 8, 1 << 13, Value::Unsigned),
    Field::new(2, 8, 1 << 3, Value::Unsigned),
    Field::new(3, 8, 1 << 4, Value::Unsigned),
    Field::new(4, 8, 1 << 5, Value::Unsigned),
    Field::new(
        5,
        16,
        1 << 9,
        Value::Scaled {
            factor: 8,
            read: ScaleRead::SignedTenthsDelta,
        },
    ),
    Field::new(6, 8, 1 << 0, Value::Angle8),
    Field::new(
        7,
        16,
        1 << 10,
        Value::Scaled {
            factor: 8,
            read: ScaleRead::SignedTenthsDelta,
        },
    ),
    Field::new(8, 8, 1 << 12, Value::Angle8),
    Field::new(
        9,
        16,
        1 << 11,
        Value::Scaled {
            factor: 8,
            read: ScaleRead::SignedTenthsDelta,
        },
    ),
    Field::new(10, 8, 1 << 1, Value::Angle8),
];
static QW_ENTITY_GROUP: [Group<true, true>; 1] = [Group {
    fields: &QW_ENTITY_FIELDS,
    presence: Presence::Fixed,
}];

/// Native protocol-28 entity record. The table's control word is the prefix
/// mask, rather than a keyed scalar value. Native entity numbers are explicit.
pub fn write_qw_entity(
    writer: &mut Writer<'_>,
    number: u32,
    from: &[u32; QW_ENTITY_WORDS],
    to: Option<&[u32; QW_ENTITY_WORDS]>,
    force: bool,
) -> Result<bool, Error> {
    if number == 0 || number >= 512 {
        return Ok(false);
    }
    let Some(to) = to else {
        writer.write_bits(number | (1 << 14), 16)?;
        return Ok(true);
    };
    let mut flags = delta::mask::<true, true>(&QW_ENTITY_FIELDS, from, to, 0, 0, 0);
    if flags & 511 != 0 {
        flags |= 1 << 15;
    }
    // Native SV_WriteDelta adds SOLID after deciding MOREBITS. Preserve that order.
    flags |= to[11] & (1 << 6);
    if flags == 0 && !force {
        return Ok(false);
    }
    writer.write_bits(number | (flags & !511), 16)?;
    if flags & (1 << 15) != 0 {
        writer.write_bits(flags & 255, 8)?;
    }
    delta::write(&QW_ENTITY_GROUP, from, to, flags, writer)?;
    Ok(true)
}
pub fn read_qw_entity(
    reader: &mut Reader<'_>,
    from: &[u32; QW_ENTITY_WORDS],
) -> Result<QwEntityDelta, Error> {
    // MSG_ReadShort sign-extends before CL_ParseDelta stores its native flags.
    let header = reader.read_bits(16)? as i16 as i32 as u32;
    let number = (header & 511) as u16;
    if header & (1 << 14) != 0 {
        return Ok(QwEntityDelta {
            number,
            words: None,
        });
    }
    let mut flags = header & !511;
    if flags & (1 << 15) != 0 {
        flags |= reader.read_bits(8)?;
    }
    let mut words = *from;
    words[11] = flags;
    delta::read(&QW_ENTITY_GROUP, &mut words, flags, reader)?;
    Ok(QwEntityDelta {
        number,
        words: Some(words),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Encoding, ErrorKind};

    #[test]
    fn qw_threshold_uses_native_double_literal_and_solid_flag_order() -> Result<(), Error> {
        let from = [0; QW_ENTITY_WORDS];
        let mut to = from;
        let mut bytes = [0; 1024];
        to[5] = f32::from_bits(0.1f32.to_bits() - 1).to_bits();
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        assert!(!write_qw_entity(&mut writer, 1, &from, Some(&to), false)?);
        to[5] = 0.1f32.to_bits();
        assert!(write_qw_entity(&mut writer, 1, &from, Some(&to), false)?);
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        let decoded = read_qw_entity(&mut reader, &from)?;
        let Some(words) = decoded.words else {
            return Err(Error {
                byte: 0,
                kind: ErrorKind::Symbol,
            });
        };
        assert_eq!(words[5], 0);
        assert_eq!(words[11], 1 << 9);
        to = from;
        to[11] = 1 << 6;
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        assert!(write_qw_entity(&mut writer, 1, &from, Some(&to), false)?);
        assert_eq!(writer.bytes(), &[1, 0]);
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        assert_eq!(read_qw_entity(&mut reader, &from)?.words, Some(from));
        to[0] = 3;
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        write_qw_entity(&mut writer, 511, &from, Some(&to), false)?;
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        let decoded = read_qw_entity(&mut reader, &from)?;
        assert_eq!(decoded.number, 511);
        assert_eq!(
            decoded.words.map(|w| w[11]),
            Some(0xffff_8000 | (1 << 6) | (1 << 2))
        );
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        assert!(!write_qw_entity(&mut writer, 512, &from, Some(&to), true)?);
        write_qw_entity(&mut writer, 1, &from, None, false)?;
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        assert!(read_qw_entity(&mut reader, &from)?.words.is_none());
        Ok(())
    }

    #[test]
    fn q2_gun_offsets_depend_on_frame_and_index_is_always_sent() -> Result<(), Error> {
        let mut from = [0; Q2_PLAYER_WORDS];
        from[22] = 1025;
        let mut to = from;
        to[24] = (-2.25f32).to_bits();
        let mut bytes = [0; 1024];
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        write_q2_player(&mut writer, &from, &to)?;
        assert_eq!(writer.size(), 8);
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        let decoded = read_q2_player(&mut reader, &from)?;
        assert_eq!(decoded[22], 1);
        assert_eq!(decoded[24], 0);
        to[23] = 1;
        to[18] = (-450.0f32).to_bits();
        to[33] = (254.0f32 / 255.0).to_bits();
        to[67] = (-32768i32) as u32;
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        write_q2_player(&mut writer, &from, &to)?;
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        let decoded = read_q2_player(&mut reader, &from)?;
        assert_eq!(decoded[24], to[24]);
        assert_eq!(decoded[18], (-90.0f32).to_bits());
        assert_eq!(decoded[33], to[33]);
        assert_eq!(decoded[67], to[67]);
        Ok(())
    }

    #[test]
    fn entity_control_bits_and_native_number_limit() -> Result<(), Error> {
        let state = [0; ENTITY_WORDS];
        let mut bytes = [0; 1024];
        let mut writer = Writer::new(&mut bytes, Encoding::Q3);
        assert!(!write_q3_entity(
            &mut writer,
            1024,
            &state,
            Some(&state),
            true
        )?);
        assert!(!write_q3_entity(
            &mut writer,
            1,
            &state,
            Some(&state),
            false
        )?);
        assert_eq!(writer.bit_position(), 0);
        assert!(write_q3_entity(
            &mut writer,
            1022,
            &state,
            Some(&state),
            true
        )?);
        let mut reader = Reader::new(writer.bytes(), Encoding::Q3);
        assert_eq!(
            read_q3_entity(&mut reader, &state)?,
            Q3EntityDelta {
                number: 1022,
                words: Some(state)
            }
        );
        let mut writer = Writer::new(&mut bytes, Encoding::Q3);
        write_q3_entity(&mut writer, 3, &state, None, false)?;
        let mut reader = Reader::new(writer.bytes(), Encoding::Q3);
        assert_eq!(
            read_q3_entity(&mut reader, &state)?,
            Q3EntityDelta {
                number: 3,
                words: None
            }
        );
        Ok(())
    }
    #[test]
    fn signed_arrays_and_native_float_zero() -> Result<(), Error> {
        let from = [0; PLAYER_WORDS];
        let mut to = from;
        to[1] = (-0.0f32).to_bits();
        to[8] = (-32768i32) as u32;
        to[28] = (-128i32) as u32;
        to[48] = (-1i32) as u32;
        to[48 + 16 + 15] = (-32768i32) as u32;
        to[48 + 32 + 7] = (-123i32) as u32;
        to[48 + 48] = 0x89abcdef;
        let mut bytes = [0; 1024];
        let mut writer = Writer::new(&mut bytes, Encoding::Q3);
        write_q3_player(&mut writer, &from, &to)?;
        let mut reader = Reader::new(writer.bytes(), Encoding::Q3);
        let decoded = read_q3_player(&mut reader, &from)?;
        to[1] = 0;
        assert_eq!(decoded, to);
        Ok(())
    }
    #[test]
    fn bad_last_changed_and_truncated_message_preserve_source() -> Result<(), Error> {
        let from = [0x1234; PLAYER_WORDS];
        let mut bytes = [0; 1024];
        let mut writer = Writer::new(&mut bytes, Encoding::Q3);
        writer.write_bits(255, 8)?;
        let mut reader = Reader::new(writer.bytes(), Encoding::Q3);
        assert_eq!(
            read_q3_player(&mut reader, &from).map_err(|e| e.kind),
            Err(ErrorKind::Width)
        );
        let mut reader = Reader::new(&[], Encoding::Q3);
        assert_eq!(
            read_q3_player(&mut reader, &from).map_err(|e| e.kind),
            Err(ErrorKind::Truncated)
        );
        assert_eq!(from, [0x1234; PLAYER_WORDS]);
        Ok(())
    }

    #[test]
    fn entity_float_payloads_and_every_truncated_prefix() -> Result<(), Error> {
        let from = [0; ENTITY_WORDS];
        let mut bytes = [0; 1024];
        for bits in [
            (-4096.0f32).to_bits(),
            4095.0f32.to_bits(),
            4096.0f32.to_bits(),
            0.125f32.to_bits(),
            0x7fc12345,
            f32::INFINITY.to_bits(),
            f32::NEG_INFINITY.to_bits(),
        ] {
            let mut to = from;
            to[1] = bits;
            to[50] = 65535;
            let mut writer = Writer::new(&mut bytes, Encoding::Q3);
            write_q3_entity(&mut writer, 1022, &from, Some(&to), false)?;
            let required = writer.bit_position().div_ceil(8);
            let encoded = writer.bytes();
            let mut reader = Reader::new(encoded, Encoding::Q3);
            assert_eq!(read_q3_entity(&mut reader, &from)?.words, Some(to));
            for end in 0..required {
                let mut reader = Reader::new(&encoded[..end], Encoding::Q3);
                assert!(read_q3_entity(&mut reader, &from).is_err());
            }
        }
        let mut writer = Writer::new(&mut bytes, Encoding::Q3);
        writer.write_bits(1, 10)?;
        writer.write_bits(0, 1)?;
        writer.write_bits(1, 1)?;
        writer.write_bits(52, 8)?;
        let mut reader = Reader::new(writer.bytes(), Encoding::Q3);
        assert_eq!(
            read_q3_entity(&mut reader, &from).map_err(|e| e.kind),
            Err(ErrorKind::Width)
        );
        Ok(())
    }
}
