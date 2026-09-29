//! FitzQuake/RMQ wide and QuakeWorld-29 codecs.
//!
//! Donor provenance: `makeWideCodec` in `src/network/q1/codecs/wide.ts`,
//! `createQw29Codec` in `src/network/q1/codecs/qw29.ts`, and profile
//! selection in `src/network/q1/profile.ts`, with field readback order from
//! `readEntity`/`readClientData` in `src/network/q1/netquake.ts`.
//!
//! Wide entity state reuses [`crate::q1`] shapes where the field widths
//! match; fields that widen (model/frame/sound indexes, entity numbers,
//! ammo counters) live in the wide structs here.

use crate::angles::q_rint;
use crate::msg::{MsgError, MsgReader, MsgWriter};
use crate::protocol::q1 as protocol;
use crate::protocol::qw as qw_protocol;
use crate::q1::ORIGIN_EPSILON;

/// Wide maximum message length (`WIDE_MAX_MSGLEN`).
pub const WIDE_MAX_MSGLEN: usize = 64000;
/// Wide maximum datagram (`WIDE_MAX_DATAGRAM`).
pub const WIDE_MAX_DATAGRAM: usize = 64000;
/// Wide precache limit (`WIDE_MAX_PRECACHE`).
pub const WIDE_MAX_PRECACHE: usize = 65536;
/// QuakeWorld wide protocol version (`PROTOCOL_QW_WIDE`).
pub const PROTOCOL_QW_WIDE: u16 = 29;
/// QuakeWorld wide default flags (`QW29_DEFAULT_FLAGS`).
pub const QW29_DEFAULT_FLAGS: u32 = protocol::PRFL_INT32COORD | protocol::PRFL_SHORTANGLE;
/// QuakeWorld wide extend bit (`U_EXTEND`).
pub const U_EXTEND: u32 = 1 << 7;
/// QuakeWorld wide high entity byte (`U_ENTITY2`).
pub const U_ENTITY2: u32 = 1 << 0;
/// QuakeWorld wide high model byte (`U_MODEL2`).
pub const U_MODEL2: u32 = 1 << 1;
/// QuakeWorld wide high frame byte (`U_FRAME2`).
pub const U_FRAME2: u32 = 1 << 2;
/// QuakeWorld wide alpha byte (`U_ALPHA`).
pub const U_ALPHA: u32 = 1 << 3;
/// QuakeWorld wide scale byte (`U_SCALE`).
pub const U_SCALE: u32 = 1 << 4;
/// QuakeWorld wide maximum entity number (`QW29_MAX_ENTITY_NUMBER`).
pub const QW29_MAX_ENTITY_NUMBER: u32 = 65536;
/// QuakeWorld wide precache limit (`MAX_MODELS`).
pub const QW29_MAX_PRECACHE: usize = 8192;

/// Encode an alpha value (`ENTALPHA_ENCODE`).
#[must_use]
pub fn entalpha_encode(value: f64) -> u8 {
    if value == 0.0 {
        return protocol::ENTALPHA_DEFAULT;
    }
    let scaled = value * 254.0 + 1.0;
    q_rint(scaled.clamp(1.0, 255.0)) as u8
}

/// Decode an alpha byte (`ENTALPHA_DECODE`).
#[must_use]
pub fn entalpha_decode(value: u8) -> f64 {
    if value == protocol::ENTALPHA_DEFAULT {
        1.0
    } else {
        f64::from(value - 1) / 254.0
    }
}

/// Alpha byte for saves (`ENTALPHA_TOSAVE`).
#[must_use]
pub fn entalpha_to_save(value: u8) -> f64 {
    if value == protocol::ENTALPHA_DEFAULT {
        0.0
    } else if value == protocol::ENTALPHA_ZERO {
        -1.0
    } else {
        f64::from(value - 1) / 254.0
    }
}

/// Encode a scale value (`ENTSCALE_ENCODE`).
#[must_use]
pub fn entscale_encode(value: f64) -> u8 {
    if value == 0.0 {
        protocol::ENTSCALE_DEFAULT
    } else {
        (value * f64::from(protocol::ENTSCALE_DEFAULT)) as u8
    }
}

/// Decode a scale byte (`ENTSCALE_DECODE`).
#[must_use]
pub fn entscale_decode(value: u8) -> f64 {
    f64::from(value) / f64::from(protocol::ENTSCALE_DEFAULT)
}

/// NetQuake protocol profile (`Q1ProtocolIdentity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NqProfile {
    /// Protocol 15.
    Netquake,
    /// FitzQuake protocol 666.
    Fitzquake,
    /// RMQ protocol 999 with flags.
    Rmq {
        /// Protocol flags.
        flags: u32,
    },
}

/// QuakeWorld protocol profile (`QuakeWorldProfile`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QwProfile {
    /// Protocol 28.
    Quakeworld,
    /// Donor wide protocol 29 with flags.
    Wide {
        /// Protocol flags.
        flags: u32,
    },
}

/// NetQuake protocol profile (`netQuakeProfile`).
pub fn net_quake_profile(version: u16, flags: u32) -> Result<NqProfile, MsgError> {
    match version {
        15 => Ok(NqProfile::Netquake),
        666 => Ok(NqProfile::Fitzquake),
        999 => {
            if (flags & !protocol::PRFL_SUPPORTED) != 0 {
                return Err(MsgError::BadSize(flags as usize));
            }
            Ok(NqProfile::Rmq { flags })
        }
        _ => Err(MsgError::BadSize(usize::from(version))),
    }
}

/// Default NetQuake profile (`defaultNetQuakeProfile`).
pub fn default_net_quake_profile(version: u16) -> Result<NqProfile, MsgError> {
    net_quake_profile(
        version,
        if version == 999 {
            protocol::PRFL_INT32COORD | protocol::PRFL_SHORTANGLE
        } else {
            0
        },
    )
}

/// QuakeWorld protocol profile (`quakeWorldProfile`).
pub fn quake_world_profile(version: u16, flags: u32) -> Result<QwProfile, MsgError> {
    if version == 28 {
        return Ok(QwProfile::Quakeworld);
    }
    if version == 29 && (flags & !protocol::PRFL_SUPPORTED) == 0 {
        return Ok(QwProfile::Wide { flags });
    }
    Err(MsgError::BadSize(usize::from(version)))
}

/// Protocol flags (`protocolFlags`).
#[must_use]
pub fn nq_protocol_flags(profile: NqProfile) -> u32 {
    match profile {
        NqProfile::Rmq { flags } => flags,
        _ => 0,
    }
}

/// QuakeWorld protocol flags.
#[must_use]
pub fn qw_protocol_flags(profile: QwProfile) -> u32 {
    match profile {
        QwProfile::Wide { flags } => flags,
        QwProfile::Quakeworld => 0,
    }
}

/// Wide visible entity state: [`crate::q1::EntityState`] plus encoded
/// alpha/scale bytes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WideEntityState {
    /// Model index (wide sends the high byte under `U_MODEL2`).
    pub modelindex: u16,
    /// Frame (wide sends the high byte under `U_FRAME2`).
    pub frame: u16,
    /// Colormap.
    pub colormap: u8,
    /// Skin.
    pub skin: u8,
    /// Effects.
    pub effects: u8,
    /// Origin.
    pub origin: [f64; 3],
    /// Angles in degrees.
    pub angles: [f64; 3],
    /// Encoded alpha.
    pub alpha: u8,
    /// Encoded scale.
    pub scale: u8,
}

