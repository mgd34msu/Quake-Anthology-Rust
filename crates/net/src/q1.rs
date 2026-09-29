//! NetQuake protocol 15 message codecs.
//!
//! Donor provenance: `createNq15Codec` in
//! `src/network/q1/codecs/nq15.ts` (entity updates, baselines, statics,
//! sounds, clientdata) with field readback order from `readEntity` and
//! `readClientData` in `src/network/q1/netquake.ts`.
//!
//! Only the protocol 15 wire shape is covered here; FitzQuake/RMQ wide
//! extensions (`wide.ts`, `qw29.ts`) are covered by [`crate::q1_wide`].

use qa_core::numeric::float_to_wrapped_i32;

use crate::msg::{MsgError, MsgReader, MsgWriter};
use crate::protocol::q1 as protocol;

/// NetQuake maximum message length (`NQ15_MAX_MSGLEN`).
pub const MAX_MSGLEN: usize = 8000;
/// NetQuake maximum datagram (`NQ15_MAX_DATAGRAM`).
pub const MAX_DATAGRAM: usize = 1024;
/// NetQuake precache limit (`NQ15_MAX_PRECACHE`).
pub const MAX_PRECACHE: usize = 256;

/// Entity origin delta threshold in units (donor `±0.1`).
pub const ORIGIN_EPSILON: f64 = 0.1;

/// Visible entity state shared by updates, baselines, and statics.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EntityState {
    /// Model index (protocol 15 sends the low byte).
    pub modelindex: u16,
    /// Frame (protocol 15 sends the low byte).
    pub frame: u16,
    /// Colormap.
    pub colormap: u8,
    /// Skin.
    pub skin: u8,
    /// Effects.
    pub effects: u8,
    /// Entity alpha (decoder checkpoint only; see [`crate::q1_checkpoint`]).
    pub alpha: u8,
    /// Entity scale (decoder checkpoint only; see [`crate::q1_checkpoint`]).
    pub scale: u8,
    /// Origin.
    pub origin: [f64; 3],
    /// Angles in degrees.
    pub angles: [f64; 3],
}

/// An entity update against a baseline, plus the step flag.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityUpdate {
    /// Current state.
    pub state: EntityState,
    /// Baseline the delta is computed against.
    pub baseline: EntityState,
    /// Step animation (`U_NOLERP`).
    pub step: bool,
}

/// Sound message (`SoundMessageT`).
#[derive(Debug, Clone, PartialEq)]
pub struct SoundMessage {
    /// Entity number (must be below 8192).
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

/// Client data (`ClientdataT`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientData {
    /// View height (default 22).
    pub viewheight: i8,
    /// Ideal pitch.
    pub idealpitch: i8,
    /// Punch angles.
    pub punchangle: [i8; 3],
    /// Velocity (sent divided by 16).
    pub velocity: [i16; 3],
    /// Item bits (always sent).
    pub items: i32,
    /// On ground (no data follows).
    pub onground: bool,
    /// In water (no data follows).
    pub inwater: bool,
    /// Weapon frame.
    pub weaponframe: u8,
    /// Armor value.
    pub armorvalue: u8,
    /// Weapon model index (always sent).
    pub weaponmodelindex: u8,
    /// Health.
    pub health: i16,
    /// Current ammo.
    pub currentammo: u8,
    /// Shells.
    pub ammo_shells: u8,
    /// Nails.
    pub ammo_nails: u8,
    /// Rockets.
    pub ammo_rockets: u8,
    /// Cells.
    pub ammo_cells: u8,
    /// Raw weapon byte on the wire.
    pub weapon: u8,
}

impl ClientData {
    /// Active weapon mask, resolving the donor's `standardQuake` rule.
    #[must_use]
    pub fn active_weapon_mask(&self, standard_quake: bool) -> u32 {
        if standard_quake {
            u32::from(self.weapon)
        } else {
            1u32 << (u32::from(self.weapon) & 31)
        }
    }
}

