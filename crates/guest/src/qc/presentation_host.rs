//! `PF_*` presentation/precache builtins, `Write*` message builtins, and the
//! QEX finale acknowledgement latch.
//!
//! Ported from donor `src/compat/qc/presentation-host.ts`
//! (`createQcPresentationBindings`, `QcBroadcastMessages`,
//! `QcFinaleAcknowledgement`, `savedQcActor`, `captureQcDestination`,
//! `readQcDestination`).
//!
//! Local mirrors: [`NqMessage`] and [`QwMessage`] mirror the NetQuake and
//! QuakeWorld wire shapes from `src/network/q1/netquake.ts` and
//! `src/network/q1/quakeworld.ts`; [`MsgBuffer`] mirrors `SizeBuf` plus the
//! `MSG_Write*` codecs from `src/network/q1/message.ts`; [`QcMessageRouter`]
//! mirrors the routed-message services; checkpoint structs mirror the save
//! rows. Temporary-entity projection reuses `super::message_effects`.

use std::collections::HashMap;

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{Bounds, Vec3};

use super::message_effects::{quake_temporary_event, QcBroadcastEffect, TempEntityEffect};
use crate::error::GuestError;
use crate::fields::FieldTable;

/// Unreliable datagram budget.
pub const MAX_DATAGRAM: usize = 1024;
/// Reliable message budget.
pub const MAX_MSGLEN: usize = 8000;
/// Maximum QW signon buffers.
pub const MAX_SIGNON_BUFFERS: u32 = 7;
/// Maximum routed buffer capacity (`MAX_MSGLEN * 5`).
pub const MAX_ROUTED_BYTES: usize = MAX_MSGLEN * 5;
/// Number of light styles.
pub const LIGHT_STYLE_COUNT: i32 = 64;

/// QC network dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKind {
    /// NetQuake.
    NetQuake,
    /// QuakeWorld.
    QuakeWorld,
}

/// Multicast visibility scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibilityScope {
    /// All clients.
    All,
    /// Potentially visible set.
    Pvs,
    /// Potentially hearable set.
    Phs,
}

/// QC message destination.
#[derive(Debug, Clone, PartialEq)]
pub enum QcMessageDestination {
    /// Broadcast datagram.
    Broadcast {
        /// Reliable delivery.
        reliable: bool,
    },
    /// Single client (always reliable).
    Client {
        /// Recipient.
        actor: ActorId,
    },
    /// Signon buffer.
    Signon,
    /// Multicast from an origin.
    Multicast {
        /// Multicast origin.
        origin: Vec3,
        /// Visibility scope.
        visibility: VisibilityScope,
        /// Reliable delivery.
        reliable: bool,
    },
}

/// Presentation event emitted to the shared sink.
#[derive(Debug, Clone, PartialEq)]
pub enum QcPresentationEvent {
    /// Positional sound.
    Sound {
        /// Emitting actor.
        actor: ActorId,
        /// Sound path.
        path: String,
        /// Source channel.
        channel: Q1SoundChannel,
        /// Volume 0-1.
        volume: f32,
        /// Attenuation.
        attenuation: f32,
    },
    /// Ambient sound emitter.
    Ambient {
        /// Emitter origin.
        origin: Vec3,
        /// Sound path.
        path: String,
        /// Volume.
        volume: f32,
        /// Attenuation.
        attenuation: f32,
    },
    /// Particle burst.
    Particles {
        /// Burst origin.
        origin: Vec3,
        /// Burst direction.
        direction: Vec3,
        /// Palette color.
        color: i32,
        /// Particle count.
        count: i32,
    },
    /// Light style pattern.
    Lightstyle {
        /// Style index.
        style: i32,
        /// Light pattern.
        pattern: String,
    },
    /// Server console command.
    ServerCommand {
        /// Command text.
        text: String,
    },
    /// Static model placement.
    StaticModel {
        /// Model path.
        path: String,
        /// Model frame.
        frame: i32,
        /// Color map.
        color_map: i32,
        /// Skin.
        skin: i32,
        /// Placement origin.
        origin: Vec3,
        /// Placement angles.
        angles: Vec3,
    },
}

/// Q1 sound channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1SoundChannel {
    /// Automatic channel.
    Auto,
    /// Weapon channel.
    Weapon,
    /// Voice channel.
    Voice,
    /// Item channel.
    Item,
    /// Body channel.
    Body,
    /// Raw channel 5-7.
    Raw(u8),
}

/// Precached resource record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcPrecachedResource {
    /// Precache index.
    pub index: i32,
    /// Requested path.
    pub path: String,
}

/// Precache table kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrecacheKind {
    /// Model table.
    Model,
    /// Sound table.
    Sound,
}

/// Client console message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage {
    /// Console print with level.
    Print {
        /// Print level.
        level: i32,
        /// Message text.
        text: String,
    },
    /// Center-screen print.
    CenterPrint {
        /// Message text.
        text: String,
    },
    /// Client command text.
    CommandText {
        /// Command text.
        text: String,
    },
}

/// `sprint` / `centerprint` / `stuffcmd` selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientPrintKind {
    /// Console print.
    Sprint,
    /// Center-screen print.
    CenterPrint,
    /// Client command.
    StuffCmd,
}

/// NetQuake message subset writable from QC.
#[derive(Debug, Clone, PartialEq)]
pub enum NqMessage {
    /// Console print.
    Print {
        /// Message text.
        text: String,
    },
    /// Center-screen print.
    CenterPrint {
        /// Message text.
        text: String,
    },
    /// Client command text.
    StuffText {
        /// Command text.
        text: String,
    },
    /// Temporary entity.
    TempEntity {
        /// Decoded effect.
        effect: TempEntityEffect,
    },
    /// Set view entity.
    SetView {
        /// View entity slot.
        entity: u16,
    },
    /// Positional sound.
    Sound {
        /// Sound entity slot.
        entity: u16,
        /// Sound channel.
        channel: u8,
        /// Sound index.
        index: u8,
        /// Sound origin.
        origin: Vec3,
        /// Volume 0-255.
        volume: u8,
        /// Attenuation.
        attenuation: f32,
    },
}

/// QuakeWorld message subset writable from QC or routed by presenters.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum QwMessage {
    /// Console print with level.
    Print {
        /// Print level.
        level: i32,
        /// Message text.
        text: String,
    },
    /// Center-screen print.
    CenterPrint {
        /// Message text.
        text: String,
    },
    /// Client command text.
    StuffText {
        /// Command text.
        text: String,
    },
    /// Temporary entity.
    TempEntity {
        /// Decoded effect.
        effect: TempEntityEffect,
    },
    /// Set view entity.
    SetView {
        /// View entity slot.
        entity: u16,
    },
    /// Positional sound.
    Sound {
        /// Sound entity slot.
        entity: u16,
        /// Sound channel.
        channel: u8,
        /// Sound index.
        index: u8,
        /// Sound origin.
        origin: Vec3,
        /// Volume 0-255.
        volume: u8,
        /// Attenuation.
        attenuation: f32,
    },
    /// Stop a looping sound.
    StopSound {
        /// Sound entity slot.
        entity: u16,
        /// Sound channel.
        channel: u8,
    },
    /// Muzzle flash.
    MuzzleFlash {
        /// Flash entity slot.
        entity: u16,
    },
    /// Kick notice.
    Kick,
    /// Intermission camera.
    Intermission {
        /// Camera origin.
        origin: Vec3,
        /// Camera angles.
        angles: Vec3,
    },
    /// CD track change.
    CdTrack {
        /// Track number.
        track: u8,
    },
    /// No-op.
    Nop,
    /// Player stat update.
    Stat {
        /// Stat index.
        stat: u8,
        /// Stat value.
        value: i32,
    },
    /// Finale text.
    Finale {
        /// Finale text.
        text: String,
    },
    /// Set entity angles.
    SetAngle {
        /// Entity slot.
        entity: u16,
        /// New angles.
        angles: Vec3,
    },
    /// Light style pattern.
    LightStyle {
        /// Style index.
        style: u8,
        /// Light pattern.
        pattern: String,
    },
    /// Static entity baseline.
    Static {
        /// Model index.
        model: u8,
        /// Frame.
        frame: u8,
        /// Color map.
        colormap: u8,
        /// Skin.
        skin: u8,
        /// Origin.
        origin: Vec3,
        /// Angles.
        angles: Vec3,
    },
    /// Static sound emitter.
    StaticSound {
        /// Emitter origin.
        origin: Vec3,
        /// Sound index.
        index: u8,
        /// Volume.
        volume: u8,
        /// Attenuation.
        attenuation: u8,
    },
    /// Damage feedback.
    Damage {
        /// Armor damage.
        armor: u8,
        /// Blood damage.
        blood: u8,
        /// Damage origin.
        origin: Vec3,
    },
    /// Pause flag.
    Pause {
        /// Paused.
        paused: bool,
    },
    /// Monster kill counter.
    KilledMonster,
    /// Secret counter.
    FoundSecret,
    /// End-of-level screen.
    SellScreen,
}

/// Routed QuakeWorld message with its captured owner.
#[derive(Debug, Clone, PartialEq)]
pub struct QcRoutedMessage {
    /// Decoded message.
    pub message: QwMessage,
    /// Owner captured when the message was written.
    pub actor: Option<ActorId>,
}

/// Routed-message services (the donor `qw`/`nq` services).
pub trait QcMessageRouter {
    /// Network dialect served.
    fn api(&self) -> ApiKind;
    /// Whether an actor is a connected client.
    fn is_client(&self, actor: &ActorId) -> bool;
    /// Whether the server is in the spawn/loading phase.
    fn loading(&self) -> bool;
    /// NetQuake native routing (delivers wire bytes itself).
    fn native(&self) -> bool;
    /// NetQuake local loopback routing.
    fn local(&self) -> bool;
    /// QuakeWorld PHS multicast support.
    fn phs(&self) -> bool;
    /// Route NetQuake messages. `view_targets` maps message indexes to the
    /// owner captured for `set-view` records.
    fn route_nq(
        &mut self,
        messages: &[NqMessage],
        destination: &QcMessageDestination,
        view_targets: &[(usize, Option<ActorId>)],
    );
    /// Route QuakeWorld messages.
    fn route_qw(&mut self, entries: &[QcRoutedMessage], destination: &QcMessageDestination);
}

// Message tags shared by the NetQuake and QuakeWorld codecs.
const TAG_PRINT: u8 = 1;
const TAG_CENTER_PRINT: u8 = 2;
const TAG_STUFFTEXT: u8 = 3;
const TAG_TEMP_POINT: u8 = 4;
const TAG_TEMP_BEAM: u8 = 5;
const TAG_EXPLOSION_COLORS: u8 = 6;
const TAG_SET_VIEW: u8 = 7;
const TAG_SOUND: u8 = 8;
const TAG_MUZZLE_FLASH: u8 = 9;
const TAG_KICK: u8 = 10;
const TAG_INTERMISSION: u8 = 11;
const TAG_CD_TRACK: u8 = 12;
const TAG_STOP_SOUND: u8 = 13;
const TAG_NOP: u8 = 14;
const TAG_STAT: u8 = 15;
const TAG_FINALE: u8 = 16;
const TAG_SET_ANGLE: u8 = 17;
const TAG_LIGHT_STYLE: u8 = 18;
const TAG_STATIC: u8 = 19;
const TAG_STATIC_SOUND: u8 = 20;
const TAG_DAMAGE: u8 = 21;
const TAG_PAUSE: u8 = 22;
const TAG_KILLED_MONSTER: u8 = 23;
const TAG_FOUND_SECRET: u8 = 24;
const TAG_SELL_SCREEN: u8 = 25;

/// Size-bounded message buffer mirroring `SizeBuf`.
#[derive(Debug, Clone)]
pub struct MsgBuffer {
    data: Vec<u8>,
    maxsize: usize,
    allowoverflow: bool,
    overflowed: bool,
}

