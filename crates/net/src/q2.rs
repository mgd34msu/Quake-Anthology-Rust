//! Quake II classic message codecs.
//!
//! Donor provenance: `MSG_WriteDeltaUsercmd` / `MSG_ReadDeltaUsercmd` and
//! `MSG_WriteDeltaEntity` in `src/network/q2/message.ts`, with entity
//! header bits, removal framing, spawn baselines, and serverdata from
//! `createVanillaContext` in `src/network/q2/codecs/vanilla.ts`.
//! `EntityStateT` / `UsercmdT` shapes come from `src/network/q2/state.ts`.
//!
//! Classic (protocol 34) framing plus the shared state shapes every
//! variant codec builds on; R1Q2, Q2Pro, rerelease, KEX, zpacket, and MVD
//! codecs live in [`crate::q2_variants`].

use qa_core::numeric::float_to_wrapped_i32;
use thiserror::Error;

use crate::angles::{q2_angle_to_short, q2_short_to_angle};
use crate::msg::{MsgError, MsgReader, MsgWriter};
use crate::protocol::q2 as protocol;

/// Maximum edicts (`MAX_EDICTS`).
pub const MAX_EDICTS: u16 = 1024;
/// Beam render effect flag (`RF_BEAM`).
pub const RF_BEAM: i32 = 128;
/// Classic stat slots (`MAX_STATS`).
pub const MAX_STATS: usize = 32;
/// Stored stat slots (`MAX_STATS_STORAGE`).
pub const MAX_STATS_STORAGE: usize = 64;
/// Precomputed vertex normals (`NUMVERTEXNORMALS`).
pub const NUMVERTEXNORMALS: usize = 162;

/// Error for entity numbers the wire format cannot represent.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q2CodecError {
    /// Entity number was never assigned.
    #[error("unset entity number")]
    UnsetNumber,
    /// Entity number exceeds the edict limit.
    #[error("entity number {0} exceeds MAX_EDICTS")]
    NumberTooLarge(u16),
    /// Underlying message failure.
    #[error("{0}")]
    Msg(#[from] MsgError),
    /// A frame parser met the wrong opcode where the donor raises `ERR_DROP`.
    #[error("expected opcode {expected}, found {found}")]
    UnexpectedOpcode { expected: u8, found: u8 },
    /// A direction byte fell outside the vertex-normal table.
    #[error("direction {0} outside vertex-normal table")]
    BadDirection(u8),
}

/// Quake II user command (`UsercmdT`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usercmd {
    /// Milliseconds.
    pub msec: u8,
    /// Buttons.
    pub buttons: u8,
    /// View angles as 16-bit wire shorts.
    pub angles: [i16; 3],
    /// Forward move.
    pub forwardmove: i16,
    /// Side move.
    pub sidemove: i16,
    /// Up move.
    pub upmove: i16,
    /// Impulse.
    pub impulse: u8,
    /// Light level.
    pub lightlevel: u8,
    /// Server frame the command was generated for (KEX `serverFrame`).
    pub server_frame: i32,
}

/// Quake II entity state (`EntityStateT`, all protocol fields).
///
/// The classic codec only wires the id Software subset; variant codecs
/// cover the extension tail (`morefx` through `old_frame`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EntityState {
    /// Entity number.
    pub number: u16,
    /// Origin.
    pub origin: [f64; 3],
    /// Angles in degrees.
    pub angles: [f64; 3],
    /// Old origin (beam endpoints, teleport trails).
    pub old_origin: [f64; 3],
    /// Model indices.
    pub modelindex: u16,
    /// Second model index.
    pub modelindex2: u16,
    /// Third model index.
    pub modelindex3: u16,
    /// Fourth model index.
    pub modelindex4: u16,
    /// Frame.
    pub frame: i32,
    /// Skin number.
    pub skinnum: i32,
    /// Effects.
    pub effects: i32,
    /// Render effects.
    pub renderfx: i32,
    /// Solid.
    pub solid: u32,
    /// Sound.
    pub sound: u16,
    /// Event.
    pub event: u8,
    /// Extended effects bits.
    pub morefx: i32,
    /// Entity alpha.
    pub alpha: f64,
    /// Entity scale.
    pub scale: f64,
    /// KEX instance bits.
    pub instance_bits: u8,
    /// Looping-sound volume.
    pub loop_volume: f64,
    /// Looping-sound attenuation (`-1` means none).
    pub loop_attenuation: f64,
    /// KEX owner entity.
    pub owner: u16,
    /// KEX previous frame.
    pub old_frame: u16,
}

/// Quake II player movement state (`PmoveStateT`).
#[derive(Debug, Clone, PartialEq)]
pub struct PmoveState {
    /// Movement type.
    pub pm_type: u8,
    /// Fixed-point origin (eighths).
    pub origin: [i32; 3],
    /// Fixed-point velocity (eighths).
    pub velocity: [i32; 3],
    /// Movement flags.
    pub pm_flags: i32,
    /// Movement timer.
    pub pm_time: i32,
    /// Gravity.
    pub gravity: i16,
    /// Delta angles.
    pub delta_angles: [i16; 3],
    /// View height.
    pub viewheight: i32,
    /// Float origin (rerelease/KEX).
    pub origin_f: [f32; 3],
    /// Float velocity (rerelease/KEX).
    pub velocity_f: [f32; 3],
    /// Float delta angles (KEX).
    pub delta_angles_f: [f32; 3],
    /// Whether delta angles compare as floats (KEX).
    pub delta_angle_float: bool,
}

impl Default for PmoveState {
    fn default() -> Self {
        Self {
            pm_type: 0,
            origin: [0; 3],
            velocity: [0; 3],
            pm_flags: 0,
            pm_time: 0,
            gravity: 0,
            delta_angles: [0; 3],
            viewheight: 0,
            origin_f: [0.0; 3],
            velocity_f: [0.0; 3],
            delta_angles_f: [0.0; 3],
            delta_angle_float: false,
        }
    }
}

/// Q2Pro player fog state (`Q2ProPlayerFog`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Q2ProFog {
    /// Fog color.
    pub color: [u8; 3],
    /// Fog density.
    pub density: u16,
    /// Sky factor.
    pub sky_factor: u16,
    /// Height-fog density.
    pub height_density: u16,
    /// Height-fog falloff.
    pub height_falloff: u16,
    /// Height-fog start color.
    pub height_start_color: [u8; 3],
    /// Height-fog end color.
    pub height_end_color: [u8; 3],
    /// Height-fog start distance (eighths).
    pub height_start_distance: i32,
    /// Height-fog end distance (eighths).
    pub height_end_distance: i32,
}

