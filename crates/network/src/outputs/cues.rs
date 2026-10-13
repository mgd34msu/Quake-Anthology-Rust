//! Native Q2 cue fields stay at the wire boundary, before asset/entity binding.
//! qsrc quake-2/client/cl_parse.c and q2proto_read_gamemsg.inc define sounds;
//! q2proto's common and q2repro muzzle readers define both monster encodings.
use super::{Error, Protocol};
use crate::message::{Encoding, Reader, Writer};
use qa_core::{
    math::{eighth_to_short, short_to_eighth},
    primitives::{
        ClientId, NativeSoundFields, SoundAction, SoundEmitter, SoundEvent, SoundId, SpatialPolicy,
        Vec3,
    },
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
    /// Bound identity and position come from the producing or receiving owner;
    /// quantized native fields are never reconstructed from normalized floats.
    pub fn into_event(
        self,
        sound: SoundId,
        spatial: SpatialPolicy,
        entity: Option<SoundEmitter>,
        position: Vec3,
        client: Option<ClientId>,
        action: SoundAction,
    ) -> SoundEvent {
        SoundEvent {
            looping: None,
            sound,
            source_ordinal: Some(u32::from(self.index)),
            native_fields: Some(NativeSoundFields {
                flags: self.flags,
                volume: self.volume,
                attenuation: self.attenuation,
                time_offset: self.time_offset,
                entity: self.entity,
            }),
            spatial,
            entity,
            channel: i32::from(self.channel),
            position,
            positioned: self.position().is_some(),
            volume: f32::from(self.volume) / 255.0,
            attenuation: f32::from(self.attenuation) / 64.0,
            time_offset: f32::from(self.time_offset) / 1000.0,
            client,
            client_lifetime: None,
            action,
            ..SoundEvent::default()
        }
    }
    pub fn entity(self) -> Option<u16> {
        (self.flags & ENTITY != 0).then_some(self.entity)
    }
    /// Source flags stay byte-exact when its ordinal is unchanged. Remapping
    /// changes only the native index-width presence bit.
    pub fn with_index(mut self, index: u32) -> Result<Self, Error> {
        let index = u16::try_from(index).map_err(|_| Error::Unsupported)?;
        if self.index != index {
            self.index = index;
            if index > u16::from(u8::MAX) {
                self.flags |= INDEX16;
            } else {
                self.flags &= !INDEX16;
            }
        }
        Ok(self)
    }
    pub fn position(self) -> Option<Vec3> {
        (self.flags & POSITION != 0).then_some(self.position)
    }
}
/// Common PCM handles have already been projected into the destination's
/// resource/entity namespaces. This is the one native sound field boundary.
pub fn sound(
    protocol: Protocol,
    event: SoundEvent,
    ordinal: u32,
    entity: Option<u32>,
    positioned: bool,
    output: &mut [u8],
) -> Result<usize, Error> {
    if event.looping.is_some() {
        return Err(Error::Unsupported);
    }
    if let Some(fields) = event.native_fields {
        let mut wire = Sound {
            flags: fields.flags,
            index: u16::try_from(event.source_ordinal.ok_or(Error::Unsupported)?)
                .map_err(|_| Error::Unsupported)?,
            volume: fields.volume,
            attenuation: fields.attenuation,
            time_offset: fields.time_offset,
            entity: if fields.flags & ENTITY != 0 {
                u16::try_from(entity.ok_or(Error::Unsupported)?).map_err(|_| Error::Unsupported)?
            } else {
                fields.entity
            },
            channel: u8::try_from(event.channel).map_err(|_| Error::Unsupported)?,
            position: event.position,
        }
        .with_index(ordinal)?;
        let cue = match event.action {
            SoundAction::Play => {
                if ordinal == 0 || ordinal >= protocol.resource_layout().resources[1].count {
                    return Err(Error::Unsupported);
                }
                if protocol == Protocol::Quake2_34 {
                    if wire.index > u16::from(u8::MAX) {
                        return Err(Error::Unsupported);
                    }
                    wire.flags &= !INDEX16;
                }
                Cue::Sound(wire)
            }
            SoundAction::StartLoop => Cue::StaticSound(wire),
            _ => return Err(Error::Unsupported),
        };
        return write_cue(protocol, project_cue(protocol, cue)?, output);
    }
    if event.action == SoundAction::StartLoop && protocol == Protocol::QuakeWorld28 {
        if ordinal == 0 || ordinal >= 256 || event.time_offset != 0.0 {
            return Err(Error::Unsupported);
        }
        return write_cue(
            protocol,
            Cue::StaticSound(Sound {
                flags: VOLUME | ATTENUATION | POSITION,
                index: ordinal as u16,
                volume: (event.volume * 255.0) as i32 as u8,
                attenuation: (event.attenuation * 64.0) as i32 as u8,
                time_offset: 0,
                entity: 0,
                channel: 0,
                position: event.position,
            }),
            output,
        );
    }
    let format = format(protocol)?;
    if event.action != SoundAction::Play
        || ordinal == 0
        || ordinal
            >= protocol
                .config_layout()
                .ok_or(Error::Unsupported)?
                .resources[1]
                .count
        || entity.is_some_and(|entity| {
            entity as usize >= crate::snapshots::snapshot_limits(protocol).native_numbers
        })
    {
        return Err(Error::Unsupported);
    }
    let mut flags = 0;
    if event.volume != 1.0 {
        flags |= VOLUME;
    }
    if event.attenuation != 1.0 {
        flags |= ATTENUATION;
    }
    if event.time_offset != 0.0 {
        flags |= OFFSET;
    }
    if entity.is_some() {
        flags |= ENTITY;
    }
    if positioned || event.positioned || entity.is_none() {
        flags |= POSITION;
    }
    if ordinal > u32::from(u8::MAX) {
        if format.sound_flags & INDEX16 == 0 {
            return Err(Error::Unsupported);
        }
        flags |= INDEX16;
    }
    write_cue(
        protocol,
        Cue::Sound(Sound {
            flags,
            index: ordinal as u16,
            volume: (event.volume * 255.0) as i32 as u8,
            attenuation: (event.attenuation * 64.0) as i32 as u8,
            time_offset: (event.time_offset * 1000.0) as i32 as u8,
            entity: entity.unwrap_or(0) as u16,
            channel: event.channel as u8 & 7,
            position: event.position,
        }),
        output,
    )
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
    /// svc_spawnstaticsound is persistent level ambience, never a one-shot.
    StaticSound(Sound),
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
/// Resource-relative ordinals have already been translated by configuration
/// publication. Only native representation limits are selected here.
pub(super) fn project_cue(protocol: Protocol, mut cue: Cue) -> Result<Cue, Error> {
    if let Cue::StaticSound(sound) = cue {
        return if matches!(protocol, Protocol::NetQuake15 | Protocol::QuakeWorld28)
            && sound.index <= u8::MAX.into()
        {
            Ok(cue)
        } else {
            Err(Error::Unsupported)
        };
    }
    let format = format(protocol)?;
    match &mut cue {
        Cue::Sound(sound) => {
            if protocol == Protocol::Quake2_34 {
                if sound.index > u8::MAX.into()
                    || sound.entity().is_some_and(|entity| {
                        usize::from(entity)
                            >= crate::snapshots::snapshot_limits(protocol).native_numbers
                    })
                {
                    return Err(Error::Unsupported);
                }
                sound.flags &= !INDEX16;
            }
        }
        Cue::MuzzleFlash(muzzle) => {
            if !format.muzzle_entity(muzzle.entity) {
                return Err(Error::Unsupported);
            }
            if protocol == Protocol::Quake2_34 && muzzle.format == MuzzleFormat::MonsterWide {
                if muzzle.flash > u8::MAX.into() {
                    return Err(Error::Unsupported);
                }
                muzzle.format = MuzzleFormat::Monster;
            }
        }
        Cue::StaticSound(_) => return Err(Error::Unsupported),
    }
    Ok(cue)
}
pub fn read_cue(protocol: Protocol, bytes: &[u8]) -> Result<(Cue, usize), Error> {
    let mut reader = Reader::new(bytes, Encoding::Bytes);
    let opcode = reader.read_bits(8)? as u8;
    if matches!(protocol, Protocol::NetQuake15 | Protocol::QuakeWorld28) {
        if opcode != 29 {
            return Err(Error::Unsupported);
        }
        let position = read_position(&mut reader, false)?;
        let sound = Sound {
            flags: VOLUME | ATTENUATION | POSITION,
            index: reader.read_bits(8)? as u16,
            volume: reader.read_bits(8)? as u8,
            attenuation: reader.read_bits(8)? as u8,
            time_offset: 0,
            entity: 0,
            channel: 0,
            position,
        };
        return Ok((Cue::StaticSound(sound), reader.byte_position()));
    }
    let format = format(protocol)?;
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
                sound.position = read_position(&mut reader, format.float_position)?;
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
    if let Cue::StaticSound(sound) = cue {
        if !matches!(protocol, Protocol::NetQuake15 | Protocol::QuakeWorld28)
            || sound.flags != (VOLUME | ATTENUATION | POSITION)
            || sound.index > u8::MAX.into()
            || sound.time_offset != 0
            || sound.entity != 0
            || sound.channel != 0
        {
            return Err(Error::Unsupported);
        }
        let mut writer = Writer::new(output, Encoding::Bytes);
        writer.write_bits(29, 8)?;
        write_position(&mut writer, sound.position, false)?;
        writer.write_bits(sound.index.into(), 8)?;
        writer.write_bits(sound.volume.into(), 8)?;
        writer.write_bits(sound.attenuation.into(), 8)?;
        return Ok(writer.size());
    }
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
                write_position(&mut writer, sound.position, format.float_position)?;
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
        Cue::StaticSound(_) => return Err(Error::Unsupported),
    }
    Ok(writer.size())
}

fn read_position(reader: &mut Reader<'_>, float: bool) -> Result<Vec3, Error> {
    let mut position = Vec3::default();
    for value in &mut position.0 {
        *value = if float {
            reader.read_float()?
        } else {
            short_to_eighth(reader.read_signed(16)? as i16)
        };
    }
    Ok(position)
}

fn write_position(writer: &mut Writer<'_>, position: Vec3, float: bool) -> Result<(), Error> {
    for value in position.0 {
        if float {
            writer.write_float(value)?;
        } else {
            writer.write_bits(eighth_to_short(value) as u16 as u32, 16)?;
        }
    }
    Ok(())
}