impl MsgBuffer {
    /// Build a buffer with a byte budget.
    #[must_use]
    pub fn new(maxsize: usize, allowoverflow: bool) -> Self {
        Self {
            data: Vec::new(),
            maxsize,
            allowoverflow,
            overflowed: false,
        }
    }

    /// Written byte count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether no bytes are written.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Byte budget.
    #[must_use]
    pub const fn maxsize(&self) -> usize {
        self.maxsize
    }

    /// Whether overflow clears instead of failing.
    #[must_use]
    pub const fn allowoverflow(&self) -> bool {
        self.allowoverflow
    }

    /// Whether the buffer overflowed.
    #[must_use]
    pub const fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// Written bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// Clear written bytes (keeps the overflow latch, like `SZ_Clear`).
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// Mark overflow state (save restore only).
    pub fn set_overflowed(&mut self, overflowed: bool) {
        self.overflowed = overflowed;
    }

    /// Replace contents (save restore only).
    pub fn set_bytes(&mut self, bytes: &[u8]) -> Result<(), GuestError> {
        if bytes.len() > self.maxsize {
            return Err(GuestError::invalid("saved message exceeds buffer capacity"));
        }
        self.data.clear();
        self.data.extend_from_slice(bytes);
        Ok(())
    }

    fn reserve(&mut self, extra: usize) -> Result<(), GuestError> {
        if self.data.len() + extra <= self.maxsize {
            return Ok(());
        }
        if !self.allowoverflow {
            return Err(GuestError::invalid("message buffer overflow"));
        }
        self.data.clear();
        self.overflowed = true;
        if extra > self.maxsize {
            return Err(GuestError::invalid("message exceeds buffer capacity"));
        }
        Ok(())
    }

    /// `MSG_WriteByte`: low 8 bits.
    pub fn write_byte(&mut self, value: i32) -> Result<(), GuestError> {
        self.reserve(1)?;
        self.data.push(value as u8);
        Ok(())
    }

    /// `MSG_WriteChar`: low 8 bits, signed on read.
    pub fn write_char(&mut self, value: i32) -> Result<(), GuestError> {
        self.write_byte(value)
    }

    /// `MSG_WriteShort`: low 16 bits, little-endian.
    pub fn write_short(&mut self, value: i32) -> Result<(), GuestError> {
        self.reserve(2)?;
        self.data.extend_from_slice(&(value as i16).to_le_bytes());
        Ok(())
    }

