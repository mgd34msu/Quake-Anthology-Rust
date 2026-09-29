//! QuakeWorld protocol 28 message codecs.
//!
//! Donor provenance: `createQw28Codec`, `qwWriteDeltaUsercmd`, and
//! `qwReadDeltaUsercmd` in `src/network/q1/codecs/qw28.ts`, with scalar
//! helpers from `src/network/q1/message.ts`.
//!
//! The protocol 29 / wide variants (`qw29.ts`, `wide.ts`) reuse these
//! shapes with wider precache counts; see [`crate::q1_wide`].

use qa_core::numeric::float_to_wrapped_i32;

use crate::msg::{MsgError, MsgReader, MsgWriter};
use crate::protocol::q1 as nq;
use crate::protocol::qw as protocol;

/// QuakeWorld maximum entity number (`QW28_MAX_ENTITY_NUMBER`).
pub const MAX_ENTITY_NUMBER: u16 = 512;
/// QuakeWorld precache limit (`QW28_MAX_PRECACHE`).
pub const MAX_PRECACHE: usize = 256;

/// Entity origin delta threshold in units (donor `±0.1`).
pub const ORIGIN_EPSILON: f64 = 0.1;

/// QuakeWorld user command (`QwUsercmdT`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QwUsercmd {
    /// Milliseconds.
    pub msec: u8,
    /// View angles in degrees.
    pub angles: [f64; 3],
    /// Forward move.
    pub forwardmove: i16,
    /// Side move.
    pub sidemove: i16,
    /// Up move.
    pub upmove: i16,
    /// Buttons.
    pub buttons: u8,
    /// Impulse.
    pub impulse: u8,
}

/// QuakeWorld entity state (`QwEntityStateT`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QwEntityState {
    /// Entity number.
    pub number: u16,
    /// Origin.
    pub origin: [f64; 3],
    /// Angles in degrees.
    pub angles: [f64; 3],
    /// Model index.
    pub modelindex: u8,
    /// Frame.
    pub frame: u8,
    /// Colormap.
    pub colormap: u8,
    /// Skin.
    pub skinnum: u8,
    /// Effects.
    pub effects: u8,
    /// Entity flags (decoder checkpoint only; see [`crate::q1_checkpoint`]).
    pub flags: u16,
    /// Entity alpha (decoder checkpoint only; see [`crate::q1_checkpoint`]).
    pub alpha: u8,
    /// Entity scale (decoder checkpoint only; see [`crate::q1_checkpoint`]).
    pub scale: u8,
    /// Solid for prediction (`U_SOLID`).
    pub solid: bool,
}

/// QuakeWorld sound message (`SoundMessageT`).
#[derive(Debug, Clone, PartialEq)]
pub struct QwSoundMessage {
    /// Entity number (must be below 1024).
    pub ent: u16,
    /// Channel (must be below 8).
    pub channel: u8,
    /// Sound index (must be below 256).
    pub sound_num: u16,
    /// Volume byte (default 255).
    pub volume: u8,
    /// Attenuation (default 1.0, sent as `atten * 64`).
    pub attenuation: f64,
    /// Origin.
    pub origin: [f64; 3],
}

/// Write the protocol version long (`writeProtocol`).
pub fn write_protocol(writer: &mut MsgWriter) -> Result<(), MsgError> {
    writer.write_long(protocol::PROTOCOL_VERSION as i32)
}