/// Wide entity update (`EntityUpdateT` with wide fields).
#[derive(Debug, Clone, PartialEq)]
pub struct WideEntityUpdate {
    /// Current state.
    pub state: WideEntityState,
    /// Baseline the delta is computed against.
    pub baseline: WideEntityState,
    /// Step animation (`U_STEP`).
    pub step: bool,
    /// Send lerp finish (`U_LERPFINISH` gate).
    pub sendinterval: bool,
    /// `nextthink - sv.time` in seconds.
    pub lerpfinish: f64,
}

/// Write the protocol version (`writeProtocol`).
pub fn write_wide_protocol(writer: &mut MsgWriter, version: u16, flags: u32, is_rmq: bool) -> Result<(), MsgError> {
    writer.write_long(i32::from(version))?;
    if is_rmq {
        writer.write_long(flags as i32)?;
    }
    Ok(())
}

/// Read RMQ protocol flags (`readProtocolFlags`).
pub fn read_wide_protocol_flags(reader: &mut MsgReader<'_>, is_rmq: bool) -> Result<u32, MsgError> {
    if is_rmq {
        Ok(reader.long()? as u32)
    } else {
        Ok(0)
    }
}

/// Compute wide entity-update bits.
#[must_use]
pub fn wide_entity_bits(number: u16, update: &WideEntityUpdate) -> u32 {
    let mut bits = 0;
    for axis in 0..3 {
        let miss = update.state.origin[axis] - update.baseline.origin[axis];
        if miss < -ORIGIN_EPSILON || miss > ORIGIN_EPSILON {
            bits |= protocol::U_ORIGIN1 << axis;
        }
    }
    if update.state.angles[0] != update.baseline.angles[0] {
        bits |= protocol::U_ANGLE1;
    }
    if update.state.angles[1] != update.baseline.angles[1] {
        bits |= protocol::U_ANGLE2;
    }
    if update.state.angles[2] != update.baseline.angles[2] {
        bits |= protocol::U_ANGLE3;
    }
    if update.step {
        bits |= protocol::U_STEP;
    }
    if update.baseline.colormap != update.state.colormap {
        bits |= protocol::U_COLORMAP;
    }
    if update.baseline.skin != update.state.skin {
        bits |= protocol::U_SKIN;
    }
    if update.baseline.frame != update.state.frame {
        bits |= protocol::U_FRAME;
    }
    if update.baseline.effects != update.state.effects {
        bits |= protocol::U_EFFECTS;
    }
    if update.baseline.modelindex != update.state.modelindex {
        bits |= protocol::U_MODEL;
    }
    if update.baseline.alpha != update.state.alpha {
        bits |= protocol::U_ALPHA;
    }
    if update.baseline.scale != update.state.scale {
        bits |= protocol::U_SCALE;
    }
    if (bits & protocol::U_FRAME) != 0 && (update.state.frame & 0xff00) != 0 {
        bits |= protocol::U_FRAME2;
    }
    if (bits & protocol::U_MODEL) != 0 && (update.state.modelindex & 0xff00) != 0 {
        bits |= protocol::U_MODEL2;
    }
    if update.sendinterval {
        bits |= protocol::U_LERPFINISH;
    }
    if bits >= 65536 {
        bits |= protocol::U_EXTEND1;
    }
    if bits >= 16777216 {
        bits |= protocol::U_EXTEND2;
    }
    if number >= 256 {
        bits |= protocol::U_LONGENTITY;
    }
    if bits >= 256 {
        bits |= protocol::U_MOREBITS;
    }
    bits
}

/// Write a wide entity update (`writeEntityUpdate`).
pub fn write_wide_entity_update(
    writer: &mut MsgWriter,
    number: u16,
    update: &WideEntityUpdate,
    flags: u32,
) -> Result<(), MsgError> {
    let bits = wide_entity_bits(number, update);
    writer.write_byte((bits | protocol::U_SIGNAL) as u8)?;
    if (bits & protocol::U_MOREBITS) != 0 {
        writer.write_byte((bits >> 8) as u8)?;
    }
    if (bits & protocol::U_EXTEND1) != 0 {
        writer.write_byte((bits >> 16) as u8)?;
    }
    if (bits & protocol::U_EXTEND2) != 0 {
        writer.write_byte((bits >> 24) as u8)?;
    }
    if (bits & protocol::U_LONGENTITY) != 0 {
        writer.write_short(number as i16)?;
    } else {
        writer.write_byte(number as u8)?;
    }
    if (bits & protocol::U_MODEL) != 0 {
        writer.write_byte(update.state.modelindex as u8)?;
    }
    if (bits & protocol::U_FRAME) != 0 {
        writer.write_byte(update.state.frame as u8)?;
    }
    if (bits & protocol::U_COLORMAP) != 0 {
        writer.write_byte(update.state.colormap)?;
    }
    if (bits & protocol::U_SKIN) != 0 {
        writer.write_byte(update.state.skin)?;
    }
    if (bits & protocol::U_EFFECTS) != 0 {
        writer.write_byte(update.state.effects)?;
    }
    if (bits & protocol::U_ORIGIN1) != 0 {
        writer.write_coord_flags(update.state.origin[0], flags)?;
    }
    if (bits & protocol::U_ANGLE1) != 0 {
        writer.write_angle_flags(update.state.angles[0], flags)?;
    }
    if (bits & protocol::U_ORIGIN2) != 0 {
        writer.write_coord_flags(update.state.origin[1], flags)?;
    }
    if (bits & protocol::U_ANGLE2) != 0 {
        writer.write_angle_flags(update.state.angles[1], flags)?;
    }
    if (bits & protocol::U_ORIGIN3) != 0 {
        writer.write_coord_flags(update.state.origin[2], flags)?;
    }
    if (bits & protocol::U_ANGLE3) != 0 {
        writer.write_angle_flags(update.state.angles[2], flags)?;
    }
    if (bits & protocol::U_ALPHA) != 0 {
        writer.write_byte(update.state.alpha)?;
    }
    if (bits & protocol::U_SCALE) != 0 {
        writer.write_byte(update.state.scale)?;
    }
    if (bits & protocol::U_FRAME2) != 0 {
        writer.write_byte((update.state.frame >> 8) as u8)?;
    }
    if (bits & protocol::U_MODEL2) != 0 {
        writer.write_byte((update.state.modelindex >> 8) as u8)?;
    }
    if (bits & protocol::U_LERPFINISH) != 0 {
        writer.write_byte(q_rint(update.lerpfinish * 255.0) as u8)?;
    }
    Ok(())
}

/// Extend entity bits with wide bytes (`readEntityBits`).
pub fn read_wide_entity_bits(reader: &mut MsgReader<'_>, bits_in: u32) -> Result<u32, MsgError> {
    let mut bits = bits_in;
    if (bits & protocol::U_EXTEND1) != 0 {
        bits |= u32::from(reader.byte()?) << 16;
    }
    if (bits & protocol::U_EXTEND2) != 0 {
        bits |= u32::from(reader.byte()?) << 24;
    }
    Ok(bits)
}