/// Write the protocol version long (`writeProtocol`).
pub fn write_protocol(writer: &mut MsgWriter) -> Result<(), MsgError> {
    writer.write_long(protocol::PROTOCOL_NETQUAKE as i32)
}

/// Compute entity-update bits for an update against its baseline.
#[must_use]
pub fn entity_bits(number: u16, update: &EntityUpdate) -> u32 {
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
        bits |= protocol::U_NOLERP;
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
    if number >= 256 {
        bits |= protocol::U_LONGENTITY;
    }
    if bits >= 256 {
        bits |= protocol::U_MOREBITS;
    }
    bits
}

/// Write an entity update (`writeEntityUpdate`).
pub fn write_entity_update(writer: &mut MsgWriter, number: u16, update: &EntityUpdate) -> Result<(), MsgError> {
    let bits = entity_bits(number, update);
    writer.write_byte((bits | protocol::U_SIGNAL) as u8)?;
    if (bits & protocol::U_MOREBITS) != 0 {
        writer.write_byte((bits >> 8) as u8)?;
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
        writer.write_coord(update.state.origin[0])?;
    }
    if (bits & protocol::U_ANGLE1) != 0 {
        writer.write_angle(update.state.angles[0])?;
    }
    if (bits & protocol::U_ORIGIN2) != 0 {
        writer.write_coord(update.state.origin[1])?;
    }
    if (bits & protocol::U_ANGLE2) != 0 {
        writer.write_angle(update.state.angles[1])?;
    }
    if (bits & protocol::U_ORIGIN3) != 0 {
        writer.write_coord(update.state.origin[2])?;
    }
    if (bits & protocol::U_ANGLE3) != 0 {
        writer.write_angle(update.state.angles[2])?;
    }
    Ok(())
}

/// Decoded entity update: number, merged state, and step flag.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedEntityUpdate {
    /// Entity number.
    pub number: u16,
    /// Baseline with transmitted fields overlaid.
    pub state: EntityState,
    /// Step flag (`U_STEP`).
    pub step: bool,
}

/// Read an entity update (`readEntity` with the NQ15 codec).
///
/// The `U_SIGNAL` marker bit is accepted and ignored exactly like the
/// donor, which dispatches on it and never tests it as data.
pub fn read_entity_update(reader: &mut MsgReader<'_>, baseline: &EntityState) -> Result<DecodedEntityUpdate, MsgError> {
    let mut bits = u32::from(reader.byte()?);
    if (bits & protocol::U_MOREBITS) != 0 {
        bits |= u32::from(reader.byte()?) << 8;
    }
    let number = if (bits & protocol::U_LONGENTITY) != 0 {
        reader.short()? as u16
    } else {
        u16::from(reader.byte()?)
    };
    let mut state = baseline.clone();
    if (bits & protocol::U_MODEL) != 0 {
        state.modelindex = u16::from(reader.byte()?);
    }
    if (bits & protocol::U_FRAME) != 0 {
        state.frame = u16::from(reader.byte()?);
    }
    if (bits & protocol::U_COLORMAP) != 0 {
        state.colormap = reader.byte()?;
    }
    if (bits & protocol::U_SKIN) != 0 {
        state.skin = reader.byte()?;
    }
    if (bits & protocol::U_EFFECTS) != 0 {
        state.effects = reader.byte()?;
    }
    if (bits & protocol::U_ORIGIN1) != 0 {
        state.origin[0] = reader.coord()?;
    }
    if (bits & protocol::U_ANGLE1) != 0 {
        state.angles[0] = reader.angle()?;
    }
    if (bits & protocol::U_ORIGIN2) != 0 {
        state.origin[1] = reader.coord()?;
    }
    if (bits & protocol::U_ANGLE2) != 0 {
        state.angles[1] = reader.angle()?;
    }
    if (bits & protocol::U_ORIGIN3) != 0 {
        state.origin[2] = reader.coord()?;
    }
    if (bits & protocol::U_ANGLE3) != 0 {
        state.angles[2] = reader.angle()?;
    }
    Ok(DecodedEntityUpdate {
        number,
        state,
        step: (bits & protocol::U_STEP) != 0,
    })
}