/// Quake II player state (`PlayerStateT`, all protocol fields).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerState {
    /// Client number (Q2Pro revision 1022+).
    pub clientnum: i32,
    /// Movement state.
    pub pmove: PmoveState,
    /// View angles in degrees.
    pub viewangles: [f64; 3],
    /// View offset.
    pub viewoffset: [f64; 3],
    /// Kick angles.
    pub kick_angles: [f64; 3],
    /// Gun angles.
    pub gunangles: [f64; 3],
    /// Gun offset.
    pub gunoffset: [f64; 3],
    /// Gun model index.
    pub gunindex: i32,
    /// Gun skin.
    pub gunskin: i32,
    /// Gun frame.
    pub gunframe: i32,
    /// Gun frame rate.
    pub gunrate: u8,
    /// Screen blend.
    pub blend: [f64; 4],
    /// Damage blend.
    pub damage_blend: [f64; 4],
    /// Field of view.
    pub fov: u8,
    /// Refresh flags.
    pub rdflags: u8,
    /// Stats (`MAX_STATS_STORAGE` slots).
    pub stats: [i16; MAX_STATS_STORAGE],
    /// KEX team id.
    pub team_id: u8,
    /// Q2Pro fog.
    pub fog: Q2ProFog,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            clientnum: 0,
            pmove: PmoveState::default(),
            viewangles: [0.0; 3],
            viewoffset: [0.0; 3],
            kick_angles: [0.0; 3],
            gunangles: [0.0; 3],
            gunoffset: [0.0; 3],
            gunindex: 0,
            gunskin: 0,
            gunframe: 0,
            gunrate: 0,
            blend: [0.0; 4],
            damage_blend: [0.0; 4],
            fov: 0,
            rdflags: 0,
            stats: [0; MAX_STATS_STORAGE],
            team_id: 0,
            fog: Q2ProFog::default(),
        }
    }
}

/// Truncate a scaled float to a wrapping `i32`.
///
/// Matches the donor's `Math.trunc(value * scale)` followed by `& 0xff`
/// / `& 0xffff` masking in `MSG_Write*`.
#[must_use]
pub fn scaled_trunc(value: f64, scale: f64) -> i32 {
    float_to_wrapped_i32((value * scale).trunc())
}

/// Convert degrees to a 16-bit wire angle (`ANGLE2SHORT`).
#[must_use]
pub fn angle_to_short(value: f64) -> u16 {
    q2_angle_to_short(value)
}

/// Convert a 16-bit wire angle to degrees (`SHORT2ANGLE`).
#[must_use]
pub fn short_to_angle(word: i16) -> f64 {
    q2_short_to_angle(word)
}

/// Server data preamble (`readServerData` / `writeServerData`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerData {
    /// Server count.
    pub servercount: i32,
    /// Attract loop.
    pub attractloop: bool,
    /// Game directory.
    pub gamedir: String,
    /// Client number.
    pub clientnum: i16,
    /// Level name.
    pub levelname: String,
}

/// Write a delta user command (`MSG_WriteDeltaUsercmd`).
pub fn write_delta_usercmd(writer: &mut MsgWriter, from: &Usercmd, cmd: &Usercmd) -> Result<(), MsgError> {
    let mut bits = 0;
    if cmd.angles[0] != from.angles[0] {
        bits |= protocol::CM_ANGLE1;
    }
    if cmd.angles[1] != from.angles[1] {
        bits |= protocol::CM_ANGLE2;
    }
    if cmd.angles[2] != from.angles[2] {
        bits |= protocol::CM_ANGLE3;
    }
    if cmd.forwardmove != from.forwardmove {
        bits |= protocol::CM_FORWARD;
    }
    if cmd.sidemove != from.sidemove {
        bits |= protocol::CM_SIDE;
    }
    if cmd.upmove != from.upmove {
        bits |= protocol::CM_UP;
    }
    if cmd.buttons != from.buttons {
        bits |= protocol::CM_BUTTONS;
    }
    if cmd.impulse != from.impulse {
        bits |= protocol::CM_IMPULSE;
    }
    writer.write_byte(bits as u8)?;
    if (bits & protocol::CM_ANGLE1) != 0 {
        writer.write_short(cmd.angles[0])?;
    }
    if (bits & protocol::CM_ANGLE2) != 0 {
        writer.write_short(cmd.angles[1])?;
    }
    if (bits & protocol::CM_ANGLE3) != 0 {
        writer.write_short(cmd.angles[2])?;
    }
    if (bits & protocol::CM_FORWARD) != 0 {
        writer.write_short(cmd.forwardmove)?;
    }
    if (bits & protocol::CM_SIDE) != 0 {
        writer.write_short(cmd.sidemove)?;
    }
    if (bits & protocol::CM_UP) != 0 {
        writer.write_short(cmd.upmove)?;
    }
    if (bits & protocol::CM_BUTTONS) != 0 {
        writer.write_byte(cmd.buttons)?;
    }
    if (bits & protocol::CM_IMPULSE) != 0 {
        writer.write_byte(cmd.impulse)?;
    }
    writer.write_byte(cmd.msec)?;
    writer.write_byte(cmd.lightlevel)
}

/// Read a delta user command (`MSG_ReadDeltaUsercmd`).
pub fn read_delta_usercmd(reader: &mut MsgReader<'_>, from: &Usercmd) -> Result<Usercmd, MsgError> {
    let mut cmd = from.clone();
    let bits = u32::from(reader.byte()?);
    if (bits & protocol::CM_ANGLE1) != 0 {
        cmd.angles[0] = reader.short()?;
    }
    if (bits & protocol::CM_ANGLE2) != 0 {
        cmd.angles[1] = reader.short()?;
    }
    if (bits & protocol::CM_ANGLE3) != 0 {
        cmd.angles[2] = reader.short()?;
    }
    if (bits & protocol::CM_FORWARD) != 0 {
        cmd.forwardmove = reader.short()?;
    }
    if (bits & protocol::CM_SIDE) != 0 {
        cmd.sidemove = reader.short()?;
    }
    if (bits & protocol::CM_UP) != 0 {
        cmd.upmove = reader.short()?;
    }
    if (bits & protocol::CM_BUTTONS) != 0 {
        cmd.buttons = reader.byte()?;
    }
    if (bits & protocol::CM_IMPULSE) != 0 {
        cmd.impulse = reader.byte()?;
    }
    cmd.msec = reader.byte()?;
    cmd.lightlevel = reader.byte()?;
    Ok(cmd)
}

/// Compute delta-entity bits for a state against its baseline.
#[must_use]
pub fn entity_bits(from: &EntityState, to: &EntityState, newentity: bool) -> u32 {
    let mut bits = 0;
    if to.number >= 256 {
        bits |= protocol::U_NUMBER16;
    }
    if to.origin[0] != from.origin[0] {
        bits |= protocol::U_ORIGIN1;
    }
    if to.origin[1] != from.origin[1] {
        bits |= protocol::U_ORIGIN2;
    }
    if to.origin[2] != from.origin[2] {
        bits |= protocol::U_ORIGIN3;
    }
    if to.angles[0] != from.angles[0] {
        bits |= protocol::U_ANGLE1;
    }
    if to.angles[1] != from.angles[1] {
        bits |= protocol::U_ANGLE2;
    }
    if to.angles[2] != from.angles[2] {
        bits |= protocol::U_ANGLE3;
    }
    if to.skinnum != from.skinnum {
        let skin = to.skinnum as u32;
        if skin < 256 {
            bits |= protocol::U_SKIN8;
        } else if skin < 0x10000 {
            bits |= protocol::U_SKIN16;
        } else {
            bits |= protocol::U_SKIN8 | protocol::U_SKIN16;
        }
    }
    if to.frame != from.frame {
        if to.frame < 256 {
            bits |= protocol::U_FRAME8;
        } else {
            bits |= protocol::U_FRAME16;
        }
    }
    if to.effects != from.effects {
        if to.effects < 256 {
            bits |= protocol::U_EFFECTS8;
        } else if to.effects < 0x8000 {
            bits |= protocol::U_EFFECTS16;
        } else {
            bits |= protocol::U_EFFECTS8 | protocol::U_EFFECTS16;
        }
    }
    if to.renderfx != from.renderfx {
        if to.renderfx < 256 {
            bits |= protocol::U_RENDERFX8;
        } else if to.renderfx < 0x8000 {
            bits |= protocol::U_RENDERFX16;
        } else {
            bits |= protocol::U_RENDERFX8 | protocol::U_RENDERFX16;
        }
    }
    if to.solid != from.solid {
        bits |= protocol::U_SOLID;
    }
    if to.event != 0 {
        bits |= protocol::U_EVENT;
    }
    if to.modelindex != from.modelindex {
        bits |= protocol::U_MODEL;
    }
    if to.modelindex2 != from.modelindex2 {
        bits |= protocol::U_MODEL2;
    }
    if to.modelindex3 != from.modelindex3 {
        bits |= protocol::U_MODEL3;
    }
    if to.modelindex4 != from.modelindex4 {
        bits |= protocol::U_MODEL4;
    }
    if to.sound != from.sound {
        bits |= protocol::U_SOUND;
    }
    if newentity || (to.renderfx & RF_BEAM) != 0 {
        bits |= protocol::U_OLDORIGIN;
    }
    bits
}