/// Wide entity update tail (`EntityUpdateTailT`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WideEntityTail {
    /// Alpha present and value.
    pub alpha: Option<u8>,
    /// Scale present and value.
    pub scale: Option<u8>,
    /// High frame byte.
    pub frame_high: Option<u8>,
    /// High model byte.
    pub model_high: Option<u8>,
    /// Lerp finish seconds.
    pub lerpfinish: Option<f64>,
}

/// Read a wide entity update tail (`readEntityUpdateTail`).
pub fn read_wide_entity_tail(reader: &mut MsgReader<'_>, bits: u32) -> Result<WideEntityTail, MsgError> {
    let mut tail = WideEntityTail::default();
    if (bits & protocol::U_ALPHA) != 0 {
        tail.alpha = Some(reader.byte()?);
    }
    if (bits & protocol::U_SCALE) != 0 {
        tail.scale = Some(reader.byte()?);
    }
    if (bits & protocol::U_FRAME2) != 0 {
        tail.frame_high = Some(reader.byte()?);
    }
    if (bits & protocol::U_MODEL2) != 0 {
        tail.model_high = Some(reader.byte()?);
    }
    if (bits & protocol::U_LERPFINISH) != 0 {
        tail.lerpfinish = Some(f64::from(reader.byte()?) / 255.0);
    }
    Ok(tail)
}

/// Write a wide baseline (`writeBaseline`).
pub fn write_wide_baseline(
    writer: &mut MsgWriter,
    number: u16,
    baseline: &WideEntityState,
    flags: u32,
) -> Result<(), MsgError> {
    let mut bits = 0;
    if (baseline.modelindex & 0xff00) != 0 {
        bits |= protocol::B_LARGEMODEL;
    }
    if (baseline.frame & 0xff00) != 0 {
        bits |= protocol::B_LARGEFRAME;
    }
    if baseline.alpha != protocol::ENTALPHA_DEFAULT {
        bits |= protocol::B_ALPHA;
    }
    if baseline.scale != protocol::ENTSCALE_DEFAULT {
        bits |= protocol::B_SCALE;
    }
    if bits != 0 {
        writer.write_byte(protocol::SVC_SPAWNBASELINE2)?;
    } else {
        writer.write_byte(protocol::Svc::Spawnbaseline as u8)?;
    }
    writer.write_short(number as i16)?;
    if bits != 0 {
        writer.write_byte(bits as u8)?;
    }
    if (bits & protocol::B_LARGEMODEL) != 0 {
        writer.write_short(baseline.modelindex as i16)?;
    } else {
        writer.write_byte(baseline.modelindex as u8)?;
    }
    if (bits & protocol::B_LARGEFRAME) != 0 {
        writer.write_short(baseline.frame as i16)?;
    } else {
        writer.write_byte(baseline.frame as u8)?;
    }
    writer.write_byte(baseline.colormap)?;
    writer.write_byte(baseline.skin)?;
    for axis in 0..3 {
        writer.write_coord_flags(baseline.origin[axis], flags)?;
        writer.write_angle_flags(baseline.angles[axis], flags)?;
    }
    if (bits & protocol::B_ALPHA) != 0 {
        writer.write_byte(baseline.alpha)?;
    }
    if (bits & protocol::B_SCALE) != 0 {
        writer.write_byte(baseline.scale)?;
    }
    Ok(())
}

/// Read a wide baseline (`readBaseline`).
pub fn read_wide_baseline(reader: &mut MsgReader<'_>, version: u8, flags: u32) -> Result<WideEntityState, MsgError> {
    let bits = if version == 2 { u32::from(reader.byte()?) } else { 0 };
    let mut baseline = WideEntityState::default();
    baseline.modelindex = if (bits & protocol::B_LARGEMODEL) != 0 {
        reader.short()? as u16
    } else {
        u16::from(reader.byte()?)
    };
    baseline.frame = if (bits & protocol::B_LARGEFRAME) != 0 {
        reader.short()? as u16
    } else {
        u16::from(reader.byte()?)
    };
    baseline.colormap = reader.byte()?;
    baseline.skin = reader.byte()?;
    for axis in 0..3 {
        baseline.origin[axis] = reader.coord_flags(flags)?;
        baseline.angles[axis] = reader.angle_flags(flags)?;
    }
    baseline.alpha = if (bits & protocol::B_ALPHA) != 0 {
        reader.byte()?
    } else {
        protocol::ENTALPHA_DEFAULT
    };
    baseline.scale = if (bits & protocol::B_SCALE) != 0 {
        reader.byte()?
    } else {
        protocol::ENTSCALE_DEFAULT
    };
    Ok(baseline)
}

/// Write a wide static (`writeStatic`).
pub fn write_wide_static(
    writer: &mut MsgWriter,
    state: &WideEntityState,
    flags: u32,
    is_rmq: bool,
) -> Result<bool, MsgError> {
    let mut bits = 0;
    if (state.modelindex & 0xff00) != 0 {
        bits |= protocol::B_LARGEMODEL;
    }
    if (state.frame & 0xff00) != 0 {
        bits |= protocol::B_LARGEFRAME;
    }
    if state.alpha != protocol::ENTALPHA_DEFAULT {
        bits |= protocol::B_ALPHA;
    }
    if is_rmq && state.scale != protocol::ENTSCALE_DEFAULT {
        bits |= protocol::B_SCALE;
    }
    if bits != 0 {
        writer.write_byte(protocol::SVC_SPAWNSTATIC2)?;
        writer.write_byte(bits as u8)?;
    } else {
        writer.write_byte(protocol::Svc::Spawnstatic as u8)?;
    }
    if (bits & protocol::B_LARGEMODEL) != 0 {
        writer.write_short(state.modelindex as i16)?;
    } else {
        writer.write_byte(state.modelindex as u8)?;
    }
    if (bits & protocol::B_LARGEFRAME) != 0 {
        writer.write_short(state.frame as i16)?;
    } else {
        writer.write_byte(state.frame as u8)?;
    }
    writer.write_byte(state.colormap)?;
    writer.write_byte(state.skin)?;
    for axis in 0..3 {
        writer.write_coord_flags(state.origin[axis], flags)?;
        writer.write_angle_flags(state.angles[axis], flags)?;
    }
    if (bits & protocol::B_ALPHA) != 0 {
        writer.write_byte(state.alpha)?;
    }
    if (bits & protocol::B_SCALE) != 0 {
        writer.write_byte(state.scale)?;
    }
    Ok(true)
}

/// Write a wide static sound (`writeStaticSound`).
pub fn write_wide_static_sound(
    writer: &mut MsgWriter,
    origin: [f64; 3],
    sound_num: u16,
    volume: f64,
    attenuation: f64,
    flags: u32,
) -> Result<bool, MsgError> {
    let large = sound_num > 255;
    if large {
        writer.write_byte(protocol::SVC_SPAWNSTATICSOUND2)?;
    } else {
        writer.write_byte(protocol::Svc::Spawnstaticsound as u8)?;
    }
    for axis in 0..3 {
        writer.write_coord_flags(origin[axis], flags)?;
    }
    if large {
        writer.write_short(sound_num as i16)?;
    } else {
        writer.write_byte(sound_num as u8)?;
    }
    writer.write_byte((volume * 255.0) as u8)?;
    writer.write_byte((attenuation * 64.0) as u8)?;
    Ok(true)
}