    /// `MSG_WriteLong`: 32 bits, little-endian.
    pub fn write_long(&mut self, value: i32) -> Result<(), GuestError> {
        self.reserve(4)?;
        self.data.extend_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// `MSG_WriteCoord`: 32-bit float, little-endian.
    pub fn write_coord(&mut self, value: f32) -> Result<(), GuestError> {
        self.reserve(4)?;
        self.data.extend_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// `MSG_WriteAngle`: 8-bit angle (`angle * 256 / 360`).
    pub fn write_angle(&mut self, value: f32) -> Result<(), GuestError> {
        self.reserve(1)?;
        self.data.push(((value * 256.0 / 360.0) as i32 & 0xff) as u8);
        Ok(())
    }

    /// `MSG_WriteString`: bytes plus a NUL terminator.
    pub fn write_string(&mut self, value: &str) -> Result<(), GuestError> {
        self.reserve(value.len() + 1)?;
        self.data.extend_from_slice(value.as_bytes());
        self.data.push(0);
        Ok(())
    }

    /// Write a vector as three coords.
    pub fn write_vector(&mut self, value: Vec3) -> Result<(), GuestError> {
        self.write_coord(value.x)?;
        self.write_coord(value.y)?;
        self.write_coord(value.z)?;
        Ok(())
    }
}

/// Byte reader for the message codecs.
struct MsgReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> MsgReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn rest(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn read_u8(&mut self) -> Result<u8, GuestError> {
        if self.rest() < 1 {
            return Err(GuestError::invalid("truncated QC message"));
        }
        let value = self.data[self.pos];
        self.pos += 1;
        Ok(value)
    }

    fn read_u16(&mut self) -> Result<u16, GuestError> {
        if self.rest() < 2 {
            return Err(GuestError::invalid("truncated QC message"));
        }
        let mut bytes = [0u8; 2];
        bytes.copy_from_slice(&self.data[self.pos..self.pos + 2]);
        self.pos += 2;
        Ok(u16::from_le_bytes(bytes))
    }

    fn read_i32(&mut self) -> Result<i32, GuestError> {
        if self.rest() < 4 {
            return Err(GuestError::invalid("truncated QC message"));
        }
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(&self.data[self.pos..self.pos + 4]);
        self.pos += 4;
        Ok(i32::from_le_bytes(bytes))
    }

    fn read_f32(&mut self) -> Result<f32, GuestError> {
        if self.rest() < 4 {
            return Err(GuestError::invalid("truncated QC message"));
        }
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(&self.data[self.pos..self.pos + 4]);
        self.pos += 4;
        Ok(f32::from_le_bytes(bytes))
    }

    fn read_vector(&mut self) -> Result<Vec3, GuestError> {
        Ok(Vec3 {
            x: self.read_f32()?,
            y: self.read_f32()?,
            z: self.read_f32()?,
        })
    }

    fn read_string(&mut self) -> Result<String, GuestError> {
        let end = self.data[self.pos..]
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| GuestError::invalid("truncated QC message"))?;
        let text = String::from_utf8_lossy(&self.data[self.pos..self.pos + end]).into_owned();
        self.pos += end + 1;
        Ok(text)
    }
}

fn encode_temp(buffer: &mut MsgBuffer, effect: &TempEntityEffect) -> Result<(), GuestError> {
    match effect {
        TempEntityEffect::ExplosionColors {
            origin,
            color_start,
            color_length,
        } => {
            buffer.write_byte(i32::from(TAG_EXPLOSION_COLORS))?;
            buffer.write_vector(*origin)?;
            buffer.write_byte(*color_start)?;
            buffer.write_byte(*color_length)?;
        }
        TempEntityEffect::Beam {
            entity,
            beam_type,
            start,
            end,
        } => {
            buffer.write_byte(i32::from(TAG_TEMP_BEAM))?;
            buffer.write_byte(i32::from(*beam_type))?;
            buffer.write_short(i32::from(*entity))?;
            buffer.write_vector(*start)?;
            buffer.write_vector(*end)?;
        }
        TempEntityEffect::Point {
            effect_type,
            origin,
            count,
        } => {
            buffer.write_byte(i32::from(TAG_TEMP_POINT))?;
            buffer.write_byte(i32::from(*effect_type))?;
            buffer.write_vector(*origin)?;
            buffer.write_byte(*count)?;
        }
    }
    Ok(())
}

fn decode_temp(reader: &mut MsgReader, tag: u8) -> Result<TempEntityEffect, GuestError> {
    match tag {
        TAG_EXPLOSION_COLORS => Ok(TempEntityEffect::ExplosionColors {
            origin: reader.read_vector()?,
            color_start: i32::from(reader.read_u8()?),
            color_length: i32::from(reader.read_u8()?),
        }),
        TAG_TEMP_BEAM => Ok(TempEntityEffect::Beam {
            beam_type: reader.read_u8()?,
            entity: reader.read_u16()?,
            start: reader.read_vector()?,
            end: reader.read_vector()?,
        }),
        TAG_TEMP_POINT => Ok(TempEntityEffect::Point {
            effect_type: reader.read_u8()?,
            origin: reader.read_vector()?,
            count: i32::from(reader.read_u8()?),
        }),
        _ => Err(GuestError::invalid(format!("unknown QC temp-entity tag {tag}"))),
    }
}

#[allow(clippy::too_many_lines)]
fn encode_qw(buffer: &mut MsgBuffer, message: &QwMessage) -> Result<(), GuestError> {
    match message {
        QwMessage::Print { level, text } => {
            buffer.write_byte(i32::from(TAG_PRINT))?;
            buffer.write_long(*level)?;
            buffer.write_string(text)?;
        }
        QwMessage::CenterPrint { text } => {
            buffer.write_byte(i32::from(TAG_CENTER_PRINT))?;
            buffer.write_string(text)?;
        }
        QwMessage::StuffText { text } => {
            buffer.write_byte(i32::from(TAG_STUFFTEXT))?;
            buffer.write_string(text)?;
        }
        QwMessage::TempEntity { effect } => encode_temp(buffer, effect)?,
        QwMessage::SetView { entity } => {
            buffer.write_byte(i32::from(TAG_SET_VIEW))?;
            buffer.write_short(i32::from(*entity))?;
        }
        QwMessage::Sound {
            entity,
            channel,
            index,
            origin,
            volume,
            attenuation,
        } => {
            buffer.write_byte(i32::from(TAG_SOUND))?;
            buffer.write_short(i32::from(*entity))?;
            buffer.write_byte(i32::from(*channel))?;
            buffer.write_byte(i32::from(*index))?;
            buffer.write_vector(*origin)?;
            buffer.write_byte(i32::from(*volume))?;
            buffer.write_coord(*attenuation)?;
        }
        QwMessage::StopSound { entity, channel } => {
            buffer.write_byte(i32::from(TAG_STOP_SOUND))?;
            buffer.write_short(i32::from(*entity))?;
            buffer.write_byte(i32::from(*channel))?;
        }
        QwMessage::MuzzleFlash { entity } => {
            buffer.write_byte(i32::from(TAG_MUZZLE_FLASH))?;
            buffer.write_short(i32::from(*entity))?;
        }
        QwMessage::Kick => {
            buffer.write_byte(i32::from(TAG_KICK))?;
        }
        QwMessage::Intermission { origin, angles } => {
            buffer.write_byte(i32::from(TAG_INTERMISSION))?;
            buffer.write_vector(*origin)?;
            buffer.write_vector(*angles)?;
        }
        QwMessage::CdTrack { track } => {
            buffer.write_byte(i32::from(TAG_CD_TRACK))?;
            buffer.write_byte(i32::from(*track))?;
        }
        QwMessage::Nop => {
            buffer.write_byte(i32::from(TAG_NOP))?;
        }
        QwMessage::Stat { stat, value } => {
            buffer.write_byte(i32::from(TAG_STAT))?;
            buffer.write_byte(i32::from(*stat))?;
            buffer.write_long(*value)?;
        }
        QwMessage::Finale { text } => {
            buffer.write_byte(i32::from(TAG_FINALE))?;
            buffer.write_string(text)?;
        }
        QwMessage::SetAngle { entity, angles } => {
            buffer.write_byte(i32::from(TAG_SET_ANGLE))?;
            buffer.write_short(i32::from(*entity))?;
            buffer.write_vector(*angles)?;
        }
        QwMessage::LightStyle { style, pattern } => {
            buffer.write_byte(i32::from(TAG_LIGHT_STYLE))?;
            buffer.write_byte(i32::from(*style))?;
            buffer.write_string(pattern)?;
        }
        QwMessage::Static {
            model,
            frame,
            colormap,
            skin,
            origin,
            angles,
        } => {
            buffer.write_byte(i32::from(TAG_STATIC))?;
            buffer.write_byte(i32::from(*model))?;
            buffer.write_byte(i32::from(*frame))?;
            buffer.write_byte(i32::from(*colormap))?;
            buffer.write_byte(i32::from(*skin))?;
            buffer.write_vector(*origin)?;
            buffer.write_vector(*angles)?;
        }
        QwMessage::StaticSound {
            origin,
            index,
            volume,
            attenuation,
        } => {
            buffer.write_byte(i32::from(TAG_STATIC_SOUND))?;
            buffer.write_vector(*origin)?;
            buffer.write_byte(i32::from(*index))?;
            buffer.write_byte(i32::from(*volume))?;
            buffer.write_byte(i32::from(*attenuation))?;
        }
        QwMessage::Damage { armor, blood, origin } => {
            buffer.write_byte(i32::from(TAG_DAMAGE))?;
            buffer.write_byte(i32::from(*armor))?;
            buffer.write_byte(i32::from(*blood))?;
            buffer.write_vector(*origin)?;
        }
        QwMessage::Pause { paused } => {
            buffer.write_byte(i32::from(TAG_PAUSE))?;
            buffer.write_byte(i32::from(*paused))?;
        }
        QwMessage::KilledMonster => {
            buffer.write_byte(i32::from(TAG_KILLED_MONSTER))?;
        }
        QwMessage::FoundSecret => {
            buffer.write_byte(i32::from(TAG_FOUND_SECRET))?;
        }
        QwMessage::SellScreen => {
            buffer.write_byte(i32::from(TAG_SELL_SCREEN))?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn decode_qw(reader: &mut MsgReader) -> Result<QwMessage, GuestError> {
    let tag = reader.read_u8()?;
    match tag {
        TAG_PRINT => Ok(QwMessage::Print {
            level: reader.read_i32()?,
            text: reader.read_string()?,
        }),
        TAG_CENTER_PRINT => Ok(QwMessage::CenterPrint {
            text: reader.read_string()?,
        }),
        TAG_STUFFTEXT => Ok(QwMessage::StuffText {
            text: reader.read_string()?,
        }),
        TAG_TEMP_POINT | TAG_TEMP_BEAM | TAG_EXPLOSION_COLORS => Ok(QwMessage::TempEntity {
            effect: decode_temp(reader, tag)?,
        }),
        TAG_SET_VIEW => Ok(QwMessage::SetView {
            entity: reader.read_u16()?,
        }),
        TAG_SOUND => Ok(QwMessage::Sound {
            entity: reader.read_u16()?,
            channel: reader.read_u8()?,
            index: reader.read_u8()?,
            origin: reader.read_vector()?,
            volume: reader.read_u8()?,
            attenuation: reader.read_f32()?,
        }),
        TAG_STOP_SOUND => Ok(QwMessage::StopSound {
            entity: reader.read_u16()?,
            channel: reader.read_u8()?,
        }),
        TAG_MUZZLE_FLASH => Ok(QwMessage::MuzzleFlash {
            entity: reader.read_u16()?,
        }),
        TAG_KICK => Ok(QwMessage::Kick),
        TAG_INTERMISSION => Ok(QwMessage::Intermission {
            origin: reader.read_vector()?,
            angles: reader.read_vector()?,
        }),
        TAG_CD_TRACK => Ok(QwMessage::CdTrack {
            track: reader.read_u8()?,
        }),
        TAG_NOP => Ok(QwMessage::Nop),
        TAG_STAT => Ok(QwMessage::Stat {
            stat: reader.read_u8()?,
            value: reader.read_i32()?,
        }),
        TAG_FINALE => Ok(QwMessage::Finale {
            text: reader.read_string()?,
        }),
        TAG_SET_ANGLE => Ok(QwMessage::SetAngle {
            entity: reader.read_u16()?,
            angles: reader.read_vector()?,
        }),
        TAG_LIGHT_STYLE => Ok(QwMessage::LightStyle {
            style: reader.read_u8()?,
            pattern: reader.read_string()?,
        }),
        TAG_STATIC => Ok(QwMessage::Static {
            model: reader.read_u8()?,
            frame: reader.read_u8()?,
            colormap: reader.read_u8()?,
            skin: reader.read_u8()?,
            origin: reader.read_vector()?,
            angles: reader.read_vector()?,
        }),
        TAG_STATIC_SOUND => Ok(QwMessage::StaticSound {
            origin: reader.read_vector()?,
            index: reader.read_u8()?,
            volume: reader.read_u8()?,
            attenuation: reader.read_u8()?,
        }),
        TAG_DAMAGE => Ok(QwMessage::Damage {
            armor: reader.read_u8()?,
            blood: reader.read_u8()?,
            origin: reader.read_vector()?,
        }),
        TAG_PAUSE => Ok(QwMessage::Pause {
            paused: reader.read_u8()? != 0,
        }),
        TAG_KILLED_MONSTER => Ok(QwMessage::KilledMonster),
        TAG_FOUND_SECRET => Ok(QwMessage::FoundSecret),
        TAG_SELL_SCREEN => Ok(QwMessage::SellScreen),
        _ => Err(GuestError::invalid(format!("unknown QC message tag {tag}"))),
    }
}

fn encode_nq(buffer: &mut MsgBuffer, message: &NqMessage) -> Result<(), GuestError> {
    match message {
        NqMessage::Print { text } => {
            buffer.write_byte(i32::from(TAG_PRINT))?;
            buffer.write_string(text)?;
        }
        NqMessage::CenterPrint { text } => {
            buffer.write_byte(i32::from(TAG_CENTER_PRINT))?;
            buffer.write_string(text)?;
        }
        NqMessage::StuffText { text } => {
            buffer.write_byte(i32::from(TAG_STUFFTEXT))?;
            buffer.write_string(text)?;
        }
        NqMessage::TempEntity { effect } => encode_temp(buffer, effect)?,
        NqMessage::SetView { entity } => {
            buffer.write_byte(i32::from(TAG_SET_VIEW))?;
            buffer.write_short(i32::from(*entity))?;
        }
        NqMessage::Sound {
            entity,
            channel,
            index,
            origin,
            volume,
            attenuation,
        } => {
            buffer.write_byte(i32::from(TAG_SOUND))?;
            buffer.write_short(i32::from(*entity))?;
            buffer.write_byte(i32::from(*channel))?;
            buffer.write_byte(i32::from(*index))?;
            buffer.write_vector(*origin)?;
            buffer.write_byte(i32::from(*volume))?;
            buffer.write_coord(*attenuation)?;
        }
    }
    Ok(())
}

fn decode_nq(reader: &mut MsgReader) -> Result<NqMessage, GuestError> {
    let tag = reader.read_u8()?;
    match tag {
        TAG_PRINT => Ok(NqMessage::Print {
            text: reader.read_string()?,
        }),
        TAG_CENTER_PRINT => Ok(NqMessage::CenterPrint {
            text: reader.read_string()?,
        }),
        TAG_STUFFTEXT => Ok(NqMessage::StuffText {
            text: reader.read_string()?,
        }),
        TAG_TEMP_POINT | TAG_TEMP_BEAM | TAG_EXPLOSION_COLORS => Ok(NqMessage::TempEntity {
            effect: decode_temp(reader, tag)?,
        }),
        TAG_SET_VIEW => Ok(NqMessage::SetView {
            entity: reader.read_u16()?,
        }),
        TAG_SOUND => Ok(NqMessage::Sound {
            entity: reader.read_u16()?,
            channel: reader.read_u8()?,
            index: reader.read_u8()?,
            origin: reader.read_vector()?,
            volume: reader.read_u8()?,
            attenuation: reader.read_f32()?,
        }),
        _ => Err(GuestError::invalid(format!("unknown QC message tag {tag}"))),
    }
}

/// Decode all NetQuake messages in a byte string.
pub fn decode_nq_bytes(bytes: &[u8]) -> Result<Vec<NqMessage>, GuestError> {
    let mut reader = MsgReader::new(bytes);
    let mut messages = Vec::new();
    while reader.rest() > 0 {
        messages.push(decode_nq(&mut reader)?);
    }
    Ok(messages)
}

/// Decode all QuakeWorld messages in a byte string.
pub fn decode_qw_bytes(bytes: &[u8]) -> Result<Vec<QwMessage>, GuestError> {
    let mut reader = MsgReader::new(bytes);
    let mut messages = Vec::new();
    while reader.rest() > 0 {
        messages.push(decode_qw(&mut reader)?);
    }
    Ok(messages)
}

/// One routed destination buffer.
#[derive(Debug)]
struct RoutedBuffer {
    buffer: MsgBuffer,
    owners: HashMap<i32, Option<ActorId>>,
    destination: Option<QcMessageDestination>,
}

/// QC writes retain their source codec and destination before joining
/// shared presentation.
pub struct QcBroadcastMessages<R> {
    router: Option<R>,
    is_qw: bool,
    buffer: MsgBuffer,
    owners: HashMap<i32, Option<ActorId>>,
    protocol_version: u32,
    protocol_flags: u32,
    signon_buffers: u32,
    routed: HashMap<String, RoutedBuffer>,
}

impl<R: QcMessageRouter> QcBroadcastMessages<R> {
    /// Build the message host. QuakeWorld requires routed services, and the
    /// router dialect must match.
    pub fn new(router: Option<R>, is_qw: bool, protocol_version: u32, protocol_flags: u32) -> Result<Self, GuestError> {
        if let Some(router) = &router {
            let api = router.api();
            if is_qw && api != ApiKind::QuakeWorld {
                return Err(GuestError::invalid("QuakeWorld cannot use NetQuake routes"));
            }
            if !is_qw && api != ApiKind::NetQuake {
                return Err(GuestError::invalid("NetQuake messages cannot use QuakeWorld routes"));
            }
        } else if is_qw {
            return Err(GuestError::invalid(
                "QuakeWorld messages require routed message services",
            ));
        }
        Ok(Self {
            router,
            is_qw,
            buffer: MsgBuffer::new(MAX_DATAGRAM, false),
            owners: HashMap::new(),
            protocol_version,
            protocol_flags,
            signon_buffers: 1,
            routed: HashMap::new(),
        })
    }

    /// Whether this host speaks QuakeWorld.
    #[must_use]
    pub const fn is_quakeworld(&self) -> bool {
        self.is_qw
    }

    /// Routed buffer count (test inspection).
    #[must_use]
    pub fn routed_count(&self) -> usize {
        self.routed.len()
    }

    /// Signon buffer count.
    #[must_use]
    pub const fn signon_buffers(&self) -> u32 {
        self.signon_buffers
    }

    /// Unrouted broadcast bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.buffer.bytes()
    }

    /// `WriteByte` to a destination.
    pub fn write_byte(
        &mut self,
        dest: i32,
        msg_entity: Option<&ActorId>,
        slots: &dyn Fn(usize) -> Option<ActorId>,
        value: i32,
    ) -> Result<(), GuestError> {
        self.write_op(dest, msg_entity, slots, &|buffer| buffer.write_byte(value))
    }

    /// `WriteChar` to a destination.
    pub fn write_char(
        &mut self,
        dest: i32,
        msg_entity: Option<&ActorId>,
        slots: &dyn Fn(usize) -> Option<ActorId>,
        value: i32,
    ) -> Result<(), GuestError> {
        self.write_op(dest, msg_entity, slots, &|buffer| buffer.write_char(value))
    }

    /// `WriteShort` to a destination.
    pub fn write_short(
        &mut self,
        dest: i32,
        msg_entity: Option<&ActorId>,
        slots: &dyn Fn(usize) -> Option<ActorId>,
        value: i32,
    ) -> Result<(), GuestError> {
        self.write_op(dest, msg_entity, slots, &|buffer| buffer.write_short(value))
    }

    /// `WriteLong` to a destination.
    pub fn write_long(
        &mut self,
        dest: i32,
        msg_entity: Option<&ActorId>,
        slots: &dyn Fn(usize) -> Option<ActorId>,
        value: i32,
    ) -> Result<(), GuestError> {
        self.write_op(dest, msg_entity, slots, &|buffer| buffer.write_long(value))
    }

    /// `WriteCoord` to a destination.
    pub fn write_coord(
        &mut self,
        dest: i32,
        msg_entity: Option<&ActorId>,
        slots: &dyn Fn(usize) -> Option<ActorId>,
        value: f32,
    ) -> Result<(), GuestError> {
        self.write_op(dest, msg_entity, slots, &|buffer| buffer.write_coord(value))
    }

    /// `WriteAngle` to a destination.
    pub fn write_angle(
        &mut self,
        dest: i32,
        msg_entity: Option<&ActorId>,
        slots: &dyn Fn(usize) -> Option<ActorId>,
        value: f32,
    ) -> Result<(), GuestError> {
        self.write_op(dest, msg_entity, slots, &|buffer| buffer.write_angle(value))
    }

    /// `WriteString` to a destination.
    pub fn write_string(
        &mut self,
        dest: i32,
        msg_entity: Option<&ActorId>,
        slots: &dyn Fn(usize) -> Option<ActorId>,
        value: &str,
    ) -> Result<(), GuestError> {
        self.write_op(dest, msg_entity, slots, &|buffer| buffer.write_string(value))
    }

    /// `WriteEntity`: write an entity slot as a short.
    pub fn write_entity(
        &mut self,
        dest: i32,
        msg_entity: Option<&ActorId>,
        slots: &dyn Fn(usize) -> Option<ActorId>,
        slot: u16,
    ) -> Result<(), GuestError> {
        self.write_op(dest, msg_entity, slots, &|buffer| buffer.write_short(i32::from(slot)))
    }

    /// QuakeWorld `multicast`: route the pending multicast buffer.
    pub fn multicast(&mut self, origin: Vec3, mode: i32) -> Result<(), GuestError> {
        if !(0..=5).contains(&mode) {
            return Err(GuestError::invalid("SV_Multicast: bad destination"));
        }
        if let Some(entry) = self.routed.remove("multicast") {
            let destination = QcMessageDestination::Multicast {
                origin,
                visibility: match mode % 3 {
                    0 => VisibilityScope::All,
                    1 => VisibilityScope::Phs,
                    _ => VisibilityScope::Pvs,
                },
                reliable: mode >= 3,
            };
            self.route_entry(&entry, &destination, &mut |_, _| {})?;
        }
        Ok(())
    }

    /// Source `SV_FlushSignon`: flush a filling signon buffer, reserving
    /// 512 bytes for the next spawn.
    pub fn flush_signon(&mut self) -> Result<(), GuestError> {
        if !self.is_qw {
            return Ok(());
        }
        let full = self
            .routed
            .get("signon")
            .is_some_and(|entry| entry.buffer.len() >= MAX_DATAGRAM - 512);
        if full {
            if self.signon_buffers == MAX_SIGNON_BUFFERS {
                return Err(GuestError::invalid("QW MAX_SIGNON_BUFFERS exhausted"));
            }
            self.signon_buffers += 1;
            if let Some(entry) = self.routed.remove("signon") {
                self.route_entry(&entry, &QcMessageDestination::Signon, &mut |_, _| {})?;
            }
        }
        Ok(())
    }

    /// Flush all routed buffers, or decode the unrouted broadcast buffer
    /// into presentation effects.
    pub fn flush(&mut self, emit: &mut dyn FnMut(&QcBroadcastEffect, Option<&ActorId>)) -> Result<(), GuestError> {
        if self.router.is_some() {
            let keys: Vec<String> = self
                .routed
                .iter()
                .filter(|(_, entry)| entry.destination.is_some())
                .map(|(key, _)| key.clone())
                .collect();
            for key in keys {
                if let Some(entry) = self.routed.remove(&key) {
                    // QW `SV_SendClientMessages` discards the entire
                    // overflowed broadcast datagram.
                    if entry.buffer.overflowed() {
                        continue;
                    }
                    let destination = entry.destination.clone().expect("filtered above");
                    self.route_entry(&entry, &destination, emit)?;
                }
            }
            return Ok(());
        }
        let messages = decode_nq_bytes(self.buffer.bytes())?;
        let mut encoded = MsgBuffer::new(self.buffer.maxsize(), false);
        for message in &messages {
            let NqMessage::TempEntity { effect } = message else {
                return Err(GuestError::invalid(format!(
                    "Unsupported QC broadcast message {}",
                    nq_kind(message)
                )));
            };
            let offset = encoded.len();
            encode_nq(&mut encoded, message)?;
            let owner = self.owners.get(&(offset as i32 + 2)).and_then(Clone::clone);
            emit(&quake_temporary_event(effect, owner.as_ref(), false)?, None);
        }
        self.buffer.clear();
        self.owners.clear();
        Ok(())
    }

    fn write_op(
        &mut self,
        dest: i32,
        msg_entity: Option<&ActorId>,
        slots: &dyn Fn(usize) -> Option<ActorId>,
        op: &dyn Fn(&mut MsgBuffer) -> Result<(), GuestError>,
    ) -> Result<(), GuestError> {
        if self.router.is_none() {
            if dest != 0 {
                return Err(GuestError::invalid(
                    "QC message destination is not the supported MSG_BROADCAST datagram",
                ));
            }
            let before = self.buffer.len();
            op(&mut self.buffer)?;
            capture_owners(self.buffer.bytes(), &mut self.owners, before, slots, false);
            return Ok(());
        }
        let (key, destination, maxsize, allowoverflow) = self.route_target(dest, msg_entity)?;
        let is_qw = self.is_qw;
        let entry = self.routed.entry(key).or_insert_with(|| RoutedBuffer {
            buffer: MsgBuffer::new(maxsize, allowoverflow),
            owners: HashMap::new(),
            destination,
        });
        let before = entry.buffer.len();
        op(&mut entry.buffer)?;
        capture_owners(entry.buffer.bytes(), &mut entry.owners, before, slots, is_qw);
        Ok(())
    }

    fn route_target(
        &self,
        dest: i32,
        msg_entity: Option<&ActorId>,
    ) -> Result<(String, Option<QcMessageDestination>, usize, bool), GuestError> {
        let router = self.router.as_ref().expect("routed writes need a router");
        match dest {
            0 => Ok((
                "broadcast".to_string(),
                Some(QcMessageDestination::Broadcast { reliable: false }),
                dest_buffer_size(self.is_qw, dest),
                true,
            )),
            1 => {
                let Some(actor) = msg_entity else {
                    return Err(GuestError::invalid("WriteDest: not a client"));
                };
                if !router.is_client(actor) {
                    return Err(GuestError::invalid("WriteDest: not a client"));
                }
                Ok((
                    format!("client:{}:{}", actor.slot(), actor.generation()),
                    Some(QcMessageDestination::Client { actor: actor.clone() }),
                    dest_buffer_size(self.is_qw, dest),
                    false,
                ))
            }
            2 => Ok((
                "all".to_string(),
                Some(QcMessageDestination::Broadcast { reliable: true }),
                dest_buffer_size(self.is_qw, dest),
                true,
            )),
            3 => {
                if self.is_qw && !router.loading() {
                    return Err(GuestError::invalid(
                        "PF_Write_*: MSG_INIT can only be written in spawn functions",
                    ));
                }
                Ok((
                    "signon".to_string(),
                    Some(QcMessageDestination::Signon),
                    dest_buffer_size(self.is_qw, dest),
                    false,
                ))
            }
            4 => {
                if !self.is_qw {
                    return Err(GuestError::invalid("WriteDest: bad destination"));
                }
                Ok((
                    "multicast".to_string(),
                    None,
                    dest_buffer_size(self.is_qw, dest),
                    dest == 0,
                ))
            }
            _ => Err(GuestError::invalid("WriteDest: bad destination")),
        }
    }

    fn route_entry(
        &mut self,
        entry: &RoutedBuffer,
        destination: &QcMessageDestination,
        emit: &mut dyn FnMut(&QcBroadcastEffect, Option<&ActorId>),
    ) -> Result<(), GuestError> {
        if self.is_qw {
            let Some(router) = self.router.as_mut() else {
                return Err(GuestError::invalid("Missing QuakeWorld routing service"));
            };
            let messages = decode_qw_bytes(entry.buffer.bytes())?;
            let mut encoded = MsgBuffer::new(entry.buffer.maxsize(), false);
            let mut routed = Vec::with_capacity(messages.len());
            for message in messages {
                let offset = encoded.len() as i32;
                encode_qw(&mut encoded, &message)?;
                let actor = match &message {
                    QwMessage::TempEntity {
                        effect: TempEntityEffect::Beam { .. },
                    } => {
                        let owner = entry.owners.get(&(offset + 2)).and_then(Clone::clone);
                        if owner.is_none() {
                            return Err(GuestError::invalid("QC beam had no owned actor when written"));
                        }
                        owner
                    }
                    QwMessage::Sound { .. } | QwMessage::StopSound { .. } => {
                        entry.owners.get(&(-offset - 2)).and_then(Clone::clone)
                    }
                    QwMessage::MuzzleFlash { .. } | QwMessage::SetView { .. } => {
                        entry.owners.get(&(offset + 1)).and_then(Clone::clone)
                    }
                    _ => None,
                };
                routed.push(QcRoutedMessage { message, actor });
            }
            router.route_qw(&routed, destination);
            return Ok(());
        }
        let Some(router) = self.router.as_mut() else {
            return Err(GuestError::invalid("Missing NetQuake routing service"));
        };
        let messages = decode_nq_bytes(entry.buffer.bytes())?;
        let mut encoded = MsgBuffer::new(entry.buffer.maxsize(), false);
        let mut view_targets = Vec::new();
        for (index, message) in messages.iter().enumerate() {
            if matches!(message, NqMessage::SetView { .. }) {
                view_targets.push((
                    index,
                    entry.owners.get(&(encoded.len() as i32 + 1)).and_then(Clone::clone),
                ));
            }
            encode_nq(&mut encoded, message)?;
        }
        router.route_nq(&messages, destination, &view_targets);
        let recipient = match destination {
            QcMessageDestination::Client { actor } => Some(actor.clone()),
            _ => None,
        };
        if !router.native()
            || router.local()
            || matches!(destination, QcMessageDestination::Broadcast { reliable: false })
        {
            let mut encoded = MsgBuffer::new(entry.buffer.maxsize(), false);
            for message in &messages {
                let offset = encoded.len();
                encode_nq(&mut encoded, message)?;
                if let NqMessage::TempEntity { effect } = message {
                    let owner = entry.owners.get(&(offset as i32 + 2)).and_then(Clone::clone);
                    emit(
                        &quake_temporary_event(effect, owner.as_ref(), false)?,
                        recipient.as_ref(),
                    );
                }
            }
        }
        Ok(())
    }
}

fn dest_buffer_size(is_qw: bool, dest: i32) -> usize {
    if is_qw {
        match dest {
            1 => MAX_ROUTED_BYTES,
            0 | 3 => MAX_DATAGRAM,
            _ => MAX_MSGLEN,
        }
    } else {
        match dest {
            0 | 2 => MAX_DATAGRAM,
            _ => MAX_MSGLEN,
        }
    }
}

fn capture_owners(
    bytes: &[u8],
    owners: &mut HashMap<i32, Option<ActorId>>,
    before: usize,
    slots: &dyn Fn(usize) -> Option<ActorId>,
    quakeworld: bool,
) {
    for end in before.max(1)..bytes.len() {
        let (Some(low), Some(high)) = (bytes.get(end - 1), bytes.get(end)) else {
            continue;
        };
        let word = usize::from(*low) + usize::from(*high) * 256;
        owners.insert(end as i32 - 1, slots(word));
        if quakeworld {
            owners.insert(-(end as i32), slots((word >> 3) & 1023));
        }
    }
}

fn nq_kind(message: &NqMessage) -> &'static str {
    match message {
        NqMessage::Print { .. } => "print",
        NqMessage::CenterPrint { .. } => "center-print",
        NqMessage::StuffText { .. } => "stufftext",
        NqMessage::TempEntity { .. } => "temporary-entity",
        NqMessage::SetView { .. } => "set-view",
        NqMessage::Sound { .. } => "sound",
    }
}

/// Encode NetQuake messages to bytes.
pub fn capture_netquake_messages(messages: &[NqMessage]) -> Result<Vec<u8>, GuestError> {
    let mut buffer = MsgBuffer::new(MAX_MSGLEN, false);
    for message in messages {
        encode_nq(&mut buffer, message)?;
    }
    Ok(buffer.bytes().to_vec())
}

/// Decode NetQuake messages from bytes.
pub fn restore_netquake_messages(bytes: &[u8]) -> Result<Vec<NqMessage>, GuestError> {
    decode_nq_bytes(bytes)
}

/// Saved routed QuakeWorld entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QwEntryCheckpoint {
    /// Encoded message bytes.
    pub bytes: Vec<u8>,
    /// Captured owner.
    pub actor: Option<SavedActorId>,
}

/// Saved QuakeWorld routed-message set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QwEntriesCheckpoint {
    /// Protocol version.
    pub version: u32,
    /// Protocol flags.
    pub flags: u32,
    /// Saved entries.
    pub entries: Vec<QwEntryCheckpoint>,
}

