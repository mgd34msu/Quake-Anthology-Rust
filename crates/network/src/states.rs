//! Native state projections in protocol table order, not engine entity storage.
use crate::{
    commands::{QwCmd, delta as command_delta, packet::ZERO_QW},
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
    read_q3_entity_body(reader, number, from)
}

pub(crate) fn read_q3_entity_body(
    reader: &mut Reader<'_>,
    number: u16,
    from: &[u32; ENTITY_WORDS],
) -> Result<Q3EntityDelta, Error> {
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
pub const Q2_RR_STATS: usize = 64;
static Q2_RR_STAT_FIELDS: [Field; Q2_RR_STATS] = mask_fields(0, 16, Value::Signed);
static Q2_RR_STAT_GROUP: [Group; 1] = [Group {
    fields: &Q2_RR_STAT_FIELDS,
    presence: Presence::Mask(64),
}];
static Q2_KEX_STAT_HALVES: [[Field; 32]; 2] = [
    mask_fields(0, 16, Value::Signed),
    mask_fields(32, 16, Value::Signed),
];
static Q2_KEX_STAT_GROUPS: [Group; 2] = [
    Group {
        fields: &Q2_KEX_STAT_HALVES[0],
        presence: Presence::Mask(32),
    },
    Group {
        fields: &Q2_KEX_STAT_HALVES[1],
        presence: Presence::Mask(32),
    },
];

/// Retail KEX: each 32-bit stat mask precedes that half's signed values.
pub fn write_q2_kex_stats(
    writer: &mut Writer<'_>,
    from: &[u32; Q2_RR_STATS],
    to: &[u32; Q2_RR_STATS],
) -> Result<(), Error> {
    delta::write(&Q2_KEX_STAT_GROUPS, from, to, 0, writer)
}
pub fn read_q2_kex_stats(
    reader: &mut Reader<'_>,
    from: &[u32; Q2_RR_STATS],
) -> Result<[u32; Q2_RR_STATS], Error> {
    let mut words = *from;
    delta::read(&Q2_KEX_STAT_GROUPS, &mut words, 0, reader)?;
    Ok(words)
}

/// Q2repro MSG_PS_RERELEASE: both halves of the stat mask precede all values.
/// Retail KEX instead interleaves each 32-bit mask and its own stat values.
pub fn write_q2_rr_stats(
    writer: &mut Writer<'_>,
    from: &[u32; Q2_RR_STATS],
    to: &[u32; Q2_RR_STATS],
) -> Result<(), Error> {
    delta::write(&Q2_RR_STAT_GROUP, from, to, 0, writer)
}
pub fn read_q2_rr_stats(
    reader: &mut Reader<'_>,
    from: &[u32; Q2_RR_STATS],
) -> Result<[u32; Q2_RR_STATS], Error> {
    let mut words = *from;
    delta::read(&Q2_RR_STAT_GROUP, &mut words, 0, reader)?;
    Ok(words)
}

/// Fixed native rerelease words, before module/game-ABI conversion. A protocol
/// table chooses each word's interpretation; this is not common player storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Q2RereleasePlayer<const WORDS: usize> {
    pub words: [u32; WORDS],
    pub stats: [u32; Q2_RR_STATS],
}
impl<const WORDS: usize> Default for Q2RereleasePlayer<WORDS> {
    fn default() -> Self {
        Self {
            words: [0; WORDS],
            stats: [0; Q2_RR_STATS],
        }
    }
}
/// Q2repro: classic order, damage blend, gunrate, viewheight and clientnum.
/// Coordinates/delta angles are float bits; view/weapon/color words are packed.
pub type Q2ReproPlayer = Q2RereleasePlayer<43>;
/// Retail KEX: same order through viewheight, without clientnum. Coordinates,
/// delta/view/weapon angles and weapon offsets are native float bits. Viewoffset,
/// kickangles and blends are packed words. Gunframe has nine native bits.
pub type Q2KexPlayer = Q2RereleasePlayer<42>;
const fn q2_repro_fields() -> [Field; 43] {
    let mut fields = [Field::new(0, 8, 0, Value::Unsigned); 43];
    let mut i = 0;
    while i < fields.len() {
        let (bits, flag, value) = match i {
            0 => (8, 1, Value::Unsigned),
            1..=2 => (32, 1 << 1, Value::RawFloat),
            3 => (32, 1 << 19, Value::RawFloat),
            4..=5 => (32, 1 << 2, Value::RawFloat),
            6 => (32, 1 << 18, Value::RawFloat),
            7 => (16, 1 << 3, Value::Unsigned),
            8 => (16, 1 << 4, Value::Unsigned),
            9 => (16, 1 << 5, Value::Signed),
            10..=12 => (16, 1 << 6, Value::Angle16),
            13..=15 => (16, 1 << 7, Value::Signed),
            16..=17 => (16, 1 << 8, Value::Signed),
            18 => (16, 1 << 20, Value::Signed),
            19..=21 => (16, 1 << 9, Value::Signed),
            22 => (16, 1 << 12, Value::Unsigned),
            23 => (16, 1 << 13, Value::Unsigned),
            24..=26 => (16, 1 << 16, Value::Signed),
            27..=29 => (16, 1 << 17, Value::Signed),
            30..=33 | 36..=39 => (8, 1 << 10, Value::Unsigned),
            34 => (8, 1 << 11, Value::Unsigned),
            35 => (8, 1 << 14, Value::Unsigned),
            40 => (8, 1 << 23, Value::Unsigned),
            41 => (8, 1 << 15, Value::Signed),
            _ => (16, 1 << 22, Value::Signed),
        };
        fields[i] = Field::new(i, bits, flag, value);
        i += 1;
    }
    fields
}
static Q2_REPRO_FIELDS: [Field; 43] = q2_repro_fields();
const fn q2_repro_blend_fields() -> [Field; 8] {
    let source = q2_repro_fields();
    let mut fields = [source[30]; 8];
    let mut i = 0;
    while i < fields.len() {
        fields[i] = source[if i < 4 { 30 + i } else { 32 + i }];
        fields[i].flag = 1 << i;
        i += 1;
    }
    fields
}
static Q2_REPRO_BLEND_FIELDS: [Field; 8] = q2_repro_blend_fields();
static Q2_REPRO_BLEND_GROUP: [Group; 1] = [Group {
    fields: &Q2_REPRO_BLEND_FIELDS,
    presence: Presence::Mask(8),
}];
static Q2_REPRO_PREFIX: [Group<true, true>; 1] = [Group {
    fields: Q2_REPRO_FIELDS.split_at(30).0,
    presence: Presence::Fixed,
}];
static Q2_REPRO_VIEW: [Group<true, true>; 1] = [Group {
    fields: Q2_REPRO_FIELDS.split_at(34).1.split_at(2).0,
    presence: Presence::Fixed,
}];
static Q2_REPRO_SUFFIX: [Group<true, true>; 1] = [Group {
    fields: Q2_REPRO_FIELDS.split_at(40).1,
    presence: Presence::Fixed,
}];
const Q2_REPRO_STAT_FLAG: u64 = 1 << 21;