/// Write a delta entity (`MSG_WriteDeltaEntity`).
///
/// Returns `Ok(false)` (writing nothing) when no bits changed and
/// `force` is false.
pub fn write_delta_entity(
    writer: &mut MsgWriter,
    from: &EntityState,
    to: &EntityState,
    force: bool,
    newentity: bool,
) -> Result<bool, Q2CodecError> {
    if to.number == 0 {
        return Err(Q2CodecError::UnsetNumber);
    }
    if to.number >= MAX_EDICTS {
        return Err(Q2CodecError::NumberTooLarge(to.number));
    }
    let mut bits = entity_bits(from, to, newentity);
    if bits == 0 && !force {
        return Ok(false);
    }
    if (bits & 0xff00_0000) != 0 {
        bits |= protocol::U_MOREBITS3 | protocol::U_MOREBITS2 | protocol::U_MOREBITS1;
    } else if (bits & 0x00ff_0000) != 0 {
        bits |= protocol::U_MOREBITS2 | protocol::U_MOREBITS1;
    } else if (bits & 0x0000_ff00) != 0 {
        bits |= protocol::U_MOREBITS1;
    }
    writer.write_byte((bits & 255) as u8)?;
    if (bits & 0xff00_0000) != 0 {
        writer.write_byte(((bits >> 8) & 255) as u8)?;
        writer.write_byte(((bits >> 16) & 255) as u8)?;
        writer.write_byte(((bits >> 24) & 255) as u8)?;
    } else if (bits & 0x00ff_0000) != 0 {
        writer.write_byte(((bits >> 8) & 255) as u8)?;
        writer.write_byte(((bits >> 16) & 255) as u8)?;
    } else if (bits & 0x0000_ff00) != 0 {
        writer.write_byte(((bits >> 8) & 255) as u8)?;
    }
    if (bits & protocol::U_NUMBER16) != 0 {
        writer.write_short(to.number as i16)?;
    } else {
        writer.write_byte(to.number as u8)?;
    }
    if (bits & protocol::U_MODEL) != 0 {
        writer.write_byte(to.modelindex as u8)?;
    }
    if (bits & protocol::U_MODEL2) != 0 {
        writer.write_byte(to.modelindex2 as u8)?;
    }
    if (bits & protocol::U_MODEL3) != 0 {
        writer.write_byte(to.modelindex3 as u8)?;
    }
    if (bits & protocol::U_MODEL4) != 0 {
        writer.write_byte(to.modelindex4 as u8)?;
    }
    if (bits & protocol::U_FRAME8) != 0 {
        writer.write_byte(to.frame as u8)?;
    }
    if (bits & protocol::U_FRAME16) != 0 {
        writer.write_short(to.frame as i16)?;
    }
    if (bits & protocol::U_SKIN8) != 0 && (bits & protocol::U_SKIN16) != 0 {
        writer.write_long(to.skinnum)?;
    } else if (bits & protocol::U_SKIN8) != 0 {
        writer.write_byte(to.skinnum as u8)?;
    } else if (bits & protocol::U_SKIN16) != 0 {
        writer.write_short(to.skinnum as i16)?;
    }
    if (bits & (protocol::U_EFFECTS8 | protocol::U_EFFECTS16)) == (protocol::U_EFFECTS8 | protocol::U_EFFECTS16) {
        writer.write_long(to.effects)?;
    } else if (bits & protocol::U_EFFECTS8) != 0 {
        writer.write_byte(to.effects as u8)?;
    } else if (bits & protocol::U_EFFECTS16) != 0 {
        writer.write_short(to.effects as i16)?;
    }
    if (bits & (protocol::U_RENDERFX8 | protocol::U_RENDERFX16)) == (protocol::U_RENDERFX8 | protocol::U_RENDERFX16) {
        writer.write_long(to.renderfx)?;
    } else if (bits & protocol::U_RENDERFX8) != 0 {
        writer.write_byte(to.renderfx as u8)?;
    } else if (bits & protocol::U_RENDERFX16) != 0 {
        writer.write_short(to.renderfx as i16)?;
    }
    if (bits & protocol::U_ORIGIN1) != 0 {
        writer.write_q2_coord(to.origin[0])?;
    }
    if (bits & protocol::U_ORIGIN2) != 0 {
        writer.write_q2_coord(to.origin[1])?;
    }
    if (bits & protocol::U_ORIGIN3) != 0 {
        writer.write_q2_coord(to.origin[2])?;
    }
    if (bits & protocol::U_ANGLE1) != 0 {
        writer.write_q2_angle(to.angles[0])?;
    }
    if (bits & protocol::U_ANGLE2) != 0 {
        writer.write_q2_angle(to.angles[1])?;
    }
    if (bits & protocol::U_ANGLE3) != 0 {
        writer.write_q2_angle(to.angles[2])?;
    }
    if (bits & protocol::U_OLDORIGIN) != 0 {
        writer.write_q2_pos(to.old_origin)?;
    }
    if (bits & protocol::U_SOUND) != 0 {
        writer.write_byte(to.sound as u8)?;
    }
    if (bits & protocol::U_EVENT) != 0 {
        writer.write_byte(to.event)?;
    }
    if (bits & protocol::U_SOLID) != 0 {
        writer.write_short(to.solid as i16)?;
    }
    Ok(true)
}

/// Read entity header bits and number (`readEntityBits`).
pub fn read_entity_bits(reader: &mut MsgReader<'_>) -> Result<(u16, u32), MsgError> {
    let mut total = u32::from(reader.byte()?);
    if (total & protocol::U_MOREBITS1) != 0 {
        total |= u32::from(reader.byte()?) << 8;
    }
    if (total & protocol::U_MOREBITS2) != 0 {
        total |= u32::from(reader.byte()?) << 16;
    }
    if (total & protocol::U_MOREBITS3) != 0 {
        total |= u32::from(reader.byte()?) << 24;
    }
    let number = if (total & protocol::U_NUMBER16) != 0 {
        reader.short()? as u16
    } else {
        u16::from(reader.byte()?)
    };
    Ok((number, total))
}