/// Read a wide static sound index (`readStaticSoundIndex`).
pub fn read_wide_static_sound_index(reader: &mut MsgReader<'_>, version: u8) -> Result<u16, MsgError> {
    if version == 2 {
        Ok(reader.short()? as u16)
    } else {
        Ok(u16::from(reader.byte()?))
    }
}

/// Wide sound message: [`crate::q1::SoundMessage`] with wide entity/index ranges.
#[derive(Debug, Clone, PartialEq)]
pub struct WideSoundMessage {
    /// Entity number.
    pub ent: u16,
    /// Channel.
    pub channel: u8,
    /// Sound index.
    pub sound_num: u16,
    /// Volume byte.
    pub volume: u8,
    /// Attenuation.
    pub attenuation: f64,
    /// Origin.
    pub origin: [f64; 3],
}

/// Write a wide sound (`writeSound`).
pub fn write_wide_sound(writer: &mut MsgWriter, sound: &WideSoundMessage, flags: u32) -> Result<bool, MsgError> {
    let mut field_mask = 0;
    if i32::from(sound.volume) != protocol::DEFAULT_SOUND_PACKET_VOLUME {
        field_mask |= protocol::SND_VOLUME;
    }
    if sound.attenuation != protocol::DEFAULT_SOUND_PACKET_ATTENUATION {
        field_mask |= protocol::SND_ATTENUATION;
    }
    if sound.ent >= 8192 {
        field_mask |= protocol::SND_LARGEENTITY;
    }
    if sound.sound_num >= 256 || sound.channel >= 8 {
        field_mask |= protocol::SND_LARGESOUND;
    }
    writer.write_byte(protocol::Svc::Sound as u8)?;
    writer.write_byte(field_mask as u8)?;
    if (field_mask & protocol::SND_VOLUME) != 0 {
        writer.write_byte(sound.volume)?;
    }
    if (field_mask & protocol::SND_ATTENUATION) != 0 {
        writer.write_byte((sound.attenuation * 64.0) as u8)?;
    }
    if (field_mask & protocol::SND_LARGEENTITY) != 0 {
        writer.write_short(sound.ent as i16)?;
        writer.write_byte(sound.channel)?;
    } else {
        writer.write_short(((u32::from(sound.ent) << 3) | u32::from(sound.channel)) as i16)?;
    }
    if (field_mask & protocol::SND_LARGESOUND) != 0 {
        writer.write_short(sound.sound_num as i16)?;
    } else {
        writer.write_byte(sound.sound_num as u8)?;
    }
    for axis in 0..3 {
        writer.write_coord_flags(sound.origin[axis], flags)?;
    }
    Ok(true)
}

/// Wide sound header (`SoundHeaderT`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WideSoundHeader {
    /// Entity number.
    pub ent: u16,
    /// Channel.
    pub channel: u8,
    /// Sound index.
    pub sound_num: u16,
}

/// Read a wide sound header (`readSoundHeader`).
pub fn read_wide_sound_header(reader: &mut MsgReader<'_>, field_mask: u32) -> Result<WideSoundHeader, MsgError> {
    let mut header = WideSoundHeader::default();
    if (field_mask & protocol::SND_LARGEENTITY) != 0 {
        header.ent = reader.short()? as u16;
        header.channel = reader.byte()?;
    } else {
        let channel = reader.short()? as u16;
        header.ent = channel >> 3;
        header.channel = (channel & 7) as u8;
    }
    header.sound_num = if (field_mask & protocol::SND_LARGESOUND) != 0 {
        reader.short()? as u16
    } else {
        u16::from(reader.byte()?)
    };
    Ok(header)
}

/// Wide client data: [`crate::q1::ClientData`] with 16-bit counters.
#[derive(Debug, Clone, PartialEq)]
pub struct WideClientData {
    /// View height.
    pub viewheight: i8,
    /// Ideal pitch.
    pub idealpitch: i8,
    /// Punch angles.
    pub punchangle: [i8; 3],
    /// Velocity (sent divided by 16).
    pub velocity: [i16; 3],
    /// Item bits.
    pub items: i32,
    /// On ground.
    pub onground: bool,
    /// In water.
    pub inwater: bool,
    /// Weapon frame.
    pub weaponframe: u16,
    /// Armor value.
    pub armorvalue: u16,
    /// Weapon model index.
    pub weaponmodelindex: u16,
    /// Health.
    pub health: i16,
    /// Current ammo.
    pub currentammo: u16,
    /// Shells.
    pub ammo_shells: u16,
    /// Nails.
    pub ammo_nails: u16,
    /// Rockets.
    pub ammo_rockets: u16,
    /// Cells.
    pub ammo_cells: u16,
    /// Raw weapon byte on the wire.
    pub weapon: u8,
    /// Weapon alpha (client entity alpha).
    pub alpha: u8,
    /// Standard Quake weapon rule.
    pub standard_quake: bool,
}

/// Compute wide clientdata bits.
#[must_use]
pub fn wide_clientdata_bits(data: &WideClientData) -> u32 {
    let mut bits = 0;
    if data.viewheight != protocol::DEFAULT_VIEWHEIGHT as i8 {
        bits |= protocol::SU_VIEWHEIGHT;
    }
    if data.idealpitch != 0 {
        bits |= protocol::SU_IDEALPITCH;
    }
    bits |= protocol::SU_ITEMS;
    if data.onground {
        bits |= protocol::SU_ONGROUND;
    }
    if data.inwater {
        bits |= protocol::SU_INWATER;
    }
    for axis in 0..3 {
        if data.punchangle[axis] != 0 {
            bits |= protocol::SU_PUNCH1 << axis;
        }
        if data.velocity[axis] != 0 {
            bits |= protocol::SU_VELOCITY1 << axis;
        }
    }
    if data.weaponframe != 0 {
        bits |= protocol::SU_WEAPONFRAME;
    }
    if data.armorvalue != 0 {
        bits |= protocol::SU_ARMOR;
    }
    bits |= protocol::SU_WEAPON;
    if (bits & protocol::SU_WEAPON) != 0 && (data.weaponmodelindex & 0xff00) != 0 {
        bits |= protocol::SU_WEAPON2;
    }
    if (data.armorvalue & 0xff00) != 0 {
        bits |= protocol::SU_ARMOR2;
    }
    if (data.currentammo & 0xff00) != 0 {
        bits |= protocol::SU_AMMO2;
    }
    if (data.ammo_shells & 0xff00) != 0 {
        bits |= protocol::SU_SHELLS2;
    }
    if (data.ammo_nails & 0xff00) != 0 {
        bits |= protocol::SU_NAILS2;
    }
    if (data.ammo_rockets & 0xff00) != 0 {
        bits |= protocol::SU_ROCKETS2;
    }
    if (data.ammo_cells & 0xff00) != 0 {
        bits |= protocol::SU_CELLS2;
    }
    if (bits & protocol::SU_WEAPONFRAME) != 0 && (data.weaponframe & 0xff00) != 0 {
        bits |= protocol::SU_WEAPONFRAME2;
    }
    if (bits & protocol::SU_WEAPON) != 0 && data.alpha != protocol::ENTALPHA_DEFAULT {
        bits |= protocol::SU_WEAPONALPHA;
    }
    if bits >= 65536 {
        bits |= protocol::SU_EXTEND1;
    }
    if bits >= 16777216 {
        bits |= protocol::SU_EXTEND2;
    }
    bits
}

