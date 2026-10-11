//! Native Q2 cue fields stay at the wire boundary, before asset/entity binding.
//! qsrc quake-2/client/cl_parse.c and q2proto_read_gamemsg.inc define sounds;
//! q2proto's common and q2repro muzzle readers define both monster encodings.
use super::{Error, Protocol};
use crate::message::{Encoding, Reader, Writer};
use qa_core::{
    math::{eighth_to_short, short_to_eighth},
    primitives::Vec3,
};

const VOLUME: u8 = 1;
const ATTENUATION: u8 = 2;
const POSITION: u8 = 4;
const ENTITY: u8 = 8;
const OFFSET: u8 = 16;
const INDEX16: u8 = 32;

/// Presence bits retain explicit defaults when a native payload is re-encoded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sound {
    pub flags: u8,
    pub index: u16,
    pub volume: u8,
    pub attenuation: u8,
    pub time_offset: u8,
    pub entity: u16,
    pub channel: u8,
    pub position: Vec3,
}
impl Sound {
    pub fn entity(self) -> Option<u16> {
        (self.flags & ENTITY != 0).then_some(self.entity)
    }
    pub fn position(self) -> Option<Vec3> {
        (self.flags & POSITION != 0).then_some(self.position)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MuzzleFormat {
    Player,
    Monster,
    MonsterWide,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MuzzleFlash {
    pub format: MuzzleFormat,
    pub entity: u16,
    pub flash: u16,
    pub silenced: bool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cue {
    Sound(Sound),
    MuzzleFlash(MuzzleFlash),
}
struct Format {
    float_position: bool,
    extended_monster: bool,
    sound_flags: u8,
}
impl Format {
    fn muzzle_entity(&self, entity: u16) -> bool {
        entity != 0 && entity < if self.extended_monster { 8192 } else { 1024 }
    }
}
fn format(protocol: Protocol) -> Result<Format, Error> {
    match protocol {
        Protocol::Quake2_34 => Ok(Format {
            float_position: false,
            extended_monster: false,
            sound_flags: 31,
        }),
        Protocol::Quake2Repro1038 => Ok(Format {
            float_position: true,
            extended_monster: true,
            sound_flags: 63,
        }),
        _ => Err(Error::Unsupported),
    }
}
pub fn read_cue(protocol: Protocol, bytes: &[u8]) -> Result<(Cue, usize), Error> {
    let format = format(protocol)?;
    let mut reader = Reader::new(bytes, Encoding::Bytes);
    let opcode = reader.read_bits(8)? as u8;
    let cue = match opcode {
        9 => {
            let flags = reader.read_bits(8)? as u8;
            if flags & !format.sound_flags != 0 {
                return Err(Error::Unsupported);
            }
            let mut sound = Sound {
                flags,
                index: reader.read_bits(if flags & INDEX16 != 0 { 16 } else { 8 })? as u16,
                volume: 255,
                attenuation: 64,
                time_offset: 0,
                entity: 0,
                channel: 0,
                position: Vec3::default(),
            };
            for (flag, output) in [
                (VOLUME, &mut sound.volume),
                (ATTENUATION, &mut sound.attenuation),
                (OFFSET, &mut sound.time_offset),
            ] {
                if flags & flag != 0 {
                    *output = reader.read_bits(8)? as u8;
                }
            }
            if flags & ENTITY != 0 {
                let channel = reader.read_bits(16)? as u16;
                sound.entity = channel >> 3;
                sound.channel = channel as u8 & 7;
            }
            if flags & POSITION != 0 {
                for value in &mut sound.position.0 {
                    *value = if format.float_position {
                        reader.read_float()?
                    } else {
                        short_to_eighth(reader.read_signed(16)? as i16)
                    };
                }
            }
            Cue::Sound(sound)
        }
        1 | 2 | 32 => {
            let kind = match opcode {
                1 => MuzzleFormat::Player,
                2 => MuzzleFormat::Monster,
                32 if format.extended_monster => MuzzleFormat::MonsterWide,
                _ => return Err(Error::Unsupported),
            };
            let mut entity = reader.read_bits(16)? as u16;
            let mut flash = reader.read_bits(if kind == MuzzleFormat::MonsterWide {
                16
            } else {
                8
            })? as u16;
            let silenced = kind == MuzzleFormat::Player && flash & 128 != 0;
            if kind == MuzzleFormat::Player {
                flash &= 127;
            }
            if kind == MuzzleFormat::Monster && format.extended_monster {
                flash |= (entity >> 13) << 8;
                entity &= 8191;
            }
            if !format.muzzle_entity(entity) {
                return Err(Error::Message(
                    reader.error(crate::message::ErrorKind::Width),
                ));
            }
            Cue::MuzzleFlash(MuzzleFlash {
                format: kind,
                entity,
                flash,
                silenced,
            })
        }
        _ => return Err(Error::Opcode),
    };
    Ok((cue, reader.byte_position()))
}

pub fn write_cue(protocol: Protocol, cue: Cue, output: &mut [u8]) -> Result<usize, Error> {
    let format = format(protocol)?;
    let mut writer = Writer::new(output, Encoding::Bytes);
    match cue {
        Cue::Sound(sound) => {
            if sound.flags & !format.sound_flags != 0
                || (sound.flags & INDEX16 == 0 && sound.index > 255)
                || (sound.flags & ENTITY != 0 && (sound.entity >= 8192 || sound.channel >= 8))
            {
                return Err(Error::Unsupported);
            }
            writer.write_bits(9, 8)?;
            writer.write_bits(sound.flags.into(), 8)?;
            writer.write_bits(
                sound.index.into(),
                if sound.flags & INDEX16 != 0 { 16 } else { 8 },
            )?;
            for (flag, value) in [
                (VOLUME, sound.volume),
                (ATTENUATION, sound.attenuation),
                (OFFSET, sound.time_offset),
            ] {
                if sound.flags & flag != 0 {
                    writer.write_bits(value.into(), 8)?;
                }
            }
            if sound.flags & ENTITY != 0 {
                writer.write_bits(
                    (u32::from(sound.entity) << 3) | u32::from(sound.channel),
                    16,
                )?;
            }
            if sound.flags & POSITION != 0 {
                for value in sound.position.0 {
                    if format.float_position {
                        writer.write_float(value)?;
                    } else {
                        writer.write_bits(eighth_to_short(value) as u16 as u32, 16)?;
                    }
                }
            }
        }
        Cue::MuzzleFlash(muzzle) => {
            if !format.muzzle_entity(muzzle.entity) {
                return Err(Error::Unsupported);
            }
            let (opcode, entity, flash, width) = match muzzle.format {
                MuzzleFormat::Player if muzzle.flash < 128 => (
                    1,
                    muzzle.entity,
                    muzzle.flash | if muzzle.silenced { 128 } else { 0 },
                    8,
                ),
                MuzzleFormat::Monster
                    if !muzzle.silenced
                        && muzzle.flash <= if format.extended_monster { 2047 } else { 255 } =>
                {
                    (
                        2,
                        muzzle.entity | ((muzzle.flash >> 8) << 13),
                        muzzle.flash & 255,
                        8,
                    )
                }
                MuzzleFormat::MonsterWide if format.extended_monster && !muzzle.silenced => {
                    (32, muzzle.entity, muzzle.flash, 16)
                }
                _ => return Err(Error::Unsupported),
            };
            writer.write_bits(opcode, 8)?;
            writer.write_bits(entity.into(), 16)?;
            writer.write_bits(flash.into(), width)?;
        }
    }
    Ok(writer.size())
}