/// Read a delta entity body (`readDeltaEntity`).
pub fn read_delta_entity(
    reader: &mut MsgReader<'_>,
    from: &EntityState,
    number: u16,
    bits: u32,
) -> Result<EntityState, MsgError> {
    let mut to = from.clone();
    to.old_origin = from.origin;
    to.number = number;
    if (bits & protocol::U_MODEL) != 0 {
        to.modelindex = u16::from(reader.byte()?);
    }
    if (bits & protocol::U_MODEL2) != 0 {
        to.modelindex2 = u16::from(reader.byte()?);
    }
    if (bits & protocol::U_MODEL3) != 0 {
        to.modelindex3 = u16::from(reader.byte()?);
    }
    if (bits & protocol::U_MODEL4) != 0 {
        to.modelindex4 = u16::from(reader.byte()?);
    }
    if (bits & protocol::U_FRAME8) != 0 {
        to.frame = i32::from(reader.byte()?);
    }
    if (bits & protocol::U_FRAME16) != 0 {
        to.frame = i32::from(reader.short()?);
    }
    if (bits & protocol::U_SKIN8) != 0 && (bits & protocol::U_SKIN16) != 0 {
        to.skinnum = reader.long()?;
    } else if (bits & protocol::U_SKIN8) != 0 {
        to.skinnum = i32::from(reader.byte()?);
    } else if (bits & protocol::U_SKIN16) != 0 {
        to.skinnum = i32::from(reader.short()?);
    }
    if (bits & (protocol::U_EFFECTS8 | protocol::U_EFFECTS16)) == (protocol::U_EFFECTS8 | protocol::U_EFFECTS16) {
        to.effects = reader.long()?;
    } else if (bits & protocol::U_EFFECTS8) != 0 {
        to.effects = i32::from(reader.byte()?);
    } else if (bits & protocol::U_EFFECTS16) != 0 {
        to.effects = i32::from(reader.short()?);
    }
    if (bits & (protocol::U_RENDERFX8 | protocol::U_RENDERFX16)) == (protocol::U_RENDERFX8 | protocol::U_RENDERFX16) {
        to.renderfx = reader.long()?;
    } else if (bits & protocol::U_RENDERFX8) != 0 {
        to.renderfx = i32::from(reader.byte()?);
    } else if (bits & protocol::U_RENDERFX16) != 0 {
        to.renderfx = i32::from(reader.short()?);
    }
    if (bits & protocol::U_ORIGIN1) != 0 {
        to.origin[0] = reader.coord()?;
    }
    if (bits & protocol::U_ORIGIN2) != 0 {
        to.origin[1] = reader.coord()?;
    }
    if (bits & protocol::U_ORIGIN3) != 0 {
        to.origin[2] = reader.coord()?;
    }
    if (bits & protocol::U_ANGLE1) != 0 {
        to.angles[0] = reader.q2_angle()?;
    }
    if (bits & protocol::U_ANGLE2) != 0 {
        to.angles[1] = reader.q2_angle()?;
    }
    if (bits & protocol::U_ANGLE3) != 0 {
        to.angles[2] = reader.q2_angle()?;
    }
    if (bits & protocol::U_OLDORIGIN) != 0 {
        to.old_origin = reader.q2_pos()?;
    }
    if (bits & protocol::U_SOUND) != 0 {
        to.sound = u16::from(reader.byte()?);
    }
    if (bits & protocol::U_EVENT) != 0 {
        to.event = reader.byte()?;
    } else {
        to.event = 0;
    }
    if (bits & protocol::U_SOLID) != 0 {
        to.solid = u32::from(reader.short()? as u16);
    }
    Ok(to)
}

/// Write an entity removal (`writeEntityRemove`).
pub fn write_entity_remove(writer: &mut MsgWriter, number: u16) -> Result<(), MsgError> {
    let mut bits = protocol::U_REMOVE;
    if number >= 256 {
        bits |= protocol::U_NUMBER16 | protocol::U_MOREBITS1;
    }
    writer.write_byte((bits & 255) as u8)?;
    if (bits & 0x0000_ff00) != 0 {
        writer.write_byte(((bits >> 8) & 255) as u8)?;
    }
    if (bits & protocol::U_NUMBER16) != 0 {
        writer.write_short(number as i16)?;
    } else {
        writer.write_byte(number as u8)?;
    }
    Ok(())
}

/// Write the packet-entities terminator (`writePacketEntitiesEnd`).
pub fn write_packet_entities_end(writer: &mut MsgWriter) -> Result<(), MsgError> {
    writer.write_short(0)
}

/// Write a spawn baseline (`writeSpawnBaseline`).
pub fn write_spawn_baseline(writer: &mut MsgWriter, base: &EntityState) -> Result<bool, Q2CodecError> {
    writer.write_byte(protocol::Svc::Spawnbaseline as u8)?;
    write_delta_entity(writer, &EntityState::default(), base, true, true)
}

/// Write server data (`writeServerData`).
pub fn write_server_data(writer: &mut MsgWriter, data: &ServerData) -> Result<(), MsgError> {
    writer.write_byte(protocol::Svc::Serverdata as u8)?;
    writer.write_long(protocol::PROTOCOL_VERSION as i32)?;
    writer.write_long(data.servercount)?;
    writer.write_byte(u8::from(data.attractloop))?;
    writer.write_string(&data.gamedir)?;
    writer.write_short(data.clientnum)?;
    writer.write_string(&data.levelname)
}

/// Read server data (`readServerData`); opcode and version are consumed by the caller.
pub fn read_server_data(reader: &mut MsgReader<'_>) -> Result<ServerData, MsgError> {
    let servercount = reader.long()?;
    let attractloop = reader.byte()? != 0;
    let gamedir = reader.string(2047);
    let clientnum = reader.short()?;
    let levelname = reader.string(2047);
    reader.finish()?;
    Ok(ServerData {
        servercount,
        attractloop,
        gamedir,
        clientnum,
        levelname,
    })
}

/// Frame write parameters shared by every Q2 frame codec (`FrameWriteParamsT`).
pub struct FrameWrite<'a> {
    /// Frame number.
    pub framenum: i32,
    /// Delta base frame (`-1` for none).
    pub lastframe: i32,
    /// Suppressed-packet count.
    pub surpress_count: i32,
    /// Area visibility bits.
    pub areabits: &'a [u8],
    /// Delta base player state (`None` means a zeroed state).
    pub ps_from: Option<&'a PlayerState>,
    /// Target player state.
    pub ps_to: &'a PlayerState,
}

/// Parsed frame header (`FrameHeaderT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameHeader {
    /// Server frame number.
    pub serverframe: i32,
    /// Delta base frame (`-1` for none).
    pub deltaframe: i32,
    /// Suppressed-packet count.
    pub surpress_count: i32,
    /// Area-bytes length.
    pub areabytes: usize,
}

/// Compute classic player-state delta flags.
#[must_use]
pub fn player_state_bits(from: &PlayerState, to: &PlayerState) -> u32 {
    let mut pflags = 0;
    if to.pmove.pm_type != from.pmove.pm_type {
        pflags |= protocol::PS_M_TYPE;
    }
    if to.pmove.origin != from.pmove.origin {
        pflags |= protocol::PS_M_ORIGIN;
    }
    if to.pmove.velocity != from.pmove.velocity {
        pflags |= protocol::PS_M_VELOCITY;
    }
    if to.pmove.pm_time != from.pmove.pm_time {
        pflags |= protocol::PS_M_TIME;
    }
    if to.pmove.pm_flags != from.pmove.pm_flags {
        pflags |= protocol::PS_M_FLAGS;
    }
    if to.pmove.gravity != from.pmove.gravity {
        pflags |= protocol::PS_M_GRAVITY;
    }
    if to.pmove.delta_angles != from.pmove.delta_angles {
        pflags |= protocol::PS_M_DELTA_ANGLES;
    }
    if to.viewoffset != from.viewoffset {
        pflags |= protocol::PS_VIEWOFFSET;
    }
    if to.viewangles != from.viewangles {
        pflags |= protocol::PS_VIEWANGLES;
    }
    if to.kick_angles != from.kick_angles {
        pflags |= protocol::PS_KICKANGLES;
    }
    if to.blend != from.blend {
        pflags |= protocol::PS_BLEND;
    }
    if to.fov != from.fov {
        pflags |= protocol::PS_FOV;
    }
    if to.rdflags != from.rdflags {
        pflags |= protocol::PS_RDFLAGS;
    }
    if to.gunframe != from.gunframe {
        pflags |= protocol::PS_WEAPONFRAME;
    }
    pflags |= protocol::PS_WEAPONINDEX;
    pflags
}