/// Write wide clientdata (`writeClientdata`).
pub fn write_wide_clientdata(writer: &mut MsgWriter, data: &WideClientData) -> Result<(), MsgError> {
    let bits = wide_clientdata_bits(data);
    writer.write_byte(protocol::Svc::Clientdata as u8)?;
    writer.write_short(bits as i16)?;
    if (bits & protocol::SU_EXTEND1) != 0 {
        writer.write_byte((bits >> 16) as u8)?;
    }
    if (bits & protocol::SU_EXTEND2) != 0 {
        writer.write_byte((bits >> 24) as u8)?;
    }
    if (bits & protocol::SU_VIEWHEIGHT) != 0 {
        writer.write_char(data.viewheight)?;
    }
    if (bits & protocol::SU_IDEALPITCH) != 0 {
        writer.write_char(data.idealpitch)?;
    }
    for axis in 0..3 {
        if (bits & (protocol::SU_PUNCH1 << axis)) != 0 {
            writer.write_char(data.punchangle[axis])?;
        }
        if (bits & (protocol::SU_VELOCITY1 << axis)) != 0 {
            writer.write_char((data.velocity[axis] / 16) as i8)?;
        }
    }
    writer.write_long(data.items)?;
    if (bits & protocol::SU_WEAPONFRAME) != 0 {
        writer.write_byte(data.weaponframe as u8)?;
    }
    if (bits & protocol::SU_ARMOR) != 0 {
        writer.write_byte(data.armorvalue as u8)?;
    }
    if (bits & protocol::SU_WEAPON) != 0 {
        writer.write_byte(data.weaponmodelindex as u8)?;
    }
    writer.write_short(data.health)?;
    writer.write_byte(data.currentammo as u8)?;
    writer.write_byte(data.ammo_shells as u8)?;
    writer.write_byte(data.ammo_nails as u8)?;
    writer.write_byte(data.ammo_rockets as u8)?;
    writer.write_byte(data.ammo_cells as u8)?;
    if data.standard_quake {
        writer.write_byte(data.weapon)?;
    } else {
        let mut weapon = 0;
        for index in 0..32 {
            if (u32::from(data.weapon)) & (1 << index) != 0 {
                weapon = index;
                break;
            }
        }
        writer.write_byte(weapon as u8)?;
    }
    if (bits & protocol::SU_WEAPON2) != 0 {
        writer.write_byte((data.weaponmodelindex >> 8) as u8)?;
    }
    if (bits & protocol::SU_ARMOR2) != 0 {
        writer.write_byte((data.armorvalue >> 8) as u8)?;
    }
    if (bits & protocol::SU_AMMO2) != 0 {
        writer.write_byte((data.currentammo >> 8) as u8)?;
    }
    if (bits & protocol::SU_SHELLS2) != 0 {
        writer.write_byte((data.ammo_shells >> 8) as u8)?;
    }
    if (bits & protocol::SU_NAILS2) != 0 {
        writer.write_byte((data.ammo_nails >> 8) as u8)?;
    }
    if (bits & protocol::SU_ROCKETS2) != 0 {
        writer.write_byte((data.ammo_rockets >> 8) as u8)?;
    }
    if (bits & protocol::SU_CELLS2) != 0 {
        writer.write_byte((data.ammo_cells >> 8) as u8)?;
    }
    if (bits & protocol::SU_WEAPONFRAME2) != 0 {
        writer.write_byte((data.weaponframe >> 8) as u8)?;
    }
    if (bits & protocol::SU_WEAPONALPHA) != 0 {
        writer.write_byte(data.alpha)?;
    }
    Ok(())
}

/// Wide clientdata tail (`ClientdataTailT`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WideClientdataTail {
    /// High weapon byte.
    pub weapon_high: u8,
    /// High armor byte.
    pub armor_high: u8,
    /// High ammo byte.
    pub ammo_high: u8,
    /// High shells byte.
    pub shells_high: u8,
    /// High nails byte.
    pub nails_high: u8,
    /// High rockets byte.
    pub rockets_high: u8,
    /// High cells byte.
    pub cells_high: u8,
    /// High weaponframe byte.
    pub weaponframe_high: u8,
    /// Weapon alpha byte.
    pub weaponalpha: u8,
}

/// Read wide clientdata bits (`readClientdataBits`).
pub fn read_wide_clientdata_bits(reader: &mut MsgReader<'_>) -> Result<u32, MsgError> {
    let mut bits = reader.short()? as u16 as u32;
    if (bits & protocol::SU_EXTEND1) != 0 {
        bits |= u32::from(reader.byte()?) << 16;
    }
    if (bits & protocol::SU_EXTEND2) != 0 {
        bits |= u32::from(reader.byte()?) << 24;
    }
    Ok(bits)
}

/// Read a wide clientdata tail (`readClientdataTail`).
pub fn read_wide_clientdata_tail(reader: &mut MsgReader<'_>, bits: u32) -> Result<WideClientdataTail, MsgError> {
    let mut tail = WideClientdataTail {
        weaponalpha: protocol::ENTALPHA_DEFAULT,
        ..WideClientdataTail::default()
    };
    if (bits & protocol::SU_WEAPON2) != 0 {
        tail.weapon_high = reader.byte()?;
    }
    if (bits & protocol::SU_ARMOR2) != 0 {
        tail.armor_high = reader.byte()?;
    }
    if (bits & protocol::SU_AMMO2) != 0 {
        tail.ammo_high = reader.byte()?;
    }
    if (bits & protocol::SU_SHELLS2) != 0 {
        tail.shells_high = reader.byte()?;
    }
    if (bits & protocol::SU_NAILS2) != 0 {
        tail.nails_high = reader.byte()?;
    }
    if (bits & protocol::SU_ROCKETS2) != 0 {
        tail.rockets_high = reader.byte()?;
    }
    if (bits & protocol::SU_CELLS2) != 0 {
        tail.cells_high = reader.byte()?;
    }
    if (bits & protocol::SU_WEAPONFRAME2) != 0 {
        tail.weaponframe_high = reader.byte()?;
    }
    if (bits & protocol::SU_WEAPONALPHA) != 0 {
        tail.weaponalpha = reader.byte()?;
    }
    Ok(tail)
}

/// QuakeWorld wide entity state: [`crate::qw::QwEntityState`] with 16-bit
/// model/frame indexes, alpha/scale bytes, and wide entity numbers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QwWideEntityState {
    /// Entity number.
    pub number: u32,
    /// Origin.
    pub origin: [f64; 3],
    /// Angles in degrees.
    pub angles: [f64; 3],
    /// Model index.
    pub modelindex: u16,
    /// Frame.
    pub frame: u16,
    /// Colormap.
    pub colormap: u8,
    /// Skin.
    pub skinnum: u8,
    /// Effects.
    pub effects: u8,
    /// Encoded alpha.
    pub alpha: u8,
    /// Encoded scale.
    pub scale: u8,
    /// Solid for prediction (`U_SOLID`).
    pub solid: bool,
}

