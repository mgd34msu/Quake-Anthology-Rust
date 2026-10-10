//! Native state projections in protocol table order, not engine entity storage.
use crate::{
    delta::{self, Field, Group, Packed, Presence, ScaleRead, Value},
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
        delta::changed::<true, false, false, false>(&ENTITY_FIELDS, from, to)
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
        .any(|group| delta::changed::<true, false, false, false>(group.fields, from, to) != 0);
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
#[inline(always)]
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
    Field::new(6, 8, 1 << 0, Value::Angle8 { integral: false }),
    Field::new(
        7,
        16,
        1 << 10,
        Value::Scaled {
            factor: 8,
            read: ScaleRead::SignedTenthsDelta,
        },
    ),
    Field::new(8, 8, 1 << 12, Value::Angle8 { integral: false }),
    Field::new(
        9,
        16,
        1 << 11,
        Value::Scaled {
            factor: 8,
            read: ScaleRead::SignedTenthsDelta,
        },
    ),
    Field::new(10, 8, 1 << 1, Value::Angle8 { integral: false }),
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
    let mut flags = delta::mask::<true, true, false, false>(&QW_ENTITY_FIELDS, from, to, 0, 0, 0);
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

/// Protocol-34 entity words. Number belongs to the record prefix.
pub const Q2_ENTITY_LAYOUT: [(&str, i8); 20] = [
    ("modelindex", 8),
    ("modelindex2", 8),
    ("modelindex3", 8),
    ("modelindex4", 8),
    ("frame", 0),
    ("skinnum", 0),
    ("effects", 0),
    ("renderfx", 0),
    ("origin[0]", -16),
    ("origin[1]", -16),
    ("origin[2]", -16),
    ("angles[0]", -8),
    ("angles[1]", -8),
    ("angles[2]", -8),
    ("old_origin[0]", -16),
    ("old_origin[1]", -16),
    ("old_origin[2]", -16),
    ("sound", 8),
    ("event", 8),
    ("solid", -16),
];
pub const Q2_ENTITY_WORDS: usize = Q2_ENTITY_LAYOUT.len();
pub type Q2EntityDelta = EntityDelta<Q2_ENTITY_WORDS>;
static Q2_ENTITY_FIELDS: [Field; 21] = [
    Field::new(0, 8, 1 << 11, Value::Unsigned),
    Field::new(1, 8, 1 << 20, Value::Unsigned),
    Field::new(2, 8, 1 << 21, Value::Unsigned),
    Field::new(3, 8, 1 << 22, Value::Unsigned),
    // Both native frame flags are read independently in their original order.
    Field::new(4, 8, 1 << 4, Value::Packed(Packed::Signed8)),
    Field::new(4, 16, 1 << 17, Value::Packed(Packed::Signed8)),
    Field::new(
        5,
        0,
        (1 << 16) | (1 << 25),
        Value::Packed(Packed::Unsigned16),
    ),
    Field::new(
        6,
        0,
        (1 << 14) | (1 << 19),
        Value::Packed(Packed::Unsigned15),
    ),
    Field::new(7, 0, (1 << 12) | (1 << 18), Value::Packed(Packed::Signed15)),
    Field::new(
        8,
        16,
        1,
        Value::Scaled {
            factor: 8,
            read: ScaleRead::Signed,
        },
    ),
    Field::new(
        9,
        16,
        1 << 1,
        Value::Scaled {
            factor: 8,
            read: ScaleRead::Signed,
        },
    ),
    Field::new(
        10,
        16,
        1 << 9,
        Value::Scaled {
            factor: 8,
            read: ScaleRead::Signed,
        },
    ),
    Field::new(11, 8, 1 << 10, Value::Angle8 { integral: false }),
    Field::new(12, 8, 1 << 2, Value::Angle8 { integral: false }),
    Field::new(13, 8, 1 << 3, Value::Angle8 { integral: false }),
    Field::new(
        14,
        16,
        1 << 24,
        Value::Scaled {
            factor: 8,
            read: ScaleRead::Signed,
        },
    ),
    Field::new(
        15,
        16,
        1 << 24,
        Value::Scaled {
            factor: 8,
            read: ScaleRead::Signed,
        },
    ),
    Field::new(
        16,
        16,
        1 << 24,
        Value::Scaled {
            factor: 8,
            read: ScaleRead::Signed,
        },
    ),
    Field::new(17, 8, 1 << 26, Value::Unsigned),
    Field::new(18, 8, 1 << 5, Value::Transient),
    Field::new(19, 16, 1 << 27, Value::Signed),
];
static Q2_ENTITY_GROUP: [Group<true, true, true>; 1] = [Group {
    fields: &Q2_ENTITY_FIELDS,
    presence: Presence::Fixed,
}];
const Q2_NUMBER16: u32 = 1 << 8;
const Q2_REMOVE: u32 = 1 << 6;
const Q2_MORE1: u32 = 1 << 7;
const Q2_MORE2: u32 = 1 << 15;
const Q2_MORE3: u32 = 1 << 23;
fn write_q2_entity_prefix(
    writer: &mut Writer<'_>,
    number: u32,
    mut flags: u32,
) -> Result<u32, Error> {
    if flags & 0xff00_0000 != 0 {
        flags |= Q2_MORE1 | Q2_MORE2 | Q2_MORE3;
    } else if flags & 0x00ff_0000 != 0 {
        flags |= Q2_MORE1 | Q2_MORE2;
    } else if flags & 0x0000_ff00 != 0 {
        flags |= Q2_MORE1;
    }
    writer.write_bits(flags, 8)?;
    if flags & Q2_MORE1 != 0 {
        writer.write_bits(flags >> 8, 8)?;
    }
    if flags & Q2_MORE2 != 0 {
        writer.write_bits(flags >> 16, 8)?;
    }
    if flags & Q2_MORE3 != 0 {
        writer.write_bits(flags >> 24, 8)?;
    }
    writer.write_bits(number, if flags & Q2_NUMBER16 != 0 { 16 } else { 8 })?;
    Ok(flags)
}
pub fn write_q2_entity(
    writer: &mut Writer<'_>,
    number: u32,
    from: &[u32; Q2_ENTITY_WORDS],
    to: Option<&[u32; Q2_ENTITY_WORDS]>,
    force: bool,
    new_entity: bool,
) -> Result<bool, Error> {
    if number == 0 || number >= 1024 {
        return Ok(false);
    }
    let number_flag = if number >= 256 { Q2_NUMBER16 } else { 0 };
    let Some(to) = to else {
        write_q2_entity_prefix(writer, number, Q2_REMOVE | number_flag)?;
        return Ok(true);
    };
    let mut flags =
        delta::mask::<true, true, true, false>(&Q2_ENTITY_FIELDS, from, to, number_flag, 15, 3);
    if new_entity || to[7] & 128 != 0 {
        flags |= 1 << 24;
    }
    // Native U_NUMBER16 precedes the unchanged-record check.
    if flags == 0 && !force {
        return Ok(false);
    }
    let flags = write_q2_entity_prefix(writer, number, flags)?;
    delta::write(&Q2_ENTITY_GROUP, from, to, flags, writer)?;
    Ok(true)
}
pub fn read_q2_entity(
    reader: &mut Reader<'_>,
    from: &[u32; Q2_ENTITY_WORDS],
) -> Result<Q2EntityDelta, Error> {
    let mut flags = reader.read_bits(8)?;
    if flags & Q2_MORE1 != 0 {
        flags |= reader.read_bits(8)? << 8;
    }
    if flags & Q2_MORE2 != 0 {
        flags |= reader.read_bits(8)? << 16;
    }
    if flags & Q2_MORE3 != 0 {
        flags |= reader.read_bits(8)? << 24;
    }
    let number = reader.read_bits(if flags & Q2_NUMBER16 != 0 { 16 } else { 8 })? as u16;
    if flags & Q2_REMOVE != 0 {
        return Ok(Q2EntityDelta {
            number,
            words: None,
        });
    }
    let mut words = *from;
    words[14..17].copy_from_slice(&from[8..11]);
    words[18] = 0;
    delta::read(&Q2_ENTITY_GROUP, &mut words, flags, reader)?;
    Ok(Q2EntityDelta {
        number,
        words: Some(words),
    })
}

/// Protocol 15: baseline byte fields are native integers; target byte fields
/// retain QuakeC floats until comparison and truncation. Origin/angles are f32.
/// The final column reports the wire no-lerp flag, not interpolation state.
pub const NQ_ENTITY_LAYOUT: [(&str, i8); 12] = [
    ("modelindex", 8),
    ("frame", 8),
    ("colormap", 8),
    ("skin", 8),
    ("effects", 8),
    ("origin[0]", -16),
    ("angles[0]", -8),
    ("origin[1]", -16),
    ("angles[1]", -8),
    ("origin[2]", -16),
    ("angles[2]", -8),
    ("no_lerp", 0),
];
pub const NQ_ENTITY_WORDS: usize = NQ_ENTITY_LAYOUT.len();
static NQ_ENTITY_FIELDS: [Field; 11] = [
    Field::new(0, 8, 1 << 10, Value::FloatInt),
    Field::new(1, 8, 1 << 6, Value::FloatInt),
    Field::new(2, 8, 1 << 11, Value::FloatInt),
    Field::new(3, 8, 1 << 12, Value::FloatInt),
    Field::new(4, 8, 1 << 13, Value::FloatInt),
    Field::new(
        5,
        16,
        1 << 1,
        Value::Scaled {
            factor: 8,
            read: ScaleRead::SignedTenthsDelta,
        },
    ),
    Field::new(6, 8, 1 << 8, Value::Angle8 { integral: true }),
    Field::new(
        7,
        16,
        1 << 2,
        Value::Scaled {
            factor: 8,
            read: ScaleRead::SignedTenthsDelta,
        },
    ),
    Field::new(8, 8, 1 << 4, Value::Angle8 { integral: true }),
    Field::new(
        9,
        16,
        1 << 3,
        Value::Scaled {
            factor: 8,
            read: ScaleRead::SignedTenthsDelta,
        },
    ),
    Field::new(10, 8, 1 << 9, Value::Angle8 { integral: true }),
];
static NQ_ENTITY_GROUP: [Group<true, true, false, true>; 1] = [Group {
    fields: &NQ_ENTITY_FIELDS,
    presence: Presence::Fixed,
}];
pub type NqEntityUpdate = EntityDelta<NQ_ENTITY_WORDS>;

/// Always emits a visible entity, including unchanged signon-baseline values.
/// Absence from a datagram is not a remove record or a new ACK requirement.
#[inline(always)]
pub fn write_nq_entity(
    writer: &mut Writer<'_>,
    number: u32,
    baseline: &[u32; NQ_ENTITY_WORDS],
    to: &[u32; NQ_ENTITY_WORDS],
    step: bool,
) -> Result<bool, Error> {
    if number == 0 || number >= 32768 {
        return Ok(false);
    }
    let mut flags = delta::mask::<true, true, false, true>(
        &NQ_ENTITY_FIELDS,
        baseline,
        to,
        u32::from(step) << 5,
        0,
        0,
    );
    if number >= 256 {
        flags |= 1 << 14;
    }
    if flags >= 256 {
        flags |= 1;
    }
    writer.write_bits(flags | 128, 8)?;
    if flags & 1 != 0 {
        writer.write_bits(flags >> 8, 8)?;
    }
    writer.write_bits(number, if flags & (1 << 14) != 0 { 16 } else { 8 })?;
    delta::write(&NQ_ENTITY_GROUP, baseline, to, flags, writer)?;
    Ok(true)
}
#[inline(always)]
pub fn read_nq_entity(
    reader: &mut Reader<'_>,
    baseline: &[u32; NQ_ENTITY_WORDS],
) -> Result<NqEntityUpdate, Error> {
    let mut flags = reader.read_bits(8)?;
    if flags & 128 == 0 {
        return Err(Error {
            byte: reader.byte_position(),
            kind: crate::message::ErrorKind::Symbol,
        });
    }
    if flags & 1 != 0 {
        flags |= reader.read_bits(8)? << 8;
    }
    let number = reader.read_bits(if flags & (1 << 14) != 0 { 16 } else { 8 })?;
    if number >= 32768 {
        return Err(Error {
            byte: reader.byte_position(),
            kind: crate::message::ErrorKind::Symbol,
        });
    }
    let mut words = *baseline;
    words[11] = u32::from(flags & (1 << 5) != 0);
    delta::read(&NQ_ENTITY_GROUP, &mut words, flags, reader)?;
    Ok(NqEntityUpdate {
        number: number as u16,
        words: Some(words),
    })
}

/// Native protocol-15 client data. View/punch/velocity and target statistic
/// values retain QuakeC floats. Items, model index and active-weapon byte are
/// already reduced at their ABI/resource boundary; flags are independent words.
pub const NQ_PLAYER_LAYOUT: [(&str, i8); 21] = [
    ("viewheight", -8),
    ("idealpitch", -8),
    ("punchangle[0]", -8),
    ("velocity[0]", -8),
    ("punchangle[1]", -8),
    ("velocity[1]", -8),
    ("punchangle[2]", -8),
    ("velocity[2]", -8),
    ("items", 32),
    ("weaponframe", 8),
    ("armor", 8),
    ("weaponmodel", 8),
    ("health", -16),
    ("ammo", 8),
    ("shells", 8),
    ("nails", 8),
    ("rockets", 8),
    ("cells", 8),
    ("activeweapon", 8),
    ("onground", 0),
    ("inwater", 0),
];
pub const NQ_PLAYER_WORDS: usize = NQ_PLAYER_LAYOUT.len();
const NQ_PLAYER_DEFAULTS: [u32; NQ_PLAYER_WORDS] = {
    let mut words = [0; NQ_PLAYER_WORDS];
    words[0] = 22.0f32.to_bits();
    words
};
static NQ_PLAYER_FIELDS: [Field; 19] = [
    Field::new(
        0,
        8,
        1 << 0,
        Value::Scaled {
            factor: 1,
            read: ScaleRead::Signed,
        },
    ),
    Field::new(
        1,
        8,
        1 << 1,
        Value::Scaled {
            factor: 1,
            read: ScaleRead::Signed,
        },
    ),
    Field::new(
        2,
        8,
        1 << 2,
        Value::Scaled {
            factor: 1,
            read: ScaleRead::Signed,
        },
    ),
    Field::new(
        3,
        8,
        1 << 5,
        Value::Scaled {
            factor: 16,
            read: ScaleRead::SignedInverse,
        },
    ),
    Field::new(
        4,
        8,
        1 << 3,
        Value::Scaled {
            factor: 1,
            read: ScaleRead::Signed,
        },
    ),
    Field::new(
        5,
        8,
        1 << 6,
        Value::Scaled {
            factor: 16,
            read: ScaleRead::SignedInverse,
        },
    ),
    Field::new(
        6,
        8,
        1 << 4,
        Value::Scaled {
            factor: 1,
            read: ScaleRead::Signed,
        },
    ),
    Field::new(
        7,
        8,
        1 << 7,
        Value::Scaled {
            factor: 16,
            read: ScaleRead::SignedInverse,
        },
    ),
    Field::new(8, 32, 0, Value::Unsigned),
    Field::new(9, 8, 1 << 12, Value::FloatInt),
    Field::new(10, 8, 1 << 13, Value::FloatInt),
    Field::new(11, 8, 1 << 14, Value::Unsigned),
    Field::new(12, 16, 0, Value::FloatInt),
    Field::new(13, 8, 0, Value::FloatInt),
    Field::new(14, 8, 0, Value::FloatInt),
    Field::new(15, 8, 0, Value::FloatInt),
    Field::new(16, 8, 0, Value::FloatInt),
    Field::new(17, 8, 0, Value::FloatInt),
    Field::new(18, 8, 0, Value::Unsigned),
];
static NQ_PLAYER_GROUP: [Group<true, true, false, true>; 1] = [Group {
    fields: &NQ_PLAYER_FIELDS,
    presence: Presence::Fixed,
}];
pub fn write_nq_player(writer: &mut Writer<'_>, to: &[u32; NQ_PLAYER_WORDS]) -> Result<(), Error> {
    let always =
        (1 << 9) | (1 << 14) | (u32::from(to[19] != 0) << 10) | (u32::from(to[20] != 0) << 11);
    let flags = delta::mask::<true, true, false, true>(
        &NQ_PLAYER_FIELDS,
        &NQ_PLAYER_DEFAULTS,
        to,
        always,
        0,
        0,
    );
    writer.write_bits(15, 8)?;
    writer.write_bits(flags, 16)?;
    delta::write(&NQ_PLAYER_GROUP, &NQ_PLAYER_DEFAULTS, to, flags, writer)
}
pub fn read_nq_player(
    reader: &mut Reader<'_>,
    active_weapon_is_mask: bool,
) -> Result<[u32; NQ_PLAYER_WORDS], Error> {
    if reader.read_bits(8)? != 15 {
        return Err(Error {
            byte: reader.byte_position(),
            kind: crate::message::ErrorKind::Symbol,
        });
    }
    let flags = reader.read_bits(16)?;
    let mut words = NQ_PLAYER_DEFAULTS;
    words[19] = u32::from(flags & (1 << 10) != 0);
    words[20] = u32::from(flags & (1 << 11) != 0);
    delta::read(&NQ_PLAYER_GROUP, &mut words, flags, reader)?;
    if active_weapon_is_mask {
        words[18] = 1u32.wrapping_shl(words[18]);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Encoding, ErrorKind};

    #[test]
    fn nq_client_defaults_signed_values_and_weapon_mask() -> Result<(), Error> {
        let mut to = NQ_PLAYER_DEFAULTS;
        let mut bytes = [0xff; 128];
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        write_nq_player(&mut writer, &to)?;
        assert_eq!(
            writer.bytes(),
            &[15, 0, 0x42, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        assert_eq!(read_nq_player(&mut reader, false)?, NQ_PLAYER_DEFAULTS);
        to[0] = 129.5f32.to_bits();
        to[1] = (-128.75f32).to_bits();
        to[3] = (-2049.0f32).to_bits();
        to[8] = 0x80000000;
        to[9] = 0.25f32.to_bits();
        to[12] = 40000.5f32.to_bits();
        to[18] = 31;
        to[19] = 1;
        to[20] = 1;
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        write_nq_player(&mut writer, &to)?;
        assert_ne!(writer.bytes()[2] & 0x10, 0);
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        let actual = read_nq_player(&mut reader, true)?;
        assert_eq!(actual[0], (-127.0f32).to_bits());
        assert_eq!(actual[1], (-128.0f32).to_bits());
        assert_eq!(actual[3], (-2048.0f32).to_bits());
        assert_eq!(actual[8], 0x80000000);
        assert_eq!(actual[9], 0);
        assert_eq!(actual[12], (-25536i32) as u32);
        assert_eq!(actual[18], 0x80000000);
        assert_eq!(&actual[19..], &[1, 1]);
        for length in 0..writer.size() {
            let mut reader = Reader::new(&writer.bytes()[..length], Encoding::Bytes);
            assert_eq!(
                read_nq_player(&mut reader, true).map_err(|e| e.kind),
                Err(ErrorKind::Truncated)
            );
        }
        Ok(())
    }

    #[test]
    fn nq_baseline_float_comparison_angle_truncation_and_number_width() -> Result<(), Error> {
        let mut baseline = [0; NQ_ENTITY_WORDS];
        baseline[1] = 4;
        baseline[11] = 1;
        let mut to = baseline;
        to[..5].fill(0.0f32.to_bits());
        to[1] = 4.5f32.to_bits();
        to[6] = 1.40625f32.to_bits();
        to[8] = (-1.40625f32).to_bits();
        let mut bytes = [0xff; 128];
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        write_nq_entity(&mut writer, 256, &baseline, &to, true)?;
        assert_eq!(writer.bytes(), &[0xf1, 0x41, 0, 1, 4, 0, 0]);
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        let update = read_nq_entity(&mut reader, &baseline)?;
        let Some(words) = update.words else {
            return Err(Error {
                byte: 0,
                kind: ErrorKind::Symbol,
            });
        };
        assert_eq!(update.number, 256);
        assert_eq!(words[1], 4);
        assert_eq!(words[6], 0.0f32.to_bits());
        assert_eq!(words[8], 0.0f32.to_bits());
        assert_eq!(words[11], 1);
        for length in 0..writer.size() {
            let mut reader = Reader::new(&writer.bytes()[..length], Encoding::Bytes);
            assert_eq!(
                read_nq_entity(&mut reader, &baseline).map_err(|e| e.kind),
                Err(ErrorKind::Truncated)
            );
        }
        to[1] = 4.0f32.to_bits();
        to[6] = 0.0f32.to_bits();
        to[8] = 0.0f32.to_bits();
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        write_nq_entity(&mut writer, 255, &baseline, &to, false)?;
        assert_eq!(writer.bytes(), &[0x80, 255]);
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        let update = read_nq_entity(&mut reader, &baseline)?;
        assert_eq!(update.words.map(|words| words[11]), Some(0));
        assert!(!write_nq_entity(&mut writer, 0, &baseline, &to, false)?);
        assert!(!write_nq_entity(&mut writer, 32768, &baseline, &to, false)?);
        let mut reader = Reader::new(&[0x80, 0], Encoding::Bytes);
        assert_eq!(read_nq_entity(&mut reader, &baseline)?.number, 0);
        Ok(())
    }

    #[test]
    fn q2_widths_defaults_transient_event_and_frame_flag_order() -> Result<(), Error> {
        let mut from = [0; Q2_ENTITY_WORDS];
        from[8] = 1.25f32.to_bits();
        from[14] = (-4.0f32).to_bits();
        from[18] = 7;
        let mut to = from;
        to[18] = 0;
        let mut bytes = [0; 128];
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        write_q2_entity(&mut writer, 256, &from, Some(&to), false, false)?;
        assert_eq!(writer.bytes(), &[128, 1, 0, 1]);
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        let decoded = read_q2_entity(&mut reader, &from)?;
        let Some(words) = decoded.words else {
            return Err(Error {
                byte: 0,
                kind: ErrorKind::Symbol,
            });
        };
        assert_eq!(words[14], from[8]);
        assert_eq!(words[18], 0);
        to[4] = 32768;
        to[5] = 65535;
        to[6] = 32768;
        to[7] = (-1i32) as u32;
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        write_q2_entity(&mut writer, 1, &from, Some(&to), false, false)?;
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        let Some(words) = read_q2_entity(&mut reader, &from)?.words else {
            return Err(Error {
                byte: 0,
                kind: ErrorKind::Symbol,
            });
        };
        assert_eq!(words[4], (-32768i32) as u32);
        assert_eq!(words[5], u32::MAX);
        assert_eq!(words[6], 32768);
        assert_eq!(words[7], 255);
        assert_eq!(words[14], to[14]);
        for size in 0..writer.size() {
            assert!(
                read_q2_entity(
                    &mut Reader::new(&writer.bytes()[..size], Encoding::Bytes),
                    &from
                )
                .is_err()
            );
        }
        let mut reader = Reader::new(&[0x90, 0x80, 2, 1, 4, 0xbf, 0xfe], Encoding::Bytes);
        assert_eq!(
            read_q2_entity(&mut reader, &from)?.words.map(|w| w[4]),
            Some((-321i32) as u32)
        );
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        write_q2_entity(&mut writer, 256, &from, None, false, false)?;
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        assert!(read_q2_entity(&mut reader, &from)?.words.is_none());
        Ok(())
    }

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