/// Write a classic player-state delta (`writePlayerStateDelta`).
pub fn write_player_state_delta(writer: &mut MsgWriter, from: &PlayerState, to: &PlayerState) -> Result<(), MsgError> {
    let pflags = player_state_bits(from, to);
    writer.write_byte(protocol::Svc::Playerinfo as u8)?;
    writer.write_short(pflags as i16)?;
    write_player_state_body(writer, to, pflags)?;
    let mut statbits = 0u32;
    for (i, slot) in to.stats.iter().take(MAX_STATS).enumerate() {
        if *slot != from.stats[i] {
            statbits |= 1 << i;
        }
    }
    writer.write_long(statbits as i32)?;
    for (i, slot) in to.stats.iter().take(MAX_STATS).enumerate() {
        if (statbits & (1 << i)) != 0 {
            writer.write_short(*slot)?;
        }
    }
    Ok(())
}

/// Write the body of a classic player-state delta after the flags.
fn write_player_state_body(writer: &mut MsgWriter, to: &PlayerState, pflags: u32) -> Result<(), MsgError> {
    if (pflags & protocol::PS_M_TYPE) != 0 {
        writer.write_byte(to.pmove.pm_type)?;
    }
    if (pflags & protocol::PS_M_ORIGIN) != 0 {
        for axis in to.pmove.origin {
            writer.write_short(axis as i16)?;
        }
    }
    if (pflags & protocol::PS_M_VELOCITY) != 0 {
        for axis in to.pmove.velocity {
            writer.write_short(axis as i16)?;
        }
    }
    if (pflags & protocol::PS_M_TIME) != 0 {
        writer.write_byte(to.pmove.pm_time as u8)?;
    }
    if (pflags & protocol::PS_M_FLAGS) != 0 {
        writer.write_byte(to.pmove.pm_flags as u8)?;
    }
    if (pflags & protocol::PS_M_GRAVITY) != 0 {
        writer.write_short(to.pmove.gravity)?;
    }
    if (pflags & protocol::PS_M_DELTA_ANGLES) != 0 {
        for axis in to.pmove.delta_angles {
            writer.write_short(axis)?;
        }
    }
    if (pflags & protocol::PS_VIEWOFFSET) != 0 {
        for axis in to.viewoffset {
            writer.write_char(scaled_trunc(axis, 4.0) as i8)?;
        }
    }
    if (pflags & protocol::PS_VIEWANGLES) != 0 {
        for axis in to.viewangles {
            writer.write_q2_angle16(axis)?;
        }
    }
    if (pflags & protocol::PS_KICKANGLES) != 0 {
        for axis in to.kick_angles {
            writer.write_char(scaled_trunc(axis, 4.0) as i8)?;
        }
    }
    if (pflags & protocol::PS_WEAPONINDEX) != 0 {
        writer.write_byte(to.gunindex as u8)?;
    }
    if (pflags & protocol::PS_WEAPONFRAME) != 0 {
        writer.write_byte(to.gunframe as u8)?;
        for axis in to.gunoffset {
            writer.write_char(scaled_trunc(axis, 4.0) as i8)?;
        }
        for axis in to.gunangles {
            writer.write_char(scaled_trunc(axis, 4.0) as i8)?;
        }
    }
    if (pflags & protocol::PS_BLEND) != 0 {
        for axis in to.blend {
            writer.write_byte(scaled_trunc(axis, 255.0) as u8)?;
        }
    }
    if (pflags & protocol::PS_FOV) != 0 {
        writer.write_byte(to.fov)?;
    }
    if (pflags & protocol::PS_RDFLAGS) != 0 {
        writer.write_byte(to.rdflags)?;
    }
    Ok(())
}

/// Read a classic player-state delta (`readPlayerStateDelta`).
///
/// Unchanged fields inherit from the delta base.
pub fn read_player_state_delta(reader: &mut MsgReader<'_>, from: &PlayerState) -> Result<PlayerState, MsgError> {
    let mut to = from.clone();
    let flags = u32::from(reader.short()? as u16);
    if (flags & protocol::PS_M_TYPE) != 0 {
        to.pmove.pm_type = reader.byte()?;
    }
    if (flags & protocol::PS_M_ORIGIN) != 0 {
        for axis in &mut to.pmove.origin {
            *axis = i32::from(reader.short()?);
        }
    }
    if (flags & protocol::PS_M_VELOCITY) != 0 {
        for axis in &mut to.pmove.velocity {
            *axis = i32::from(reader.short()?);
        }
    }
    if (flags & protocol::PS_M_TIME) != 0 {
        to.pmove.pm_time = i32::from(reader.byte()?);
    }
    if (flags & protocol::PS_M_FLAGS) != 0 {
        to.pmove.pm_flags = i32::from(reader.byte()?);
    }
    if (flags & protocol::PS_M_GRAVITY) != 0 {
        to.pmove.gravity = reader.short()?;
    }
    if (flags & protocol::PS_M_DELTA_ANGLES) != 0 {
        for axis in &mut to.pmove.delta_angles {
            *axis = reader.short()?;
        }
    }
    if (flags & protocol::PS_VIEWOFFSET) != 0 {
        for axis in &mut to.viewoffset {
            *axis = f64::from(reader.char()?) * 0.25;
        }
    }
    if (flags & protocol::PS_VIEWANGLES) != 0 {
        for axis in &mut to.viewangles {
            *axis = short_to_angle(reader.short()?);
        }
    }
    if (flags & protocol::PS_KICKANGLES) != 0 {
        for axis in &mut to.kick_angles {
            *axis = f64::from(reader.char()?) * 0.25;
        }
    }
    if (flags & protocol::PS_WEAPONINDEX) != 0 {
        to.gunindex = i32::from(reader.byte()?);
    }
    if (flags & protocol::PS_WEAPONFRAME) != 0 {
        to.gunframe = i32::from(reader.byte()?);
        for axis in &mut to.gunoffset {
            *axis = f64::from(reader.char()?) * 0.25;
        }
        for axis in &mut to.gunangles {
            *axis = f64::from(reader.char()?) * 0.25;
        }
    }
    if (flags & protocol::PS_BLEND) != 0 {
        for axis in &mut to.blend {
            *axis = f64::from(reader.byte()?) / 255.0;
        }
    }
    if (flags & protocol::PS_FOV) != 0 {
        to.fov = reader.byte()?;
    }
    if (flags & protocol::PS_RDFLAGS) != 0 {
        to.rdflags = reader.byte()?;
    }
    let statbits = reader.long()? as u32;
    for i in 0..MAX_STATS {
        if (statbits & (1 << i)) != 0 {
            to.stats[i] = reader.short()?;
        }
    }
    Ok(to)
}