/// The returned extra flags belong to the native frame prefix; the body starts
/// with its 16-bit player flags. This is Q2repro 1038, not retail KEX 2023.
pub fn write_q2_repro_player(
    writer: &mut Writer<'_>,
    from: &Q2ReproPlayer,
    to: &Q2ReproPlayer,
) -> Result<u8, Error> {
    let mut flags =
        delta::mask::<true, true, false, false>(&Q2_REPRO_FIELDS, &from.words, &to.words, 0, 0, 0);
    if delta::changed::<false, false, false, false>(&Q2_RR_STAT_FIELDS, &from.stats, &to.stats) != 0
    {
        flags |= Q2_REPRO_STAT_FLAG;
    }
    writer.write_bits(flags as u32, 16)?;
    delta::write(&Q2_REPRO_PREFIX, &from.words, &to.words, flags, writer)?;
    if flags & (1 << 10) != 0 {
        delta::write(&Q2_REPRO_BLEND_GROUP, &from.words, &to.words, 0, writer)?;
    }
    delta::write(&Q2_REPRO_VIEW, &from.words, &to.words, flags, writer)?;
    if flags & Q2_REPRO_STAT_FLAG != 0 {
        write_q2_rr_stats(writer, &from.stats, &to.stats)?;
    }
    delta::write(&Q2_REPRO_SUFFIX, &from.words, &to.words, flags, writer)?;
    Ok((flags >> 16) as u8)
}
pub fn read_q2_repro_player(
    reader: &mut Reader<'_>,
    from: &Q2ReproPlayer,
    extra_flags: u8,
) -> Result<Q2ReproPlayer, Error> {
    let flags = u64::from(reader.read_bits(16)?) | (u64::from(extra_flags) << 16);
    let mut to = *from;
    delta::read(&Q2_REPRO_PREFIX, &mut to.words, flags, reader)?;
    if flags & (1 << 10) != 0 {
        delta::read(&Q2_REPRO_BLEND_GROUP, &mut to.words, 0, reader)?;
    }
    delta::read(&Q2_REPRO_VIEW, &mut to.words, flags, reader)?;
    if flags & Q2_REPRO_STAT_FLAG != 0 {
        to.stats = read_q2_rr_stats(reader, &from.stats)?;
    }
    delta::read(&Q2_REPRO_SUFFIX, &mut to.words, flags, reader)?;
    Ok(to)
}
const fn q2_kex_fields() -> [Field; 42] {
    let source = q2_repro_fields();
    let mut fields = [source[0]; 42];
    let mut i = 0;
    while i < fields.len() {
        fields[i] = match i {
            1..=3 => Field::new(i, 32, 1 << 1, Value::RawFloat),
            4..=6 => Field::new(i, 32, 1 << 2, Value::RawFloat),
            10..=12 => Field::new(i, 32, 1 << 6, Value::RawFloat),
            16..=18 => Field::new(i, 32, 1 << 8, Value::RawFloat),
            23 => Field::new(i, 9, 1 << 13, Value::Unsigned),
            24..=29 => Field::new(i, 32, 1 << 13, Value::RawFloat),
            36..=39 => Field::new(i, 8, 1 << 16, Value::Unsigned),
            40 => Field::new(i, 8, 1 << 13, Value::Unsigned),
            41 => Field::new(i, 8, 1 << 7, Value::Signed),
            _ => source[i],
        };
        i += 1;
    }
    fields
}
static Q2_KEX_FIELDS: [Field; 42] = q2_kex_fields();
const fn q2_kex_prefix_fields() -> [Field; 24] {
    let source = q2_kex_fields();
    let mut fields = [source[0]; 24];
    let mut i = 0;
    while i < fields.len() {
        fields[i] = source[if i < 16 {
            i
        } else if i == 16 {
            41
        } else {
            i - 1
        }];
        i += 1;
    }
    fields
}
static Q2_KEX_PREFIX_FIELDS: [Field; 24] = q2_kex_prefix_fields();
static Q2_KEX_PREFIX: [Group<true, true>; 1] = [Group {
    fields: &Q2_KEX_PREFIX_FIELDS,
    presence: Presence::Fixed,
}];
const fn q2_kex_gun_fields() -> [Field; 7] {
    let source = q2_kex_fields();
    let mut fields = [source[24]; 7];
    let mut i = 0;
    while i < fields.len() {
        fields[i] = source[if i < 6 { 24 + i } else { 40 }];
        fields[i].flag = 1 << i;
        i += 1;
    }
    fields
}
static Q2_KEX_GUN_FIELDS: [Field; 7] = q2_kex_gun_fields();
static Q2_KEX_GUN: [Group<true>; 1] = [Group {
    fields: &Q2_KEX_GUN_FIELDS,
    presence: Presence::PackedMask {
        bits: 16,
        shift: 9,
        word: 23,
    },
}];
static Q2_KEX_VIEW: [Group<true, true>; 1] = [Group {
    fields: Q2_KEX_FIELDS.split_at(30).1.split_at(6).0,
    presence: Presence::Fixed,
}];
static Q2_KEX_DAMAGE: [Group<true, true>; 1] = [Group {
    fields: Q2_KEX_FIELDS.split_at(36).1.split_at(4).0,
    presence: Presence::Fixed,
}];