/// Write a spawn baseline (`writeBaseline`).
///
/// Wide model/frame values are clamped to zero, matching the donor.
pub fn write_baseline(writer: &mut MsgWriter, number: u16, baseline: &EntityState) -> Result<(), MsgError> {
    let modelindex = if baseline.modelindex & 0xff00 != 0 {
        0
    } else {
        baseline.modelindex as u8
    };
    let frame = if baseline.frame & 0xff00 != 0 {
        0
    } else {
        baseline.frame as u8
    };
    writer.write_byte(protocol::Svc::Spawnbaseline as u8)?;
    writer.write_short(number as i16)?;
    writer.write_byte(modelindex)?;
    writer.write_byte(frame)?;
    writer.write_byte(baseline.colormap)?;
    writer.write_byte(baseline.skin)?;
    for axis in 0..3 {
        writer.write_coord(baseline.origin[axis])?;
        writer.write_angle(baseline.angles[axis])?;
    }
    Ok(())
}

/// Read a spawn baseline body (`readBaseline`); the opcode is consumed by the caller.
pub fn read_baseline(reader: &mut MsgReader<'_>) -> Result<(u16, EntityState), MsgError> {
    let number = reader.short()? as u16;
    let (modelindex, frame, colormap, skin) = (
        u16::from(reader.byte()?),
        u16::from(reader.byte()?),
        reader.byte()?,
        reader.byte()?,
    );
    let mut state = EntityState {
        modelindex,
        frame,
        colormap,
        skin,
        ..Default::default()
    };
    for axis in 0..3 {
        state.origin[axis] = reader.coord()?;
        state.angles[axis] = reader.angle()?;
    }
    Ok((number, state))
}

/// Write a static entity (`writeStatic`); `false` leaves nothing written.
#[must_use = "a false return means the static was skipped"]
pub fn write_static(writer: &mut MsgWriter, state: &EntityState) -> Result<bool, MsgError> {
    if state.modelindex & 0xff00 != 0 || state.frame & 0xff00 != 0 {
        return Ok(false);
    }
    writer.write_byte(protocol::Svc::Spawnstatic as u8)?;
    writer.write_byte(state.modelindex as u8)?;
    writer.write_byte(state.frame as u8)?;
    writer.write_byte(state.colormap)?;
    writer.write_byte(state.skin)?;
    for axis in 0..3 {
        writer.write_coord(state.origin[axis])?;
        writer.write_angle(state.angles[axis])?;
    }
    Ok(true)
}

/// Read a static entity body; the opcode is consumed by the caller.
pub fn read_static(reader: &mut MsgReader<'_>) -> Result<EntityState, MsgError> {
    let (modelindex, frame, colormap, skin) = (
        u16::from(reader.byte()?),
        u16::from(reader.byte()?),
        reader.byte()?,
        reader.byte()?,
    );
    let mut state = EntityState {
        modelindex,
        frame,
        colormap,
        skin,
        ..Default::default()
    };
    for axis in 0..3 {
        state.origin[axis] = reader.coord()?;
        state.angles[axis] = reader.angle()?;
    }
    Ok(state)
}