/// Write a classic frame (`writeFrame`).
pub fn write_frame<E>(
    writer: &mut MsgWriter,
    params: &FrameWrite<'_>,
    write_entities: impl FnOnce(&mut MsgWriter) -> Result<(), E>,
) -> Result<(), E>
where
    E: From<MsgError>,
{
    writer.write_byte(protocol::Svc::Frame as u8)?;
    writer.write_long(params.framenum)?;
    writer.write_long(params.lastframe)?;
    writer.write_byte(params.surpress_count as u8)?;
    writer.write_byte(params.areabits.len() as u8)?;
    writer.write_bytes(params.areabits)?;
    let base = PlayerState::default();
    let from = params.ps_from.unwrap_or(&base);
    write_player_state_delta(writer, from, params.ps_to)?;
    write_entities(writer)
}

/// Read a classic frame header (`readFrameHeader`).
pub fn read_frame_header(
    reader: &mut MsgReader<'_>,
    areabits: &mut Vec<u8>,
    read_suppress_byte: bool,
) -> Result<FrameHeader, MsgError> {
    let serverframe = reader.long()?;
    let deltaframe = reader.long()?;
    let surpress_count = if read_suppress_byte {
        i32::from(reader.byte()?)
    } else {
        0
    };
    let len = usize::from(reader.byte()?);
    areabits.clear();
    areabits.extend_from_slice(reader.bytes(len)?);
    Ok(FrameHeader {
        serverframe,
        deltaframe,
        surpress_count,
        areabytes: len,
    })
}

/// Read the player state of a classic frame (`readFramePlayerstate`).
pub fn read_frame_playerstate(reader: &mut MsgReader<'_>, from: &PlayerState) -> Result<PlayerState, Q2CodecError> {
    let opcode = reader.byte()?;
    if opcode != protocol::Svc::Playerinfo as u8 {
        return Err(Q2CodecError::UnexpectedOpcode {
            expected: protocol::Svc::Playerinfo as u8,
            found: opcode,
        });
    }
    Ok(read_player_state_delta(reader, from)?)
}

/// Consume the packet-entities opcode of a classic frame (`readPacketEntitiesBegin`).
pub fn read_packet_entities_begin(reader: &mut MsgReader<'_>) -> Result<(), Q2CodecError> {
    let opcode = reader.byte()?;
    if opcode != protocol::Svc::Packetentities as u8 {
        return Err(Q2CodecError::UnexpectedOpcode {
            expected: protocol::Svc::Packetentities as u8,
            found: opcode,
        });
    }
    Ok(())
}

/// Encode a string as single-byte characters (`stringToBytes`).
///
/// Each UTF-16 unit contributes its low byte, matching the donor's
/// `charCodeAt(i) & 0xff` exactly, including surrogate halves.
#[must_use]
pub fn string_to_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16().map(|unit| unit as u8).collect()
}

/// Write the closest vertex normal to a direction (`MSG_WriteDir`).
///
/// A missing direction writes index 0; ties keep the first index, matching
/// the donor's strictly-greater search over the same table.
pub fn write_dir(writer: &mut MsgWriter, dir: Option<[f64; 3]>) -> Result<(), MsgError> {
    let Some(dir) = dir else {
        return writer.write_byte(0);
    };
    let mut best_dot = 0.0f64;
    let mut best = 0u8;
    for (index, normal) in BYTEDIRS.iter().enumerate() {
        let dot = dir[0] * f64::from(normal[0]) + dir[1] * f64::from(normal[1]) + dir[2] * f64::from(normal[2]);
        if dot > best_dot {
            best_dot = dot;
            best = index as u8;
        }
    }
    writer.write_byte(best)
}

/// Read a direction byte into a vertex normal (`MSG_ReadDir`).
pub fn read_dir(reader: &mut MsgReader<'_>) -> Result<[f64; 3], Q2CodecError> {
    let index = reader.byte()?;
    if usize::from(index) >= NUMVERTEXNORMALS {
        return Err(Q2CodecError::BadDirection(index));
    }
    let dir = BYTEDIRS[usize::from(index)];
    Ok([f64::from(dir[0]), f64::from(dir[1]), f64::from(dir[2])])
}