impl<R: QcMessageRouter> QcBroadcastMessages<R> {
    /// Checkpoint routed entries to bytes.
    pub fn capture_entries(&self, entries: &[QcRoutedMessage]) -> Result<QwEntriesCheckpoint, GuestError> {
        let mut saved = Vec::with_capacity(entries.len());
        for entry in entries {
            let mut buffer = MsgBuffer::new(MAX_ROUTED_BYTES, false);
            encode_qw(&mut buffer, &entry.message)?;
            saved.push(QwEntryCheckpoint {
                bytes: buffer.bytes().to_vec(),
                actor: saved_qc_actor(entry.actor.as_ref()),
            });
        }
        Ok(QwEntriesCheckpoint {
            version: self.protocol_version,
            flags: self.protocol_flags,
            entries: saved,
        })
    }

    /// Restore routed entries, resolving owners.
    pub fn restore_entries(
        &self,
        saved: &QwEntriesCheckpoint,
        resolve: &dyn Fn(&SavedActorId) -> Option<ActorId>,
    ) -> Result<Vec<QcRoutedMessage>, GuestError> {
        let _ = (saved.version, saved.flags);
        let mut entries = Vec::with_capacity(saved.entries.len());
        for entry in &saved.entries {
            let messages = decode_qw_bytes(&entry.bytes)?;
            if messages.len() != 1 {
                return Err(GuestError::invalid("invalid saved source message"));
            }
            entries.push(QcRoutedMessage {
                message: messages.into_iter().next().expect("length checked"),
                actor: entry.actor.as_ref().and_then(resolve),
            });
        }
        Ok(entries)
    }
}