/// Write a static sound (`writeStaticSound`); `false` leaves nothing written.
pub fn write_static_sound(
    writer: &mut MsgWriter,
    origin: [f64; 3],
    sound_num: u16,
    volume: f64,
    attenuation: f64,
) -> Result<bool, MsgError> {
    if sound_num > 255 {
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
#[must_use = "a false return means the sound was skipped"]
pub fn write_sound(writer: &mut MsgWriter, sound: &SoundMessage) -> Result<bool, MsgError> {
    if sound.ent >= 8192 || sound.sound_num >= 256 || sound.channel >= 8 {
        return Ok(false);
    }
    let mut mask = 0;
    if sound.volume != protocol::DEFAULT_SOUND_PACKET_VOLUME as u8 {
        mask |= protocol::SND_VOLUME;
    }
    if sound.attenuation != protocol::DEFAULT_SOUND_PACKET_ATTENUATION {
        mask |= protocol::SND_ATTENUATION;
    }
    writer.write_byte(protocol::Svc::Sound as u8)?;
    writer.write_byte(mask as u8)?;
    if (mask & protocol::SND_VOLUME) != 0 {
        writer.write_byte(sound.volume)?;
    }
    if (mask & protocol::SND_ATTENUATION) != 0 {
        writer.write_byte(float_to_wrapped_i32(sound.attenuation * 64.0) as u8)?;
    }
    writer.write_short(((u32::from(sound.ent) << 3) | u32::from(sound.channel)) as i16)?;
    writer.write_byte(sound.sound_num as u8)?;
    for axis in 0..3 {
        writer.write_coord(sound.origin[axis])?;
    }
    Ok(true)
}

/// Read a sound message body; the opcode and field mask are consumed here.
pub fn read_sound(reader: &mut MsgReader<'_>) -> Result<SoundMessage, MsgError> {
    let mask = u32::from(reader.byte()?);
    let volume = if (mask & protocol::SND_VOLUME) != 0 {
        reader.byte()?
    } else {
        protocol::DEFAULT_SOUND_PACKET_VOLUME as u8
    };
    let attenuation = if (mask & protocol::SND_ATTENUATION) != 0 {
        f64::from(reader.byte()?) / 64.0
    } else {
        protocol::DEFAULT_SOUND_PACKET_ATTENUATION
    };
    let channel = reader.short()? as u16;
    let sound_num = u16::from(reader.byte()?);
    let mut origin = [0.0; 3];
    for slot in &mut origin {
        *slot = reader.coord()?;
    }
    Ok(SoundMessage {
        ent: channel >> 3,
        channel: (channel & 7) as u8,
        sound_num,
        volume,
        attenuation,
        origin,
    })
}

/// Compute clientdata bits for a client state.
#[must_use]
pub fn clientdata_bits(data: &ClientData) -> u32 {
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
    bits
}

/// Write client data (`writeClientdata`).
///
/// `standard_quake` selects the weapon-byte rule: the raw weapon value
/// for standard Quake, otherwise the first set bit position.
pub fn write_clientdata(writer: &mut MsgWriter, data: &ClientData, standard_quake: bool) -> Result<(), MsgError> {
    let bits = clientdata_bits(data);
    writer.write_byte(protocol::Svc::Clientdata as u8)?;
    writer.write_short(bits as i16)?;
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
        writer.write_byte(data.weaponframe)?;
    }
    if (bits & protocol::SU_ARMOR) != 0 {
        writer.write_byte(data.armorvalue)?;
    }
    writer.write_byte(data.weaponmodelindex)?;
    writer.write_short(data.health)?;
    writer.write_byte(data.currentammo)?;
    writer.write_byte(data.ammo_shells)?;
    writer.write_byte(data.ammo_nails)?;
    writer.write_byte(data.ammo_rockets)?;
    writer.write_byte(data.ammo_cells)?;
    if standard_quake {
        writer.write_byte(data.weapon)?;
    } else {
        let mut weapon = 0;
        for index in 0..32 {
            if (u32::from(data.weapon) & (1 << index)) != 0 {
                weapon = index;
                break;
            }
        }
        writer.write_byte(weapon as u8)?;
    }
    Ok(())
}

/// Read client data (`readClientData` with the NQ15 codec).
pub fn read_clientdata(reader: &mut MsgReader<'_>) -> Result<ClientData, MsgError> {
    let bits = u32::from(reader.short()? as u16);
    let viewheight = if (bits & protocol::SU_VIEWHEIGHT) != 0 {
        reader.char()?
    } else {
        protocol::DEFAULT_VIEWHEIGHT as i8
    };
    let idealpitch = if (bits & protocol::SU_IDEALPITCH) != 0 {
        reader.char()?
    } else {
        0
    };
    let mut punchangle = [0; 3];
    let mut velocity = [0; 3];
    for axis in 0..3 {
        punchangle[axis] = if (bits & (protocol::SU_PUNCH1 << axis)) != 0 {
            reader.char()?
        } else {
            0
        };
        velocity[axis] = if (bits & (protocol::SU_VELOCITY1 << axis)) != 0 {
            i16::from(reader.char()?) * 16
        } else {
            0
        };
    }
    let items = reader.long()?;
    let weaponframe = if (bits & protocol::SU_WEAPONFRAME) != 0 {
        reader.byte()?
    } else {
        0
    };
    let armorvalue = if (bits & protocol::SU_ARMOR) != 0 {
        reader.byte()?
    } else {
        0
    };
    let weaponmodelindex = if (bits & protocol::SU_WEAPON) != 0 {
        reader.byte()?
    } else {
        0
    };
    let health = reader.short()?;
    let currentammo = reader.byte()?;
    let ammo_shells = reader.byte()?;
    let ammo_nails = reader.byte()?;
    let ammo_rockets = reader.byte()?;
    let ammo_cells = reader.byte()?;
    let weapon = reader.byte()?;
    Ok(ClientData {
        viewheight,
        idealpitch,
        punchangle,
        velocity,
        items,
        onground: (bits & protocol::SU_ONGROUND) != 0,
        inwater: (bits & protocol::SU_INWATER) != 0,
        weaponframe,
        armorvalue,
        weaponmodelindex,
        health,
        currentammo,
        ammo_shells,
        ammo_nails,
        ammo_rockets,
        ammo_cells,
        weapon,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_state() -> EntityState {
        EntityState {
            modelindex: 3,
            frame: 7,
            colormap: 11,
            skin: 2,
            effects: 5,
            alpha: 0,
            scale: 16,
            origin: [12.5, -4.25, 100.0],
            angles: [0.0, 90.0, 180.0],
        }
    }

    #[test]
    fn entity_update_round_trip() {
        for number in [7u16, 300u16] {
            let update = EntityUpdate {
                state: full_state(),
                baseline: EntityState::default(),
                step: true,
            };
            let mut writer = MsgWriter::new(MAX_MSGLEN, false);
            write_entity_update(&mut writer, number, &update).unwrap();
            let bytes = writer.bytes().to_vec();
            assert_ne!(bytes[0] & protocol::U_SIGNAL as u8, 0);
            let mut reader = MsgReader::new(&bytes);
            let decoded = read_entity_update(&mut reader, &EntityState::default()).unwrap();
            reader.finish().unwrap();
            assert_eq!(decoded.number, number);
            assert!(decoded.step);
            assert_eq!(decoded.state.modelindex, 3);
            assert_eq!(decoded.state.frame, 7);
            assert_eq!(decoded.state.colormap, 11);
            assert_eq!(decoded.state.origin, [12.5, -4.25, 100.0]);
            assert_eq!(decoded.state.angles[1], 90.0);
        }
    }

    #[test]
    fn entity_update_omits_unchanged_fields() {
        let baseline = full_state();
        let update = EntityUpdate {
            state: baseline.clone(),
            baseline: baseline.clone(),
            step: false,
        };
        assert_eq!(entity_bits(9, &update), 0);
        let mut writer = MsgWriter::new(MAX_MSGLEN, false);
        write_entity_update(&mut writer, 9, &update).unwrap();
        assert_eq!(writer.bytes(), &[protocol::U_SIGNAL as u8, 9]);

        let mut near = baseline.clone();
        near.origin[0] += 0.05;
        let update = EntityUpdate {
            state: near,
            baseline,
            step: false,
        };
        assert_eq!(entity_bits(9, &update), 0);
    }

    #[test]
    fn baseline_and_static_round_trip() {
        let state = full_state();
        let mut writer = MsgWriter::new(MAX_MSGLEN, false);
        write_baseline(&mut writer, 41, &state).unwrap();
        assert!(write_static(&mut writer, &state).unwrap());
        let wide = EntityState {
            modelindex: 0x1200,
            frame: 0x1300,
            ..full_state()
        };
        assert!(!write_static(&mut writer, &wide).unwrap());
        write_baseline(&mut writer, 42, &wide).unwrap();
        let bytes = writer.bytes().to_vec();

        let mut reader = MsgReader::new(&bytes);
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Spawnbaseline as u8);
        let (number, decoded) = read_baseline(&mut reader).unwrap();
        assert_eq!((number, decoded.modelindex, decoded.frame), (41, 3, 7));
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Spawnstatic as u8);
        let decoded = read_static(&mut reader).unwrap();
        assert_eq!((decoded.skin, decoded.origin[0]), (2, 12.5));
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Spawnbaseline as u8);
        let (number, decoded) = read_baseline(&mut reader).unwrap();
        assert_eq!((number, decoded.modelindex, decoded.frame), (42, 0, 0));
        reader.finish().unwrap();
    }

    #[test]
    fn sound_round_trip_with_defaults() {
        let loud = SoundMessage {
            ent: 512,
            channel: 3,
            sound_num: 44,
            volume: 200,
            attenuation: 2.0,
            origin: [1.0, 2.0, 3.0],
        };
        let quiet = SoundMessage {
            ent: 5,
            channel: 1,
            sound_num: 9,
            volume: protocol::DEFAULT_SOUND_PACKET_VOLUME as u8,
            attenuation: protocol::DEFAULT_SOUND_PACKET_ATTENUATION,
            origin: [0.0, 0.0, 0.0],
        };
        let mut writer = MsgWriter::new(MAX_MSGLEN, false);
        assert!(write_sound(&mut writer, &loud).unwrap());
        assert!(write_sound(&mut writer, &quiet).unwrap());
        assert!(!write_sound(
            &mut writer,
            &SoundMessage {
                ent: 8192,
                ..loud.clone()
            }
        )
        .unwrap());
        let bytes = writer.bytes().to_vec();

        let mut reader = MsgReader::new(&bytes);
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Sound as u8);
        let decoded = read_sound(&mut reader).unwrap();
        assert_eq!(decoded.ent, 512);
        assert_eq!(decoded.volume, 200);
        assert_eq!(decoded.attenuation, 2.0);
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Sound as u8);
        let decoded = read_sound(&mut reader).unwrap();
        assert_eq!(decoded.volume, protocol::DEFAULT_SOUND_PACKET_VOLUME as u8);
        assert_eq!(decoded.attenuation, protocol::DEFAULT_SOUND_PACKET_ATTENUATION);
        reader.finish().unwrap();
    }

    #[test]
    fn clientdata_round_trip() {
        let data = ClientData {
            viewheight: 24,
            idealpitch: -3,
            punchangle: [1, 0, -2],
            velocity: [16, -32, 0],
            items: 0x1234_5678,
            onground: true,
            inwater: false,
            weaponframe: 4,
            armorvalue: 100,
            weaponmodelindex: 6,
            health: 87,
            currentammo: 12,
            ammo_shells: 40,
            ammo_nails: 60,
            ammo_rockets: 8,
            ammo_cells: 30,
            weapon: 5,
        };
        let mut writer = MsgWriter::new(MAX_MSGLEN, false);
        write_clientdata(&mut writer, &data, true).unwrap();
        write_protocol(&mut writer).unwrap();
        let bytes = writer.bytes().to_vec();

        let mut reader = MsgReader::new(&bytes);
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Clientdata as u8);
        let decoded = read_clientdata(&mut reader).unwrap();
        assert_eq!(decoded, data);
        assert_eq!(decoded.active_weapon_mask(true), 5);
        assert_eq!(decoded.active_weapon_mask(false), 1 << 5);
        assert_eq!(reader.long().unwrap(), 15);
        reader.finish().unwrap();
    }
}