fn write_qw29_entity_header(writer: &mut MsgWriter, entnum: u32, bits_in: u32, ext: u32) -> Result<(), MsgError> {
    let mut bits = bits_in;
    if ext != 0 {
        bits |= U_EXTEND;
    }
    if (bits & 511) != 0 {
        bits |= qw_protocol::U_MOREBITS;
    }
    writer.write_word(((entnum & 511) | (bits & !511)) as u16)?;
    if (bits & qw_protocol::U_MOREBITS) != 0 {
        writer.write_byte((bits & 255) as u8)?;
    }
    if (bits & U_EXTEND) != 0 {
        writer.write_byte(ext as u8)?;
    }
    if (ext & U_ENTITY2) != 0 {
        writer.write_byte(((entnum >> 9) & 255) as u8)?;
    }
    Ok(())
}

/// Write a QuakeWorld wide delta entity (`writeDeltaEntity`).
///
/// Returns `false` when the entity number exceeds the wide limit.
pub fn write_qw29_delta_entity(
    writer: &mut MsgWriter,
    from: &QwWideEntityState,
    to: &QwWideEntityState,
    force: bool,
    flags: u32,
) -> Result<bool, MsgError> {
    let mut bits = 0;
    let mut ext = 0;
    for axis in 0..3 {
        let miss = to.origin[axis] - from.origin[axis];
        if miss < -ORIGIN_EPSILON || miss > ORIGIN_EPSILON {
            bits |= qw_protocol::U_ORIGIN1 << axis;
        }
    }
    if to.angles[0] != from.angles[0] {
        bits |= qw_protocol::U_ANGLE1;
    }
    if to.angles[1] != from.angles[1] {
        bits |= qw_protocol::U_ANGLE2;
    }
    if to.angles[2] != from.angles[2] {
        bits |= qw_protocol::U_ANGLE3;
    }
    if to.colormap != from.colormap {
        bits |= qw_protocol::U_COLORMAP;
    }
    if to.skinnum != from.skinnum {
        bits |= qw_protocol::U_SKIN;
    }
    if to.frame != from.frame {
        bits |= qw_protocol::U_FRAME;
    }
    if to.effects != from.effects {
        bits |= qw_protocol::U_EFFECTS;
    }
    if to.modelindex != from.modelindex {
        bits |= qw_protocol::U_MODEL;
    }
    if (bits & qw_protocol::U_MODEL) != 0 && (to.modelindex & 0xff00) != 0 {
        ext |= U_MODEL2;
    }
    if (bits & qw_protocol::U_FRAME) != 0 && (to.frame & 0xff00) != 0 {
        ext |= U_FRAME2;
    }
    if to.alpha != from.alpha {
        ext |= U_ALPHA;
    }
    if to.scale != from.scale {
        ext |= U_SCALE;
    }
    if to.solid {
        bits |= qw_protocol::U_SOLID;
    }
    if to.number >= QW29_MAX_ENTITY_NUMBER {
        return Ok(false);
    }
    if bits == 0 && ext == 0 && !force {
        return Ok(true);
    }
    if to.number >= 512 {
        ext |= U_ENTITY2;
    }
    if (bits & qw_protocol::U_REMOVE) != 0 {
        return Err(MsgError::BadSize(to.number as usize));
    }
    write_qw29_entity_header(writer, to.number, bits, ext)?;
    if (bits & qw_protocol::U_MODEL) != 0 {
        writer.write_byte((to.modelindex & 255) as u8)?;
    }
    if (bits & qw_protocol::U_FRAME) != 0 {
        writer.write_byte((to.frame & 255) as u8)?;
    }
    if (bits & qw_protocol::U_COLORMAP) != 0 {
        writer.write_byte(to.colormap)?;
    }
    if (bits & qw_protocol::U_SKIN) != 0 {
        writer.write_byte(to.skinnum)?;
    }
    if (bits & qw_protocol::U_EFFECTS) != 0 {
        writer.write_byte(to.effects)?;
    }
    if (bits & qw_protocol::U_ORIGIN1) != 0 {
        writer.write_coord_flags(to.origin[0], flags)?;
    }
    if (bits & qw_protocol::U_ANGLE1) != 0 {
        writer.write_angle_flags(to.angles[0], flags)?;
    }
    if (bits & qw_protocol::U_ORIGIN2) != 0 {
        writer.write_coord_flags(to.origin[1], flags)?;
    }
    if (bits & qw_protocol::U_ANGLE2) != 0 {
        writer.write_angle_flags(to.angles[1], flags)?;
    }
    if (bits & qw_protocol::U_ORIGIN3) != 0 {
        writer.write_coord_flags(to.origin[2], flags)?;
    }
    if (bits & qw_protocol::U_ANGLE3) != 0 {
        writer.write_angle_flags(to.angles[2], flags)?;
    }
    if (ext & U_MODEL2) != 0 {
        writer.write_byte(((to.modelindex >> 8) & 255) as u8)?;
    }
    if (ext & U_FRAME2) != 0 {
        writer.write_byte(((to.frame >> 8) & 255) as u8)?;
    }
    if (ext & U_ALPHA) != 0 {
        writer.write_byte(to.alpha)?;
    }
    if (ext & U_SCALE) != 0 {
        writer.write_byte(to.scale)?;
    }
    Ok(true)
}

/// Write a QuakeWorld wide entity removal (`writeRemoveEntity`).
pub fn write_qw29_remove_entity(writer: &mut MsgWriter, entnum: u32) -> Result<(), MsgError> {
    write_qw29_entity_header(
        writer,
        entnum,
        qw_protocol::U_REMOVE,
        if entnum >= 512 { U_ENTITY2 } else { 0 },
    )
}

/// Write the packet-entities terminator (`writePacketEntitiesEnd`).
pub fn write_qw29_packet_entities_end(writer: &mut MsgWriter) -> Result<(), MsgError> {
    writer.write_short(0)
}

/// Decoded QuakeWorld wide entity header (`QwEntityWordT`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QwWideEntityWord {
    /// Update bits.
    pub bits: u32,
    /// Extend byte.
    pub ext: u32,
    /// Entity number.
    pub number: u32,
    /// Removal flag.
    pub remove: bool,
}

/// Read a QuakeWorld wide entity header (`readDeltaEntityHeader`).
pub fn read_qw29_entity_header(reader: &mut MsgReader<'_>, word: u16) -> Result<QwWideEntityWord, MsgError> {
    let mut number = u32::from(word & 511);
    let mut bits = u32::from(word) & !511;
    if (bits & qw_protocol::U_MOREBITS) != 0 {
        bits |= u32::from(reader.byte()?);
    }
    let mut ext = 0;
    if (bits & U_EXTEND) != 0 {
        ext = u32::from(reader.byte()?);
        if (ext & U_ENTITY2) != 0 {
            number |= u32::from(reader.byte()?) << 9;
        }
    }
    Ok(QwWideEntityWord {
        remove: (bits & qw_protocol::U_REMOVE) != 0,
        bits,
        ext,
        number,
    })
}