/// Saved message destination.
#[derive(Debug, Clone, PartialEq)]
pub enum SavedDestination {
    /// Broadcast.
    Broadcast {
        /// Reliable delivery.
        reliable: bool,
    },
    /// Single client.
    Client {
        /// Recipient.
        actor: SavedActorId,
    },
    /// Signon.
    Signon,
    /// Multicast.
    Multicast {
        /// Multicast origin.
        origin: Vec3,
        /// Visibility scope.
        visibility: VisibilityScope,
        /// Reliable delivery.
        reliable: bool,
    },
}

/// Capture a destination for saves.
#[must_use]
pub fn capture_qc_destination(destination: &QcMessageDestination) -> SavedDestination {
    match destination {
        QcMessageDestination::Broadcast { reliable } => SavedDestination::Broadcast { reliable: *reliable },
        QcMessageDestination::Client { actor } => SavedDestination::Client {
            actor: SavedActorId::from(actor),
        },
        QcMessageDestination::Signon => SavedDestination::Signon,
        QcMessageDestination::Multicast {
            origin,
            visibility,
            reliable,
        } => SavedDestination::Multicast {
            origin: *origin,
            visibility: *visibility,
            reliable: *reliable,
        },
    }
}

/// Restore a destination, resolving the recipient.
pub fn read_qc_destination(
    saved: &SavedDestination,
    resolve: &dyn Fn(&SavedActorId) -> Option<ActorId>,
) -> Result<QcMessageDestination, GuestError> {
    match saved {
        SavedDestination::Broadcast { reliable } => Ok(QcMessageDestination::Broadcast { reliable: *reliable }),
        SavedDestination::Signon => Ok(QcMessageDestination::Signon),
        SavedDestination::Client { actor } => {
            let Some(actor) = resolve(actor) else {
                return Err(GuestError::invalid("missing message recipient"));
            };
            Ok(QcMessageDestination::Client { actor })
        }
        SavedDestination::Multicast {
            origin,
            visibility,
            reliable,
        } => Ok(QcMessageDestination::Multicast {
            origin: *origin,
            visibility: *visibility,
            reliable: *reliable,
        }),
    }
}

/// Capture an optional actor for saves.
#[must_use]
pub fn saved_qc_actor(actor: Option<&ActorId>) -> Option<SavedActorId> {
    actor.map(SavedActorId::from)
}

/// Saved routed buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutedCheckpoint {
    /// Buffer key (`client` is rebuilt from the destination).
    pub key: String,
    /// Buffer bytes.
    pub bytes: Vec<u8>,
    /// Buffer budget.
    pub maxsize: usize,
    /// Overflow behavior.
    pub allowoverflow: bool,
    /// Overflow latch.
    pub overflowed: bool,
    /// Captured owners.
    pub owners: Vec<(i32, Option<SavedActorId>)>,
    /// Buffer destination.
    pub destination: Option<SavedDestination>,
}

/// Saved broadcast-message state.
#[derive(Debug, Clone, PartialEq)]
pub struct QcMessageCheckpoint {
    /// Unrouted broadcast bytes.
    pub buffer: Vec<u8>,
    /// Broadcast overflow latch.
    pub overflowed: bool,
    /// Broadcast owners.
    pub owners: Vec<(i32, Option<SavedActorId>)>,
    /// Signon buffer count.
    pub signon_buffers: u32,
    /// Protocol version.
    pub protocol_version: u32,
    /// Protocol flags.
    pub protocol_flags: u32,
    /// Routed buffers.
    pub routed: Vec<RoutedCheckpoint>,
}

impl<R: QcMessageRouter> QcBroadcastMessages<R> {
    /// Checkpoint all message state.
    #[must_use]
    pub fn capture(&self) -> QcMessageCheckpoint {
        let owners = |values: &HashMap<i32, Option<ActorId>>| {
            let mut saved: Vec<(i32, Option<SavedActorId>)> = values
                .iter()
                .map(|(offset, actor)| (*offset, saved_qc_actor(actor.as_ref())))
                .collect();
            saved.sort_by_key(|(offset, _)| *offset);
            saved
        };
        let mut routed: Vec<RoutedCheckpoint> = self
            .routed
            .iter()
            .map(|(key, entry)| RoutedCheckpoint {
                key: if key.starts_with("client:") {
                    "client".to_string()
                } else {
                    key.clone()
                },
                bytes: entry.buffer.bytes().to_vec(),
                maxsize: entry.buffer.maxsize(),
                allowoverflow: entry.buffer.allowoverflow(),
                overflowed: entry.buffer.overflowed(),
                owners: owners(&entry.owners),
                destination: entry.destination.as_ref().map(capture_qc_destination),
            })
            .collect();
        routed.sort_by(|left, right| left.key.cmp(&right.key));
        QcMessageCheckpoint {
            buffer: self.buffer.bytes().to_vec(),
            overflowed: self.buffer.overflowed(),
            owners: owners(&self.owners),
            signon_buffers: self.signon_buffers,
            protocol_version: self.protocol_version,
            protocol_flags: self.protocol_flags,
            routed,
        }
    }

    /// Restore message state, resolving owners.
    pub fn restore(
        &mut self,
        saved: &QcMessageCheckpoint,
        resolve: &dyn Fn(&SavedActorId) -> Option<ActorId>,
    ) -> Result<(), GuestError> {
        self.buffer.set_bytes(&saved.buffer)?;
        self.buffer.set_overflowed(saved.overflowed);
        self.owners = saved
            .owners
            .iter()
            .map(|(offset, actor)| (*offset, actor.as_ref().and_then(resolve)))
            .collect();
        if saved.signon_buffers < 1 || saved.signon_buffers > MAX_SIGNON_BUFFERS {
            return Err(GuestError::invalid("invalid signon buffer count"));
        }
        self.signon_buffers = saved.signon_buffers;
        self.protocol_version = saved.protocol_version;
        self.protocol_flags = saved.protocol_flags;
        self.routed.clear();
        for entry in &saved.routed {
            if !["client", "broadcast", "all", "signon", "multicast"].contains(&entry.key.as_str()) {
                return Err(GuestError::invalid(format!("unknown routed buffer {}", entry.key)));
            }
            if entry.maxsize < 1 || entry.maxsize > MAX_ROUTED_BYTES.max(MAX_MSGLEN) {
                return Err(GuestError::invalid("invalid routed buffer capacity"));
            }
            let destination = entry
                .destination
                .as_ref()
                .map(|saved| read_qc_destination(saved, resolve))
                .transpose()?;
            let key = match &destination {
                Some(QcMessageDestination::Client { actor }) => {
                    format!("client:{}:{}", actor.slot(), actor.generation())
                }
                _ => entry.key.clone(),
            };
            if self.routed.contains_key(&key) {
                return Err(GuestError::invalid("duplicate routed buffer"));
            }
            let mut buffer = MsgBuffer::new(entry.maxsize, entry.allowoverflow);
            buffer.set_bytes(&entry.bytes)?;
            buffer.set_overflowed(entry.overflowed);
            self.routed.insert(
                key,
                RoutedBuffer {
                    buffer,
                    owners: entry
                        .owners
                        .iter()
                        .map(|(offset, actor)| (*offset, actor.as_ref().and_then(resolve)))
                        .collect(),
                    destination,
                },
            );
        }
        Ok(())
    }
}

/// Presentation services behind the `PF_*` builtins.
pub trait QcPresentationServices {
    /// Precache a resource by name.
    fn precache(&mut self, kind: PrecacheKind, name: &str) -> Result<QcPrecachedResource, GuestError>;
    /// Look up a precached resource.
    fn lookup(&self, kind: PrecacheKind, name: &str) -> Option<QcPrecachedResource>;
    /// Whether the server is in the spawn/loading phase.
    fn loading(&self) -> bool;
    /// Print console text.
    fn print(&mut self, text: &str);
    /// Broadcast print; returns true when a shared sink handled it.
    fn broadcast_print(&mut self, text: &str, level: i32) -> bool;
    /// Whether console prints already reach clients.
    fn print_broadcasts_to_clients(&self) -> bool;
    /// Emit a presentation event.
    fn emit(&mut self, content: &str, event: QcPresentationEvent);
    /// Register a used resource with the shared sink.
    fn register_resource(&mut self, content: &str, path: &str, resource: &QcPrecachedResource);
    /// Deliver a client message; returns false when no sink is installed.
    fn message(&mut self, event: ClientMessage, actor: &ActorId) -> bool;
    /// QEX finale state, when installed.
    fn finale_finished(&self) -> Option<bool>;
}

/// `PF_*` presentation builtins.
pub struct QcPresentationBindings<S, R> {
    services: S,
    router: Option<R>,
    api: ApiKind,
    content: String,
}

impl<S: QcPresentationServices, R: QcMessageRouter> QcPresentationBindings<S, R> {
    /// Bind presentation builtins. QuakeWorld requires routed services.
    pub fn new(services: S, router: Option<R>, api: ApiKind, content: &str) -> Result<Self, GuestError> {
        if api == ApiKind::QuakeWorld {
            match &router {
                Some(router) if router.api() == ApiKind::QuakeWorld => {}
                _ => {
                    return Err(GuestError::invalid(
                        "QuakeWorld presentation requires routed message services",
                    ))
                }
            }
        }
        Ok(Self {
            services,
            router,
            api,
            content: content.to_string(),
        })
    }

    /// Borrow the services (test inspection).
    #[must_use]
    pub fn services(&self) -> &S {
        &self.services
    }

    /// `ex_finaleFinished`: QEX finale state, when installed.
    #[must_use]
    pub fn ex_finale_finished(&self) -> Option<f32> {
        self.services.finale_finished().map(f32::from)
    }