/// Retail KEX's svc_playerinfo body and opcode; no frame/channel header.
pub fn write_q2_kex_player(
    writer: &mut Writer<'_>,
    from: &Q2KexPlayer,
    to: &Q2KexPlayer,
) -> Result<(), Error> {
    let mut flags =
        delta::mask::<true, true, false, false>(&Q2_KEX_FIELDS, &from.words, &to.words, 0, 0, 0);
    if flags >> 16 != 0 {
        flags |= 1 << 15;
    }
    writer.write_bits(17, 8)?;
    writer.write_bits(flags as u32, 16)?;
    if flags & (1 << 15) != 0 {
        writer.write_bits((flags >> 16) as u32, 16)?;
    }
    delta::write(&Q2_KEX_PREFIX, &from.words, &to.words, flags, writer)?;
    if flags & (1 << 13) != 0 {
        delta::write(&Q2_KEX_GUN, &from.words, &to.words, 0, writer)?;
    }
    delta::write(&Q2_KEX_VIEW, &from.words, &to.words, flags, writer)?;
    write_q2_kex_stats(writer, &from.stats, &to.stats)?;
    delta::write(&Q2_KEX_DAMAGE, &from.words, &to.words, flags, writer)
}
pub fn read_q2_kex_player(
    reader: &mut Reader<'_>,
    from: &Q2KexPlayer,
) -> Result<Q2KexPlayer, Error> {
    if reader.read_bits(8)? != 17 {
        return Err(Error {
            byte: reader.byte_position(),
            kind: crate::message::ErrorKind::Width,
        });
    }
    let mut flags = u64::from(reader.read_bits(16)?);
    if flags & (1 << 15) != 0 {
        flags |= u64::from(reader.read_bits(16)?) << 16;
    }
    let mut to = *from;
    delta::read(&Q2_KEX_PREFIX, &mut to.words, flags, reader)?;
    if flags & (1 << 13) != 0 {
        delta::read(&Q2_KEX_GUN, &mut to.words, 0, reader)?;
    }
    delta::read(&Q2_KEX_VIEW, &mut to.words, flags, reader)?;
    to.stats = read_q2_kex_stats(reader, &from.stats)?;
    delta::read(&Q2_KEX_DAMAGE, &mut to.words, flags, reader)?;
    // The reference parser consumes this native capability without exposing it.
    if flags & (1 << 17) != 0 {
        reader.read_bits(8)?;
    }
    Ok(to)
}
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
    let mut flags =
        delta::mask::<true, true, false, false>(&QW_ENTITY_FIELDS, from, to, 0, 0, 0) as u32;
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
    delta::write(&QW_ENTITY_GROUP, from, to, u64::from(flags), writer)?;
    Ok(true)
}
pub fn read_qw_entity(
    reader: &mut Reader<'_>,
    from: &[u32; QW_ENTITY_WORDS],
) -> Result<QwEntityDelta, Error> {
    let header = read_qw_entity_header(reader)?;
    read_qw_entity_body(reader, header, from)
}
pub(crate) fn read_qw_entity_header(reader: &mut Reader<'_>) -> Result<EntityHeader, Error> {
    // MSG_ReadShort sign-extends before CL_ParseDelta stores its native flags.
    let header = reader.read_bits(16)? as i16 as i32 as u32;
    let number = (header & 511) as u16;
    let mut flags = header & !511;
    if flags & (1 << 14) == 0 && flags & (1 << 15) != 0 {
        flags |= reader.read_bits(8)?;
    }
    Ok(EntityHeader {
        number,
        flags: u64::from(flags),
    })
}
pub(crate) fn read_qw_entity_body(
    reader: &mut Reader<'_>,
    header: EntityHeader,
    from: &[u32; QW_ENTITY_WORDS],
) -> Result<QwEntityDelta, Error> {
    if header.flags & (1 << 14) != 0 {
        return Ok(QwEntityDelta {
            number: header.number,
            words: None,
        });
    }
    let mut words = *from;
    words[11] = header.flags as u32;
    delta::read(&QW_ENTITY_GROUP, &mut words, header.flags, reader)?;
    Ok(QwEntityDelta {
        number: header.number,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityHeader {
    pub number: u16,
    pub flags: u64,
}
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
const Q2_NUMBER16: u64 = 1 << 8;
const Q2_REMOVE: u64 = 1 << 6;
/// Native entity header shared by Q2-34 and the extended rerelease dialects.
/// `extended` selects the fifth flags byte at the negotiated protocol boundary.
/// Entity-number admission remains the connection's independent native limit.
pub fn write_q2_entity_prefix(
    writer: &mut Writer<'_>,
    number: u16,
    mut flags: u64,
    extended: bool,
) -> Result<u64, Error> {
    let octets = if extended { 5 } else { 4 };
    if flags >> (octets * 8) != 0 {
        return Err(Error {
            byte: writer.size(),
            kind: crate::message::ErrorKind::Width,
        });
    }
    if number >= 256 {
        flags |= Q2_NUMBER16;
    }
    for byte in 1..octets {
        if flags >> (byte * 8) != 0 {
            flags |= 1 << (byte * 8 - 1);
        }
    }
    writer.write_bits(flags as u32, 8)?;
    for byte in 1..octets {
        if flags & (1 << (byte * 8 - 1)) != 0 {
            writer.write_bits((flags >> (byte * 8)) as u32, 8)?;
        }
    }
    writer.write_bits(
        u32::from(number),
        if flags & Q2_NUMBER16 != 0 { 16 } else { 8 },
    )?;
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
        write_q2_entity_prefix(writer, number as u16, Q2_REMOVE | number_flag, false)?;
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
    let flags = write_q2_entity_prefix(writer, number as u16, flags, false)?;
    delta::write(&Q2_ENTITY_GROUP, from, to, flags, writer)?;
    Ok(true)
}
pub fn read_q2_entity(
    reader: &mut Reader<'_>,
    from: &[u32; Q2_ENTITY_WORDS],
) -> Result<Q2EntityDelta, Error> {
    let header = read_q2_entity_prefix(reader, false)?;
    read_q2_entity_body(reader, header, from)
}

pub fn read_q2_entity_prefix(
    reader: &mut Reader<'_>,
    extended: bool,
) -> Result<EntityHeader, Error> {
    let mut flags = u64::from(reader.read_bits(8)?);
    for byte in 1..if extended { 5 } else { 4 } {
        if flags & (1 << (byte * 8 - 1)) != 0 {
            flags |= u64::from(reader.read_bits(8)?) << (byte * 8);
        }
    }
    let number = reader.read_bits(if flags & Q2_NUMBER16 != 0 { 16 } else { 8 })? as u16;
    Ok(EntityHeader { number, flags })
}

pub(crate) fn q2_unchanged_entity(from: &[u32; Q2_ENTITY_WORDS]) -> [u32; Q2_ENTITY_WORDS] {
    let mut words = *from;
    words[14..17].copy_from_slice(&from[8..11]);
    words[18] = 0;
    words
}

pub(crate) fn read_q2_entity_body(
    reader: &mut Reader<'_>,
    header: EntityHeader,
    from: &[u32; Q2_ENTITY_WORDS],
) -> Result<Q2EntityDelta, Error> {
    let EntityHeader { number, flags } = header;
    if flags & Q2_REMOVE != 0 {
        return Ok(Q2EntityDelta {
            number,
            words: None,
        });
    }
    let mut words = q2_unchanged_entity(from);
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
        u64::from(step) << 5,
        0,
        0,
    ) as u32;
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
    delta::write(&NQ_ENTITY_GROUP, baseline, to, u64::from(flags), writer)?;
    Ok(true)
}
#[inline(always)]
pub fn read_nq_entity(
    reader: &mut Reader<'_>,
    baseline: &[u32; NQ_ENTITY_WORDS],
) -> Result<NqEntityUpdate, Error> {
    let header = read_nq_entity_header(reader)?;
    read_nq_entity_body(reader, header, baseline)
}
pub(crate) fn read_nq_entity_header(reader: &mut Reader<'_>) -> Result<EntityHeader, Error> {
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
    Ok(EntityHeader {
        number: number as u16,
        flags: u64::from(flags),
    })
}
pub(crate) fn read_nq_entity_body(
    reader: &mut Reader<'_>,
    header: EntityHeader,
    baseline: &[u32; NQ_ENTITY_WORDS],
) -> Result<NqEntityUpdate, Error> {
    let EntityHeader { number, flags } = header;
    let mut words = *baseline;
    words[11] = u32::from(flags & (1 << 5) != 0);
    delta::read(&NQ_ENTITY_GROUP, &mut words, flags, reader)?;
    Ok(NqEntityUpdate {
        number,
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
#[inline(always)]
pub fn write_nq_player(writer: &mut Writer<'_>, to: &[u32; NQ_PLAYER_WORDS]) -> Result<(), Error> {
    let always =
        (1 << 9) | (1 << 14) | (u32::from(to[19] != 0) << 10) | (u32::from(to[20] != 0) << 11);
    let flags = delta::mask::<true, true, false, true>(
        &NQ_PLAYER_FIELDS,
        &NQ_PLAYER_DEFAULTS,
        to,
        u64::from(always),
        0,
        0,
    ) as u32;
    writer.write_bits(15, 8)?;
    writer.write_bits(flags, 16)?;
    delta::write(
        &NQ_PLAYER_GROUP,
        &NQ_PLAYER_DEFAULTS,
        to,
        u64::from(flags),
        writer,
    )
}
#[inline(always)]
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
    delta::read(&NQ_PLAYER_GROUP, &mut words, u64::from(flags), reader)?;
    if active_weapon_is_mask {
        words[18] = 1u32.wrapping_shl(words[18]);
    }
    Ok(words)
}

/// Protocol-28 player information. Scalars preserve QC floats until write;
/// flags/msec are native integers. Body yaw is input metadata for corpse commands.
pub const QW_PLAYER_LAYOUT: [(&str, i8); 14] = [
    ("origin[0]", -16),
    ("origin[1]", -16),
    ("origin[2]", -16),
    ("frame", 8),
    ("msec", 8),
    ("velocity[0]", -16),
    ("velocity[1]", -16),
    ("velocity[2]", -16),
    ("modelindex", 8),
    ("skinnum", 8),
    ("effects", 8),
    ("weaponframe", 8),
    ("flags", -16),
    ("body_yaw_input", 0),
];
pub const QW_PLAYER_WORDS: usize = QW_PLAYER_LAYOUT.len();
static QW_PLAYER_PREFIX: [Group<true, true, false, true>; 1] = [Group {
    fields: &[
        Field::new(
            0,
            16,
            0,
            Value::Scaled {
                factor: 8,
                read: ScaleRead::Signed,
            },
        ),
        Field::new(
            1,
            16,
            0,
            Value::Scaled {
                factor: 8,
                read: ScaleRead::Signed,
            },
        ),
        Field::new(
            2,
            16,
            0,
            Value::Scaled {
                factor: 8,
                read: ScaleRead::Signed,
            },
        ),
        Field::new(3, 8, 0, Value::FloatInt),
        Field::new(4, 8, 1 << 0, Value::Unsigned),
    ],
    presence: Presence::Fixed,
}];
static QW_PLAYER_SUFFIX: [Group<true, true, false, true>; 1] = [Group {
    fields: &[
        Field::new(
            5,
            16,
            1 << 2,
            Value::Scaled {
                factor: 1,
                read: ScaleRead::Signed,
            },
        ),
        Field::new(
            6,
            16,
            1 << 3,
            Value::Scaled {
                factor: 1,
                read: ScaleRead::Signed,
            },
        ),
        Field::new(
            7,
            16,
            1 << 4,
            Value::Scaled {
                factor: 1,
                read: ScaleRead::Signed,
            },
        ),
        Field::new(8, 8, 1 << 5, Value::FloatInt),
        Field::new(9, 8, 1 << 6, Value::FloatInt),
        Field::new(10, 8, 1 << 7, Value::FloatInt),
        Field::new(11, 8, 1 << 8, Value::FloatInt),
    ],
    presence: Presence::Fixed,
}];
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QwPlayerInfo {
    pub number: u8,
    pub words: [u32; QW_PLAYER_WORDS],
    pub command: QwCmd,
}
/// The provider supplies native self/spectator/tracked-player flags after its
/// visibility selection. The command is sanitized at this native wire boundary.
pub fn write_qw_player(
    writer: &mut Writer<'_>,
    number: u32,
    to: &[u32; QW_PLAYER_WORDS],
    mut command: QwCmd,
) -> Result<bool, Error> {
    if number >= 32 {
        return Ok(false);
    }
    let flags = to[12];
    let mut words = *to;
    if words[4] as i32 > 255 {
        words[4] = 255;
    }
    writer.write_bits(42, 8)?;
    writer.write_bits(number, 8)?;
    writer.write_bits(flags, 16)?;
    delta::write(&QW_PLAYER_PREFIX, &words, &words, u64::from(flags), writer)?;
    if flags & (1 << 1) != 0 {
        if flags & (1 << 9) != 0 {
            // qsrc resets pitch twice; its roll remains the supplied command.
            command.view_angles.0[0] = 0.0;
            command.view_angles.0[1] = f32::from_bits(to[13]);
        }
        command.buttons = 0;
        command.impulse = 0;
        command_delta::write_qw(writer, ZERO_QW, command)?;
    }
    delta::write(&QW_PLAYER_SUFFIX, &words, &words, u64::from(flags), writer)?;
    Ok(true)
}
pub fn read_qw_player(
    reader: &mut Reader<'_>,
    default_model: u32,
    prior_slot_command: impl FnOnce(u8) -> QwCmd,
) -> Result<QwPlayerInfo, Error> {
    if reader.read_bits(8)? != 42 {
        return Err(Error {
            byte: reader.byte_position(),
            kind: crate::message::ErrorKind::Symbol,
        });
    }
    let number = reader.read_bits(8)?;
    if number >= 32 {
        return Err(Error {
            byte: reader.byte_position(),
            kind: crate::message::ErrorKind::Symbol,
        });
    }
    let flags = reader.read_bits(16)?;
    let mut words = [0; QW_PLAYER_WORDS];
    words[8] = default_model;
    words[12] = flags as i16 as i32 as u32;
    delta::read(&QW_PLAYER_PREFIX, &mut words, u64::from(flags), reader)?;
    let command = if flags & (1 << 1) != 0 {
        command_delta::read_qw(reader, ZERO_QW)?
    } else {
        prior_slot_command(number as u8)
    };
    delta::read(&QW_PLAYER_SUFFIX, &mut words, u64::from(flags), reader)?;
    Ok(QwPlayerInfo {
        number: number as u8,
        words,
        command,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Encoding, ErrorKind};

    #[test]
    fn qw_player_retains_slot_command_and_native_corpse_roll() -> Result<(), Error> {
        let prior = QwCmd {
            view_angles: qa_core::primitives::Vec3([10.0, 20.0, 30.0]),
            movement: [17, -18, 19],
            msec: 20,
            buttons: 3,
            impulse: 4,
        };
        let mut to = [0; QW_PLAYER_WORDS];
        to[0] = 1.125f32.to_bits();
        to[3] = 7.5f32.to_bits();
        let mut bytes = [0xff; 128];
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        assert!(write_qw_player(&mut writer, 0, &to, ZERO_QW)?);
        assert_eq!(writer.size(), 11);
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        let decoded = read_qw_player(&mut reader, 44, |_| prior)?;
        assert_eq!(decoded.number, 0);
        assert_eq!(decoded.command, prior);
        assert_eq!(decoded.words[3], 7);
        assert_eq!(decoded.words[8], 44);
        assert_eq!(&decoded.words[5..8], &[0; 3]);
        to[4] = 1000;
        to[5] = (-300.5f32).to_bits();
        to[12] = 3 | (1 << 2) | (1 << 9);
        to[13] = 90.0f32.to_bits();
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        assert!(write_qw_player(&mut writer, 31, &to, prior)?);
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        let decoded = read_qw_player(&mut reader, 44, |_| ZERO_QW)?;
        assert_eq!(decoded.words[4], 255);
        assert_eq!(decoded.words[5], (-300.0f32).to_bits());
        assert_eq!(decoded.command.buttons, 0);
        assert_eq!(decoded.command.impulse, 0);
        assert_eq!(decoded.command.view_angles.0[0], 0.0);
        assert_eq!(decoded.command.view_angles.0[1], 90.0);
        assert_eq!(decoded.command.view_angles.0[2], 5461.0 * (360.0 / 65536.0));
        for length in 0..writer.size() {
            let mut reader = Reader::new(&writer.bytes()[..length], Encoding::Bytes);
            assert_eq!(
                read_qw_player(&mut reader, 44, |_| prior)
                    .map_err(|e| e.kind)
                    .err(),
                Some(ErrorKind::Truncated)
            );
        }
        assert!(!write_qw_player(&mut writer, 32, &to, prior)?);
        Ok(())
    }

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
