//! Native state projections in protocol table order, not engine entity storage.
use crate::{
    delta::{self, Field, Group, Presence, Value},
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
    let mut result = [Field {
        word: 0,
        bits: 0,
        flag: 0,
        value: Value::Unsigned,
    }; N];
    let mut i = 0;
    while i < N {
        let width = layout[i].1;
        result[i] = Field {
            word: i,
            bits: width.unsigned_abs(),
            flag: 0,
            value: if width == 0 {
                Value::Float { zero: entity }
            } else if width < 0 {
                Value::Signed
            } else if entity {
                Value::ZeroUnsigned
            } else {
                Value::Unsigned
            },
        };
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
const fn array_fields(array: usize) -> [Field; 16] {
    let mut fields = [Field {
        word: 0,
        bits: 16,
        flag: 0,
        value: Value::Signed,
    }; 16];
    let mut i = 0;
    while i < fields.len() {
        fields[i].word = PLAYER_LAYOUT.len() + array * 16 + i;
        fields[i].flag = 1 << i;
        if array == 3 {
            fields[i].bits = 32;
            fields[i].value = Value::Unsigned;
        }
        i += 1;
    }
    fields
}
static STATS: [Field; 16] = array_fields(0);
static PERSISTANT: [Field; 16] = array_fields(1);
static AMMO: [Field; 16] = array_fields(2);
static POWERUPS: [Field; 16] = array_fields(3);
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
    let count = to.map_or(0, |to| delta::changed(&ENTITY_FIELDS, from, to));
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
pub struct Q3EntityDelta {
    pub number: u16,
    pub words: Option<[u32; ENTITY_WORDS]>,
}
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
        .any(|group| delta::changed(group.fields, from, to) != 0);
    writer.write_bits(u32::from(arrays), 1)?;
    if arrays {
        delta::write(&PLAYER_ARRAYS, from, to, 0, writer)?;
    }
    Ok(())
}
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Encoding, ErrorKind};

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