    /// `bprint`: broadcast print. The level applies to QuakeWorld only;
    /// NetQuake callers pass 2.
    pub fn bprint(&mut self, level: i32, text: &str) -> Result<(), GuestError> {
        if self.services.broadcast_print(text, level) {
            return Ok(());
        }
        self.services.print(text);
        if self.services.print_broadcasts_to_clients() {
            return Ok(());
        }
        if self.api == ApiKind::QuakeWorld {
            if let Some(router) = self.router.as_mut() {
                router.route_qw(
                    &[QcRoutedMessage {
                        message: QwMessage::Print {
                            level,
                            text: text.to_string(),
                        },
                        actor: None,
                    }],
                    &QcMessageDestination::Broadcast { reliable: true },
                );
            }
        } else if let Some(router) = self.router.as_mut() {
            router.route_nq(
                &[NqMessage::Print { text: text.to_string() }],
                &QcMessageDestination::Broadcast { reliable: true },
                &[],
            );
        }
        Ok(())
    }

    /// `localcmd`: run a server console command.
    pub fn localcmd(&mut self, text: &str) {
        let content = self.content.clone();
        self.services
            .emit(&content, QcPresentationEvent::ServerCommand { text: text.to_string() });
    }

    /// `makestatic`: freeze an entity into a static model, then remove it.
    pub fn makestatic(
        &mut self,
        fields: &mut FieldTable,
        actor: &ActorId,
        reference: i32,
        remove: &mut dyn FnMut(&mut FieldTable, i32) -> Result<(), GuestError>,
    ) -> Result<(), GuestError> {
        let path = fields.get(actor, "model")?.as_text("model")?.to_string();
        let resource = if path.is_empty() {
            None
        } else {
            self.services.lookup(PrecacheKind::Model, &path)
        };
        if !path.is_empty() && resource.is_none() {
            return Err(GuestError::invalid(format!(
                "makestatic model was not precached: {path}"
            )));
        }
        if let Some(resource) = &resource {
            let content = self.content.clone();
            let path = resource.path.clone();
            self.services.register_resource(&content, &path, resource);
        }
        let event = QcPresentationEvent::StaticModel {
            path,
            frame: fields.get(actor, "frame")?.as_float("frame")? as i32,
            color_map: fields.get(actor, "colormap")?.as_float("colormap")? as i32,
            skin: fields.get(actor, "skin")?.as_float("skin")? as i32,
            origin: fields.get(actor, "origin")?.as_vector("origin")?,
            angles: fields.get(actor, "angles")?.as_vector("angles")?,
        };
        let content = self.content.clone();
        self.services.emit(&content, event);
        remove(fields, reference)
    }

    /// `sprint` / `centerprint` / `stuffcmd`: message one client. The level
    /// applies to QuakeWorld `sprint` only.
    pub fn client_message(
        &mut self,
        kind: ClientPrintKind,
        actor: &ActorId,
        level: i32,
        text: &str,
    ) -> Result<(), GuestError> {
        let name = match kind {
            ClientPrintKind::Sprint => "sprint",
            ClientPrintKind::CenterPrint => "centerprint",
            ClientPrintKind::StuffCmd => "stuffcmd",
        };
        if self.api == ApiKind::QuakeWorld {
            let Some(router) = self.router.as_mut() else {
                return Err(GuestError::invalid(
                    "QuakeWorld presentation requires routed message services",
                ));
            };
            if !router.is_client(actor) {
                self.services.print(&format!("tried to {name} to a non-client\n"));
                return Ok(());
            }
            let message = match kind {
                ClientPrintKind::Sprint => QwMessage::Print {
                    level,
                    text: text.to_string(),
                },
                ClientPrintKind::CenterPrint => QwMessage::CenterPrint { text: text.to_string() },
                ClientPrintKind::StuffCmd => QwMessage::StuffText { text: text.to_string() },
            };
            router.route_qw(
                &[QcRoutedMessage {
                    message,
                    actor: Some(actor.clone()),
                }],
                &QcMessageDestination::Client { actor: actor.clone() },
            );
            return Ok(());
        }
        if let Some(router) = self.router.as_mut() {
            if !router.is_client(actor) {
                return Err(GuestError::invalid("Client message addressed a non-client"));
            }
            let message = match kind {
                ClientPrintKind::Sprint => NqMessage::Print { text: text.to_string() },
                ClientPrintKind::CenterPrint => NqMessage::CenterPrint { text: text.to_string() },
                ClientPrintKind::StuffCmd => NqMessage::StuffText { text: text.to_string() },
            };
            router.route_nq(&[message], &QcMessageDestination::Client { actor: actor.clone() }, &[]);
            return Ok(());
        }
        let event = match kind {
            ClientPrintKind::Sprint => ClientMessage::Print {
                level: 2,
                text: text.to_string(),
            },
            ClientPrintKind::CenterPrint => ClientMessage::CenterPrint { text: text.to_string() },
            ClientPrintKind::StuffCmd => ClientMessage::CommandText { text: text.to_string() },
        };
        if !self.services.message(event, actor) {
            return Err(GuestError::invalid("Missing client message service"));
        }
        Ok(())
    }

    /// `precache_sound` / `precache_model`: precache during spawn; returns
    /// the string reference unchanged.
    pub fn precache(&mut self, kind: PrecacheKind, name: &str, string_ref: i32) -> Result<i32, GuestError> {
        if !self.services.loading() {
            return Err(GuestError::invalid(
                "PF_Precache_*: Precache can only be done in spawn functions",
            ));
        }
        if name.is_empty() || name.as_bytes()[0] <= 32 {
            return Err(GuestError::invalid("Bad string"));
        }
        let resource = self.services.precache(kind, name)?;
        let content = self.content.clone();
        let path = resource.path.clone();
        self.services.register_resource(&content, &path, &resource);
        Ok(string_ref)
    }

    /// `precache_file`: no-op returning the string reference unchanged.
    #[must_use]
    pub const fn precache_file(string_ref: i32) -> i32 {
        string_ref
    }

    /// `sound`: start a positional sound with `SV_StartSound` validation.
    #[allow(clippy::too_many_arguments)]
    pub fn sound(
        &mut self,
        fields: &FieldTable,
        actor: &ActorId,
        slot: usize,
        channel: i32,
        path: &str,
        volume01: f32,
        attenuation: f32,
    ) -> Result<(), GuestError> {
        let volume = (volume01 * 255.0) as i32;
        if !(0..=255).contains(&volume) {
            return Err(GuestError::invalid(format!("SV_StartSound: volume = {volume}")));
        }
        if !attenuation.is_finite() || !(0.0..=4.0).contains(&attenuation) {
            return Err(GuestError::invalid(format!(
                "SV_StartSound: attenuation = {attenuation}"
            )));
        }
        let max_channel = if self.api == ApiKind::QuakeWorld { 15 } else { 7 };
        if !(0..=max_channel).contains(&channel) {
            return Err(GuestError::invalid(format!("SV_StartSound: channel = {channel}")));
        }
        let channel_id = (channel & 7) as u8;
        let source_channel = match channel_id {
            0 => Q1SoundChannel::Auto,
            1 => Q1SoundChannel::Weapon,
            2 => Q1SoundChannel::Voice,
            3 => Q1SoundChannel::Item,
            4 => Q1SoundChannel::Body,
            raw => Q1SoundChannel::Raw(raw),
        };
        let Some(resource) = self.services.lookup(PrecacheKind::Sound, path) else {
            self.services.print(&format!("SV_StartSound: {path} not precacheed\n"));
            return Ok(());
        };
        let content = self.content.clone();
        let registered = resource.path.clone();
        self.services.register_resource(&content, &registered, &resource);
        if self.api == ApiKind::QuakeWorld {
            let origin = fields.get(actor, "origin")?.as_vector("origin")?;
            let position = if fields.get(actor, "solid")?.as_float("solid")? == 4.0 {
                let mins = fields.get(actor, "mins")?.as_vector("mins")?;
                let maxs = fields.get(actor, "maxs")?.as_vector("maxs")?;
                Vec3 {
                    x: origin.x + (mins.x + maxs.x) * 0.5,
                    y: origin.y + (mins.y + maxs.y) * 0.5,
                    z: origin.z + (mins.z + maxs.z) * 0.5,
                }
            } else {
                origin
            };
            let Some(router) = self.router.as_mut() else {
                return Err(GuestError::invalid(
                    "QuakeWorld presentation requires routed message services",
                ));
            };
            let reliable = channel & 8 != 0;
            router.route_qw(
                &[QcRoutedMessage {
                    message: QwMessage::Sound {
                        entity: slot as u16,
                        channel: channel_id,
                        index: resource.index as u8,
                        origin: position,
                        volume: volume as u8,
                        attenuation,
                    },
                    actor: Some(actor.clone()),
                }],
                &QcMessageDestination::Multicast {
                    origin: position,
                    visibility: if reliable || !router.phs() {
                        VisibilityScope::All
                    } else {
                        VisibilityScope::Phs
                    },
                    reliable,
                },
            );
            return Ok(());
        }
        if self.router.as_ref().is_some_and(QcMessageRouter::native) {
            let origin = fields.get(actor, "origin")?.as_vector("origin")?;
            let mins = fields.get(actor, "mins")?.as_vector("mins")?;
            let maxs = fields.get(actor, "maxs")?.as_vector("maxs")?;
            let center = Vec3 {
                x: origin.x + (mins.x + maxs.x) * 0.5,
                y: origin.y + (mins.y + maxs.y) * 0.5,
                z: origin.z + (mins.z + maxs.z) * 0.5,
            };
            if let Some(router) = self.router.as_mut() {
                router.route_nq(
                    &[NqMessage::Sound {
                        entity: slot as u16,
                        channel: channel_id,
                        index: resource.index as u8,
                        origin: center,
                        volume: volume as u8,
                        attenuation,
                    }],
                    &QcMessageDestination::Broadcast { reliable: false },
                    &[],
                );
            }
        }
        let content = self.content.clone();
        self.services.emit(
            &content,
            QcPresentationEvent::Sound {
                actor: actor.clone(),
                path: path.to_string(),
                channel: source_channel,
                volume: f32::from(volume as u8) / 255.0,
                attenuation,
            },
        );
        Ok(())
    }

    /// `ambientsound`: register an ambient sound emitter.
    pub fn ambientsound(&mut self, origin: Vec3, path: &str, volume: f32, attenuation: f32) {
        let Some(resource) = self.services.lookup(PrecacheKind::Sound, path) else {
            self.services.print(&format!("no precache: {path}\n"));
            return;
        };
        let content = self.content.clone();
        let registered = resource.path.clone();
        self.services.register_resource(&content, &registered, &resource);
        self.services.emit(
            &content,
            QcPresentationEvent::Ambient {
                origin,
                path: path.to_string(),
                volume,
                attenuation,
            },
        );
    }

    /// `particle`: emit a particle burst.
    pub fn particle(&mut self, origin: Vec3, direction: Vec3, color: i32, count: i32) {
        let content = self.content.clone();
        self.services.emit(
            &content,
            QcPresentationEvent::Particles {
                origin,
                direction,
                color,
                count,
            },
        );
    }

    /// `lightstyle`: set a light style pattern.
    pub fn lightstyle(&mut self, style: i32, pattern: &str) -> Result<(), GuestError> {
        if !(0..LIGHT_STYLE_COUNT).contains(&style) {
            return Err(GuestError::invalid("lightstyle outside source style table"));
        }
        let content = self.content.clone();
        self.services.emit(
            &content,
            QcPresentationEvent::Lightstyle {
                style,
                pattern: pattern.to_string(),
            },
        );
        Ok(())
    }
}

/// QEX finale polling ignores held attack on entry and latches a
/// subsequent press.
#[derive(Debug, Default)]
pub struct QcFinaleAcknowledgement {
    held: HashMap<ActorId, bool>,
    last_poll: Option<f32>,
    acknowledged: bool,
}