/// Precomputed vertex normals (`bytedirs`).
///
/// Mechanically generated from `src/network/q2/anorms.ts`.
pub const BYTEDIRS: [[f32; 3]; NUMVERTEXNORMALS] = [
    [-0.525731, 0.000000, 0.850651],
    [-0.442863, 0.238856, 0.864188],
    [-0.295242, 0.000000, 0.955423],
    [-0.309017, 0.500000, 0.809017],
    [-0.162460, 0.262866, 0.951056],
    [0.000000, 0.000000, 1.000000],
    [0.000000, 0.850651, 0.525731],
    [-0.147621, 0.716567, 0.681718],
    [0.147621, 0.716567, 0.681718],
    [0.000000, 0.525731, 0.850651],
    [0.309017, 0.500000, 0.809017],
    [0.525731, 0.000000, 0.850651],
    [0.295242, 0.000000, 0.955423],
    [0.442863, 0.238856, 0.864188],
    [0.162460, 0.262866, 0.951056],
    [-0.681718, 0.147621, 0.716567],
    [-0.809017, 0.309017, 0.500000],
    [-0.587785, 0.425325, 0.688191],
    [-0.850651, 0.525731, 0.000000],
    [-0.864188, 0.442863, 0.238856],
    [-0.716567, 0.681718, 0.147621],
    [-0.688191, 0.587785, 0.425325],
    [-0.500000, 0.809017, 0.309017],
    [-0.238856, 0.864188, 0.442863],
    [-0.425325, 0.688191, 0.587785],
    [-0.716567, 0.681718, -0.147621],
    [-0.500000, 0.809017, -0.309017],
    [-0.525731, 0.850651, 0.000000],
    [0.000000, 0.850651, -0.525731],
    [-0.238856, 0.864188, -0.442863],
    [0.000000, 0.955423, -0.295242],
    [-0.262866, 0.951056, -0.162460],
    [0.000000, 1.000000, 0.000000],
    [0.000000, 0.955423, 0.295242],
    [-0.262866, 0.951056, 0.162460],
    [0.238856, 0.864188, 0.442863],
    [0.262866, 0.951056, 0.162460],
    [0.500000, 0.809017, 0.309017],
    [0.238856, 0.864188, -0.442863],
    [0.262866, 0.951056, -0.162460],
    [0.500000, 0.809017, -0.309017],
    [0.850651, 0.525731, 0.000000],
    [0.716567, 0.681718, 0.147621],
    [0.716567, 0.681718, -0.147621],
    [0.525731, 0.850651, 0.000000],
    [0.425325, 0.688191, 0.587785],
    [0.864188, 0.442863, 0.238856],
    [0.688191, 0.587785, 0.425325],
    [0.809017, 0.309017, 0.500000],
    [0.681718, 0.147621, 0.716567],
    [0.587785, 0.425325, 0.688191],
    [0.955423, 0.295242, 0.000000],
    [1.000000, 0.000000, 0.000000],
    [0.951056, 0.162460, 0.262866],
    [0.850651, -0.525731, 0.000000],
    [0.955423, -0.295242, 0.000000],
    [0.864188, -0.442863, 0.238856],
    [0.951056, -0.162460, 0.262866],
    [0.809017, -0.309017, 0.500000],
    [0.681718, -0.147621, 0.716567],
    [0.850651, 0.000000, 0.525731],
    [0.864188, 0.442863, -0.238856],
    [0.809017, 0.309017, -0.500000],
    [0.951056, 0.162460, -0.262866],
    [0.525731, 0.000000, -0.850651],
    [0.681718, 0.147621, -0.716567],
    [0.681718, -0.147621, -0.716567],
    [0.850651, 0.000000, -0.525731],
    [0.809017, -0.309017, -0.500000],
    [0.864188, -0.442863, -0.238856],
    [0.951056, -0.162460, -0.262866],
    [0.147621, 0.716567, -0.681718],
    [0.309017, 0.500000, -0.809017],
    [0.425325, 0.688191, -0.587785],
    [0.442863, 0.238856, -0.864188],
    [0.587785, 0.425325, -0.688191],
    [0.688191, 0.587785, -0.425325],
    [-0.147621, 0.716567, -0.681718],
    [-0.309017, 0.500000, -0.809017],
    [0.000000, 0.525731, -0.850651],
    [-0.525731, 0.000000, -0.850651],
    [-0.442863, 0.238856, -0.864188],
    [-0.295242, 0.000000, -0.955423],
    [-0.162460, 0.262866, -0.951056],
    [0.000000, 0.000000, -1.000000],
    [0.295242, 0.000000, -0.955423],
    [0.162460, 0.262866, -0.951056],
    [-0.442863, -0.238856, -0.864188],
    [-0.309017, -0.500000, -0.809017],
    [-0.162460, -0.262866, -0.951056],
    [0.000000, -0.850651, -0.525731],
    [-0.147621, -0.716567, -0.681718],
    [0.147621, -0.716567, -0.681718],
    [0.000000, -0.525731, -0.850651],
    [0.309017, -0.500000, -0.809017],
    [0.442863, -0.238856, -0.864188],
    [0.162460, -0.262866, -0.951056],
    [0.238856, -0.864188, -0.442863],
    [0.500000, -0.809017, -0.309017],
    [0.425325, -0.688191, -0.587785],
    [0.716567, -0.681718, -0.147621],
    [0.688191, -0.587785, -0.425325],
    [0.587785, -0.425325, -0.688191],
    [0.000000, -0.955423, -0.295242],
    [0.000000, -1.000000, 0.000000],
    [0.262866, -0.951056, -0.162460],
    [0.000000, -0.850651, 0.525731],
    [0.000000, -0.955423, 0.295242],
    [0.238856, -0.864188, 0.442863],
    [0.262866, -0.951056, 0.162460],
    [0.500000, -0.809017, 0.309017],
    [0.716567, -0.681718, 0.147621],
    [0.525731, -0.850651, 0.000000],
    [-0.238856, -0.864188, -0.442863],
    [-0.500000, -0.809017, -0.309017],
    [-0.262866, -0.951056, -0.162460],
    [-0.850651, -0.525731, 0.000000],
    [-0.716567, -0.681718, -0.147621],
    [-0.716567, -0.681718, 0.147621],
    [-0.525731, -0.850651, 0.000000],
    [-0.500000, -0.809017, 0.309017],
    [-0.238856, -0.864188, 0.442863],
    [-0.262866, -0.951056, 0.162460],
    [-0.864188, -0.442863, 0.238856],
    [-0.809017, -0.309017, 0.500000],
    [-0.688191, -0.587785, 0.425325],
    [-0.681718, -0.147621, 0.716567],
    [-0.442863, -0.238856, 0.864188],
    [-0.587785, -0.425325, 0.688191],
    [-0.309017, -0.500000, 0.809017],
    [-0.147621, -0.716567, 0.681718],
    [-0.425325, -0.688191, 0.587785],
    [-0.162460, -0.262866, 0.951056],
    [0.442863, -0.238856, 0.864188],
    [0.162460, -0.262866, 0.951056],
    [0.309017, -0.500000, 0.809017],
    [0.147621, -0.716567, 0.681718],
    [0.000000, -0.525731, 0.850651],
    [0.425325, -0.688191, 0.587785],
    [0.587785, -0.425325, 0.688191],
    [0.688191, -0.587785, 0.425325],
    [-0.955423, 0.295242, 0.000000],
    [-0.951056, 0.162460, 0.262866],
    [-1.000000, 0.000000, 0.000000],
    [-0.850651, 0.000000, 0.525731],
    [-0.955423, -0.295242, 0.000000],
    [-0.951056, -0.162460, 0.262866],
    [-0.864188, 0.442863, -0.238856],
    [-0.951056, 0.162460, -0.262866],
    [-0.809017, 0.309017, -0.500000],
    [-0.864188, -0.442863, -0.238856],
    [-0.951056, -0.162460, -0.262866],
    [-0.809017, -0.309017, -0.500000],
    [-0.681718, 0.147621, -0.716567],
    [-0.681718, -0.147621, -0.716567],
    [-0.850651, 0.000000, -0.525731],
    [-0.688191, 0.587785, -0.425325],
    [-0.587785, 0.425325, -0.688191],
    [-0.425325, 0.688191, -0.587785],
    [-0.425325, -0.688191, -0.587785],
    [-0.587785, -0.425325, -0.688191],
    [-0.688191, -0.587785, -0.425325],
];

#[cfg(test)]
mod tests {
    use super::*;

    fn full_state(number: u16) -> EntityState {
        EntityState {
            number,
            origin: [12.5, -4.25, 100.0],
            angles: [0.0, 90.0, 180.0],
            old_origin: [1.0, 2.0, 3.0],
            modelindex: 3,
            modelindex2: 4,
            modelindex3: 5,
            modelindex4: 6,
            frame: 300,
            skinnum: 70000,
            effects: 0x9000,
            renderfx: 41,
            solid: 31,
            sound: 9,
            event: 7,
            ..EntityState::default()
        }
    }

    #[test]
    fn usercmd_round_trip() {
        let from = Usercmd::default();
        let cmd = Usercmd {
            msec: 50,
            buttons: 3,
            angles: [100, -200, 300],
            forwardmove: 400,
            sidemove: -400,
            upmove: 10,
            impulse: 8,
            lightlevel: 200,
            server_frame: 0,
        };
        let mut writer = MsgWriter::new(protocol::MAX_MSGLEN, false);
        write_delta_usercmd(&mut writer, &from, &cmd).unwrap();
        write_delta_usercmd(&mut writer, &cmd, &cmd).unwrap();
        let bytes = writer.bytes().to_vec();

        let mut reader = MsgReader::new(&bytes);
        let decoded = read_delta_usercmd(&mut reader, &from).unwrap();
        assert_eq!(decoded, cmd);
        let decoded = read_delta_usercmd(&mut reader, &decoded).unwrap();
        assert_eq!(decoded, cmd);
        reader.finish().unwrap();
    }

    #[test]
    fn delta_entity_round_trip_all_widths() {
        for number in [7u16, 300u16] {
            let from = EntityState::default();
            let mut to = full_state(number);
            to.number = number;
            let mut writer = MsgWriter::new(4096, false);
            assert!(write_delta_entity(&mut writer, &from, &to, false, true).unwrap());
            let bytes = writer.bytes().to_vec();

            let mut reader = MsgReader::new(&bytes);
            let (decoded_number, bits) = read_entity_bits(&mut reader).unwrap();
            assert_eq!(decoded_number, number);
            let decoded = read_delta_entity(&mut reader, &from, decoded_number, bits).unwrap();
            reader.finish().unwrap();
            assert_eq!(decoded.number, number);
            assert_eq!(decoded.origin, to.origin);
            assert_eq!(decoded.angles[1], 90.0);
            assert_eq!(decoded.old_origin, to.old_origin);
            assert_eq!(decoded.frame, 300);
            assert_eq!(decoded.skinnum, 70000);
            assert_eq!(decoded.effects, 0x9000);
            assert_eq!(decoded.solid, 31);
            assert_eq!(decoded.event, 7);
        }
    }