/// Read a QuakeWorld wide delta entity (`readDeltaEntity`).
pub fn read_qw29_delta_entity(
    reader: &mut MsgReader<'_>,
    from: &QwWideEntityState,
    header: &QwWideEntityWord,
    flags: u32,
) -> Result<QwWideEntityState, MsgError> {
    let mut to = from.clone();
    to.number = header.number;
    let bits = header.bits;
    let ext = header.ext;
    to.solid = (bits & qw_protocol::U_SOLID) != 0;
    if (bits & qw_protocol::U_MODEL) != 0 {
        to.modelindex = u16::from(reader.byte()?);
    }
    if (bits & qw_protocol::U_FRAME) != 0 {
        to.frame = u16::from(reader.byte()?);
    }
    if (bits & qw_protocol::U_COLORMAP) != 0 {
        to.colormap = reader.byte()?;
    }
    if (bits & qw_protocol::U_SKIN) != 0 {
        to.skinnum = reader.byte()?;
    }
    if (bits & qw_protocol::U_EFFECTS) != 0 {
        to.effects = reader.byte()?;
    }
    if (bits & qw_protocol::U_ORIGIN1) != 0 {
        to.origin[0] = reader.coord_flags(flags)?;
    }
    if (bits & qw_protocol::U_ANGLE1) != 0 {
        to.angles[0] = reader.angle_flags(flags)?;
    }
    if (bits & qw_protocol::U_ORIGIN2) != 0 {
        to.origin[1] = reader.coord_flags(flags)?;
    }
    if (bits & qw_protocol::U_ANGLE2) != 0 {
        to.angles[1] = reader.angle_flags(flags)?;
    }
    if (bits & qw_protocol::U_ORIGIN3) != 0 {
        to.origin[2] = reader.coord_flags(flags)?;
    }
    if (bits & qw_protocol::U_ANGLE3) != 0 {
        to.angles[2] = reader.angle_flags(flags)?;
    }
    if (ext & U_MODEL2) != 0 {
        to.modelindex |= u16::from(reader.byte()?) << 8;
    }
    if (ext & U_FRAME2) != 0 {
        to.frame |= u16::from(reader.byte()?) << 8;
    }
    if (ext & U_ALPHA) != 0 {
        to.alpha = reader.byte()?;
    }
    if (ext & U_SCALE) != 0 {
        to.scale = reader.byte()?;
    }
    Ok(to)
}

/// Write a QuakeWorld wide baseline (`writeQwBaseline`).
pub fn write_qw29_baseline(writer: &mut MsgWriter, state: &QwWideEntityState, flags: u32) -> Result<(), MsgError> {
    writer.write_short(state.modelindex as i16)?;
    writer.write_short(state.frame as i16)?;
    writer.write_byte(state.colormap)?;
    writer.write_byte(state.skinnum)?;
    writer.write_byte(state.alpha)?;
    writer.write_byte(state.scale)?;
    for axis in 0..3 {
        writer.write_coord_flags(state.origin[axis], flags)?;
        writer.write_angle_flags(state.angles[axis], flags)?;
    }
    Ok(())
}

/// Read a QuakeWorld wide baseline (`readQwBaseline`).
pub fn read_qw29_baseline(reader: &mut MsgReader<'_>, flags: u32) -> Result<QwWideEntityState, MsgError> {
    let mut state = QwWideEntityState::default();
    state.modelindex = reader.short()? as u16;
    state.frame = reader.short()? as u16;
    state.colormap = reader.byte()?;
    state.skinnum = reader.byte()?;
    state.alpha = reader.byte()?;
    state.scale = reader.byte()?;
    for axis in 0..3 {
        state.origin[axis] = reader.coord_flags(flags)?;
        state.angles[axis] = reader.angle_flags(flags)?;
    }
    Ok(state)
}

/// Write a wide model index (`writeModelIndex`).
pub fn write_qw29_model_index(writer: &mut MsgWriter, index: u16) -> Result<(), MsgError> {
    writer.write_short(index as i16)
}

/// Read a wide model index (`readModelIndex`).
pub fn read_qw29_model_index(reader: &mut MsgReader<'_>) -> Result<u16, MsgError> {
    Ok(reader.short()? as u16)
}

/// Write a wide sound index (`writeSoundIndex`).
pub fn write_qw29_sound_index(writer: &mut MsgWriter, index: u16) -> Result<(), MsgError> {
    writer.write_short(index as i16)
}

/// Read a wide sound index (`readSoundIndex`).
pub fn read_qw29_sound_index(reader: &mut MsgReader<'_>) -> Result<u16, MsgError> {
    Ok(reader.short()? as u16)
}

/// Write a wide precache count (`writePrecacheCount`).
pub fn write_qw29_precache_count(writer: &mut MsgWriter, count: u16) -> Result<(), MsgError> {
    writer.write_short(count as i16)
}

/// Read a wide precache count (`readPrecacheCount`).
pub fn read_qw29_precache_count(reader: &mut MsgReader<'_>) -> Result<u16, MsgError> {
    Ok(reader.short()? as u16)
}

/// Write the QuakeWorld wide protocol (`writeProtocol`).
pub fn write_qw29_protocol(writer: &mut MsgWriter, flags: u32) -> Result<(), MsgError> {
    writer.write_long(i32::from(PROTOCOL_QW_WIDE))?;
    writer.write_long(flags as i32)
}

/// Read the QuakeWorld wide protocol flags (`readProtocolFlags`).
pub fn read_qw29_protocol_flags(reader: &mut MsgReader<'_>) -> Result<u32, MsgError> {
    Ok(reader.long()? as u32)
}

/// Write a QuakeWorld wide static (`writeStatic`).
pub fn write_qw29_static(writer: &mut MsgWriter, state: &WideEntityState, flags: u32) -> Result<bool, MsgError> {
    // `frame` is 16-bit here, so the donor's `frame & 0xffff0000` check is vacuous.
    if state.modelindex >= QW29_MAX_PRECACHE as u16 {
        return Ok(false);
    }
    writer.write_byte(qw_protocol::Svc::Spawnstatic as u8)?;
    writer.write_short(state.modelindex as i16)?;
    writer.write_short(state.frame as i16)?;
    writer.write_byte(state.colormap)?;
    writer.write_byte(state.skin)?;
    writer.write_byte(state.alpha)?;
    writer.write_byte(state.scale)?;
    for axis in 0..3 {
        writer.write_coord_flags(state.origin[axis], flags)?;
        writer.write_angle_flags(state.angles[axis], flags)?;
    }
    Ok(true)
}

/// Write a QuakeWorld wide static sound (`writeStaticSound`).
pub fn write_qw29_static_sound(
    writer: &mut MsgWriter,
    origin: [f64; 3],
    sound_num: u32,
    volume: f64,
    attenuation: f64,
    flags: u32,
) -> Result<bool, MsgError> {
    if sound_num > 65535 {
        return Ok(false);
    }
    writer.write_byte(qw_protocol::Svc::Spawnstaticsound as u8)?;
    for axis in 0..3 {
        writer.write_coord_flags(origin[axis], flags)?;
    }
    writer.write_short(sound_num as i16)?;
    writer.write_byte((volume * 255.0) as u8)?;
    writer.write_byte((attenuation * 64.0) as u8)?;
    Ok(true)
}