impl QcFinaleAcknowledgement {
    /// Fresh latch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the latch.
    pub fn reset(&mut self) {
        self.last_poll = None;
        self.acknowledged = false;
        self.held.clear();
    }

    /// Poll button state at `seconds`; returns whether acknowledged.
    pub fn poll(&mut self, seconds: f32, buttons: &[(ActorId, bool)]) -> bool {
        let restart = self.last_poll.is_none_or(|last| seconds < last || seconds - last > 1.0);
        if restart {
            self.acknowledged = false;
            self.held.clear();
            for (actor, down) in buttons {
                self.held.insert(actor.clone(), *down);
            }
        }
        self.last_poll = Some(seconds);
        self.held.retain(|actor, _| buttons.iter().any(|(id, _)| id == actor));
        for (actor, down) in buttons {
            if *down && self.held.get(actor) != Some(&true) {
                self.acknowledged = true;
            }
            self.held.insert(actor.clone(), *down);
        }
        self.acknowledged
    }

    /// Dismiss the finale immediately.
    pub fn dismiss(&mut self, seconds: f32) {
        self.last_poll = Some(seconds);
        self.acknowledged = true;
    }
}

/// Bounds helper re-exported for presenter tests.
#[must_use]
pub fn presentation_bounds(min: Vec3, max: Vec3) -> Bounds {
    Bounds { min, max }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::{FieldLayout, FieldValue};
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    type NqRoute = (Vec<NqMessage>, QcMessageDestination, Vec<(usize, Option<ActorId>)>);

    struct FakeRouter {
        api: ApiKind,
        clients: Vec<ActorId>,
        loading: bool,
        native: bool,
        local: bool,
        phs: bool,
        nq: Vec<NqRoute>,
        qw: Vec<(Vec<QcRoutedMessage>, QcMessageDestination)>,
    }

    impl QcMessageRouter for FakeRouter {
        fn api(&self) -> ApiKind {
            self.api
        }

        fn is_client(&self, actor: &ActorId) -> bool {
            self.clients.contains(actor)
        }

        fn loading(&self) -> bool {
            self.loading
        }

        fn native(&self) -> bool {
            self.native
        }

        fn local(&self) -> bool {
            self.local
        }

        fn phs(&self) -> bool {
            self.phs
        }

        fn route_nq(
            &mut self,
            messages: &[NqMessage],
            destination: &QcMessageDestination,
            view_targets: &[(usize, Option<ActorId>)],
        ) {
            self.nq
                .push((messages.to_vec(), destination.clone(), view_targets.to_vec()));
        }

        fn route_qw(&mut self, entries: &[QcRoutedMessage], destination: &QcMessageDestination) {
            self.qw.push((entries.to_vec(), destination.clone()));
        }
    }

    fn nq_router() -> FakeRouter {
        FakeRouter {
            api: ApiKind::NetQuake,
            clients: Vec::new(),
            loading: false,
            native: false,
            local: false,
            phs: false,
            nq: Vec::new(),
            qw: Vec::new(),
        }
    }

    struct FakeServices {
        loading: bool,
        precached: HashMap<(PrecacheKind, String), QcPrecachedResource>,
        prints: Vec<String>,
        events: Vec<QcPresentationEvent>,
        resources: Vec<String>,
        messages: Vec<(ClientMessage, ActorId)>,
        message_sink: bool,
        broadcast_sink: bool,
        broadcast_to_clients: bool,
        finale: Option<bool>,
    }

    impl QcPresentationServices for FakeServices {
        fn precache(&mut self, kind: PrecacheKind, name: &str) -> Result<QcPrecachedResource, GuestError> {
            let resource = QcPrecachedResource {
                index: self.precached.len() as i32 + 1,
                path: name.to_string(),
            };
            self.precached.insert((kind, name.to_string()), resource.clone());
            Ok(resource)
        }

        fn lookup(&self, kind: PrecacheKind, name: &str) -> Option<QcPrecachedResource> {
            self.precached.get(&(kind, name.to_string())).cloned()
        }

        fn loading(&self) -> bool {
            self.loading
        }

        fn print(&mut self, text: &str) {
            self.prints.push(text.to_string());
        }

        fn broadcast_print(&mut self, text: &str, level: i32) -> bool {
            if self.broadcast_sink {
                self.prints.push(format!("[{level}]{text}"));
                true
            } else {
                false
            }
        }

        fn print_broadcasts_to_clients(&self) -> bool {
            self.broadcast_to_clients
        }

        fn emit(&mut self, _content: &str, event: QcPresentationEvent) {
            self.events.push(event);
        }

        fn register_resource(&mut self, _content: &str, path: &str, _resource: &QcPrecachedResource) {
            self.resources.push(path.to_string());
        }

        fn message(&mut self, event: ClientMessage, actor: &ActorId) -> bool {
            if self.message_sink {
                self.messages.push((event, actor.clone()));
                true
            } else {
                false
            }
        }

        fn finale_finished(&self) -> Option<bool> {
            self.finale
        }
    }

    fn services() -> FakeServices {
        FakeServices {
            loading: true,
            precached: HashMap::new(),
            prints: Vec::new(),
            events: Vec::new(),
            resources: Vec::new(),
            messages: Vec::new(),
            message_sink: true,
            broadcast_sink: false,
            broadcast_to_clients: false,
            finale: Some(true),
        }
    }

    fn layout() -> FieldLayout {
        FieldLayout::qc_entity().field("mins", "vector").field("maxs", "vector")
    }

    #[test]
    fn msg_buffer_codecs_round_trip() {
        let mut buffer = MsgBuffer::new(64, false);
        buffer.write_byte(0x1ff).unwrap();
        buffer.write_char(-1).unwrap();
        buffer.write_short(0x1234).unwrap();
        buffer.write_long(-7).unwrap();
        buffer.write_coord(1.5).unwrap();
        buffer.write_angle(90.0).unwrap();
        buffer.write_string("hi").unwrap();
        assert_eq!(buffer.bytes()[0], 0xff);
        assert_eq!(buffer.bytes()[1], 0xff);
        assert_eq!(buffer.bytes()[4], 0xf9);
        assert!(!buffer.is_empty());
        let mut reader = MsgReader::new(buffer.bytes());
        assert_eq!(reader.read_u8().unwrap(), 0xff);
        assert_eq!(reader.read_u8().unwrap(), 0xff);
        assert_eq!(reader.read_u16().unwrap(), 0x1234);
        assert_eq!(reader.read_i32().unwrap(), -7);
        assert_eq!(reader.read_f32().unwrap(), 1.5);
        assert_eq!(reader.read_u8().unwrap(), 64);
        assert_eq!(reader.read_string().unwrap(), "hi");
    }

    #[test]
    fn msg_buffer_overflow_modes() {
        let mut strict = MsgBuffer::new(2, false);
        strict.write_short(1).unwrap();
        assert!(strict.write_byte(1).is_err());
        let mut lenient = MsgBuffer::new(2, true);
        lenient.write_short(1).unwrap();
        lenient.write_byte(9).unwrap();
        assert!(lenient.overflowed());
        assert_eq!(lenient.bytes(), &[9]);
        assert!(lenient.set_bytes(&[1, 2, 3]).is_err());
    }

    #[test]
    fn nq_codec_round_trip() {
        let messages = vec![
            NqMessage::Print {
                text: "hello".to_string(),
            },
            NqMessage::TempEntity {
                effect: TempEntityEffect::Point {
                    effect_type: 3,
                    origin: vec3(1.0, 2.0, 3.0),
                    count: 9,
                },
            },
            NqMessage::SetView { entity: 5 },
            NqMessage::Sound {
                entity: 2,
                channel: 1,
                index: 3,
                origin: vec3(0.0, 0.0, 0.0),
                volume: 200,
                attenuation: 1.0,
            },
        ];
        let bytes = capture_netquake_messages(&messages).unwrap();
        assert_eq!(restore_netquake_messages(&bytes).unwrap(), messages);
        assert!(restore_netquake_messages(&[99]).is_err());
        assert!(restore_netquake_messages(&[TAG_PRINT]).is_err());
    }

    #[test]
    fn qw_codec_round_trip() {
        let messages = vec![
            QwMessage::Print {
                level: 2,
                text: "hi".to_string(),
            },
            QwMessage::MuzzleFlash { entity: 4 },
            QwMessage::Intermission {
                origin: vec3(1.0, 1.0, 1.0),
                angles: vec3(0.0, 90.0, 0.0),
            },
            QwMessage::Pause { paused: true },
            QwMessage::KilledMonster,
        ];
        let mut buffer = MsgBuffer::new(MAX_MSGLEN, false);
        for message in &messages {
            encode_qw(&mut buffer, message).unwrap();
        }
        assert_eq!(decode_qw_bytes(buffer.bytes()).unwrap(), messages);
    }

    #[test]
    fn unrouted_broadcast_emits_temp_entities_only() {
        let mut host = QcBroadcastMessages::<FakeRouter>::new(None, false, 15, 0).unwrap();
        assert!(!host.is_quakeworld());
        let slots = |_: usize| None;
        host.write_byte(0, None, &slots, i32::from(TAG_TEMP_POINT)).unwrap();
        host.write_byte(0, None, &slots, 3).unwrap();
        host.write_coord(0, None, &slots, 1.0).unwrap();
        host.write_coord(0, None, &slots, 2.0).unwrap();
        host.write_coord(0, None, &slots, 3.0).unwrap();
        host.write_byte(0, None, &slots, 9).unwrap();
        assert!(!host.bytes().is_empty());
        let mut effects = Vec::new();
        host.flush(&mut |effect, _| effects.push(effect.clone())).unwrap();
        assert_eq!(effects.len(), 1);
        assert!(host.bytes().is_empty());
        assert!(host.write_byte(1, None, &slots, 1).is_err());
    }

    #[test]
    fn unrouted_non_temp_messages_are_rejected() {
        let mut host = QcBroadcastMessages::<FakeRouter>::new(None, false, 15, 0).unwrap();
        let slots = |_: usize| None;
        host.write_byte(0, None, &slots, i32::from(TAG_PRINT)).unwrap();
        host.write_string(0, None, &slots, "nope").unwrap();
        assert!(host.flush(&mut |_, _| {}).is_err());
    }

    #[test]
    fn routed_nq_buffers_route_and_capture_view_targets() {
        let owner = IdentityOwner::create("presentation").unwrap();
        let client = owner.actor(1, 1);
        let mut router = nq_router();
        router.clients.push(client.clone());
        let mut host = QcBroadcastMessages::new(Some(router), false, 15, 0).unwrap();
        let slots = |slot: usize| (slot == 2).then(|| owner.actor(2, 1));
        host.write_byte(2, None, &slots, i32::from(TAG_SET_VIEW)).unwrap();
        host.write_short(2, None, &slots, 2).unwrap();
        assert_eq!(host.routed_count(), 1);
        host.flush(&mut |_, _| {}).unwrap();
        assert_eq!(host.routed_count(), 0);
    }

    #[test]
    fn qw_multicast_modes_route_with_visibility() {
        let owner = IdentityOwner::create("presentation").unwrap();
        let router = FakeRouter {
            api: ApiKind::QuakeWorld,
            clients: vec![owner.actor(1, 1)],
            loading: true,
            native: false,
            local: false,
            phs: true,
            nq: Vec::new(),
            qw: Vec::new(),
        };
        let mut host = QcBroadcastMessages::new(Some(router), true, 28, 0).unwrap();
        assert!(host.is_quakeworld());
        let slots = |_: usize| None;
        host.write_byte(4, None, &slots, i32::from(TAG_NOP)).unwrap();
        host.multicast(vec3(0.0, 0.0, 0.0), 4).unwrap();
        assert!(host.multicast(vec3(0.0, 0.0, 0.0), 9).is_err());
    }

    #[test]
    fn signon_flush_reserves_space() {
        let router = FakeRouter {
            api: ApiKind::QuakeWorld,
            clients: Vec::new(),
            loading: true,
            native: false,
            local: false,
            phs: false,
            nq: Vec::new(),
            qw: Vec::new(),
        };
        let mut host = QcBroadcastMessages::new(Some(router), true, 28, 0).unwrap();
        let slots = |_: usize| None;
        // INIT writes are rejected outside spawn functions.
        let cold = FakeRouter {
            api: ApiKind::QuakeWorld,
            clients: Vec::new(),
            loading: false,
            native: false,
            local: false,
            phs: false,
            nq: Vec::new(),
            qw: Vec::new(),
        };
        let mut frozen = QcBroadcastMessages::new(Some(cold), true, 28, 0).unwrap();
        assert!(frozen.write_byte(3, None, &slots, 1).is_err());
        for _ in 0..(MAX_DATAGRAM - 512) {
            host.write_byte(3, None, &slots, i32::from(TAG_NOP)).unwrap();
        }
        assert_eq!(host.signon_buffers(), 1);
        assert_eq!(host.routed_count(), 1);
        host.flush_signon().unwrap();
        assert_eq!(host.signon_buffers(), 2);
        assert_eq!(host.routed_count(), 0);
    }

    #[test]
    fn message_checkpoint_round_trip() {
        let owner = IdentityOwner::create("presentation").unwrap();
        let client = owner.actor(1, 1);
        let mut router = nq_router();
        router.clients.push(client.clone());
        let mut host = QcBroadcastMessages::new(Some(router), false, 15, 3).unwrap();
        let slots = |_: usize| None;
        host.write_byte(2, None, &slots, i32::from(TAG_CENTER_PRINT)).unwrap();
        host.write_string(2, None, &slots, "go").unwrap();
        host.write_byte(1, Some(&client), &slots, i32::from(TAG_NOP)).unwrap();
        let saved = host.capture();
        assert_eq!(saved.protocol_version, 15);
        let mut router = nq_router();
        router.clients.push(client.clone());
        let mut restored = QcBroadcastMessages::new(Some(router), false, 15, 3).unwrap();
        let live = client.clone();
        restored
            .restore(&saved, &|saved| {
                (*saved == SavedActorId::from(&live)).then(|| live.clone())
            })
            .unwrap();
        assert_eq!(restored.routed_count(), 2);
        let live = client.clone();
        let entries = restored
            .capture_entries(&[QcRoutedMessage {
                message: QwMessage::Nop,
                actor: Some(client.clone()),
            }])
            .unwrap();
        let back = restored
            .restore_entries(&entries, &|saved| {
                (*saved == SavedActorId::from(&live)).then(|| live.clone())
            })
            .unwrap();
        assert_eq!(back[0].actor, Some(client));
        let bad = QwEntriesCheckpoint {
            version: 28,
            flags: 0,
            entries: vec![QwEntryCheckpoint {
                bytes: vec![],
                actor: None,
            }],
        };
        assert!(restored.restore_entries(&bad, &|_| None).is_err());
    }

    #[test]
    fn destination_capture_round_trip() {
        let owner = IdentityOwner::create("presentation").unwrap();
        let actor = owner.actor(1, 1);
        let dest = QcMessageDestination::Multicast {
            origin: vec3(1.0, 2.0, 3.0),
            visibility: VisibilityScope::Pvs,
            reliable: true,
        };
        let back = read_qc_destination(&capture_qc_destination(&dest), &|_| None).unwrap();
        assert_eq!(back, dest);
        let client = QcMessageDestination::Client { actor: actor.clone() };
        let live = actor.clone();
        let back = read_qc_destination(&capture_qc_destination(&client), &|saved| {
            (*saved == SavedActorId::from(&live)).then(|| live.clone())
        })
        .unwrap();
        assert_eq!(back, client);
        assert!(read_qc_destination(&capture_qc_destination(&client), &|_| None).is_err());
        assert_eq!(saved_qc_actor(None), None);
    }

    #[test]
    fn bprint_and_client_messages_route() {
        let owner = IdentityOwner::create("presentation").unwrap();
        let client = owner.actor(1, 1);
        let mut router = nq_router();
        router.clients.push(client.clone());
        let mut bindings = QcPresentationBindings::new(services(), Some(router), ApiKind::NetQuake, "q1").unwrap();
        bindings.bprint(2, "hello\n").unwrap();
        bindings
            .client_message(ClientPrintKind::Sprint, &client, 2, "hi")
            .unwrap();
        bindings
            .client_message(ClientPrintKind::StuffCmd, &client, 0, "+attack\n")
            .unwrap();
        assert!(bindings.services().prints.iter().any(|text| text == "hello\n"));
        assert!(bindings
            .client_message(ClientPrintKind::Sprint, &owner.actor(9, 9), 2, "x")
            .is_err());
        assert_eq!(bindings.ex_finale_finished(), Some(1.0));
    }

    #[test]
    fn qw_client_messages_reject_non_clients_with_print() {
        let owner = IdentityOwner::create("presentation").unwrap();
        let router = FakeRouter {
            api: ApiKind::QuakeWorld,
            clients: Vec::new(),
            loading: false,
            native: false,
            local: false,
            phs: false,
            nq: Vec::new(),
            qw: Vec::new(),
        };
        let mut bindings = QcPresentationBindings::new(services(), Some(router), ApiKind::QuakeWorld, "q1").unwrap();
        bindings
            .client_message(ClientPrintKind::Sprint, &owner.actor(1, 1), 2, "hi")
            .unwrap();
        assert!(bindings
            .services()
            .prints
            .iter()
            .any(|text| text.contains("non-client")));
        assert!(QcPresentationBindings::new(services(), None::<FakeRouter>, ApiKind::QuakeWorld, "q1").is_err());
    }

    #[test]
    fn precache_sound_model_and_file() {
        let mut bindings =
            QcPresentationBindings::new(services(), None::<FakeRouter>, ApiKind::NetQuake, "q1").unwrap();
        assert_eq!(
            bindings
                .precache(PrecacheKind::Sound, "weapons/shotgun.wav", 11)
                .unwrap(),
            11
        );
        assert_eq!(
            bindings.precache(PrecacheKind::Model, "progs/player.mdl", 12).unwrap(),
            12
        );
        assert_eq!(
            QcPresentationBindings::<FakeServices, FakeRouter>::precache_file(13),
            13
        );
        assert!(bindings.precache(PrecacheKind::Sound, "", 1).is_err());
        bindings.services.loading = false;
        assert!(bindings.precache(PrecacheKind::Sound, "x.wav", 1).is_err());
    }

    #[test]
    fn sound_validates_and_routes() {
        let owner = IdentityOwner::create("presentation").unwrap();
        let actor = owner.actor(1, 1);
        let mut fields = FieldTable::new();
        fields.allocate(&actor, &layout()).unwrap();
        let mut router = nq_router();
        router.native = true;
        let mut bindings = QcPresentationBindings::new(services(), Some(router), ApiKind::NetQuake, "q1").unwrap();
        bindings.precache(PrecacheKind::Sound, "weapons/ax1.wav", 1).unwrap();
        bindings
            .sound(&fields, &actor, 1, 1, "weapons/ax1.wav", 1.0, 1.0)
            .unwrap();
        assert!(bindings
            .services()
            .events
            .iter()
            .any(|event| matches!(event, QcPresentationEvent::Sound { .. })));
        assert!(bindings
            .sound(&fields, &actor, 1, 1, "weapons/ax1.wav", 2.0, 1.0)
            .is_err());
        assert!(bindings
            .sound(&fields, &actor, 1, 1, "weapons/ax1.wav", 1.0, 9.0)
            .is_err());
        assert!(bindings
            .sound(&fields, &actor, 1, 9, "weapons/ax1.wav", 1.0, 1.0)
            .is_err());
        // Unprecached sounds print instead of failing.
        bindings.sound(&fields, &actor, 1, 1, "missing.wav", 1.0, 1.0).unwrap();
        assert!(bindings
            .services()
            .prints
            .iter()
            .any(|text| text.contains("not precacheed")));
    }

    #[test]
    fn qw_sound_multicasts_from_brush_center() {
        let owner = IdentityOwner::create("presentation").unwrap();
        let actor = owner.actor(1, 1);
        let mut fields = FieldTable::new();
        fields.allocate(&actor, &layout()).unwrap();
        fields.set(&actor, "solid", FieldValue::Float(4.0)).unwrap();
        fields
            .set(&actor, "origin", FieldValue::Vector(vec3(0.0, 0.0, 0.0)))
            .unwrap();
        fields
            .set(&actor, "mins", FieldValue::Vector(vec3(-8.0, -8.0, -8.0)))
            .unwrap();
        fields
            .set(&actor, "maxs", FieldValue::Vector(vec3(8.0, 8.0, 8.0)))
            .unwrap();
        let router = FakeRouter {
            api: ApiKind::QuakeWorld,
            clients: vec![actor.clone()],
            loading: false,
            native: false,
            local: false,
            phs: true,
            nq: Vec::new(),
            qw: Vec::new(),
        };
        let mut bindings = QcPresentationBindings::new(services(), Some(router), ApiKind::QuakeWorld, "q1").unwrap();
        bindings.precache(PrecacheKind::Sound, "doors/dr1.wav", 1).unwrap();
        bindings
            .sound(&fields, &actor, 3, 8, "doors/dr1.wav", 0.5, 2.0)
            .unwrap();
    }

    #[test]
    fn makestatic_ambient_particle_lightstyle_and_localcmd() {
        let owner = IdentityOwner::create("presentation").unwrap();
        let actor = owner.actor(1, 1);
        let mut fields = FieldTable::new();
        fields.allocate(&actor, &layout()).unwrap();
        fields
            .set(&actor, "model", FieldValue::Text("progs/player.mdl".to_string()))
            .unwrap();
        let mut bindings =
            QcPresentationBindings::new(services(), None::<FakeRouter>, ApiKind::NetQuake, "q1").unwrap();
        bindings.precache(PrecacheKind::Model, "progs/player.mdl", 1).unwrap();
        let mut removed = Vec::new();
        bindings
            .makestatic(&mut fields, &actor, 1, &mut |_, reference| {
                removed.push(reference);
                Ok(())
            })
            .unwrap();
        assert_eq!(removed, vec![1]);
        bindings.ambientsound(vec3(0.0, 0.0, 0.0), "missing.wav", 1.0, 1.0);
        bindings.precache(PrecacheKind::Sound, "amb/water.wav", 2).unwrap();
        bindings.ambientsound(vec3(1.0, 2.0, 3.0), "amb/water.wav", 0.5, 2.0);
        bindings.particle(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), 73, 20);
        bindings.lightstyle(3, "mmmaaa").unwrap();
        assert!(bindings.lightstyle(64, "x").is_err());
        bindings.localcmd("god\n");
        assert!(bindings.services().events.len() >= 5);
        assert_eq!(
            presentation_bounds(vec3(0.0, 0.0, 0.0), vec3(1.0, 1.0, 1.0)).max,
            vec3(1.0, 1.0, 1.0)
        );
    }

    #[test]
    fn finale_latch_ignores_held_attack() {
        let owner = IdentityOwner::create("presentation").unwrap();
        let player = owner.actor(1, 1);
        let mut latch = QcFinaleAcknowledgement::new();
        assert!(!latch.poll(0.0, &[(player.clone(), true)]));
        assert!(!latch.poll(0.5, &[(player.clone(), true)]));
        assert!(!latch.poll(0.6, &[(player.clone(), false)]));
        assert!(latch.poll(0.7, &[(player.clone(), true)]));
        latch.reset();
        assert!(!latch.poll(5.0, &[(player.clone(), false)]));
        latch.dismiss(6.0);
        assert!(latch.poll(6.1, &[(player, false)]));
    }
}