    #[test]
    fn delta_entity_skips_unchanged_and_validates_numbers() {
        let mut state = full_state(11);
        state.event = 0;
        let mut writer = MsgWriter::new(4096, false);
        assert!(!write_delta_entity(&mut writer, &state, &state, false, false).unwrap());
        assert_eq!(writer.cursize(), 0);

        let unset = EntityState::default();
        assert_eq!(
            write_delta_entity(&mut writer, &unset, &unset, true, false),
            Err(Q2CodecError::UnsetNumber)
        );
        let mut wide = full_state(MAX_EDICTS);
        wide.number = MAX_EDICTS;
        assert_eq!(
            write_delta_entity(&mut writer, &unset, &wide, true, false),
            Err(Q2CodecError::NumberTooLarge(MAX_EDICTS))
        );
    }

    fn full_player() -> PlayerState {
        let mut ps = PlayerState::default();
        ps.pmove.pm_type = 1;
        ps.pmove.origin = [800, -400, 120];
        ps.pmove.velocity = [10, -20, 30];
        ps.pmove.pm_time = 5;
        ps.pmove.pm_flags = 3;
        ps.pmove.gravity = 800;
        ps.pmove.delta_angles = [100, 200, 300];
        ps.viewoffset = [0.25, -0.5, 1.0];
        ps.viewangles = [10.0, 20.0, 30.0];
        ps.kick_angles = [1.0, 2.0, 3.0];
        ps.gunindex = 5;
        ps.gunframe = 7;
        ps.gunoffset = [0.5, 0.5, 0.5];
        ps.gunangles = [4.0, 5.0, 6.0];
        ps.blend = [0.1, 0.2, 0.3, 0.4];
        ps.fov = 90;
        ps.rdflags = 1;
        ps.stats[0] = 100;
        ps.stats[5] = -3;
        ps
    }

    fn decode_hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn playerstate_byte_exact() {
        let mut writer = MsgWriter::new(4096, false);
        write_player_state_delta(&mut writer, &PlayerState::default(), &full_player()).unwrap();
        assert_eq!(
            writer.bytes(),
            decode_hex("11ff7f01200370fe78000a00ecff1e00050320036400c8002c0101fe041c07380e551504080c050702020210141819334c665a01210000006400fdff").as_slice()
        );
        let mut reader = MsgReader::new(writer.bytes());
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Playerinfo as u8);
        let decoded = read_player_state_delta(&mut reader, &PlayerState::default()).unwrap();
        reader.finish().unwrap();
        assert_eq!(decoded.pmove.pm_type, 1);
        assert_eq!(decoded.pmove.origin, [800, -400, 120]);
        assert_eq!(decoded.pmove.velocity, [10, -20, 30]);
        assert_eq!(decoded.pmove.pm_time, 5);
        assert_eq!(decoded.pmove.pm_flags, 3);
        assert_eq!(decoded.gunindex, 5);
        assert_eq!(decoded.gunframe, 7);
        assert_eq!(decoded.gunoffset, [0.5, 0.5, 0.5]);
        assert_eq!(decoded.fov, 90);
        assert_eq!(decoded.stats[0], 100);
        assert_eq!(decoded.stats[5], -3);
    }

    #[test]
    fn frame_byte_exact() {
        let ps = PlayerState {
            gunframe: 7,
            fov: 90,
            ..Default::default()
        };
        let mut writer = MsgWriter::new(4096, false);
        write_frame(
            &mut writer,
            &FrameWrite {
                framenum: 10,
                lastframe: 9,
                surpress_count: 1,
                areabits: &[0x3c],
                ps_from: None,
                ps_to: &ps,
            },
            |w| {
                w.write_byte(protocol::Svc::Packetentities as u8)?;
                w.write_short(0)
            },
        )
        .unwrap();
        assert_eq!(
            writer.bytes(),
            decode_hex("140a0000000900000001013c11003800070000000000005a00000000120000").as_slice()
        );
        let mut reader = MsgReader::new(writer.bytes());
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Frame as u8);
        let mut areas = Vec::new();
        let header = read_frame_header(&mut reader, &mut areas, true).unwrap();
        assert_eq!(
            header,
            FrameHeader {
                serverframe: 10,
                deltaframe: 9,
                surpress_count: 1,
                areabytes: 1,
            }
        );
        assert_eq!(areas, vec![0x3c]);
        let decoded = read_frame_playerstate(&mut reader, &PlayerState::default()).unwrap();
        assert_eq!(decoded.gunframe, 7);
        assert_eq!(decoded.fov, 90);
        read_packet_entities_begin(&mut reader).unwrap();
        assert_eq!(reader.short().unwrap(), 0);
        reader.finish().unwrap();
    }

    #[test]
    fn dir_and_opcode_errors() {
        let mut reader = MsgReader::new(&[0]);
        let dir = read_dir(&mut reader).unwrap();
        assert_eq!(dir[0] as f32, BYTEDIRS[0][0]);
        let mut reader = MsgReader::new(&[162]);
        assert_eq!(read_dir(&mut reader), Err(Q2CodecError::BadDirection(162)));
        let mut reader = MsgReader::new(&[6]);
        assert_eq!(
            read_frame_playerstate(&mut reader, &PlayerState::default()),
            Err(Q2CodecError::UnexpectedOpcode {
                expected: protocol::Svc::Playerinfo as u8,
                found: 6,
            })
        );
        let mut reader = MsgReader::new(&[6]);
        assert_eq!(
            read_packet_entities_begin(&mut reader),
            Err(Q2CodecError::UnexpectedOpcode {
                expected: protocol::Svc::Packetentities as u8,
                found: 6,
            })
        );
    }

    #[test]
    fn removal_baseline_and_serverdata_round_trip() {
        let base = full_state(55);
        let data = ServerData {
            servercount: 3,
            attractloop: true,
            gamedir: "baseq2".to_string(),
            clientnum: 1,
            levelname: "base1".to_string(),
        };
        let mut writer = MsgWriter::new(4096, false);
        write_entity_remove(&mut writer, 9).unwrap();
        write_entity_remove(&mut writer, 300).unwrap();
        write_packet_entities_end(&mut writer).unwrap();
        assert!(write_spawn_baseline(&mut writer, &base).unwrap());
        write_server_data(&mut writer, &data).unwrap();
        let bytes = writer.bytes().to_vec();

        let mut reader = MsgReader::new(&bytes);
        let (number, bits) = read_entity_bits(&mut reader).unwrap();
        assert_eq!((number, bits & protocol::U_REMOVE != 0), (9, true));
        let (number, bits) = read_entity_bits(&mut reader).unwrap();
        assert_eq!((number, bits & protocol::U_REMOVE != 0), (300, true));
        let (number, bits) = read_entity_bits(&mut reader).unwrap();
        assert_eq!((number, bits), (0, 0));
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Spawnbaseline as u8);
        let (number, bits) = read_entity_bits(&mut reader).unwrap();
        let decoded = read_delta_entity(&mut reader, &EntityState::default(), number, bits).unwrap();
        assert_eq!((decoded.number, decoded.frame), (55, 300));
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Serverdata as u8);
        assert_eq!(reader.long().unwrap(), 34);
        let decoded = read_server_data(&mut reader).unwrap();
        assert_eq!(decoded, data);
        reader.finish().unwrap();
    }
}