/// Read a QuakeWorld wide static sound index (`readStaticSoundIndex`).
pub fn read_qw29_static_sound_index(reader: &mut MsgReader<'_>) -> Result<u16, MsgError> {
    Ok(reader.short()? as u16)
}

/// Write a QuakeWorld wide sound (`writeSound`).
pub fn write_qw29_sound(writer: &mut MsgWriter, sound: &WideSoundMessage, flags: u32) -> Result<bool, MsgError> {
    if sound.ent >= 1024 || sound.channel >= 8 || u32::from(sound.sound_num) > 65535 {
        return Ok(false);
    }
    let mut channel = (u32::from(sound.ent) << 3) | u32::from(sound.channel);
    if i32::from(sound.volume) != protocol::DEFAULT_SOUND_PACKET_VOLUME {
        channel |= qw_protocol::SND_VOLUME;
    }
    if sound.attenuation != protocol::DEFAULT_SOUND_PACKET_ATTENUATION {
        channel |= qw_protocol::SND_ATTENUATION;
    }
    writer.write_byte(qw_protocol::Svc::Sound as u8)?;
    writer.write_short(channel as i16)?;
    if (channel & qw_protocol::SND_VOLUME) != 0 {
        writer.write_byte(sound.volume)?;
    }
    if (channel & qw_protocol::SND_ATTENUATION) != 0 {
        writer.write_byte((sound.attenuation * 64.0) as u8)?;
    }
    writer.write_short(sound.sound_num as i16)?;
    for axis in 0..3 {
        writer.write_coord_flags(sound.origin[axis], flags)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide_update() -> WideEntityUpdate {
        let mut state = WideEntityState::default();
        state.modelindex = 300;
        state.frame = 512;
        state.origin = [16.0, 0.0, 0.0];
        state.alpha = 128;
        WideEntityUpdate {
            state,
            baseline: WideEntityState::default(),
            step: false,
            sendinterval: false,
            lerpfinish: 0.0,
        }
    }

    #[test]
    fn wide_entity_update_round_trips() {
        let update = wide_update();
        let mut writer = MsgWriter::new(WIDE_MAX_MSGLEN, false);
        write_wide_entity_update(&mut writer, 300, &update, 0).unwrap();
        let bytes = writer.bytes().to_vec();
        let mut reader = MsgReader::new(&bytes);
        let initial = u32::from(reader.byte().unwrap() & 127);
        let mut bits = initial;
        if (bits & protocol::U_MOREBITS) != 0 {
            bits |= u32::from(reader.byte().unwrap()) << 8;
        }
        bits = read_wide_entity_bits(&mut reader, bits).unwrap();
        assert!((bits & protocol::U_MODEL2) != 0);
        assert!((bits & protocol::U_FRAME2) != 0);
        assert!((bits & protocol::U_ALPHA) != 0);
    }

    #[test]
    fn wide_baseline_selects_version2() {
        let mut baseline = WideEntityState::default();
        baseline.modelindex = 300;
        baseline.alpha = 200;
        let mut writer = MsgWriter::new(WIDE_MAX_MSGLEN, false);
        write_wide_baseline(&mut writer, 7, &baseline, 0).unwrap();
        let bytes = writer.bytes().to_vec();
        assert_eq!(bytes[0], protocol::SVC_SPAWNBASELINE2);
        let mut reader = MsgReader::new(&bytes);
        assert_eq!(reader.byte().unwrap(), protocol::SVC_SPAWNBASELINE2);
        let number = reader.short().unwrap();
        assert_eq!(number, 7);
        let decoded = read_wide_baseline(&mut reader, 2, 0).unwrap();
        assert_eq!(decoded.modelindex, 300);
        assert_eq!(decoded.alpha, 200);
    }

    #[test]
    fn wide_sound_selects_large_sound() {
        let sound = WideSoundMessage {
            ent: 5,
            channel: 1,
            sound_num: 300,
            volume: 200,
            attenuation: 1.0,
            origin: [0.0, 0.0, 0.0],
        };
        let mut writer = MsgWriter::new(WIDE_MAX_MSGLEN, false);
        assert!(write_wide_sound(&mut writer, &sound, 0).unwrap());
        let bytes = writer.bytes().to_vec();
        let mut reader = MsgReader::new(&bytes);
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Sound as u8);
        let mask = u32::from(reader.byte().unwrap());
        assert!((mask & protocol::SND_LARGESOUND) != 0);
        assert!((mask & protocol::SND_VOLUME) != 0);
    }

    #[test]
    fn wide_clientdata_round_trips_high_bytes() {
        let data = WideClientData {
            viewheight: 22,
            idealpitch: 0,
            punchangle: [0, 0, 0],
            velocity: [0, 0, 0],
            items: 3,
            onground: true,
            inwater: false,
            weaponframe: 300,
            armorvalue: 0,
            weaponmodelindex: 512,
            health: 100,
            currentammo: 0,
            ammo_shells: 0,
            ammo_nails: 0,
            ammo_rockets: 0,
            ammo_cells: 0,
            weapon: 2,
            alpha: 0,
            standard_quake: true,
        };
        let bits = wide_clientdata_bits(&data);
        assert!((bits & protocol::SU_WEAPONFRAME2) != 0);
        assert!((bits & protocol::SU_WEAPON2) != 0);
        let mut writer = MsgWriter::new(WIDE_MAX_MSGLEN, false);
        write_wide_clientdata(&mut writer, &data).unwrap();
        assert!(!writer.bytes().is_empty());
    }

    #[test]
    fn qw29_delta_entity_round_trips() {
        let from = QwWideEntityState::default();
        let mut to = QwWideEntityState::default();
        to.number = 600;
        to.modelindex = 300;
        to.frame = 511;
        to.origin = [24.0, 0.0, 0.0];
        let mut writer = MsgWriter::new(WIDE_MAX_MSGLEN, false);
        assert!(write_qw29_delta_entity(&mut writer, &from, &to, false, QW29_DEFAULT_FLAGS).unwrap());
        let bytes = writer.bytes().to_vec();
        let mut reader = MsgReader::new(&bytes);
        let word = reader.word().unwrap();
        let header = read_qw29_entity_header(&mut reader, word).unwrap();
        assert_eq!(header.number, 600);
        assert!((header.ext & U_ENTITY2) != 0);
        assert!((header.ext & U_MODEL2) != 0);
        let decoded = read_qw29_delta_entity(&mut reader, &from, &header, QW29_DEFAULT_FLAGS).unwrap();
        assert_eq!(decoded.modelindex, 300);
        assert_eq!(decoded.origin[0], 24.0);
    }

    #[test]
    fn profiles_reject_unknown_versions() {
        assert!(net_quake_profile(15, 0).is_ok());
        assert!(net_quake_profile(999, 1 << 31).is_err());
        assert!(quake_world_profile(29, QW29_DEFAULT_FLAGS).is_ok());
        assert!(quake_world_profile(30, 0).is_err());
    }
}