/// Write a delta user command (`qwWriteDeltaUsercmd`).
pub fn write_delta_usercmd(writer: &mut MsgWriter, from: &QwUsercmd, cmd: &QwUsercmd) -> Result<(), MsgError> {
    let mut bits = 0;
    if cmd.angles[0] != from.angles[0] {
        bits |= protocol::CM_ANGLE1;
    }
    if cmd.angles[1] != from.angles[1] {
        // Donor order: ANGLE1, ANGLE2(yaw), ANGLE3, then moves.
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
        writer.write_qw_angle16(cmd.angles[0])?;
    }
    if (bits & protocol::CM_ANGLE2) != 0 {
        writer.write_qw_angle16(cmd.angles[1])?;
    }
    if (bits & protocol::CM_ANGLE3) != 0 {
        writer.write_qw_angle16(cmd.angles[2])?;
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
    writer.write_byte(cmd.msec)
}

/// Read a delta user command (`qwReadDeltaUsercmd`).
pub fn read_delta_usercmd(reader: &mut MsgReader<'_>, from: &QwUsercmd) -> Result<QwUsercmd, MsgError> {
    let mut cmd = from.clone();
    let bits = u32::from(reader.byte()?);
    if (bits & protocol::CM_ANGLE1) != 0 {
        cmd.angles[0] = reader.qw_angle16()?;
    }
    if (bits & protocol::CM_ANGLE2) != 0 {
        cmd.angles[1] = reader.qw_angle16()?;
    }
    if (bits & protocol::CM_ANGLE3) != 0 {
        cmd.angles[2] = reader.qw_angle16()?;
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
    Ok(cmd)
}

/// Compute delta-entity bits for a state against its baseline.
#[must_use]
pub fn entity_bits(from: &QwEntityState, to: &QwEntityState) -> u32 {
    let mut bits = 0;
    for axis in 0..3 {
        let miss = to.origin[axis] - from.origin[axis];
        if miss < -ORIGIN_EPSILON || miss > ORIGIN_EPSILON {
            bits |= protocol::U_ORIGIN1 << axis;
        }
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
    if to.colormap != from.colormap {
        bits |= protocol::U_COLORMAP;
    }
    if to.skinnum != from.skinnum {
        bits |= protocol::U_SKIN;
    }
    if to.frame != from.frame {
        bits |= protocol::U_FRAME;
    }
    if to.effects != from.effects {
        bits |= protocol::U_EFFECTS;
    }
    if to.modelindex != from.modelindex {
        bits |= protocol::U_MODEL;
    }
    if (bits & 511) != 0 {
        bits |= protocol::U_MOREBITS;
    }
    if to.solid {
        bits |= protocol::U_SOLID;
    }
    bits
}

/// Write a delta entity (`writeDeltaEntity`).
///
/// Returns `false` (writing nothing) when the number is out of range;
/// when nothing changed and `force` is false the donor also writes
/// nothing but reports success.
pub fn write_delta_entity(
    writer: &mut MsgWriter,
    from: &QwEntityState,
    to: &QwEntityState,
    force: bool,
) -> Result<bool, MsgError> {
    let bits = entity_bits(from, to);
    if to.number >= MAX_ENTITY_NUMBER {
        return Ok(false);
    }
    if bits == 0 && !force {
        return Ok(true);
    }
    let word = u32::from(to.number) | (bits & !511);
    debug_assert!(word & protocol::U_REMOVE == 0, "delta entity must not set U_REMOVE");
    if (word & protocol::U_REMOVE) != 0 {
        return Ok(false);
    }
    writer.write_short(word as i16)?;
    if (bits & protocol::U_MOREBITS) != 0 {
        writer.write_byte((bits & 255) as u8)?;
    }
    if (bits & protocol::U_MODEL) != 0 {
        writer.write_byte(to.modelindex)?;
    }
    if (bits & protocol::U_FRAME) != 0 {
        writer.write_byte(to.frame)?;
    }
    if (bits & protocol::U_COLORMAP) != 0 {
        writer.write_byte(to.colormap)?;
    }
    if (bits & protocol::U_SKIN) != 0 {
        writer.write_byte(to.skinnum)?;
    }
    if (bits & protocol::U_EFFECTS) != 0 {
        writer.write_byte(to.effects)?;
    }
    if (bits & protocol::U_ORIGIN1) != 0 {
        writer.write_coord(to.origin[0])?;
    }
    if (bits & protocol::U_ANGLE1) != 0 {
        writer.write_qw_angle(to.angles[0])?;
    }
    if (bits & protocol::U_ORIGIN2) != 0 {
        writer.write_coord(to.origin[1])?;
    }
    if (bits & protocol::U_ANGLE2) != 0 {
        writer.write_qw_angle(to.angles[1])?;
    }
    if (bits & protocol::U_ORIGIN3) != 0 {
        writer.write_coord(to.origin[2])?;
    }
    if (bits & protocol::U_ANGLE3) != 0 {
        writer.write_qw_angle(to.angles[2])?;
    }
    Ok(true)
}

/// Decoded entity header: number, bits, and remove flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityHeader {
    /// Entity number.
    pub number: u16,
    /// Update bits.
    pub bits: u32,
    /// Remove flag (`U_REMOVE`).
    pub remove: bool,
}

/// Read an entity header word (`readDeltaEntityHeader`).
pub fn read_entity_header(reader: &mut MsgReader<'_>) -> Result<EntityHeader, MsgError> {
    let word = u32::from(reader.short()? as u16);
    let number = (word & 511) as u16;
    let mut bits = word & !511;
    if (bits & protocol::U_MOREBITS) != 0 {
        bits |= u32::from(reader.byte()?);
    }
    Ok(EntityHeader {
        number,
        bits,
        remove: (bits & protocol::U_REMOVE) != 0,
    })
}

/// Read a delta entity body (`readDeltaEntity`).
pub fn read_delta_entity(
    reader: &mut MsgReader<'_>,
    from: &QwEntityState,
    header: EntityHeader,
) -> Result<QwEntityState, MsgError> {
    let mut to = from.clone();
    to.number = header.number;
    let bits = header.bits;
    to.solid = (bits & protocol::U_SOLID) != 0;
    if (bits & protocol::U_MODEL) != 0 {
        to.modelindex = reader.byte()?;
    }
    if (bits & protocol::U_FRAME) != 0 {
        to.frame = reader.byte()?;
    }
    if (bits & protocol::U_COLORMAP) != 0 {
        to.colormap = reader.byte()?;
    }
    if (bits & protocol::U_SKIN) != 0 {
        to.skinnum = reader.byte()?;
    }
    if (bits & protocol::U_EFFECTS) != 0 {
        to.effects = reader.byte()?;
    }
    if (bits & protocol::U_ORIGIN1) != 0 {
        to.origin[0] = reader.coord()?;
    }
    if (bits & protocol::U_ANGLE1) != 0 {
        to.angles[0] = reader.angle()?;
    }
    if (bits & protocol::U_ORIGIN2) != 0 {
        to.origin[1] = reader.coord()?;
    }
    if (bits & protocol::U_ANGLE2) != 0 {
        to.angles[1] = reader.angle()?;
    }
    if (bits & protocol::U_ORIGIN3) != 0 {
        to.origin[2] = reader.coord()?;
    }
    if (bits & protocol::U_ANGLE3) != 0 {
        to.angles[2] = reader.angle()?;
    }
    Ok(to)
}

/// Write an entity removal (`writeRemoveEntity`).
pub fn write_remove_entity(writer: &mut MsgWriter, number: u16) -> Result<(), MsgError> {
    writer.write_short((u32::from(number) | protocol::U_REMOVE) as i16)
}

/// Write the packet-entities terminator (`writePacketEntitiesEnd`).
pub fn write_packet_entities_end(writer: &mut MsgWriter) -> Result<(), MsgError> {
    writer.write_short(0)
}

/// Write a QuakeWorld baseline (`writeQwBaseline`).
pub fn write_baseline(writer: &mut MsgWriter, state: &QwEntityState) -> Result<(), MsgError> {
    writer.write_byte(state.modelindex)?;
    writer.write_byte(state.frame)?;
    writer.write_byte(state.colormap)?;
    writer.write_byte(state.skinnum)?;
    for axis in 0..3 {
        writer.write_coord(state.origin[axis])?;
        writer.write_qw_angle(state.angles[axis])?;
    }
    Ok(())
}

/// Read a QuakeWorld baseline (`readQwBaseline`).
pub fn read_baseline(reader: &mut MsgReader<'_>) -> Result<QwEntityState, MsgError> {
    let (modelindex, frame, colormap, skinnum) = (reader.byte()?, reader.byte()?, reader.byte()?, reader.byte()?);
    let mut state = QwEntityState {
        modelindex,
        frame,
        colormap,
        skinnum,
        ..Default::default()
    };
    for axis in 0..3 {
        state.origin[axis] = reader.coord()?;
        state.angles[axis] = reader.angle()?;
    }
    Ok(state)
}

/// Write a static entity (`writeStatic`); `false` leaves nothing written.
pub fn write_static(writer: &mut MsgWriter, state: &QwEntityState) -> Result<bool, MsgError> {
    if state.modelindex as usize >= MAX_PRECACHE {
        return Ok(false);
    }
    writer.write_byte(protocol::Svc::Spawnstatic as u8)?;
    writer.write_byte(state.modelindex)?;
    writer.write_byte(state.frame)?;
    writer.write_byte(state.colormap)?;
    writer.write_byte(state.skinnum)?;
    for axis in 0..3 {
        writer.write_coord(state.origin[axis])?;
        writer.write_qw_angle(state.angles[axis])?;
    }
    Ok(true)
}

/// Write a static sound (`writeStaticSound`); `false` leaves nothing written.
pub fn write_static_sound(
    writer: &mut MsgWriter,
    origin: [f64; 3],
    sound_num: u16,
    volume: f64,
    attenuation: f64,
) -> Result<bool, MsgError> {
    if sound_num as usize >= MAX_PRECACHE {
        return Ok(false);
    }
    writer.write_byte(protocol::Svc::Spawnstaticsound as u8)?;
    for value in origin {
        writer.write_coord(value)?;
    }
    writer.write_byte(sound_num as u8)?;
    writer.write_byte(float_to_wrapped_i32(volume * 255.0) as u8)?;
    writer.write_byte(float_to_wrapped_i32(attenuation * 64.0) as u8)?;
    Ok(true)
}

/// Write a sound message (`writeSound`); `false` leaves nothing written.
pub fn write_sound(writer: &mut MsgWriter, sound: &QwSoundMessage) -> Result<bool, MsgError> {
    if sound.ent >= 1024 || sound.channel >= 8 || sound.sound_num as usize >= MAX_PRECACHE {
        return Ok(false);
    }
    let mut channel = (u32::from(sound.ent) << 3) | u32::from(sound.channel);
    if sound.volume != nq::DEFAULT_SOUND_PACKET_VOLUME as u8 {
        channel |= protocol::SND_VOLUME;
    }
    if sound.attenuation != nq::DEFAULT_SOUND_PACKET_ATTENUATION {
        channel |= protocol::SND_ATTENUATION;
    }
    writer.write_byte(protocol::Svc::Sound as u8)?;
    writer.write_short(channel as i16)?;
    if (channel & protocol::SND_VOLUME) != 0 {
        writer.write_byte(sound.volume)?;
    }
    if (channel & protocol::SND_ATTENUATION) != 0 {
        writer.write_byte(float_to_wrapped_i32(sound.attenuation * 64.0) as u8)?;
    }
    writer.write_byte(sound.sound_num as u8)?;
    for axis in 0..3 {
        writer.write_coord(sound.origin[axis])?;
    }
    Ok(true)
}

/// Read a sound message body; the opcode is consumed by the caller.
pub fn read_sound(reader: &mut MsgReader<'_>) -> Result<QwSoundMessage, MsgError> {
    let channel = u32::from(reader.short()? as u16);
    let volume = if (channel & protocol::SND_VOLUME) != 0 {
        reader.byte()?
    } else {
        nq::DEFAULT_SOUND_PACKET_VOLUME as u8
    };
    let attenuation = if (channel & protocol::SND_ATTENUATION) != 0 {
        f64::from(reader.byte()?) / 64.0
    } else {
        nq::DEFAULT_SOUND_PACKET_ATTENUATION
    };
    let sound_num = u16::from(reader.byte()?);
    let mut origin = [0.0; 3];
    for slot in &mut origin {
        *slot = reader.coord()?;
    }
    Ok(QwSoundMessage {
        ent: ((channel & 0x3ff8) >> 3) as u16,
        channel: (channel & 7) as u8,
        sound_num,
        volume,
        attenuation,
        origin,
    })
}

/// Write a model index byte (`writeModelIndex`).
pub fn write_model_index(writer: &mut MsgWriter, index: u8) -> Result<(), MsgError> {
    writer.write_byte(index)
}

/// Read a model index byte (`readModelIndex`).
pub fn read_model_index(reader: &mut MsgReader<'_>) -> Result<u8, MsgError> {
    reader.byte()
}

/// Write a sound index byte (`writeSoundIndex`).
pub fn write_sound_index(writer: &mut MsgWriter, index: u8) -> Result<(), MsgError> {
    writer.write_byte(index)
}

/// Read a sound index byte (`readSoundIndex`).
pub fn read_sound_index(reader: &mut MsgReader<'_>) -> Result<u8, MsgError> {
    reader.byte()
}

/// Write a precache count byte (`writePrecacheCount`).
pub fn write_precache_count(writer: &mut MsgWriter, count: u8) -> Result<(), MsgError> {
    writer.write_byte(count)
}

/// Read a precache count byte (`readPrecacheCount`).
pub fn read_precache_count(reader: &mut MsgReader<'_>) -> Result<u8, MsgError> {
    reader.byte()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_state(number: u16) -> QwEntityState {
        QwEntityState {
            number,
            origin: [12.5, -4.25, 100.0],
            angles: [10.0, 90.0, 200.0],
            modelindex: 3,
            frame: 7,
            colormap: 11,
            skinnum: 2,
            effects: 5,
            flags: 0,
            alpha: 0,
            scale: 16,
            solid: true,
        }
    }

    #[test]
    fn usercmd_round_trip() {
        let from = QwUsercmd::default();
        let cmd = QwUsercmd {
            msec: 33,
            angles: [45.0, 90.0, 135.0],
            forwardmove: 400,
            sidemove: -200,
            upmove: 0,
            buttons: 5,
            impulse: 9,
        };
        let mut writer = MsgWriter::new(protocol::MAX_MSGLEN, false);
        write_delta_usercmd(&mut writer, &from, &cmd).unwrap();
        write_delta_usercmd(&mut writer, &cmd, &cmd).unwrap();
        let bytes = writer.bytes().to_vec();

        let mut reader = MsgReader::new(&bytes);
        let decoded = read_delta_usercmd(&mut reader, &from).unwrap();
        assert_eq!(decoded.msec, 33);
        assert_eq!(decoded.forwardmove, 400);
        assert_eq!(decoded.sidemove, -200);
        assert_eq!(decoded.buttons, 5);
        assert!((decoded.angles[0] - 45.0).abs() < 0.01);
        let decoded = read_delta_usercmd(&mut reader, &decoded).unwrap();
        assert_eq!(
            decoded,
            read_delta_usercmd(&mut MsgReader::new(&[0, 33]), &cmd).unwrap()
        );
        reader.finish().unwrap();
    }

    #[test]
    fn delta_entity_round_trip() {
        let from = QwEntityState::default();
        let to = full_state(77);
        let mut writer = MsgWriter::new(protocol::MAX_MSGLEN, false);
        assert!(write_delta_entity(&mut writer, &from, &to, false).unwrap());
        assert!(write_delta_entity(&mut writer, &to, &to, false).unwrap());
        let unchanged_end = writer.cursize();
        write_remove_entity(&mut writer, 9).unwrap();
        assert!(writer.cursize() > unchanged_end);
        write_packet_entities_end(&mut writer).unwrap();
        let bytes = writer.bytes().to_vec();

        let mut reader = MsgReader::new(&bytes);
        let header = read_entity_header(&mut reader).unwrap();
        assert_eq!(header.number, 77);
        assert!(!header.remove);
        let decoded = read_delta_entity(&mut reader, &from, header).unwrap();
        assert_eq!(decoded.modelindex, 3);
        assert!(decoded.solid);
        assert_eq!(decoded.origin, [12.5, -4.25, 100.0]);
        // The donor sets MOREBITS before OR-ing SOLID, so a lone SOLID
        // is masked out of the header word and does not survive alone
        // (qw28.ts `writeDeltaEntity`); other low bits carry it along,
        // as the first update above shows.
        let header = read_entity_header(&mut reader).unwrap();
        assert_eq!((header.number, header.bits), (77, 0));
        let decoded = read_delta_entity(&mut reader, &decoded, header).unwrap();
        assert!(!decoded.solid);
        let header = read_entity_header(&mut reader).unwrap();
        assert_eq!((header.number, header.remove), (9, true));
        let header = read_entity_header(&mut reader).unwrap();
        assert_eq!((header.number, header.bits), (0, 0));
        reader.finish().unwrap();
    }

    #[test]
    fn delta_entity_rejects_wide_numbers() {
        let mut writer = MsgWriter::new(protocol::MAX_MSGLEN, false);
        let to = full_state(MAX_ENTITY_NUMBER);
        assert!(!write_delta_entity(&mut writer, &QwEntityState::default(), &to, true).unwrap());
        assert_eq!(writer.cursize(), 0);
    }

    #[test]
    fn baseline_sound_and_precache_round_trip() {
        let state = full_state(0);
        let sound = QwSoundMessage {
            ent: 700,
            channel: 2,
            sound_num: 33,
            volume: 180,
            attenuation: 0.5,
            origin: [1.0, 2.0, 3.0],
        };
        let mut writer = MsgWriter::new(protocol::MAX_MSGLEN, false);
        write_baseline(&mut writer, &state).unwrap();
        assert!(write_sound(&mut writer, &sound).unwrap());
        assert!(!write_sound(
            &mut writer,
            &QwSoundMessage {
                ent: 1024,
                ..sound.clone()
            }
        )
        .unwrap());
        write_model_index(&mut writer, 12).unwrap();
        write_sound_index(&mut writer, 34).unwrap();
        write_precache_count(&mut writer, 56).unwrap();
        write_protocol(&mut writer).unwrap();
        let bytes = writer.bytes().to_vec();

        let mut reader = MsgReader::new(&bytes);
        let decoded = read_baseline(&mut reader).unwrap();
        assert_eq!((decoded.frame, decoded.skinnum), (7, 2));
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Sound as u8);
        let decoded = read_sound(&mut reader).unwrap();
        assert_eq!((decoded.ent, decoded.channel, decoded.sound_num), (700, 2, 33));
        assert_eq!(decoded.volume, 180);
        assert_eq!(decoded.attenuation, 0.5);
        assert_eq!(read_model_index(&mut reader).unwrap(), 12);
        assert_eq!(read_sound_index(&mut reader).unwrap(), 34);
        assert_eq!(read_precache_count(&mut reader).unwrap(), 56);
        assert_eq!(reader.long().unwrap(), 28);
        reader.finish().unwrap();
    }
}
