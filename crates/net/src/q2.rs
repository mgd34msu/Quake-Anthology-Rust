//! Quake II classic message codecs.
//!
//! Donor provenance: `MSG_WriteDeltaUsercmd` / `MSG_ReadDeltaUsercmd` and
//! `MSG_WriteDeltaEntity` in `src/network/q2/message.ts`, with entity
//! header bits, removal framing, spawn baselines, and serverdata from
//! `createVanillaContext` in `src/network/q2/codecs/vanilla.ts`.
//! `EntityStateT` / `UsercmdT` shapes come from `src/network/q2/state.ts`.
//!
//! Only the classic (protocol 34) wire shape is covered; R1Q2, Q2Pro,
//! rerelease, and KEX codec variants are future work.

use thiserror::Error;

use crate::msg::{MsgError, MsgReader, MsgWriter};
use crate::protocol::q2 as protocol;

/// Maximum edicts (`MAX_EDICTS`).
pub const MAX_EDICTS: u16 = 1024;
/// Beam render effect flag (`RF_BEAM`).
pub const RF_BEAM: i32 = 128;

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
}

/// Quake II entity state (`EntityStateT`, wire-covered fields).
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
    pub modelindex: u8,
    /// Second model index.
    pub modelindex2: u8,
    /// Third model index.
    pub modelindex3: u8,
    /// Fourth model index.
    pub modelindex4: u8,
    /// Frame.
    pub frame: i32,
    /// Skin number.
    pub skinnum: i32,
    /// Effects.
    pub effects: i32,
    /// Render effects.
    pub renderfx: i32,
    /// Solid.
    pub solid: u16,
    /// Sound.
    pub sound: u8,
    /// Event.
    pub event: u8,
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
        writer.write_byte(to.modelindex)?;
    }
    if (bits & protocol::U_MODEL2) != 0 {
        writer.write_byte(to.modelindex2)?;
    }
    if (bits & protocol::U_MODEL3) != 0 {
        writer.write_byte(to.modelindex3)?;
    }
    if (bits & protocol::U_MODEL4) != 0 {
        writer.write_byte(to.modelindex4)?;
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
        writer.write_byte(to.sound)?;
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
        to.modelindex = reader.byte()?;
    }
    if (bits & protocol::U_MODEL2) != 0 {
        to.modelindex2 = reader.byte()?;
    }
    if (bits & protocol::U_MODEL3) != 0 {
        to.modelindex3 = reader.byte()?;
    }
    if (bits & protocol::U_MODEL4) != 0 {
        to.modelindex4 = reader.byte()?;
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
        to.sound = reader.byte()?;
    }
    if (bits & protocol::U_EVENT) != 0 {
        to.event = reader.byte()?;
    } else {
        to.event = 0;
    }
    if (bits & protocol::U_SOLID) != 0 {
        to.solid = reader.short()? as u16;
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
